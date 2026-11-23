//! Persistent Spot WS API session with state machine, correlation map, ping/pong,
//! watchdog, make-before-break 23h rotation, `serverShutdown`, and reconnect backoff (§4.2).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bot_types::Venue;
use futures_util::{SinkExt, StreamExt};
use rand::RngExt;
use serde_json::Value;
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::auth::Ed25519Credentials;
use crate::dedup::EventDeduplicator;
use crate::error::{GatewayError, SessionState};
use crate::gateway::GatewayEvent;
use crate::rate_limit::{RateLimitTracker, ReconnectBackoff};
use crate::spot::rest::now_unix_ms;
use crate::spot::wire::{
    InboundFrame, WsRequest, WsResponse, next_request_id, parse_inbound_frame,
};

/// Configuration for [`SpotWsSession`].
#[derive(Debug, Clone)]
pub struct SpotSessionConfig {
    /// WebSocket API endpoint URL (default: Spot Testnet).
    pub ws_url: String,
    /// Per-request timeout (default: 10 s per Binance server timeout).
    pub request_timeout: Duration,
    /// Watchdog idle timeout (default: 60 s for WS API per §4.2).
    pub stale_timeout: Duration,
    /// Proactive connection rotation interval (default: 23 hours per §4.2).
    pub rotation_interval: Duration,
    /// Symmetric random jitter around `rotation_interval` (default: 10 minutes per §4.2).
    pub rotation_jitter: Duration,
    /// Maximum time to wait for in-flight requests on old connection during rotation/shutdown (default: 5 s).
    pub drain_timeout: Duration,
    /// Optional `recvWindow` in milliseconds for signed/authenticated requests.
    pub recv_window_ms: Option<u64>,
    /// Bounded user-data event channel capacity (default: 4096 per §3.3).
    pub event_channel_capacity: usize,
    /// Reconnect exponential backoff base (default: 250 ms per §4.2).
    pub backoff_base: Duration,
    /// Reconnect exponential backoff ceiling (default: 30 s per §4.2).
    pub backoff_max: Duration,
    /// Event deduplicator ring capacity during connection overlap.
    pub dedup_capacity: usize,
}

impl Default for SpotSessionConfig {
    fn default() -> Self {
        Self {
            ws_url: "wss://ws-api.testnet.binance.vision:443/ws-api/v3".to_owned(),
            request_timeout: Duration::from_secs(10),
            stale_timeout: Duration::from_secs(60),
            rotation_interval: Duration::from_secs(23 * 3600),
            rotation_jitter: Duration::from_secs(10 * 60),
            drain_timeout: Duration::from_secs(5),
            recv_window_ms: Some(5_000),
            event_channel_capacity: 4096,
            backoff_base: Duration::from_millis(250),
            backoff_max: Duration::from_secs(30),
            dedup_capacity: 4096,
        }
    }
}

type PendingMap = Arc<Mutex<HashMap<String, oneshot::Sender<WsResponse>>>>;
type RpcDispatchResult = Result<(String, oneshot::Receiver<WsResponse>, PendingMap), GatewayError>;

/// Low-level handle to a single live WebSocket connection.
struct LiveConn {
    conn_id: u64,
    tx: mpsc::Sender<Message>,
    pending: PendingMap,
    pending_notify: Arc<Notify>,
    subscription_id: Option<i64>,
    reader_handle: JoinHandle<()>,
    writer_handle: JoinHandle<()>,
}

impl LiveConn {
    fn pending_count(&self) -> usize {
        self.pending.lock().map_or(0, |m| m.len())
    }

    /// Send a JSON-RPC request over this connection and return the response receiver.
    async fn send_rpc(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(String, oneshot::Receiver<WsResponse>), GatewayError> {
        let id = next_request_id();
        let req = WsRequest {
            id: id.clone(),
            method: method.to_owned(),
            params,
        };
        let payload =
            serde_json::to_string(&req).map_err(|e| GatewayError::WireFormat(e.to_string()))?;

        let (resp_tx, resp_rx) = oneshot::channel();
        {
            let mut map = self
                .pending
                .lock()
                .map_err(|_| GatewayError::Transport("pending map poisoned".into()))?;
            map.insert(id.clone(), resp_tx);
        }

        if self.tx.send(Message::Text(payload.into())).await.is_err() {
            if let Ok(mut map) = self.pending.lock() {
                map.remove(&id);
            }
            self.pending_notify.notify_waiters();
            return Err(GatewayError::SessionClosed);
        }

        Ok((id, resp_rx))
    }

    fn cancel_pending(&self, id: &str) {
        if let Ok(mut map) = self.pending.lock() {
            map.remove(id);
        }
        self.pending_notify.notify_waiters();
    }

    async fn drain_and_close(self, drain_timeout: Duration) {
        let _ = tokio::time::timeout(drain_timeout, async {
            loop {
                if self.pending_count() == 0 {
                    break;
                }
                self.pending_notify.notified().await;
            }
        })
        .await;

        let _ = self.tx.send(Message::Close(None)).await;
        drop(self.tx);
        let _ = tokio::time::timeout(Duration::from_millis(500), self.writer_handle).await;
        self.reader_handle.abort();
    }
}

/// Internal signals sent from connection reader tasks to the Session Manager.
#[derive(Debug)]
enum ConnSignal {
    Closed { conn_id: u64, reason: String },
    ServerShutdown { conn_id: u64 },
}

/// Commands from [`SpotWsSession`] callers to the background Session Manager.
enum ManagerCommand {
    Request {
        method: String,
        params: Option<Value>,
        reply: oneshot::Sender<RpcDispatchResult>,
    },
    ForceRotate {
        reply: oneshot::Sender<Result<(), GatewayError>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// Persistent Spot WS API session handle (cheaply cloneable via internal `Arc`).
#[derive(Clone)]
pub struct SpotWsSession {
    config: SpotSessionConfig,
    cmd_tx: mpsc::Sender<ManagerCommand>,
    state_rx: watch::Receiver<SessionState>,
    rate_limits: Arc<RateLimitTracker>,
    clock_offset_ms: Arc<AtomicI64>,
}

impl std::fmt::Debug for SpotWsSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpotWsSession")
            .field("ws_url", &self.config.ws_url)
            .field("state", &*self.state_rx.borrow())
            .finish_non_exhaustive()
    }
}

impl SpotWsSession {
    /// Connect, authenticate via Ed25519 `session.logon`, subscribe to `userDataStream.subscribe`,
    /// and spawn the background Session Manager task.
    ///
    /// Returns `(session, event_rx, subscription_id)`.
    ///
    /// # Errors
    /// Returns [`GatewayError`] if the initial connection, logon, or subscription fails.
    pub async fn connect(
        config: SpotSessionConfig,
        credentials: Arc<Ed25519Credentials>,
        rate_limits: Arc<RateLimitTracker>,
        clock_offset_ms: Arc<AtomicI64>,
    ) -> Result<(Self, mpsc::Receiver<GatewayEvent>, Option<i64>), GatewayError> {
        let (state_tx, state_rx) = watch::channel(SessionState::Disconnected);
        let (event_tx, event_rx) = mpsc::channel(config.event_channel_capacity.max(16));
        let (sig_tx, sig_rx) = mpsc::channel::<ConnSignal>(64);
        let dedup = Arc::new(Mutex::new(EventDeduplicator::new(config.dedup_capacity)));

        let initial_conn = establish_connection(
            1,
            &config,
            &credentials,
            &rate_limits,
            &clock_offset_ms,
            &dedup,
            &event_tx,
            &sig_tx,
            Some(&state_tx),
        )
        .await?;

        let subscription_id = initial_conn.subscription_id;
        let _ = state_tx.send(SessionState::Ready);

        let (cmd_tx, cmd_rx) = mpsc::channel::<ManagerCommand>(256);

        let mgr_config = config.clone();
        let mgr_rate_limits = Arc::clone(&rate_limits);
        let mgr_clock_offset = Arc::clone(&clock_offset_ms);
        tokio::spawn(run_manager(
            initial_conn,
            mgr_config,
            credentials,
            mgr_rate_limits,
            mgr_clock_offset,
            dedup,
            event_tx,
            state_tx,
            cmd_rx,
            sig_tx,
            sig_rx,
        ));

        let session = Self {
            config,
            cmd_tx,
            state_rx,
            rate_limits,
            clock_offset_ms,
        };

        Ok((session, event_rx, subscription_id))
    }

    /// Current session lifecycle state.
    #[must_use]
    pub fn state(&self) -> SessionState {
        *self.state_rx.borrow()
    }

    /// Reference to the shared rate limit tracker.
    #[must_use]
    pub fn rate_limits(&self) -> &Arc<RateLimitTracker> {
        &self.rate_limits
    }

    /// Current server-adjusted UTC timestamp in milliseconds.
    #[must_use]
    pub fn server_now_ms(&self) -> i64 {
        now_unix_ms().saturating_add(self.clock_offset_ms.load(Ordering::Relaxed))
    }

    /// Send a WS API request and await its correlated response within `request_timeout`.
    ///
    /// If `ambiguous_client_order_id` is `Some(...)`, a request timeout or 5xx response is
    /// mapped to [`GatewayError::UnknownNeedsQuery`] so the caller can query by `clientOrderId`.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on rate limit, session closed, timeout, or exchange rejection.
    pub async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        ambiguous_client_order_id: Option<String>,
    ) -> Result<Value, GatewayError> {
        let now_ms = now_unix_ms();
        if let Some(delay) = self.rate_limits.retry_after_delay(now_ms) {
            let retry_at = now_ms.saturating_add(i64::try_from(delay.as_millis()).unwrap_or(0));
            if self.rate_limits.is_ip_banned(now_ms) {
                return Err(GatewayError::IpBanned {
                    retry_after_ms: Some(retry_at),
                    message: "request blocked locally while IP ban retryAfter is active".into(),
                });
            }
            return Err(GatewayError::RateLimited {
                status: 429,
                code: Some(-1003),
                retry_after_ms: Some(retry_at),
                message: "request blocked locally while rate-limit retryAfter is active".into(),
            });
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagerCommand::Request {
                method: method.to_owned(),
                params,
                reply: reply_tx,
            })
            .await
            .map_err(|_| GatewayError::SessionClosed)?;

        let (req_id, resp_rx, pending_map) =
            reply_rx.await.map_err(|_| GatewayError::SessionClosed)??;

        match tokio::time::timeout(self.config.request_timeout, resp_rx).await {
            Ok(Ok(ws_resp)) => ws_resp.into_result(ambiguous_client_order_id),
            Ok(Err(_closed)) => {
                if let Some(cid) = ambiguous_client_order_id {
                    Err(GatewayError::UnknownNeedsQuery {
                        client_order_id: Some(cid),
                        reason: "connection closed while awaiting order response".into(),
                    })
                } else {
                    Err(GatewayError::SessionClosed)
                }
            }
            Err(_elapsed) => {
                if let Ok(mut map) = pending_map.lock() {
                    map.remove(&req_id);
                }
                if let Some(cid) = ambiguous_client_order_id {
                    Err(GatewayError::UnknownNeedsQuery {
                        client_order_id: Some(cid),
                        reason: format!(
                            "WS API request `{method}` timed out after {:?}",
                            self.config.request_timeout
                        ),
                    })
                } else {
                    Err(GatewayError::UnknownNeedsQuery {
                        client_order_id: None,
                        reason: format!(
                            "WS API request `{method}` timed out after {:?}",
                            self.config.request_timeout
                        ),
                    })
                }
            }
        }
    }

    /// Trigger an immediate make-before-break connection rotation (primarily for tests/ops).
    ///
    /// # Errors
    /// Returns [`GatewayError`] if the replacement connection cannot be established.
    pub async fn rotate_now(&self) -> Result<(), GatewayError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(ManagerCommand::ForceRotate { reply: reply_tx })
            .await
            .map_err(|_| GatewayError::SessionClosed)?;
        reply_rx.await.map_err(|_| GatewayError::SessionClosed)?
    }

    /// Cleanly drain in-flight requests and close the session without canceling any orders.
    pub async fn shutdown(&self) {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(ManagerCommand::Shutdown { reply: reply_tx })
            .await
            .is_ok()
        {
            let _ = reply_rx.await;
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn establish_connection(
    conn_id: u64,
    config: &SpotSessionConfig,
    credentials: &Ed25519Credentials,
    rate_limits: &Arc<RateLimitTracker>,
    clock_offset_ms: &Arc<AtomicI64>,
    dedup: &Arc<Mutex<EventDeduplicator>>,
    event_tx: &mpsc::Sender<GatewayEvent>,
    sig_tx: &mpsc::Sender<ConnSignal>,
    state_tx: Option<&watch::Sender<SessionState>>,
) -> Result<LiveConn, GatewayError> {
    if let Some(st) = state_tx {
        let _ = st.send(SessionState::Connecting);
    }
    debug!(conn_id, url = %config.ws_url, "connecting Spot WS API");

    let (ws_stream, _resp) = connect_async(&config.ws_url)
        .await
        .map_err(|e| GatewayError::Transport(e.to_string()))?;

    let (mut ws_sink, mut ws_read) = ws_stream.split();
    let (tx, mut out_rx) = mpsc::channel::<Message>(256);
    let pending: Arc<Mutex<HashMap<String, oneshot::Sender<WsResponse>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let pending_notify = Arc::new(Notify::new());

    let writer_handle = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    let r_pending = Arc::clone(&pending);
    let r_notify = Arc::clone(&pending_notify);
    let r_rate_limits = Arc::clone(rate_limits);
    let r_dedup = Arc::clone(dedup);
    let r_event_tx = event_tx.clone();
    let r_sig_tx = sig_tx.clone();
    let r_pong_tx = tx.clone();
    let stale_timeout = config.stale_timeout;

    let reader_handle = tokio::spawn(async move {
        let close_reason: String;
        loop {
            match tokio::time::timeout(stale_timeout, ws_read.next()).await {
                Ok(Some(Ok(msg))) => match msg {
                    Message::Ping(payload) => {
                        // Verified Binance rule: reply Pong with copy of Ping payload ASAP.
                        let _ = r_pong_tx.send(Message::Pong(payload)).await;
                    }
                    Message::Pong(_) | Message::Frame(_) => {}
                    Message::Text(text) => match parse_inbound_frame(text.as_str()) {
                        Ok(InboundFrame::Response(resp)) => {
                            if !resp.rate_limits.is_empty() {
                                r_rate_limits.record_readings(&resp.rate_limits);
                            }
                            if let Some(ra) = resp.retry_after_ms() {
                                r_rate_limits.record_retry_after(ra, resp.status == 418);
                            }
                            let waiter = r_pending
                                .lock()
                                .ok()
                                .and_then(|mut map| map.remove(&resp.id));
                            if let Some(sender) = waiter {
                                let _ = sender.send(resp);
                                r_notify.notify_waiters();
                            }
                        }
                        Ok(InboundFrame::Event { event, .. }) => {
                            let should_forward = match &event {
                                GatewayEvent::Exec(exec) => r_dedup.lock().is_ok_and(|mut d| {
                                    d.check_execution_report(
                                        exec.order_id,
                                        exec.trade_id,
                                        exec.event_time_ms,
                                        &exec.exec_type,
                                    )
                                }),
                                _ => true,
                            };

                            if matches!(event, GatewayEvent::ServerShutdown { .. }) {
                                let _ = r_sig_tx.try_send(ConnSignal::ServerShutdown { conn_id });
                            }

                            if should_forward && r_event_tx.send(event).await.is_err() {
                                close_reason = "event receiver dropped".into();
                                break;
                            }
                        }
                        Err(err) => {
                            warn!(conn_id, error = %err, "failed to parse inbound WS API frame");
                        }
                    },
                    Message::Binary(_) => {
                        warn!(conn_id, "unexpected binary WS frame on JSON session");
                    }
                    Message::Close(frame) => {
                        close_reason = format!("server closed WS: {frame:?}");
                        break;
                    }
                },
                Ok(Some(Err(err))) => {
                    close_reason = format!("WS read error: {err}");
                    break;
                }
                Ok(None) => {
                    close_reason = "WS stream ended".into();
                    break;
                }
                Err(_elapsed) => {
                    close_reason = format!("watchdog stale timeout ({stale_timeout:?})");
                    break;
                }
            }
        }

        // Fail all remaining in-flight waiters immediately.
        if let Ok(mut map) = r_pending.lock() {
            map.clear();
        }
        r_notify.notify_waiters();
        let _ = r_sig_tx
            .send(ConnSignal::Closed {
                conn_id,
                reason: close_reason,
            })
            .await;
    });

    let mut conn = LiveConn {
        conn_id,
        tx,
        pending,
        pending_notify,
        subscription_id: None,
        reader_handle,
        writer_handle,
    };

    // Step 1: Ed25519 session.logon
    if let Some(st) = state_tx {
        let _ = st.send(SessionState::LoggingOn);
    }
    let ts = now_unix_ms().saturating_add(clock_offset_ms.load(Ordering::Relaxed));
    let logon_params = credentials.build_session_logon_params(ts, config.recv_window_ms);
    let logon_res = rpc_on_conn(
        &conn,
        "session.logon",
        Some(logon_params),
        config.request_timeout,
    )
    .await;
    if let Err(err) = logon_res {
        conn.drain_and_close(Duration::from_millis(200)).await;
        return Err(err);
    }

    // Step 2: userDataStream.subscribe
    if let Some(st) = state_tx {
        let _ = st.send(SessionState::Subscribing);
    }
    let sub_res = rpc_on_conn(
        &conn,
        "userDataStream.subscribe",
        None,
        config.request_timeout,
    )
    .await;
    match sub_res {
        Ok(val) => {
            conn.subscription_id = val.get("subscriptionId").and_then(Value::as_i64);
            info!(
                conn_id,
                subscription_id = ?conn.subscription_id,
                "Spot WS API authenticated and subscribed"
            );
            Ok(conn)
        }
        Err(err) => {
            conn.drain_and_close(Duration::from_millis(200)).await;
            Err(err)
        }
    }
}

async fn rpc_on_conn(
    conn: &LiveConn,
    method: &str,
    params: Option<Value>,
    timeout: Duration,
) -> Result<Value, GatewayError> {
    let (id, rx) = conn.send_rpc(method, params).await?;
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(resp)) => resp.into_result(None),
        Ok(Err(_)) => Err(GatewayError::SessionClosed),
        Err(_) => {
            conn.cancel_pending(&id);
            Err(GatewayError::Transport(format!(
                "handshake `{method}` timed out after {timeout:?}"
            )))
        }
    }
}

fn next_rotation_delay(interval: Duration, jitter: Duration) -> Duration {
    let jitter_ms = u64::try_from(jitter.as_millis()).unwrap_or(0);
    if jitter_ms == 0 {
        return interval;
    }
    let offset_ms = rand::rng().random_range(0..=(jitter_ms.saturating_mul(2)));
    let base_ms = u64::try_from(interval.as_millis()).unwrap_or(u64::MAX);
    let adjusted_ms = base_ms
        .saturating_add(offset_ms)
        .saturating_sub(jitter_ms)
        .max(1);
    Duration::from_millis(adjusted_ms)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn run_manager(
    initial_conn: LiveConn,
    config: SpotSessionConfig,
    credentials: Arc<Ed25519Credentials>,
    rate_limits: Arc<RateLimitTracker>,
    clock_offset_ms: Arc<AtomicI64>,
    dedup: Arc<Mutex<EventDeduplicator>>,
    event_tx: mpsc::Sender<GatewayEvent>,
    state_tx: watch::Sender<SessionState>,
    mut cmd_rx: mpsc::Receiver<ManagerCommand>,
    sig_tx: mpsc::Sender<ConnSignal>,
    mut sig_rx: mpsc::Receiver<ConnSignal>,
) {
    let mut active: Option<LiveConn> = Some(initial_conn);
    let mut next_conn_id: u64 = 2;
    let mut backoff = ReconnectBackoff::new(config.backoff_base, config.backoff_max);

    let rot_sleep = tokio::time::sleep(next_rotation_delay(
        config.rotation_interval,
        config.rotation_jitter,
    ));
    tokio::pin!(rot_sleep);

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(ManagerCommand::Request { method, params, reply }) => {
                        let Some(conn) = active.as_ref() else {
                            let _ = reply.send(Err(GatewayError::SessionNotReady(*state_tx.borrow())));
                            continue;
                        };
                        let res = conn.send_rpc(&method, params).await.map(|(id, rx)| {
                            (id, rx, Arc::clone(&conn.pending))
                        });
                        let _ = reply.send(res);
                    }
                    Some(ManagerCommand::ForceRotate { reply }) => {
                        let res = perform_make_before_break(
                            &mut active,
                            &mut next_conn_id,
                            &config,
                            &credentials,
                            &rate_limits,
                            &clock_offset_ms,
                            &dedup,
                            &event_tx,
                            &sig_tx,
                        ).await;
                        if res.is_ok() {
                            rot_sleep.as_mut().reset(
                                tokio::time::Instant::now()
                                    + next_rotation_delay(config.rotation_interval, config.rotation_jitter),
                            );
                        }
                        let _ = reply.send(res);
                    }
                    Some(ManagerCommand::Shutdown { reply }) => {
                        let _ = state_tx.send(SessionState::Draining);
                        if let Some(conn) = active.take() {
                            conn.drain_and_close(config.drain_timeout).await;
                        }
                        let _ = state_tx.send(SessionState::Disconnected);
                        let _ = reply.send(());
                        break;
                    }
                    None => {
                        let _ = state_tx.send(SessionState::Draining);
                        if let Some(conn) = active.take() {
                            conn.drain_and_close(config.drain_timeout).await;
                        }
                        let _ = state_tx.send(SessionState::Disconnected);
                        break;
                    }
                }
            }
            () = &mut rot_sleep => {
                if active.is_some() {
                    info!("proactive Spot WS API rotation timer fired (make-before-break)");
                    if let Err(err) = perform_make_before_break(
                        &mut active,
                        &mut next_conn_id,
                        &config,
                        &credentials,
                        &rate_limits,
                        &clock_offset_ms,
                        &dedup,
                        &event_tx,
                        &sig_tx,
                    ).await {
                        warn!(error = %err, "proactive rotation failed; keeping active connection and retrying later");
                    }
                }
                rot_sleep.as_mut().reset(
                    tokio::time::Instant::now()
                        + next_rotation_delay(config.rotation_interval, config.rotation_jitter),
                );
            }
            Some(sig) = sig_rx.recv() => {
                match sig {
                    ConnSignal::ServerShutdown { conn_id } => {
                        if active.as_ref().is_some_and(|c| c.conn_id == conn_id) {
                            info!(conn_id, "serverShutdown received; starting make-before-break rotation");
                            if let Err(err) = perform_make_before_break(
                                &mut active,
                                &mut next_conn_id,
                                &config,
                                &credentials,
                                &rate_limits,
                                &clock_offset_ms,
                                &dedup,
                                &event_tx,
                                &sig_tx,
                            ).await {
                                warn!(error = %err, "make-before-break on serverShutdown failed");
                            } else {
                                rot_sleep.as_mut().reset(
                                    tokio::time::Instant::now()
                                        + next_rotation_delay(config.rotation_interval, config.rotation_jitter),
                                );
                            }
                        }
                    }
                    ConnSignal::Closed { conn_id, reason } => {
                        let is_active = active.as_ref().is_some_and(|c| c.conn_id == conn_id);
                        if !is_active {
                            debug!(conn_id, %reason, "drained/old connection closed");
                            continue;
                        }
                        warn!(conn_id, %reason, "active Spot WS API connection closed; reconnecting");
                        active = None;
                        let _ = state_tx.send(SessionState::Disconnected);

                        // Reconnect loop honoring backoff + retryAfter while still handling Shutdown.
                        let mut shutdown_requested = false;
                        while active.is_none() {
                            let ra = rate_limits.retry_after_delay(now_unix_ms());
                            let delay = backoff.next_delay(ra);
                            debug!(?delay, "waiting before Spot WS API reconnect");

                            let sleep_fut = tokio::time::sleep(delay);
                            tokio::pin!(sleep_fut);
                            loop {
                                tokio::select! {
                                    () = &mut sleep_fut => break,
                                    cmd = cmd_rx.recv() => {
                                        match cmd {
                                            Some(ManagerCommand::Shutdown { reply }) => {
                                                let _ = reply.send(());
                                                shutdown_requested = true;
                                                break;
                                            }
                                            None => {
                                                shutdown_requested = true;
                                                break;
                                            }
                                            Some(ManagerCommand::Request { reply, .. }) => {
                                                let _ = reply.send(Err(GatewayError::SessionNotReady(SessionState::Disconnected)));
                                            }
                                            Some(ManagerCommand::ForceRotate { reply }) => {
                                                let _ = reply.send(Err(GatewayError::SessionNotReady(SessionState::Disconnected)));
                                            }
                                        }
                                    }
                                }
                            }
                            if shutdown_requested {
                                break;
                            }

                            let cid = next_conn_id;
                            next_conn_id = next_conn_id.saturating_add(1);
                            match establish_connection(
                                cid,
                                &config,
                                &credentials,
                                &rate_limits,
                                &clock_offset_ms,
                                &dedup,
                                &event_tx,
                                &sig_tx,
                                Some(&state_tx),
                            ).await {
                                Ok(new_conn) => {
                                    let sub_id = new_conn.subscription_id;
                                    active = Some(new_conn);
                                    backoff.reset();
                                    let _ = state_tx.send(SessionState::Ready);
                                    rot_sleep.as_mut().reset(
                                        tokio::time::Instant::now()
                                            + next_rotation_delay(config.rotation_interval, config.rotation_jitter),
                                    );
                                    let _ = event_tx
                                        .send(GatewayEvent::Reconnected {
                                            venue: Venue::Spot,
                                            subscription_id: sub_id,
                                        })
                                        .await;
                                }
                                Err(err) => {
                                    warn!(error = %err, "Spot WS API reconnect attempt failed");
                                    let _ = state_tx.send(SessionState::Disconnected);
                                }
                            }
                        }

                        if shutdown_requested {
                            let _ = state_tx.send(SessionState::Disconnected);
                            break;
                        }
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn perform_make_before_break(
    active: &mut Option<LiveConn>,
    next_conn_id: &mut u64,
    config: &SpotSessionConfig,
    credentials: &Arc<Ed25519Credentials>,
    rate_limits: &Arc<RateLimitTracker>,
    clock_offset_ms: &Arc<AtomicI64>,
    dedup: &Arc<Mutex<EventDeduplicator>>,
    event_tx: &mpsc::Sender<GatewayEvent>,
    sig_tx: &mpsc::Sender<ConnSignal>,
) -> Result<(), GatewayError> {
    let cid = *next_conn_id;
    *next_conn_id = next_conn_id.saturating_add(1);

    // Open connection B (logon + subscribe) while connection A stays Ready and serves requests.
    let new_conn = establish_connection(
        cid,
        config,
        credentials,
        rate_limits,
        clock_offset_ms,
        dedup,
        event_tx,
        sig_tx,
        None,
    )
    .await?;

    // Atomically swap active connection to B, then drain A in the background (up to drain_timeout).
    let old_conn = active.replace(new_conn);
    if let Some(old) = old_conn {
        let drain_timeout = config.drain_timeout;
        tokio::spawn(async move {
            old.drain_and_close(drain_timeout).await;
        });
    }
    Ok(())
}
