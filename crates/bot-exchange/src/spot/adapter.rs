//! `SpotAdapter` implementing `ExchangeGateway` and `VenueAdapter` (§3.5, §4.4).

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bot_types::{Symbol, Venue, VenueCaps};
use tokio::sync::{RwLock, mpsc};

use crate::auth::Ed25519Credentials;
use crate::error::{GatewayError, SessionState};
use crate::gateway::{
    AccountSnapshot, ActivationReport, CancelOrderAck, CancelOrderListAck, CancelOrderListRequest,
    CancelOrderRequest, CancelReplaceAck, CancelReplaceRequest, ExchangeGateway,
    ExchangeInfoSnapshot, GatewayEvent, OcoOrderAck, OrderAck, OrderSnapshot, PlaceOcoRequest,
    PlaceOrderRequest, QueryOrderRequest, VenueAdapter,
};
use crate::rate_limit::RateLimitTracker;
use crate::spot::rest::SpotRestClient;
use crate::spot::session::{SpotSessionConfig, SpotWsSession};
use crate::spot::wire::{
    build_account_status_params, build_cancel_list_params, build_cancel_order_params,
    build_cancel_replace_params, build_open_orders_params, build_place_oco_params,
    build_place_order_params, build_query_order_params, parse_account_snapshot,
    parse_cancel_list_ack, parse_cancel_order_ack, parse_cancel_replace_ack, parse_oco_ack,
    parse_order_ack, parse_order_snapshot,
};

/// Configuration for [`SpotAdapter`].
#[derive(Debug, Clone)]
pub struct SpotAdapterConfig {
    /// REST API base URL (default: Spot Testnet).
    pub rest_url: String,
    /// REST HTTP timeout (default: 10 s).
    pub rest_timeout: Duration,
    /// Persistent WS API session settings.
    pub session: SpotSessionConfig,
}

impl Default for SpotAdapterConfig {
    fn default() -> Self {
        Self {
            rest_url: SpotRestClient::TESTNET_URL.to_owned(),
            rest_timeout: Duration::from_secs(10),
            session: SpotSessionConfig::default(),
        }
    }
}

/// Binance Spot [`VenueAdapter`] combining REST bootstrap (`time`, `exchangeInfo`)
/// and a single persistent WS API session (`order.*`, `orderList.*`, `account.status`,
/// and `userDataStream.subscribe`).
pub struct SpotAdapter {
    config: SpotAdapterConfig,
    rest: SpotRestClient,
    credentials: Arc<Ed25519Credentials>,
    rate_limits: Arc<RateLimitTracker>,
    clock_offset_ms: Arc<AtomicI64>,
    session: RwLock<Option<SpotWsSession>>,
    cached_exchange_info: RwLock<Option<ExchangeInfoSnapshot>>,
    event_rx: Mutex<Option<mpsc::Receiver<GatewayEvent>>>,
}

impl std::fmt::Debug for SpotAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpotAdapter")
            .field("rest_url", &self.config.rest_url)
            .field("ws_url", &self.config.session.ws_url)
            .field("credentials", &self.credentials)
            .finish_non_exhaustive()
    }
}

impl SpotAdapter {
    /// Construct an inactive [`SpotAdapter`]. Call [`VenueAdapter::activate`] to connect.
    ///
    /// # Errors
    /// Returns [`GatewayError::Http`] if the REST client cannot be built.
    pub fn new(
        config: SpotAdapterConfig,
        credentials: Ed25519Credentials,
    ) -> Result<Self, GatewayError> {
        let rate_limits = Arc::new(RateLimitTracker::new());
        let rest = SpotRestClient::with_rate_limits(
            &config.rest_url,
            config.rest_timeout,
            Arc::clone(&rate_limits),
        )?;
        Ok(Self {
            config,
            rest,
            credentials: Arc::new(credentials),
            rate_limits,
            clock_offset_ms: Arc::new(AtomicI64::new(0)),
            session: RwLock::new(None),
            cached_exchange_info: RwLock::new(None),
            event_rx: Mutex::new(None),
        })
    }

    /// Take the bounded `mpsc::Receiver<GatewayEvent>` populated during [`VenueAdapter::activate`].
    #[must_use]
    pub fn take_event_rx(&self) -> Option<mpsc::Receiver<GatewayEvent>> {
        self.event_rx.lock().ok().and_then(|mut g| g.take())
    }

    /// Access the shared rate limit tracker for this Spot adapter.
    #[must_use]
    pub fn rate_limits(&self) -> &Arc<RateLimitTracker> {
        &self.rate_limits
    }

    /// Current WS API session state (`SessionState::Disconnected` if inactive).
    pub async fn session_state(&self) -> SessionState {
        self.session
            .read()
            .await
            .as_ref()
            .map_or(SessionState::Disconnected, SpotWsSession::state)
    }

    /// Trigger an immediate make-before-break WS API rotation on the active session.
    ///
    /// # Errors
    /// Returns [`GatewayError`] if inactive or if the replacement connection fails.
    pub async fn rotate_session_now(&self) -> Result<(), GatewayError> {
        let sess = self.active_session().await?;
        sess.rotate_now().await
    }

    async fn active_session(&self) -> Result<SpotWsSession, GatewayError> {
        self.session
            .read()
            .await
            .clone()
            .ok_or(GatewayError::SessionNotReady(SessionState::Disconnected))
    }
}

#[async_trait]
impl ExchangeGateway for SpotAdapter {
    async fn place_order(&self, req: PlaceOrderRequest) -> Result<OrderAck, GatewayError> {
        let sess = self.active_session().await?;
        let params = build_place_order_params(
            &req,
            sess.server_now_ms(),
            self.config.session.recv_window_ms,
        )?;
        let val = sess
            .request("order.place", Some(params), Some(req.client_order_id))
            .await?;
        parse_order_ack(&val)
    }

    async fn cancel_order(&self, req: CancelOrderRequest) -> Result<CancelOrderAck, GatewayError> {
        let sess = self.active_session().await?;
        let params = build_cancel_order_params(
            &req,
            sess.server_now_ms(),
            self.config.session.recv_window_ms,
        )?;
        let val = sess.request("order.cancel", Some(params), None).await?;
        parse_cancel_order_ack(&val)
    }

    async fn cancel_replace(
        &self,
        req: CancelReplaceRequest,
    ) -> Result<CancelReplaceAck, GatewayError> {
        let sess = self.active_session().await?;
        let params = build_cancel_replace_params(
            &req,
            sess.server_now_ms(),
            self.config.session.recv_window_ms,
        )?;
        let val = sess
            .request(
                "order.cancelReplace",
                Some(params),
                Some(req.new_client_order_id),
            )
            .await?;
        parse_cancel_replace_ack(&val)
    }

    async fn place_oco(&self, req: PlaceOcoRequest) -> Result<OcoOrderAck, GatewayError> {
        let sess = self.active_session().await?;
        let params = build_place_oco_params(
            &req,
            sess.server_now_ms(),
            self.config.session.recv_window_ms,
        )?;
        let val = sess
            .request(
                "orderList.place.oco",
                Some(params),
                Some(req.list_client_order_id),
            )
            .await?;
        parse_oco_ack(&val)
    }

    async fn cancel_list(
        &self,
        req: CancelOrderListRequest,
    ) -> Result<CancelOrderListAck, GatewayError> {
        let sess = self.active_session().await?;
        let params = build_cancel_list_params(
            &req,
            sess.server_now_ms(),
            self.config.session.recv_window_ms,
        )?;
        let val = sess.request("orderList.cancel", Some(params), None).await?;
        parse_cancel_list_ack(&val)
    }

    async fn query_order(&self, req: QueryOrderRequest) -> Result<OrderSnapshot, GatewayError> {
        let sess = self.active_session().await?;
        let params = build_query_order_params(
            &req,
            sess.server_now_ms(),
            self.config.session.recv_window_ms,
        )?;
        let val = sess.request("order.status", Some(params), None).await?;
        parse_order_snapshot(&val)
    }

    async fn open_orders(
        &self,
        symbol: Option<&Symbol>,
    ) -> Result<Vec<OrderSnapshot>, GatewayError> {
        let sess = self.active_session().await?;
        let params = build_open_orders_params(
            symbol,
            sess.server_now_ms(),
            self.config.session.recv_window_ms,
        );
        let val = sess
            .request("openOrders.status", Some(params), None)
            .await?;
        let arr = val
            .as_array()
            .ok_or_else(|| GatewayError::WireFormat("openOrders.status result not array".into()))?;
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            out.push(parse_order_snapshot(item)?);
        }
        Ok(out)
    }

    async fn account_snapshot(&self) -> Result<AccountSnapshot, GatewayError> {
        let sess = self.active_session().await?;
        let params =
            build_account_status_params(sess.server_now_ms(), self.config.session.recv_window_ms);
        let val = sess.request("account.status", Some(params), None).await?;
        parse_account_snapshot(&val)
    }

    async fn exchange_info(&self) -> Result<ExchangeInfoSnapshot, GatewayError> {
        let info = self.rest.exchange_info().await?;
        self.rate_limits.apply_rules(&info.rate_limits);
        *self.cached_exchange_info.write().await = Some(info.clone());
        Ok(info)
    }
}

#[async_trait]
impl VenueAdapter for SpotAdapter {
    fn venue(&self) -> Venue {
        Venue::Spot
    }

    fn caps(&self) -> VenueCaps {
        VenueCaps::spot()
    }

    async fn activate(&self) -> Result<ActivationReport, GatewayError> {
        // 1. Synchronize server clock via REST GET /api/v3/time.
        let sync = self.rest.server_time().await?;
        self.clock_offset_ms
            .store(sync.offset_ms, Ordering::Relaxed);

        // 2. Load exchangeInfo (symbol rules + rateLimits) via REST.
        let info = self.exchange_info().await?;
        let symbols_loaded = info.rules.len();

        // 3. Establish persistent WS API session (connect -> session.logon -> userDataStream.subscribe).
        let (ws_session, rx, subscription_id) = SpotWsSession::connect(
            self.config.session.clone(),
            Arc::clone(&self.credentials),
            Arc::clone(&self.rate_limits),
            Arc::clone(&self.clock_offset_ms),
        )
        .await?;

        *self.session.write().await = Some(ws_session);
        if let Ok(mut guard) = self.event_rx.lock() {
            *guard = Some(rx);
        }

        Ok(ActivationReport {
            venue: Venue::Spot,
            server_time_offset_ms: sync.offset_ms,
            symbols_loaded,
            subscription_id,
        })
    }

    async fn deactivate(&self) -> Result<(), GatewayError> {
        // Hard rule #7: deactivate cleanly closes the session and NEVER cancels protective orders.
        let existing = self.session.write().await.take();
        if let Some(sess) = existing {
            sess.shutdown().await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{SpotAdapter, SpotAdapterConfig};
    use crate::auth::Ed25519Credentials;
    use crate::gateway::ExchangeGateway;
    use crate::rate_limit::{RateLimitBucketKey, RateLimitInterval, RateLimitType};

    #[tokio::test]
    async fn spot_adapter_shares_rate_limit_tracker_with_rest_client() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await.expect("read");
            let body = r#"{"serverTime":1700000000000,"rateLimits":[{"rateLimitType":"REQUEST_WEIGHT","interval":"MINUTE","intervalNum":1,"limit":6000}],"symbols":[]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-MBX-USED-WEIGHT-1M: 77\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.expect("write");
        });

        let creds = Ed25519Credentials::from_seed("test-key".to_owned(), [9u8; 32]).expect("creds");
        let cfg = SpotAdapterConfig {
            rest_url: format!("http://{addr}"),
            rest_timeout: Duration::from_secs(2),
            ..SpotAdapterConfig::default()
        };
        let adapter = SpotAdapter::new(cfg, creds).expect("adapter");
        assert!(
            Arc::ptr_eq(adapter.rate_limits(), adapter.rest.rate_limits()),
            "SpotAdapter and SpotRestClient must share the same RateLimitTracker"
        );

        let _info = adapter.exchange_info().await.expect("exchange_info");
        let snap = adapter.rate_limits().snapshot();
        let weight = snap
            .get(&RateLimitBucketKey {
                rate_limit_type: RateLimitType::RequestWeight,
                interval: RateLimitInterval::Minute,
                interval_num: 1,
            })
            .expect("weight bucket populated from REST header");
        assert_eq!(weight.count, 77);
        assert_eq!(weight.limit, 6000);

        server.await.expect("server join");
    }
}
