#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::cast_possible_wrap
)]
//! Security, auth, nonce, conflation, UDS, and HTTP/WS integration tests for `bot-ipc` (M10).

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use bot_config::BotConfig;
use bot_ipc::auth::{AuthScope, OriginPolicy, PeerAuthPolicy, RateLimiter, TokenAuth};
use bot_ipc::command::{CommandDispatcher, InMemoryIpcBackend, IpcBackend};
use bot_ipc::conflate::{TelemetryHub, conflate};
use bot_ipc::http::{HttpServerConfig, build_router};
use bot_ipc::nonce::ConfirmNonceStore;
use bot_ipc::protocol::{
    ClientMessage, CommandKind, CommandPayload, ConfirmAction, DeltaPayload, OnOpenPositionsArg,
    ServerMessage, WireEnvelope,
};
use bot_ipc::uds::{UdsServerConfig, serve_uds};
use bot_store::{JournalEvent, SqliteStore};
use bot_types::{EngineMode, Secret, Venue};
use futures_util::{SinkExt, StreamExt};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixStream};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

fn test_token_auth() -> TokenAuth {
    TokenAuth::new(
        Secret::new("full-secret-token-12345".to_owned()),
        Some(Secret::new("ro-secret-token-67890".to_owned())),
    )
}

async fn spawn_http_server(
    dispatcher: Arc<CommandDispatcher<InMemoryIpcBackend>>,
    hub: Arc<TelemetryHub>,
    rate_limit_burst: u32,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let cfg = HttpServerConfig {
        auth: test_token_auth(),
        origin_policy: OriginPolicy::loopback_default(addr.port()),
        rate_limiter: Arc::new(RateLimiter::new(rate_limit_burst, rate_limit_burst)),
    };
    let app = build_router(dispatcher, hub, cfg);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, handle)
}

#[tokio::test]
async fn negative_auth_origin_query_token_and_rate_limit_rejected() {
    let dir = TempDir::new().expect("tempdir");
    let store = Arc::new(Mutex::new(
        SqliteStore::open(&dir.path().join("ipc.db")).expect("open sqlite"),
    ));
    let backend = Arc::new(InMemoryIpcBackend::new(BotConfig::default()));
    let dispatcher = Arc::new(CommandDispatcher::new(
        backend,
        Some(store.clone()),
        ConfirmNonceStore::new(Duration::from_secs(10)),
    ));
    let hub = Arc::new(TelemetryHub::new(16));
    let (addr, handle) = spawn_http_server(dispatcher, hub, 20).await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // 1. No token on GET /api/v1/health and /api/v1/state -> 401
    let resp = client
        .get(format!("{base}/api/v1/health"))
        .send()
        .await
        .expect("req");
    assert_eq!(resp.status(), 401);
    assert!(
        resp.headers().contains_key("content-security-policy"),
        "CSP header must be present"
    );
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["error"]["code"], "unauthorized");

    // 2. Token in query string -> rejected (400/401)
    let resp = client
        .get(format!("{base}/api/v1/state?token=full-secret-token-12345"))
        .header("Authorization", "Bearer full-secret-token-12345")
        .send()
        .await
        .expect("req");
    assert_eq!(resp.status(), 400);

    // 3. Read-only token can GET /api/v1/state -> 200
    let resp = client
        .get(format!("{base}/api/v1/state"))
        .header("Authorization", "Bearer ro-secret-token-67890")
        .send()
        .await
        .expect("req");
    assert_eq!(resp.status(), 200);

    // 4. Read-only token calling mutation POST /api/v1/commands/pause -> 403
    let resp = client
        .post(format!("{base}/api/v1/commands/pause"))
        .header("Authorization", "Bearer ro-secret-token-67890")
        .json(&serde_json::json!({ "cmd_id": Uuid::now_v7() }))
        .send()
        .await
        .expect("req");
    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["error"]["code"], "forbidden");

    // 5. Wrong Origin header -> 403
    let resp = client
        .get(format!("{base}/api/v1/state"))
        .header("Authorization", "Bearer full-secret-token-12345")
        .header("Origin", "https://evil.attacker.example")
        .send()
        .await
        .expect("req");
    assert_eq!(resp.status(), 403);

    // 6. WebSocket with query string token -> rejected
    let ws_bad_query = format!("ws://{addr}/ws/v1/stream?token=full-secret-token-12345");
    let err = tokio_tungstenite::connect_async(&ws_bad_query).await;
    assert!(err.is_err(), "WS with query token must be rejected");

    // 7. WebSocket with wrong Origin -> rejected
    let mut req = format!("ws://{addr}/ws/v1/stream")
        .into_client_request()
        .expect("ws req");
    req.headers_mut().insert(
        "Authorization",
        "Bearer full-secret-token-12345".parse().expect("hdr"),
    );
    req.headers_mut().insert(
        "Origin",
        "https://evil.attacker.example".parse().expect("hdr"),
    );
    let err = tokio_tungstenite::connect_async(req).await;
    assert!(err.is_err(), "WS with invalid Origin must be rejected");

    // 8. WebSocket with first-message auth -> receives hello + snapshot
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/v1/stream"))
        .await
        .expect("ws connect");
    let auth_env = WireEnvelope::client(
        1,
        1_790_000_000_000,
        ClientMessage::Auth(bot_ipc::protocol::AuthPayload {
            token: "ro-secret-token-67890".to_owned(),
        }),
    );
    ws.send(WsMessage::Text(
        serde_json::to_string(&auth_env).expect("ser").into(),
    ))
    .await
    .expect("send auth");

    let first = ws.next().await.expect("hello frame").expect("ok");
    let hello_env: WireEnvelope<ServerMessage> =
        serde_json::from_str(first.to_text().expect("text")).expect("parse hello");
    assert!(matches!(hello_env.body, ServerMessage::Hello(_)));

    let second = ws.next().await.expect("snapshot frame").expect("ok");
    let snap_env: WireEnvelope<ServerMessage> =
        serde_json::from_str(second.to_text().expect("text")).expect("parse snapshot");
    assert!(matches!(snap_env.body, ServerMessage::Snapshot(_)));

    // Read-only WS client sending mutation command -> command_result forbidden
    let cmd_env = WireEnvelope::client(
        2,
        1_790_000_000_100,
        ClientMessage::Command(CommandPayload {
            cmd_id: Uuid::now_v7(),
            cmd: CommandKind::Pause,
        }),
    );
    ws.send(WsMessage::Text(
        serde_json::to_string(&cmd_env).expect("ser").into(),
    ))
    .await
    .expect("send pause");
    let third = ws.next().await.expect("cmd result").expect("ok");
    let res_env: WireEnvelope<ServerMessage> =
        serde_json::from_str(third.to_text().expect("text")).expect("parse result");
    match res_env.body {
        ServerMessage::CommandResult(r) => {
            assert!(!r.ok);
            assert_eq!(r.code, "forbidden");
        }
        other => panic!("expected CommandResult, got {other:?}"),
    }

    handle.abort();
}

#[tokio::test]
async fn rate_limiter_blocks_excess_requests() {
    let backend = Arc::new(InMemoryIpcBackend::new(BotConfig::default()));
    let dispatcher = Arc::new(CommandDispatcher::new(
        backend,
        None,
        ConfirmNonceStore::new(Duration::from_secs(10)),
    ));
    let hub = Arc::new(TelemetryHub::new(16));
    let (addr, handle) = spawn_http_server(dispatcher, hub, 2).await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    for _ in 0..2 {
        let resp = client
            .get(format!("{base}/api/v1/health"))
            .header("Authorization", "Bearer full-secret-token-12345")
            .send()
            .await
            .expect("req");
        assert_eq!(resp.status(), 200);
    }
    let resp = client
        .get(format!("{base}/api/v1/health"))
        .header("Authorization", "Bearer full-secret-token-12345")
        .send()
        .await
        .expect("req");
    assert_eq!(resp.status(), 429);

    handle.abort();
}

#[tokio::test]
async fn commands_nonce_expiry_idempotency_tighten_only_switch_stub_and_audit() {
    let dir = TempDir::new().expect("tempdir");
    let store = Arc::new(Mutex::new(
        SqliteStore::open(&dir.path().join("audit.db")).expect("open sqlite"),
    ));
    let backend = Arc::new(InMemoryIpcBackend::new(BotConfig::default()));
    let dispatcher = CommandDispatcher::new(
        backend.clone(),
        Some(store.clone()),
        ConfirmNonceStore::new(Duration::from_millis(300)),
    );

    // 1. switch_venue stub returns not_supported
    let res = dispatcher
        .dispatch(
            "http:full",
            AuthScope::Full,
            CommandPayload {
                cmd_id: Uuid::now_v7(),
                cmd: CommandKind::SwitchVenue {
                    target: Venue::UsdsFutures,
                    on_open_positions: OnOpenPositionsArg::Refuse,
                    confirm_nonce: "any".to_owned(),
                },
            },
        )
        .await;
    assert!(!res.ok);
    assert_eq!(res.code, "not_supported");

    // 2. set_param tighten-only: tighten 0.5 -> 0.25 succeeds, loosen 0.25 -> 0.80 rejected
    let tighten_ok = dispatcher
        .dispatch(
            "http:full",
            AuthScope::Full,
            CommandPayload {
                cmd_id: Uuid::now_v7(),
                cmd: CommandKind::SetParam {
                    key: "risk.risk_per_trade_pct".to_owned(),
                    value: "0.25".to_owned(),
                },
            },
        )
        .await;
    assert!(tighten_ok.ok, "tightening must succeed: {tighten_ok:?}");

    let loosen_err = dispatcher
        .dispatch(
            "http:full",
            AuthScope::Full,
            CommandPayload {
                cmd_id: Uuid::now_v7(),
                cmd: CommandKind::SetParam {
                    key: "risk.risk_per_trade_pct".to_owned(),
                    value: "0.80".to_owned(),
                },
            },
        )
        .await;
    assert!(!loosen_err.ok);
    assert_eq!(loosen_err.code, "loosen_rejected");

    // 3. Expired confirmation nonce rejected
    let req_expired = dispatcher
        .dispatch(
            "http:full",
            AuthScope::Full,
            CommandPayload {
                cmd_id: Uuid::now_v7(),
                cmd: CommandKind::RequestConfirm {
                    action: ConfirmAction::PanicFlatten,
                },
            },
        )
        .await;
    assert!(req_expired.ok);
    let expired_nonce = req_expired.confirm_nonce.expect("nonce issued");
    tokio::time::sleep(Duration::from_millis(350)).await;

    let panic_expired = dispatcher
        .dispatch(
            "http:full",
            AuthScope::Full,
            CommandPayload {
                cmd_id: Uuid::now_v7(),
                cmd: CommandKind::PanicFlatten {
                    confirm_nonce: expired_nonce,
                },
            },
        )
        .await;
    assert!(!panic_expired.ok);
    assert_eq!(panic_expired.code, "invalid_confirm_nonce");

    // 4. Fresh confirmation nonce succeeds once, replay fails, and cmd_id is idempotent
    let req_fresh = dispatcher
        .dispatch(
            "http:full",
            AuthScope::Full,
            CommandPayload {
                cmd_id: Uuid::now_v7(),
                cmd: CommandKind::RequestConfirm {
                    action: ConfirmAction::PanicFlatten,
                },
            },
        )
        .await;
    let fresh_nonce = req_fresh.confirm_nonce.expect("fresh nonce");
    let panic_cmd_id = Uuid::now_v7();
    let panic_ok = dispatcher
        .dispatch(
            "http:full",
            AuthScope::Full,
            CommandPayload {
                cmd_id: panic_cmd_id,
                cmd: CommandKind::PanicFlatten {
                    confirm_nonce: fresh_nonce.clone(),
                },
            },
        )
        .await;
    assert!(panic_ok.ok);
    assert_eq!(backend.engine_mode().await, EngineMode::Killed);

    // Same cmd_id returns cached idempotent ok result
    let panic_idem = dispatcher
        .dispatch(
            "http:full",
            AuthScope::Full,
            CommandPayload {
                cmd_id: panic_cmd_id,
                cmd: CommandKind::PanicFlatten {
                    confirm_nonce: fresh_nonce.clone(),
                },
            },
        )
        .await;
    assert_eq!(panic_idem, panic_ok);

    // New cmd_id reusing consumed nonce is rejected
    let panic_replay = dispatcher
        .dispatch(
            "http:full",
            AuthScope::Full,
            CommandPayload {
                cmd_id: Uuid::now_v7(),
                cmd: CommandKind::PanicFlatten {
                    confirm_nonce: fresh_nonce,
                },
            },
        )
        .await;
    assert!(!panic_replay.ok);
    assert_eq!(panic_replay.code, "invalid_confirm_nonce");

    // 5. Verify journal audit records exist with kind == "Command"
    let events = store
        .lock()
        .await
        .read_journal_after(0)
        .expect("read journal");
    let cmd_events: Vec<_> = events
        .iter()
        .filter(|(_, _, env)| env.kind() == "Command")
        .collect();
    assert!(
        cmd_events.len() >= 6,
        "expected >= 6 audited Command journal events, got {}",
        cmd_events.len()
    );
    assert!(
        cmd_events
            .iter()
            .any(|(_, _, env)| matches!(&env.event, JournalEvent::Command { command, .. } if command == "panic_flatten"))
    );
}

#[tokio::test]
async fn uds_server_enforces_mode_0660_and_so_peercred() {
    let dir = TempDir::new().expect("tempdir");
    let sock_path = dir.path().join("bot.sock");

    let backend = Arc::new(InMemoryIpcBackend::new(BotConfig::default()));
    let dispatcher = Arc::new(CommandDispatcher::new(
        backend,
        None,
        ConfirmNonceStore::new(Duration::from_secs(10)),
    ));
    let hub = Arc::new(TelemetryHub::new(16));

    let server_handle = serve_uds(
        UdsServerConfig {
            socket_path: sock_path.clone(),
            peer_auth: PeerAuthPolicy::allow_current_process(),
        },
        dispatcher.clone(),
        hub.clone(),
    )
    .await
    .expect("start uds server");

    // Verify 0660 permissions on socket file
    let mode = std::fs::metadata(&sock_path)
        .expect("socket metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o660, "UDS socket must have mode 0660");

    // Connect as current user -> receives hello + snapshot, then executes pause
    let stream = UnixStream::connect(&sock_path).await.expect("connect uds");
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    let mut line = String::new();
    reader.read_line(&mut line).await.expect("read hello");
    let hello: WireEnvelope<ServerMessage> = serde_json::from_str(&line).expect("parse hello");
    assert!(matches!(hello.body, ServerMessage::Hello(_)));

    line.clear();
    reader.read_line(&mut line).await.expect("read snapshot");
    let snap: WireEnvelope<ServerMessage> = serde_json::from_str(&line).expect("parse snapshot");
    assert!(matches!(snap.body, ServerMessage::Snapshot(_)));

    let pause_cmd = WireEnvelope::client(
        1,
        1_790_000_000_000,
        ClientMessage::Command(CommandPayload {
            cmd_id: Uuid::now_v7(),
            cmd: CommandKind::Pause,
        }),
    );
    let mut payload = serde_json::to_string(&pause_cmd).expect("ser");
    payload.push('\n');
    write_half
        .write_all(payload.as_bytes())
        .await
        .expect("write");

    line.clear();
    reader.read_line(&mut line).await.expect("read result");
    let result: WireEnvelope<ServerMessage> = serde_json::from_str(&line).expect("parse result");
    assert!(matches!(
        result.body,
        ServerMessage::CommandResult(ref r) if r.ok && r.code == "ok"
    ));

    server_handle.abort();

    // Unauthorized peer policy (disallow all UIDs/GIDs) -> connection closed immediately
    let denied_path = dir.path().join("denied.sock");
    let denied_handle = serve_uds(
        UdsServerConfig {
            socket_path: denied_path.clone(),
            peer_auth: PeerAuthPolicy::new(vec![], vec![]),
        },
        dispatcher,
        hub,
    )
    .await
    .expect("start denied uds");

    let denied_stream = UnixStream::connect(&denied_path)
        .await
        .expect("connect denied");
    let mut denied_reader = BufReader::new(denied_stream);
    let mut buf = String::new();
    let n = denied_reader.read_line(&mut buf).await.expect("read eof");
    assert_eq!(n, 0, "unauthorized peer must be disconnected immediately");

    denied_handle.abort();
}

#[tokio::test]
async fn conflation_keeps_latest_value_per_key_and_slow_client_is_disconnected() {
    // 1. Conflation test (§10.4): 50 updates for 2 keys within one tick -> 1 batch with latest values
    let (in_tx, in_rx) = mpsc::channel::<(String, u32)>(128);
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<(String, u32)>>(16);
    let conf_handle = tokio::spawn(conflate(in_rx, 10, out_tx));

    for i in 1..=50_u32 {
        in_tx
            .send(("pos:BTCUSDT".to_owned(), i))
            .await
            .expect("send");
        in_tx
            .send(("pos:ETHUSDT".to_owned(), i * 10))
            .await
            .expect("send");
    }
    drop(in_tx);

    let mut last_btc = 0_u32;
    let mut last_eth = 0_u32;
    let mut batch_count = 0_usize;
    while let Some(batch) = out_rx.recv().await {
        batch_count += 1;
        for (k, v) in batch {
            if k == "pos:BTCUSDT" {
                last_btc = v;
            } else if k == "pos:ETHUSDT" {
                last_eth = v;
            }
        }
    }
    conf_handle.await.expect("conflate join");
    assert!(
        batch_count <= 3,
        "conflation should collapse 100 updates into <= 3 ticks, got {batch_count}"
    );
    assert_eq!(last_btc, 50);
    assert_eq!(last_eth, 500);

    // 2. Slow-client disconnect test: slow client with buffer=2 is dropped without blocking hub
    let hub = TelemetryHub::new(2);
    let (_slow_id, mut slow_rx) = hub.subscribe().await;
    let (_fast_id, mut fast_rx) = hub.subscribe().await;

    // Drain fast client concurrently while bursting 10 deltas; slow client never reads
    let fast_task = tokio::spawn(async move {
        let mut received = 0_usize;
        while let Ok(Some(_)) =
            tokio::time::timeout(Duration::from_millis(200), fast_rx.recv()).await
        {
            received += 1;
            if received == 10 {
                break;
            }
        }
        received
    });

    for seq in 1..=10_u64 {
        hub.publish(ServerMessage::Delta(DeltaPayload::default()), seq as i64)
            .await;
        tokio::task::yield_now().await;
    }

    let fast_received = fast_task.await.expect("fast join");
    assert_eq!(fast_received, 10, "fast client must receive all 10 frames");

    // Slow client must have been dropped (channel closed after at most 2 buffered items)
    let mut slow_count = 0_usize;
    while slow_rx.try_recv().is_ok() {
        slow_count += 1;
    }
    assert!(slow_count <= 2);
    assert!(
        slow_rx.recv().await.is_none(),
        "slow client channel must be closed after overflow"
    );
}
