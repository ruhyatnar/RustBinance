//! USDⓈ-M Futures `FuturesAdapter` with WS API (`/ws-fapi/v1`) primary transport,
//! REST (`/fapi/v1`) fallback + pre-activation account safety checks, and isolated `/fapi`
//! rate-limit tracking (§4.5).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bot_types::{Symbol, Venue, VenueCaps};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

use crate::auth::Ed25519Credentials;
use crate::balance_cache::{BalanceCache, BalanceProvenance, BalanceSource, CachedAccountSnapshot};
use crate::clock::ClockSyncState;
use crate::error::{GatewayError, enforce_order_rate_budget};
use crate::futures::rest::{
    FuturesRestClient, build_place_order_params, parse_account_snapshot, parse_algo_order_ack,
    parse_cancel_ack, parse_cancel_algo_ack, parse_order_ack, parse_order_snapshot,
};
use crate::futures::session::FuturesWsSession;
use crate::futures::wire::{
    build_cancel_algo_order_params, build_place_algo_order_params, parse_futures_user_gateway_event,
};
use crate::gateway::{
    AccountEvent, AccountSnapshot, ActivationReport, AlgoOrderAck, CancelAlgoOrderAck,
    CancelAlgoOrderRequest, CancelOrderAck, CancelOrderListAck, CancelOrderListRequest,
    CancelOrderRequest, CancelReplaceAck, CancelReplaceRequest, ExchangeGateway,
    ExchangeInfoSnapshot, GatewayEvent, OcoOrderAck, OrderAck, OrderSnapshot,
    PlaceAlgoOrderRequest, PlaceOcoRequest, PlaceOrderRequest, QueryOrderRequest, VenueAdapter,
};
use crate::rate_limit::RateLimitTracker;

/// Margin mode reported by Binance USDⓈ-M Futures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FuturesMarginType {
    /// Isolated margin (required by §4.5).
    Isolated,
    /// Cross margin (rejected at activation).
    Crossed,
}

/// Snapshot of USDⓈ-M Futures account configuration inspected during `activate()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuturesAccountSetup {
    /// `true` = Hedge Mode (rejected); `false` = One-Way Mode (required).
    pub dual_side_position: bool,
    /// `true` = Multi-Assets Mode ON (rejected); `false` = Single-Asset (required).
    pub multi_assets_margin: bool,
    /// Active margin type for target symbols (`Isolated` required).
    pub margin_type: FuturesMarginType,
    /// Configured leverage (`<= max_leverage_cap` required).
    pub leverage: u32,
    /// Whether any non-zero Futures position is currently open (must be `false` for activation).
    pub has_open_positions: bool,
}

impl FuturesAccountSetup {
    /// Validate that the Futures account meets all safety preconditions before activation.
    ///
    /// Never mutates exchange state when preconditions fail (hard safety rule: reject, do not force).
    ///
    /// # Errors
    /// Returns [`GatewayError::InvalidRequest`] describing the violated safety precondition.
    pub fn validate_for_activation(&self, max_leverage_cap: u32) -> Result<(), GatewayError> {
        if self.has_open_positions {
            return Err(GatewayError::InvalidRequest(
                "Futures activation rejected: account is not flat (open positions detected)".into(),
            ));
        }
        if self.dual_side_position {
            return Err(GatewayError::InvalidRequest(
                "Futures activation rejected: Hedge Mode detected (One-Way mode required)".into(),
            ));
        }
        if self.multi_assets_margin {
            return Err(GatewayError::InvalidRequest(
                "Futures activation rejected: Multi-Assets margin is ON (must be OFF)".into(),
            ));
        }
        if self.margin_type != FuturesMarginType::Isolated {
            return Err(GatewayError::InvalidRequest(
                "Futures activation rejected: Cross margin detected (ISOLATED required)".into(),
            ));
        }
        if self.leverage == 0 || self.leverage > max_leverage_cap {
            return Err(GatewayError::InvalidRequest(format!(
                "Futures activation rejected: leverage {} exceeds cap {}",
                self.leverage, max_leverage_cap
            )));
        }
        Ok(())
    }
}

/// Configuration for [`FuturesAdapter`].
#[derive(Debug, Clone)]
pub struct FuturesAdapterConfig {
    /// REST base URL (e.g. `https://testnet.binancefuture.com`).
    pub rest_base_url: String,
    /// WS API URL (e.g. `wss://testnet.binancefuture.com/ws-fapi/v1`).
    pub ws_api_url: String,
    /// Hard leverage ceiling enforced during activation.
    pub max_leverage_cap: u32,
}

/// Default REST reconcile interval for the Futures WS-first balance cache (5 minutes).
const DEFAULT_BALANCE_RECONCILE_INTERVAL_MS: u64 = 300_000;
/// Bounded capacity for the Futures user-data event channel (§3.3).
const USER_EVENT_CHANNEL_CAPACITY: usize = 4096;
/// Keepalive interval for Binance Futures `listenKey` (30 minutes; expires after 60 minutes).
const LISTEN_KEY_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30 * 60);

struct LiveTransport {
    ws: FuturesWsSession,
    rest: FuturesRestClient,
}

struct UserStreamTask {
    shutdown_tx: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

/// USDⓈ-M Futures adapter implementing [`ExchangeGateway`] and [`VenueAdapter`] (§4.5).
pub struct FuturesAdapter {
    config: FuturesAdapterConfig,
    /// Optional explicit base URL for the Futures User Data Stream (defaults from `ws_api_url`).
    user_stream_base_url: Option<String>,
    /// Isolated `/fapi` rate-limit tracker (completely separate from Spot).
    rate_limits: Arc<RateLimitTracker>,
    /// Shared clock synchronization state between Futures REST and WS API.
    clock: ClockSyncState,
    /// WS-first account balance cache with provenance tracking.
    balance_cache: Arc<BalanceCache>,
    /// Whether the Futures user-data stream is currently connected and delivering updates.
    user_stream_ready: Arc<AtomicBool>,
    /// Offline/injected account setup snapshot (used when constructed without live transport).
    account_setup: Option<FuturesAccountSetup>,
    /// WS API primary + REST fallback transport (present when constructed via `with_transport`).
    transport: Option<LiveTransport>,
    /// Bounded receiver taken once by the execution engine (`take_event_rx()`).
    event_rx: StdMutex<Option<mpsc::Receiver<GatewayEvent>>>,
    /// Sender kept for user-data stream task and offline frame injection.
    event_tx: StdMutex<Option<mpsc::Sender<GatewayEvent>>>,
    /// Background `listenKey` user-data stream task handle, if active.
    user_stream_task: AsyncMutex<Option<UserStreamTask>>,
}

impl FuturesAdapter {
    /// Construct an offline [`FuturesAdapter`] with a static `account_setup` snapshot (for unit tests).
    #[must_use]
    pub fn new(config: FuturesAdapterConfig, account_setup: FuturesAccountSetup) -> Self {
        Self {
            config,
            user_stream_base_url: None,
            rate_limits: Arc::new(RateLimitTracker::new()),
            clock: ClockSyncState::new(),
            balance_cache: Arc::new(BalanceCache::new(
                Venue::UsdsFutures,
                DEFAULT_BALANCE_RECONCILE_INTERVAL_MS,
            )),
            user_stream_ready: Arc::new(AtomicBool::new(false)),
            account_setup: Some(account_setup),
            transport: None,
            event_rx: StdMutex::new(None),
            event_tx: StdMutex::new(None),
            user_stream_task: AsyncMutex::new(None),
        }
    }

    /// Construct a network-capable [`FuturesAdapter`] using WS API (`/ws-fapi/v1`) as primary
    /// transport and REST (`/fapi/v1`) for setup metadata and automatic fallback.
    ///
    /// # Errors
    /// Returns [`GatewayError::Http`] if the underlying REST client fails to initialize.
    pub fn with_transport(
        config: FuturesAdapterConfig,
        creds: Arc<Ed25519Credentials>,
        timeout: Duration,
    ) -> Result<Self, GatewayError> {
        let rate_limits = Arc::new(RateLimitTracker::new());
        let rest = FuturesRestClient::new(
            config.rest_base_url.clone(),
            timeout,
            Some(Arc::clone(&creds)),
            Arc::clone(&rate_limits),
        )?;
        let clock = rest.clock_state().clone();
        let ws = FuturesWsSession::with_clock(
            config.ws_api_url.clone(),
            creds,
            Arc::clone(&rate_limits),
            clock.clone(),
            timeout,
        );
        Ok(Self {
            config,
            user_stream_base_url: None,
            rate_limits,
            clock,
            balance_cache: Arc::new(BalanceCache::new(
                Venue::UsdsFutures,
                DEFAULT_BALANCE_RECONCILE_INTERVAL_MS,
            )),
            user_stream_ready: Arc::new(AtomicBool::new(false)),
            account_setup: None,
            transport: Some(LiveTransport { ws, rest }),
            event_rx: StdMutex::new(None),
            event_tx: StdMutex::new(None),
            user_stream_task: AsyncMutex::new(None),
        })
    }

    /// Override the base WebSocket URL used for the Futures User Data Stream (`/ws/<listenKey>`).
    #[must_use]
    pub fn with_user_stream_base_url(mut self, url: impl Into<String>) -> Self {
        self.user_stream_base_url = Some(url.into().trim_end_matches('/').to_owned());
        self
    }

    /// Take the bounded `mpsc::Receiver<GatewayEvent>` populated during [`VenueAdapter::activate`],
    /// mirroring [`crate::SpotAdapter::take_event_rx`].
    #[must_use]
    pub fn take_event_rx(&self) -> Option<mpsc::Receiver<GatewayEvent>> {
        self.event_rx.lock().ok().and_then(|mut g| g.take())
    }

    /// Install an injectable user-data event channel (primarily for offline/mock tests) and mark
    /// the user-data stream ready, returning the sender end.
    #[must_use]
    pub fn inject_event_channel(&self) -> mpsc::Sender<GatewayEvent> {
        let (tx, rx) = mpsc::channel(USER_EVENT_CHANNEL_CAPACITY);
        if let Ok(mut guard) = self.event_rx.lock() {
            *guard = Some(rx);
        }
        if let Ok(mut guard) = self.event_tx.lock() {
            *guard = Some(tx.clone());
        }
        self.user_stream_ready.store(true, Ordering::Release);
        tx
    }

    /// Parse and process a raw Futures User Data Stream JSON frame (`ORDER_TRADE_UPDATE`,
    /// `ACCOUNT_UPDATE`, `ALGO_UPDATE`, `listenKeyExpired`), updating [`BalanceCache`] and
    /// forwarding the translated [`GatewayEvent`] to the active event channel if present.
    ///
    /// # Errors
    /// Returns [`GatewayError::WireFormat`] if the frame is malformed.
    pub fn ingest_user_data_frame(&self, raw: &str) -> Result<Option<GatewayEvent>, GatewayError> {
        let maybe_event = parse_futures_user_gateway_event(raw)?;
        if let Some(ref event) = maybe_event {
            apply_event_to_balance_cache(&self.balance_cache, &self.user_stream_ready, event);
            if let Ok(guard) = self.event_tx.lock()
                && let Some(tx) = guard.as_ref()
            {
                let _ = tx.try_send(event.clone());
            }
        }
        Ok(maybe_event)
    }

    /// Borrow the isolated `/fapi` rate-limit tracker.
    #[must_use]
    pub fn rate_limits(&self) -> &Arc<RateLimitTracker> {
        &self.rate_limits
    }

    /// Borrow the shared clock synchronization state used by both Futures REST and WS API.
    #[must_use]
    pub const fn clock_state(&self) -> &ClockSyncState {
        &self.clock
    }

    /// Borrow the WS-first account balance cache for this Futures adapter.
    #[must_use]
    pub fn balance_cache(&self) -> &Arc<BalanceCache> {
        &self.balance_cache
    }

    /// Return the current WS-cached account snapshot if fresh, otherwise fetch and seed from
    /// [`ExchangeGateway::account_snapshot`].
    ///
    /// # Errors
    /// Returns [`GatewayError`] if the cache is stale/unseeded and the fallback snapshot query fails.
    pub async fn cached_account_snapshot(&self) -> Result<CachedAccountSnapshot, GatewayError> {
        let now_ms = now_unix_ms();
        let ws_ready = self.user_stream_ready.load(Ordering::Acquire);
        if let Some(cached) = self.balance_cache.get_if_fresh(ws_ready, now_ms) {
            return Ok(cached);
        }
        let snapshot = self.account_snapshot().await?;
        Ok(CachedAccountSnapshot {
            snapshot,
            provenance: BalanceProvenance {
                source: BalanceSource::Rest,
                age_ms: 0,
            },
        })
    }

    /// Return the current [`BalanceProvenance`] if the WS balance cache is currently fresh.
    #[must_use]
    pub fn balance_provenance(&self) -> Option<BalanceProvenance> {
        let ws_ready = self.user_stream_ready.load(Ordering::Acquire);
        self.balance_cache
            .get_if_fresh(ws_ready, now_unix_ms())
            .map(|c| c.provenance)
    }

    /// Borrow adapter configuration.
    #[must_use]
    pub fn config(&self) -> &FuturesAdapterConfig {
        &self.config
    }

    fn resolved_user_stream_base_url(&self) -> String {
        if let Some(ref explicit) = self.user_stream_base_url {
            return explicit.clone();
        }
        derive_user_stream_base_url(&self.config.ws_api_url)
    }

    fn require_transport(&self, op: &str) -> Result<&LiveTransport, GatewayError> {
        self.transport.as_ref().ok_or_else(|| {
            GatewayError::Unsupported(format!(
                "FuturesAdapter '{op}' requires transport (constructed via FuturesAdapter::with_transport)"
            ))
        })
    }
}

/// Decide whether a Futures WS API failure may safely fall back to REST.
///
/// Never falls back on non-retryable / permanent errors (such as `-1022`, `-2014`, `-2015`,
/// `-4046`, `-4059`, `InvalidRequest`, `IpBanned`, `TimestampOutOfWindow`) or ambiguous
/// `UnknownNeedsQuery` / `RateLimited` responses.
fn should_fallback_to_rest(err: &GatewayError) -> bool {
    if !err.is_retryable() {
        return false;
    }
    matches!(
        err,
        GatewayError::Transport(_) | GatewayError::SessionNotReady(_)
    )
}

#[async_trait]
impl ExchangeGateway for FuturesAdapter {
    async fn place_order(&self, req: PlaceOrderRequest) -> Result<OrderAck, GatewayError> {
        enforce_order_rate_budget(&self.rate_limits, now_unix_ms())?;
        let tr = self.require_transport("place_order")?;
        let params = build_place_order_params(&req);
        match tr.ws.request("order.place", params).await {
            Ok(val) => parse_order_ack(&val.to_string()),
            Err(err) if should_fallback_to_rest(&err) => {
                warn!(error = %err, "Futures WS order.place failed; falling back to REST /fapi/v1/order");
                tr.rest.place_order(&req).await
            }
            Err(other) => Err(other),
        }
    }

    async fn cancel_order(&self, req: CancelOrderRequest) -> Result<CancelOrderAck, GatewayError> {
        let tr = self.require_transport("cancel_order")?;
        let mut params = BTreeMap::new();
        params.insert("symbol".to_owned(), req.symbol.as_str().to_owned());
        if let Some(id) = req.order_id {
            params.insert("orderId".to_owned(), id.to_string());
        }
        if let Some(cid) = &req.orig_client_order_id {
            params.insert("origClientOrderId".to_owned(), cid.clone());
        }
        match tr.ws.request("order.cancel", params).await {
            Ok(val) => parse_cancel_ack(&val.to_string()),
            Err(err) if should_fallback_to_rest(&err) => {
                warn!(error = %err, "Futures WS order.cancel failed; falling back to REST /fapi/v1/order");
                tr.rest.cancel_order(&req).await
            }
            Err(other) => Err(other),
        }
    }

    async fn cancel_replace(
        &self,
        _req: CancelReplaceRequest,
    ) -> Result<CancelReplaceAck, GatewayError> {
        Err(GatewayError::Unsupported(
            "cancel_replace is not supported on USDⓈ-M Futures".into(),
        ))
    }

    async fn place_oco(&self, _req: PlaceOcoRequest) -> Result<OcoOrderAck, GatewayError> {
        Err(GatewayError::Unsupported(
            "place_oco is Spot-only; USDⓈ-M Futures uses place_algo_order".into(),
        ))
    }

    async fn cancel_list(
        &self,
        _req: CancelOrderListRequest,
    ) -> Result<CancelOrderListAck, GatewayError> {
        Err(GatewayError::Unsupported(
            "cancel_list is Spot-only; USDⓈ-M Futures uses cancel_algo_order".into(),
        ))
    }

    async fn query_order(&self, req: QueryOrderRequest) -> Result<OrderSnapshot, GatewayError> {
        let tr = self.require_transport("query_order")?;
        let mut params = BTreeMap::new();
        params.insert("symbol".to_owned(), req.symbol.as_str().to_owned());
        if let Some(id) = req.order_id {
            params.insert("orderId".to_owned(), id.to_string());
        }
        if let Some(cid) = &req.orig_client_order_id {
            params.insert("origClientOrderId".to_owned(), cid.clone());
        }
        match tr.ws.request("order.status", params).await {
            Ok(val) => parse_order_snapshot(&val.to_string()),
            Err(err) if should_fallback_to_rest(&err) => {
                warn!(error = %err, "Futures WS order.status failed; falling back to REST /fapi/v1/order");
                tr.rest.query_order(&req).await
            }
            Err(other) => Err(other),
        }
    }

    async fn open_orders(
        &self,
        symbol: Option<&Symbol>,
    ) -> Result<Vec<OrderSnapshot>, GatewayError> {
        let tr = self.require_transport("open_orders")?;
        tr.rest.open_orders(symbol).await
    }

    async fn account_snapshot(&self) -> Result<AccountSnapshot, GatewayError> {
        let tr = self.require_transport("account_snapshot")?;
        let snap = match tr.ws.request("v2/account.status", BTreeMap::new()).await {
            Ok(val) => parse_account_snapshot(&val.to_string())?,
            Err(err) if should_fallback_to_rest(&err) => {
                warn!(error = %err, "Futures WS v2/account.status failed; falling back to REST /fapi/v2/account");
                tr.rest.account_snapshot().await?
            }
            Err(other) => return Err(other),
        };
        self.balance_cache
            .seed_from_rest(snap.clone(), now_unix_ms());
        Ok(snap)
    }

    async fn exchange_info(&self) -> Result<ExchangeInfoSnapshot, GatewayError> {
        let tr = self.require_transport("exchange_info")?;
        let info = tr.rest.exchange_info().await?;
        self.rate_limits.apply_rules(&info.rate_limits);
        Ok(info)
    }

    async fn place_algo_order(
        &self,
        req: PlaceAlgoOrderRequest,
    ) -> Result<AlgoOrderAck, GatewayError> {
        enforce_order_rate_budget(&self.rate_limits, now_unix_ms())?;
        let params = crate::futures::rest::json_map_to_btree(build_place_algo_order_params(&req)?);
        let tr = self.require_transport("place_algo_order")?;
        match tr.ws.request("algoOrder.place", params).await {
            Ok(val) => parse_algo_order_ack(&val.to_string()),
            Err(err) if should_fallback_to_rest(&err) => {
                warn!(error = %err, "Futures WS algoOrder.place failed; falling back to REST /fapi/v1/algoOrder");
                tr.rest.place_algo_order(&req).await
            }
            Err(other) => Err(other),
        }
    }

    async fn cancel_algo_order(
        &self,
        req: CancelAlgoOrderRequest,
    ) -> Result<CancelAlgoOrderAck, GatewayError> {
        let params = crate::futures::rest::json_map_to_btree(build_cancel_algo_order_params(&req)?);
        let tr = self.require_transport("cancel_algo_order")?;
        match tr.ws.request("algoOrder.cancel", params).await {
            Ok(val) => parse_cancel_algo_ack(&val.to_string(), &req.symbol),
            Err(err) if should_fallback_to_rest(&err) => {
                warn!(error = %err, "Futures WS algoOrder.cancel failed; falling back to REST /fapi/v1/algoOrder");
                tr.rest.cancel_algo_order(&req).await
            }
            Err(other) => Err(other),
        }
    }
}

#[async_trait]
impl VenueAdapter for FuturesAdapter {
    fn venue(&self) -> Venue {
        Venue::UsdsFutures
    }

    fn caps(&self) -> VenueCaps {
        VenueCaps::usds_futures(self.config.max_leverage_cap)
    }

    async fn activate(&self) -> Result<ActivationReport, GatewayError> {
        // Initialize user-data event channel unless caller already installed one via inject_event_channel.
        let event_tx = {
            let existing_tx = self.event_tx.lock().ok().and_then(|g| g.clone());
            if let Some(tx) = existing_tx {
                tx
            } else {
                let (tx, rx) = mpsc::channel(USER_EVENT_CHANNEL_CAPACITY);
                if let Ok(mut guard) = self.event_rx.lock() {
                    *guard = Some(rx);
                }
                if let Ok(mut guard) = self.event_tx.lock() {
                    *guard = Some(tx.clone());
                }
                tx
            }
        };

        if let Some(tr) = &self.transport {
            // 1. Synchronize server clock so both FuturesRestClient and FuturesWsSession share offset.
            let server_time_offset_ms = match tr.rest.server_time().await {
                Ok(sync) => sync.offset_ms,
                Err(err) => {
                    warn!(error = %err, "Futures server_time sync failed during activate; continuing with local clock");
                    self.clock.offset_ms().unwrap_or(0)
                }
            };

            // 2. Validate pre-activation account safety preconditions.
            let setup = tr.rest.fetch_account_setup().await?;
            setup.validate_for_activation(self.config.max_leverage_cap)?;

            // 3. Connect primary WS API session.
            if let Err(err) = tr.ws.connect().await {
                warn!(error = %err, "Futures WS connect failed during activate; REST fallback remains active");
            }

            // 4. Start Futures User Data Stream via listenKey (best-effort so WS-API/REST still works if unavailable).
            match tr.rest.create_listen_key().await {
                Ok(listen_key) => {
                    let stream_url =
                        format!("{}/ws/{listen_key}", self.resolved_user_stream_base_url());
                    let (shutdown_tx, shutdown_rx) = watch::channel(false);
                    let rest_clone = tr.rest.clone();
                    let cache_clone = Arc::clone(&self.balance_cache);
                    let ready_clone = Arc::clone(&self.user_stream_ready);
                    let handle = tokio::spawn(run_futures_user_stream(
                        stream_url,
                        rest_clone,
                        cache_clone,
                        ready_clone,
                        event_tx,
                        shutdown_rx,
                    ));
                    let mut task_guard = self.user_stream_task.lock().await;
                    if let Some(prev) = task_guard.replace(UserStreamTask {
                        shutdown_tx,
                        handle,
                    }) {
                        let _ = prev.shutdown_tx.send(true);
                        prev.handle.abort();
                    }
                }
                Err(err) => {
                    warn!(error = %err, "Futures listenKey creation failed during activate; user-data stream inactive");
                }
            }

            return Ok(ActivationReport {
                venue: Venue::UsdsFutures,
                server_time_offset_ms,
                subscription_id: None,
                symbols_loaded: 0,
            });
        }
        if let Some(setup) = &self.account_setup {
            setup.validate_for_activation(self.config.max_leverage_cap)?;
        }
        Ok(ActivationReport {
            venue: Venue::UsdsFutures,
            server_time_offset_ms: self.clock.offset_ms().unwrap_or(0),
            subscription_id: None,
            symbols_loaded: 0,
        })
    }

    async fn deactivate(&self) -> Result<(), GatewayError> {
        // Hard rule #7: never cancel protective Algo orders on deactivate.
        self.user_stream_ready.store(false, Ordering::Release);
        self.balance_cache.mark_ws_disconnected();

        if let Some(task) = self.user_stream_task.lock().await.take() {
            let _ = task.shutdown_tx.send(true);
            if tokio::time::timeout(Duration::from_secs(2), task.handle)
                .await
                .is_err()
            {
                debug!("Futures user-data stream task did not exit within 2s during deactivate");
            }
        }

        if let Some(tr) = &self.transport {
            let _ = tr.rest.close_listen_key().await;
            tr.ws.close().await;
        }
        Ok(())
    }
}

fn apply_event_to_balance_cache(
    balance_cache: &BalanceCache,
    user_stream_ready: &AtomicBool,
    event: &GatewayEvent,
) {
    match event {
        GatewayEvent::Account(AccountEvent::PositionUpdate {
            balances,
            event_time_ms,
            ..
        }) => {
            if !balances.is_empty() {
                balance_cache.apply_ws_delta(balances, *event_time_ms);
            }
        }
        GatewayEvent::Account(AccountEvent::BalanceDelta { .. }) => {
            balance_cache.mark_ws_disconnected();
        }
        GatewayEvent::StreamTerminated { .. } => {
            user_stream_ready.store(false, Ordering::Release);
            balance_cache.mark_ws_disconnected();
        }
        _ => {}
    }
}

async fn run_futures_user_stream(
    stream_url: String,
    rest: FuturesRestClient,
    balance_cache: Arc<BalanceCache>,
    user_stream_ready: Arc<AtomicBool>,
    event_tx: mpsc::Sender<GatewayEvent>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let ws_stream = match tokio_tungstenite::connect_async(&stream_url).await {
        Ok((ws, _resp)) => ws,
        Err(err) => {
            warn!(error = %err, "Futures user-data WS connect failed");
            user_stream_ready.store(false, Ordering::Release);
            balance_cache.mark_ws_disconnected();
            return;
        }
    };

    user_stream_ready.store(true, Ordering::Release);
    let (mut sink, mut read) = ws_stream.split();
    let mut keepalive = tokio::time::interval(LISTEN_KEY_KEEPALIVE_INTERVAL);
    // Skip the immediate first tick; keepalive runs after 30 minutes.
    keepalive.tick().await;

    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    let _ = sink.send(Message::Close(None)).await;
                    break;
                }
            }
            _ = keepalive.tick() => {
                if let Err(err) = rest.keepalive_listen_key().await {
                    warn!(error = %err, "Futures listenKey keepalive failed");
                }
            }
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        match parse_futures_user_gateway_event(txt.as_str()) {
                            Ok(Some(event)) => {
                                apply_event_to_balance_cache(&balance_cache, &user_stream_ready, &event);
                                if event_tx.send(event).await.is_err() {
                                    break;
                                }
                            }
                            Ok(None) => {}
                            Err(err) => {
                                warn!(error = %err, "failed to parse Futures user-data frame");
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = sink.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(err)) => {
                        warn!(error = %err, "Futures user-data WS read error");
                        break;
                    }
                }
            }
        }
    }

    user_stream_ready.store(false, Ordering::Release);
    balance_cache.mark_ws_disconnected();
}

fn derive_user_stream_base_url(ws_api_url: &str) -> String {
    if ws_api_url.contains("testnet.binancefuture.com") {
        return "wss://stream.binancefuture.com".to_owned();
    }
    if ws_api_url.contains("ws-fapi.binance.com") {
        return "wss://fstream.binance.com".to_owned();
    }
    if let Ok(mut parsed) = url::Url::parse(ws_api_url) {
        parsed.set_path("");
        parsed.set_query(None);
        return parsed.to_string().trim_end_matches('/').to_owned();
    }
    ws_api_url.trim_end_matches('/').to_owned()
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
