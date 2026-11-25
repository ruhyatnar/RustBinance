//! Local loopback Axum HTTP + WebSocket IPC gateway (§10.1, §10.3).
//!
//! Security invariants:
//! - Every endpoint requires authentication (no unauthenticated routes).
//! - Tokens are NEVER accepted via query string (`?token=` / `?access_token=` explicitly rejected).
//! - Strict `Origin` header check (CORS default closed).
//! - Strict `Content-Security-Policy` header on every HTTP response.
//! - Per-caller rate limiting.
//! - Read-only tokens may access `GET` and WS telemetry streams only; mutating commands require full token.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bot_types::Venue;
use futures_util::SinkExt as _;
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{AuthScope, OriginPolicy, RateLimiter, TokenAuth};
use crate::command::{CommandDispatcher, IpcBackend};
use crate::conflate::TelemetryHub;
use crate::error::IpcError;
use crate::nonce::unix_now_ms;
use crate::protocol::{
    ClientMessage, CommandKind, CommandPayload, CommandResultPayload, ConfirmAction,
    HttpErrorResponse, KlinesPayload, OnOpenPositionsArg, ServerMessage, TradesPayload,
    WireEnvelope,
};

/// Strict Content-Security-Policy header applied to all HTTP responses (§10.1).
const STRICT_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self' ws://127.0.0.1:* ws://localhost:*; frame-ancestors 'none'";

/// Timeout for first-message WebSocket authentication when no header/subprotocol token was supplied.
const WS_FIRST_MESSAGE_AUTH_TIMEOUT: Duration = Duration::from_secs(5);

/// Configuration for the local HTTP/WS server.
#[derive(Clone)]
pub struct HttpServerConfig {
    /// Bearer token authenticator (full + optional read-only).
    pub auth: TokenAuth,
    /// Strict `Origin` allowlist.
    pub origin_policy: OriginPolicy,
    /// Per-caller token-bucket rate limiter.
    pub rate_limiter: Arc<RateLimiter>,
}

struct AppState<B: IpcBackend> {
    dispatcher: Arc<CommandDispatcher<B>>,
    hub: Arc<TelemetryHub>,
    config: HttpServerConfig,
}

/// Build the Axum [`Router`] for all §10.3 endpoints.
pub fn build_router<B: IpcBackend>(
    dispatcher: Arc<CommandDispatcher<B>>,
    hub: Arc<TelemetryHub>,
    config: HttpServerConfig,
) -> Router {
    let state = Arc::new(AppState {
        dispatcher,
        hub,
        config,
    });

    Router::new()
        .route("/api/v1/health", get(handle_health::<B>))
        .route("/api/v1/state", get(handle_state::<B>))
        .route("/api/v1/trades", get(handle_trades::<B>))
        .route("/api/v1/klines", get(handle_klines::<B>))
        .route("/metrics", get(handle_metrics::<B>))
        .route("/ws/v1/stream", get(handle_ws_upgrade::<B>))
        .route("/api/v1/commands/pause", post(handle_cmd_pause::<B>))
        .route("/api/v1/commands/resume", post(handle_cmd_resume::<B>))
        .route(
            "/api/v1/commands/switch-venue",
            post(handle_cmd_switch_venue::<B>),
        )
        .route(
            "/api/v1/commands/request-confirm",
            post(handle_cmd_request_confirm::<B>),
        )
        .route(
            "/api/v1/commands/panic-flatten",
            post(handle_cmd_panic_flatten::<B>),
        )
        .route(
            "/api/v1/commands/reset-kill",
            post(handle_cmd_reset_kill::<B>),
        )
        .route(
            "/api/v1/commands/close-position",
            post(handle_cmd_close_position::<B>),
        )
        .route(
            "/api/v1/commands/set-param",
            post(handle_cmd_set_param::<B>),
        )
        .route(
            "/api/v1/commands/disable-strategy",
            post(handle_cmd_disable_strategy::<B>),
        )
        .route(
            "/api/v1/commands/ack-alert",
            post(handle_cmd_ack_alert::<B>),
        )
        .with_state(state)
}

fn with_security_headers(mut resp: Response) -> Response {
    resp.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_static(STRICT_CSP),
    );
    resp.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    resp
}

fn error_response(err: &IpcError) -> Response {
    let status = match err {
        IpcError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
        IpcError::Forbidden(_) => StatusCode::FORBIDDEN,
        IpcError::BadRequest(_)
        | IpcError::InvalidConfirmNonce(_)
        | IpcError::LoosenRejected(_)
        | IpcError::Json(_) => StatusCode::BAD_REQUEST,
        IpcError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        IpcError::NotSupported(_) => StatusCode::NOT_IMPLEMENTED,
        IpcError::NotFound(_) => StatusCode::NOT_FOUND,
        IpcError::Audit(_) | IpcError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    with_security_headers(
        (
            status,
            Json(HttpErrorResponse::new(err.code(), err.to_string())),
        )
            .into_response(),
    )
}

fn reject_query_string_token(uri: &Uri) -> Result<(), IpcError> {
    if let Some(query) = uri.query() {
        for pair in query.split('&') {
            let key = pair.split('=').next().unwrap_or("");
            if key.eq_ignore_ascii_case("token")
                || key.eq_ignore_ascii_case("access_token")
                || key.eq_ignore_ascii_case("bearer")
            {
                return Err(IpcError::BadRequest(
                    "tokens must not be passed in URL query strings".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn extract_bearer_from_headers(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get("authorization")?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
}

fn extract_ws_subprotocol_token(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get("sec-websocket-protocol")?.to_str().ok()?;
    for item in raw.split(',').map(str::trim) {
        if let Some(tok) = item.strip_prefix("bearer.") {
            return Some(tok);
        }
    }
    None
}

fn check_http_request<B: IpcBackend>(
    state: &AppState<B>,
    uri: &Uri,
    headers: &HeaderMap,
    require_mutate: bool,
) -> Result<(String, AuthScope), IpcError> {
    reject_query_string_token(uri)?;

    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    state.config.origin_policy.validate(origin)?;

    let token = extract_bearer_from_headers(headers)
        .ok_or_else(|| IpcError::Unauthorized("missing Authorization: Bearer token".to_owned()))?;
    let scope = state.config.auth.authenticate(token)?;

    let rate_key = format!("http:{}", scope.as_str());
    state.config.rate_limiter.check(&rate_key)?;

    if require_mutate && !scope.can_mutate() {
        return Err(IpcError::Forbidden(
            "read-only token cannot execute mutating commands".to_owned(),
        ));
    }

    Ok((rate_key, scope))
}

async fn handle_health<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let (_principal, scope) = match check_http_request(&state, &uri, &headers, false) {
        Ok(auth) => auth,
        Err(err) => return error_response(&err),
    };
    let mode = state.dispatcher.backend().engine_mode().await;
    with_security_headers(
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "engine_mode": mode,
                "auth_scope": scope.as_str(),
                "ts_ms": unix_now_ms()
            })),
        )
            .into_response(),
    )
}

async fn handle_state<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if let Err(err) = check_http_request(&state, &uri, &headers, false) {
        return error_response(&err);
    }
    let snap = state.dispatcher.backend().snapshot().await;
    with_security_headers((StatusCode::OK, Json(snap)).into_response())
}

#[derive(Deserialize)]
struct TradesQuery {
    venue: Option<Venue>,
    from: Option<i64>,
    to: Option<i64>,
}

async fn handle_trades<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Query(q): Query<TradesQuery>,
) -> Response {
    if let Err(err) = check_http_request(&state, &uri, &headers, false) {
        return error_response(&err);
    }
    let trades = state
        .dispatcher
        .backend()
        .list_trades(q.venue, q.from, q.to)
        .await;
    with_security_headers(
        (
            StatusCode::OK,
            Json(serde_json::json!({ "trades": trades })),
        )
            .into_response(),
    )
}

#[derive(Deserialize)]
struct KlinesQuery {
    #[serde(default = "default_venue")]
    venue: Venue,
    #[serde(default)]
    symbol: String,
    #[serde(default = "default_tf")]
    tf: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

const fn default_venue() -> Venue {
    Venue::Spot
}

fn default_tf() -> String {
    "15m".to_owned()
}

const fn default_limit() -> usize {
    200
}

async fn handle_klines<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Query(q): Query<KlinesQuery>,
) -> Response {
    if let Err(err) = check_http_request(&state, &uri, &headers, false) {
        return error_response(&err);
    }
    let clamped = q.limit.clamp(1, 1000);
    let klines = state
        .dispatcher
        .backend()
        .list_klines(q.venue, &q.symbol, &q.tf, clamped)
        .await;
    with_security_headers(
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "venue": q.venue,
                "symbol": q.symbol,
                "tf": q.tf,
                "limit": clamped,
                "klines": klines
            })),
        )
            .into_response(),
    )
}

async fn handle_metrics<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if let Err(err) = check_http_request(&state, &uri, &headers, false) {
        return error_response(&err);
    }
    let body = state.dispatcher.backend().prometheus_metrics().await;
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(
        "content-type",
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    with_security_headers(resp)
}

#[derive(Deserialize)]
struct SimpleCmdBody {
    #[serde(default = "Uuid::now_v7")]
    cmd_id: Uuid,
}

#[derive(Deserialize)]
struct SwitchVenueBody {
    #[serde(default = "Uuid::now_v7")]
    cmd_id: Uuid,
    target: Venue,
    on_open_positions: OnOpenPositionsArg,
    #[serde(default)]
    confirm_nonce: String,
}

#[derive(Deserialize)]
struct RequestConfirmBody {
    #[serde(default = "Uuid::now_v7")]
    cmd_id: Uuid,
    action: ConfirmAction,
}

#[derive(Deserialize)]
struct ConfirmNonceBody {
    #[serde(default = "Uuid::now_v7")]
    cmd_id: Uuid,
    confirm_nonce: String,
}

#[derive(Deserialize)]
struct ClosePositionBody {
    #[serde(default = "Uuid::now_v7")]
    cmd_id: Uuid,
    position_id: Uuid,
}

#[derive(Deserialize)]
struct SetParamBody {
    #[serde(default = "Uuid::now_v7")]
    cmd_id: Uuid,
    key: String,
    value: String,
}

#[derive(Deserialize)]
struct IdBody {
    #[serde(default = "Uuid::now_v7")]
    cmd_id: Uuid,
    id: String,
}

async fn dispatch_http_command<B: IpcBackend>(
    state: &AppState<B>,
    uri: &Uri,
    headers: &HeaderMap,
    cmd_id: Uuid,
    cmd: CommandKind,
) -> Response {
    let (principal, scope) = match check_http_request(state, uri, headers, true) {
        Ok(auth) => auth,
        Err(err) => return error_response(&err),
    };
    let result = state
        .dispatcher
        .dispatch(&principal, scope, CommandPayload { cmd_id, cmd })
        .await;
    command_result_to_http(result)
}

fn command_result_to_http(res: CommandResultPayload) -> Response {
    let status = if res.ok {
        StatusCode::OK
    } else {
        match res.code.as_str() {
            "forbidden" => StatusCode::FORBIDDEN,
            "not_supported" => StatusCode::NOT_IMPLEMENTED,
            "not_found" => StatusCode::NOT_FOUND,
            _ => StatusCode::BAD_REQUEST,
        }
    };
    with_security_headers((status, Json(res)).into_response())
}

async fn handle_cmd_pause<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<SimpleCmdBody>,
) -> Response {
    dispatch_http_command(&state, &uri, &headers, body.cmd_id, CommandKind::Pause).await
}

async fn handle_cmd_resume<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<SimpleCmdBody>,
) -> Response {
    dispatch_http_command(&state, &uri, &headers, body.cmd_id, CommandKind::Resume).await
}

async fn handle_cmd_switch_venue<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<SwitchVenueBody>,
) -> Response {
    dispatch_http_command(
        &state,
        &uri,
        &headers,
        body.cmd_id,
        CommandKind::SwitchVenue {
            target: body.target,
            on_open_positions: body.on_open_positions,
            confirm_nonce: body.confirm_nonce,
        },
    )
    .await
}

async fn handle_cmd_request_confirm<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<RequestConfirmBody>,
) -> Response {
    dispatch_http_command(
        &state,
        &uri,
        &headers,
        body.cmd_id,
        CommandKind::RequestConfirm {
            action: body.action,
        },
    )
    .await
}

async fn handle_cmd_panic_flatten<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<ConfirmNonceBody>,
) -> Response {
    dispatch_http_command(
        &state,
        &uri,
        &headers,
        body.cmd_id,
        CommandKind::PanicFlatten {
            confirm_nonce: body.confirm_nonce,
        },
    )
    .await
}

async fn handle_cmd_reset_kill<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<ConfirmNonceBody>,
) -> Response {
    dispatch_http_command(
        &state,
        &uri,
        &headers,
        body.cmd_id,
        CommandKind::ResetKill {
            confirm_nonce: body.confirm_nonce,
        },
    )
    .await
}

async fn handle_cmd_close_position<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<ClosePositionBody>,
) -> Response {
    dispatch_http_command(
        &state,
        &uri,
        &headers,
        body.cmd_id,
        CommandKind::ClosePosition {
            position_id: body.position_id,
        },
    )
    .await
}

async fn handle_cmd_set_param<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<SetParamBody>,
) -> Response {
    dispatch_http_command(
        &state,
        &uri,
        &headers,
        body.cmd_id,
        CommandKind::SetParam {
            key: body.key,
            value: body.value,
        },
    )
    .await
}

async fn handle_cmd_disable_strategy<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<IdBody>,
) -> Response {
    dispatch_http_command(
        &state,
        &uri,
        &headers,
        body.cmd_id,
        CommandKind::DisableStrategy { id: body.id },
    )
    .await
}

async fn handle_cmd_ack_alert<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    Json(body): Json<IdBody>,
) -> Response {
    dispatch_http_command(
        &state,
        &uri,
        &headers,
        body.cmd_id,
        CommandKind::AckAlert { id: body.id },
    )
    .await
}

async fn handle_ws_upgrade<B: IpcBackend>(
    State(state): State<Arc<AppState<B>>>,
    uri: Uri,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(err) = reject_query_string_token(&uri) {
        return error_response(&err);
    }
    let origin = headers.get("origin").and_then(|v| v.to_str().ok());
    if let Err(err) = state.config.origin_policy.validate(origin) {
        return error_response(&err);
    }

    // Optional pre-upgrade auth via Authorization header or Sec-WebSocket-Protocol (`bearer.<token>`).
    let pre_auth = if let Some(tok) =
        extract_bearer_from_headers(&headers).or_else(|| extract_ws_subprotocol_token(&headers))
    {
        match state.config.auth.authenticate(tok) {
            Ok(scope) => Some(scope),
            Err(err) => return error_response(&err),
        }
    } else {
        None
    };

    ws.on_upgrade(move |socket| handle_ws_session(socket, state, pre_auth))
}

async fn authenticate_ws_first_message<B: IpcBackend>(
    socket: &mut WebSocket,
    state: &AppState<B>,
) -> Option<AuthScope> {
    let Ok(Some(Ok(AxumWsMessage::Text(txt)))) =
        tokio::time::timeout(WS_FIRST_MESSAGE_AUTH_TIMEOUT, socket.recv()).await
    else {
        let _ = socket.close().await;
        return None;
    };
    let Ok(env) = serde_json::from_str::<WireEnvelope<ClientMessage>>(&txt) else {
        let _ = socket.close().await;
        return None;
    };
    let ClientMessage::Auth(auth_payload) = env.body else {
        let _ = socket.close().await;
        return None;
    };
    if let Ok(scope) = state.config.auth.authenticate(&auth_payload.token) {
        Some(scope)
    } else {
        let _ = socket.close().await;
        None
    }
}

async fn handle_ws_session<B: IpcBackend>(
    mut socket: WebSocket,
    state: Arc<AppState<B>>,
    pre_auth: Option<AuthScope>,
) {
    let scope = if let Some(s) = pre_auth {
        s
    } else if let Some(s) = authenticate_ws_first_message(&mut socket, &state).await {
        s
    } else {
        return;
    };

    let principal = format!("ws:{}", scope.as_str());
    let mut seq = 1_u64;

    let hello = WireEnvelope::server(
        seq,
        unix_now_ms(),
        ServerMessage::Hello(state.dispatcher.hello_payload()),
    );
    seq += 1;
    if send_ws_envelope(&mut socket, &hello).await.is_err() {
        return;
    }

    let snap = WireEnvelope::server(
        seq,
        unix_now_ms(),
        ServerMessage::Snapshot(state.dispatcher.backend().snapshot().await),
    );
    seq += 1;
    if send_ws_envelope(&mut socket, &snap).await.is_err() {
        return;
    }

    let (client_id, mut rx) = state.hub.subscribe().await;

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(AxumWsMessage::Text(txt))) => {
                        let Ok(env) = serde_json::from_str::<WireEnvelope<ClientMessage>>(&txt) else {
                            break;
                        };
                        let Some(reply) = build_ws_reply(&state, &principal, scope, seq, env.body).await else {
                            continue;
                        };
                        seq += 1;
                        if send_ws_envelope(&mut socket, &reply).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(AxumWsMessage::Ping(p))) => {
                        if socket.send(AxumWsMessage::Pong(p)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(AxumWsMessage::Pong(_))) => {}
                    Some(Ok(AxumWsMessage::Close(_) | AxumWsMessage::Binary(_)) | Err(_)) | None => break,
                }
            }
            maybe_msg = rx.recv() => {
                let Some(mut env) = maybe_msg else {
                    break;
                };
                env.seq = seq;
                seq += 1;
                if send_ws_envelope(&mut socket, &env).await.is_err() {
                    break;
                }
            }
        }
    }

    state.hub.unsubscribe(client_id).await;
}

async fn build_ws_reply<B: IpcBackend>(
    state: &AppState<B>,
    principal: &str,
    scope: AuthScope,
    seq: u64,
    msg: ClientMessage,
) -> Option<WireEnvelope<ServerMessage>> {
    let body = match msg {
        ClientMessage::Command(cmd) => {
            if state.config.rate_limiter.check(principal).is_err() {
                ServerMessage::CommandResult(CommandResultPayload {
                    cmd_id: cmd.cmd_id,
                    ok: false,
                    code: "rate_limited".to_owned(),
                    message: "rate limit exceeded".to_owned(),
                    confirm_nonce: None,
                    expires_at_ms: None,
                })
            } else {
                let res = state.dispatcher.dispatch(principal, scope, cmd).await;
                ServerMessage::CommandResult(res)
            }
        }
        ClientMessage::RequestSnapshot(_) => {
            let s = state.dispatcher.backend().snapshot().await;
            ServerMessage::Snapshot(s)
        }
        ClientMessage::GetKlines(q) => {
            let bars = state
                .dispatcher
                .backend()
                .list_klines(q.venue, &q.symbol, &q.tf, q.limit.clamp(1, 1000))
                .await;
            ServerMessage::Klines(KlinesPayload {
                req_id: q.req_id,
                venue: q.venue,
                symbol: q.symbol,
                tf: q.tf,
                klines: bars,
            })
        }
        ClientMessage::GetTrades(q) => {
            let trades = state
                .dispatcher
                .backend()
                .list_trades(q.venue, q.from_ms, q.to_ms)
                .await;
            ServerMessage::Trades(TradesPayload {
                req_id: q.req_id,
                venue: q.venue,
                trades,
            })
        }
        ClientMessage::Auth(_) => return None,
    };
    Some(WireEnvelope::server(seq, unix_now_ms(), body))
}

async fn send_ws_envelope(
    socket: &mut WebSocket,
    env: &WireEnvelope<ServerMessage>,
) -> Result<(), ()> {
    let text = serde_json::to_string(env).map_err(|_| ())?;
    socket
        .send(AxumWsMessage::Text(text.into()))
        .await
        .map_err(|_| ())
}

/// Bind `addr` (loopback HTTP/WS IPC + optional static `web-ui` SPA directory) and spawn `axum::serve`.
///
/// # Errors
/// Returns [`IpcError::Io`] if binding `addr` fails.
pub async fn serve_http<B: IpcBackend>(
    addr: std::net::SocketAddr,
    dispatcher: Arc<CommandDispatcher<B>>,
    hub: Arc<TelemetryHub>,
    config: HttpServerConfig,
    static_dir: Option<std::path::PathBuf>,
) -> Result<tokio::task::JoinHandle<()>, IpcError> {
    let mut router = build_router(dispatcher, hub, config);
    if let Some(dir) = static_dir.filter(|p| p.is_dir()) {
        let dir_arc = Arc::new(dir);
        router = router.fallback(move |uri: Uri| {
            let root = Arc::clone(&dir_arc);
            async move { serve_static_asset(&root, uri.path()) }
        });
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, router).await {
            tracing::warn!(error = %err, "HTTP/WS IPC server exited with error");
        }
    });
    Ok(handle)
}

fn serve_static_asset(root: &std::path::Path, req_path: &str) -> Response {
    let trimmed = req_path.trim_start_matches('/');
    // Reject path traversal segments explicitly.
    if trimmed.split('/').any(|seg| seg == ".." || seg == "\\") {
        return with_security_headers((StatusCode::BAD_REQUEST, "invalid path").into_response());
    }
    let candidate = if trimmed.is_empty() {
        root.join("index.html")
    } else {
        let p = root.join(trimmed);
        if p.is_file() {
            p
        } else {
            // SPA client-side route fallback to index.html.
            root.join("index.html")
        }
    };
    let Ok(bytes) = std::fs::read(&candidate) else {
        return with_security_headers((StatusCode::NOT_FOUND, "not found").into_response());
    };
    let content_type = match candidate.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    };
    let mut resp = (StatusCode::OK, bytes).into_response();
    resp.headers_mut()
        .insert("content-type", HeaderValue::from_static(content_type));
    with_security_headers(resp)
}
