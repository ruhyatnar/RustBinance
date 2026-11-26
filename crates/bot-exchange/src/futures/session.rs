//! USDⓈ-M Futures WS API session (`/ws-fapi/v1`) with per-request Ed25519 signing (§4.5).
//!
//! Per `docs/verified-facts.md`, Futures WS API does not have `/session.logon`; every
//! authenticated request carries `apiKey`, `timestamp`, and `signature`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;

use crate::auth::Ed25519Credentials;
use crate::clock::ClockSyncState;
use crate::error::GatewayError;
use crate::rate_limit::RateLimitTracker;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Primary USDⓈ-M Futures WS API request/response session (`/ws-fapi/v1`).
pub struct FuturesWsSession {
    url: String,
    creds: Arc<Ed25519Credentials>,
    rate_limits: Arc<RateLimitTracker>,
    clock: ClockSyncState,
    timeout: Duration,
    next_id: AtomicU64,
    connected: AtomicBool,
    stream: Mutex<Option<WsStream>>,
}

impl std::fmt::Debug for FuturesWsSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FuturesWsSession")
            .field("url", &self.url)
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

impl FuturesWsSession {
    /// Create a new (disconnected) Futures WS API session targeting `url`.
    #[must_use]
    pub fn new(
        url: impl Into<String>,
        creds: Arc<Ed25519Credentials>,
        rate_limits: Arc<RateLimitTracker>,
        timeout: Duration,
    ) -> Self {
        Self::with_clock(url, creds, rate_limits, ClockSyncState::new(), timeout)
    }

    /// Create a new (disconnected) Futures WS API session sharing `clock` with REST.
    #[must_use]
    pub fn with_clock(
        url: impl Into<String>,
        creds: Arc<Ed25519Credentials>,
        rate_limits: Arc<RateLimitTracker>,
        clock: ClockSyncState,
        timeout: Duration,
    ) -> Self {
        Self {
            url: url.into(),
            creds,
            rate_limits,
            clock,
            timeout,
            next_id: AtomicU64::new(1),
            connected: AtomicBool::new(false),
            stream: Mutex::new(None),
        }
    }

    /// Return the shared clock synchronization state used for request timestamps.
    #[must_use]
    pub const fn clock_state(&self) -> &ClockSyncState {
        &self.clock
    }

    /// Returns `true` if a live WebSocket connection is currently held.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    /// Connect (or reconnect) to the configured `/ws-fapi/v1` endpoint.
    ///
    /// # Errors
    /// Returns [`GatewayError::Transport`] if the WebSocket handshake fails or times out.
    pub async fn connect(&self) -> Result<(), GatewayError> {
        let (ws, _resp) =
            tokio::time::timeout(self.timeout, tokio_tungstenite::connect_async(&self.url))
                .await
                .map_err(|_| GatewayError::Transport("Futures WS connect timeout".into()))?
                .map_err(|e| GatewayError::Transport(e.to_string()))?;
        let mut guard = self.stream.lock().await;
        *guard = Some(ws);
        self.connected.store(true, Ordering::Release);
        Ok(())
    }

    /// Close the active WebSocket connection if open.
    pub async fn close(&self) {
        let mut guard = self.stream.lock().await;
        self.connected.store(false, Ordering::Release);
        if let Some(mut ws) = guard.take() {
            let _ = ws.close(None).await;
        }
    }

    /// Send a signed WS API request (`method`, `params`) and await its correlated JSON `result`.
    ///
    /// # Errors
    /// Returns [`GatewayError::Transport`] on transport/disconnect failure (allowing caller to fall
    /// back to REST), or a classified [`GatewayError`] if Binance returns an API error payload.
    pub async fn request(
        &self,
        method: &str,
        mut params: BTreeMap<String, String>,
    ) -> Result<Value, GatewayError> {
        let ts_ms = self.clock.adjusted_now_ms(now_unix_ms());
        params.insert("apiKey".to_owned(), self.creds.api_key().to_owned());
        params.insert("timestamp".to_owned(), ts_ms.to_string());
        let signature = self.creds.sign_params(&params);
        params.insert("signature".to_owned(), signature);

        let req_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut params_obj = Map::new();
        for (k, v) in params {
            params_obj.insert(k, Value::String(v));
        }
        let frame = json!({
            "id": req_id,
            "method": method,
            "params": Value::Object(params_obj),
        });

        let mut guard = self.stream.lock().await;
        let Some(ws) = guard.as_mut() else {
            self.connected.store(false, Ordering::Release);
            return Err(GatewayError::Transport(
                "Futures WS session not connected".into(),
            ));
        };

        if let Err(err) = ws.send(Message::Text(frame.to_string().into())).await {
            *guard = None;
            self.connected.store(false, Ordering::Release);
            return Err(GatewayError::Transport(err.to_string()));
        }

        let read_res = tokio::time::timeout(self.timeout, async {
            while let Some(msg) = ws.next().await {
                match msg {
                    Ok(Message::Text(txt)) => {
                        let parsed: Value = serde_json::from_str(&txt)
                            .map_err(|e| GatewayError::WireFormat(e.to_string()))?;
                        if parsed.get("id").and_then(Value::as_u64) == Some(req_id) {
                            return Ok(parsed);
                        }
                    }
                    Ok(Message::Ping(payload)) => {
                        let _ = ws.send(Message::Pong(payload)).await;
                    }
                    Ok(Message::Close(_)) => {
                        return Err(GatewayError::Transport("Futures WS closed by peer".into()));
                    }
                    Ok(_) => {}
                    Err(err) => return Err(GatewayError::Transport(err.to_string())),
                }
            }
            Err(GatewayError::Transport("Futures WS stream ended".into()))
        })
        .await;

        let parsed = match read_res {
            Ok(Ok(val)) => val,
            Ok(Err(err)) => {
                *guard = None;
                self.connected.store(false, Ordering::Release);
                return Err(err);
            }
            Err(_) => {
                *guard = None;
                self.connected.store(false, Ordering::Release);
                return Err(GatewayError::Transport(
                    "Futures WS response timeout".into(),
                ));
            }
        };

        // Track rate limits if present in response.
        record_ws_rate_limits(&self.rate_limits, &parsed);

        let status = u16::try_from(parsed.get("status").and_then(Value::as_u64).unwrap_or(500))
            .unwrap_or(500);
        if status == 200 {
            parsed
                .get("result")
                .cloned()
                .ok_or_else(|| GatewayError::WireFormat("missing 'result' in WS response".into()))
        } else {
            let code = parsed.pointer("/error/code").and_then(Value::as_i64);
            let msg = parsed
                .pointer("/error/msg")
                .and_then(Value::as_str)
                .unwrap_or("unknown Futures WS error")
                .to_owned();
            Err(GatewayError::from_exchange_error(
                status, code, msg, None, None,
            ))
        }
    }
}

fn record_ws_rate_limits(rate_limits: &RateLimitTracker, parsed: &Value) {
    if let Some(limits) = parsed.get("rateLimits").and_then(Value::as_array) {
        for item in limits {
            if item.get("rateLimitType").and_then(Value::as_str) == Some("REQUEST_WEIGHT")
                && let Some(count) = item.get("count").and_then(Value::as_u64)
            {
                rate_limits.record_readings(&[crate::rate_limit::RateLimitReading {
                    rate_limit_type: crate::rate_limit::RateLimitType::RequestWeight,
                    interval: crate::rate_limit::RateLimitInterval::Minute,
                    interval_num: 1,
                    limit: 2400,
                    count,
                }]);
            }
        }
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
