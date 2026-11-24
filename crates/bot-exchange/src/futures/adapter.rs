//! USDⓈ-M Futures `FuturesAdapter` with WS API (`/ws-fapi/v1`) primary transport,
//! REST (`/fapi/v1`) fallback + pre-activation account safety checks, and isolated `/fapi`
//! rate-limit tracking (§4.5).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bot_types::{Symbol, Venue, VenueCaps};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::auth::Ed25519Credentials;
use crate::error::GatewayError;
use crate::futures::rest::{
    FuturesRestClient, build_place_order_params, parse_account_snapshot, parse_algo_order_ack,
    parse_cancel_ack, parse_cancel_algo_ack, parse_order_ack, parse_order_snapshot,
};
use crate::futures::session::FuturesWsSession;
use crate::futures::wire::{build_cancel_algo_order_params, build_place_algo_order_params};
use crate::gateway::{
    AccountSnapshot, ActivationReport, AlgoOrderAck, CancelAlgoOrderAck, CancelAlgoOrderRequest,
    CancelOrderAck, CancelOrderListAck, CancelOrderListRequest, CancelOrderRequest,
    CancelReplaceAck, CancelReplaceRequest, ExchangeGateway, ExchangeInfoSnapshot, OcoOrderAck,
    OrderAck, OrderSnapshot, PlaceAlgoOrderRequest, PlaceOcoRequest, PlaceOrderRequest,
    QueryOrderRequest, VenueAdapter,
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

struct LiveTransport {
    ws: FuturesWsSession,
    rest: FuturesRestClient,
}

/// USDⓈ-M Futures adapter implementing [`ExchangeGateway`] and [`VenueAdapter`] (§4.5).
pub struct FuturesAdapter {
    config: FuturesAdapterConfig,
    /// Isolated `/fapi` rate-limit tracker (completely separate from Spot).
    rate_limits: Arc<RateLimitTracker>,
    /// Offline/injected account setup snapshot (used when constructed without live transport).
    account_setup: Option<FuturesAccountSetup>,
    /// WS API primary + REST fallback transport (present when constructed via `with_transport`).
    transport: Option<LiveTransport>,
}

impl FuturesAdapter {
    /// Construct an offline [`FuturesAdapter`] with a static `account_setup` snapshot (for unit tests).
    #[must_use]
    pub fn new(config: FuturesAdapterConfig, account_setup: FuturesAccountSetup) -> Self {
        Self {
            config,
            rate_limits: Arc::new(RateLimitTracker::new()),
            account_setup: Some(account_setup),
            transport: None,
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
        let ws = FuturesWsSession::new(
            config.ws_api_url.clone(),
            creds,
            Arc::clone(&rate_limits),
            timeout,
        );
        Ok(Self {
            config,
            rate_limits,
            account_setup: None,
            transport: Some(LiveTransport { ws, rest }),
        })
    }

    /// Borrow the isolated `/fapi` rate-limit tracker.
    #[must_use]
    pub fn rate_limits(&self) -> &Arc<RateLimitTracker> {
        &self.rate_limits
    }

    /// Borrow adapter configuration.
    #[must_use]
    pub fn config(&self) -> &FuturesAdapterConfig {
        &self.config
    }

    fn require_transport(&self, op: &str) -> Result<&LiveTransport, GatewayError> {
        self.transport.as_ref().ok_or_else(|| {
            GatewayError::Unsupported(format!(
                "FuturesAdapter '{op}' requires transport (constructed via FuturesAdapter::with_transport)"
            ))
        })
    }
}

#[async_trait]
impl ExchangeGateway for FuturesAdapter {
    async fn place_order(&self, req: PlaceOrderRequest) -> Result<OrderAck, GatewayError> {
        let tr = self.require_transport("place_order")?;
        let params = build_place_order_params(&req);
        match tr.ws.request("order.place", params).await {
            Ok(val) => parse_order_ack(&val.to_string()),
            Err(GatewayError::Transport(err)) => {
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
            Err(GatewayError::Transport(err)) => {
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
            Err(GatewayError::Transport(err)) => {
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
        match tr.ws.request("v2/account.status", BTreeMap::new()).await {
            Ok(val) => parse_account_snapshot(&val.to_string()),
            Err(GatewayError::Transport(err)) => {
                warn!(error = %err, "Futures WS v2/account.status failed; falling back to REST /fapi/v2/account");
                tr.rest.account_snapshot().await
            }
            Err(other) => Err(other),
        }
    }

    async fn exchange_info(&self) -> Result<ExchangeInfoSnapshot, GatewayError> {
        let tr = self.require_transport("exchange_info")?;
        tr.rest.exchange_info().await
    }

    async fn place_algo_order(
        &self,
        req: PlaceAlgoOrderRequest,
    ) -> Result<AlgoOrderAck, GatewayError> {
        let params = crate::futures::rest::json_map_to_btree(build_place_algo_order_params(&req)?);
        let tr = self.require_transport("place_algo_order")?;
        match tr.ws.request("algoOrder.place", params).await {
            Ok(val) => parse_algo_order_ack(&val.to_string()),
            Err(GatewayError::Transport(err)) => {
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
            Err(GatewayError::Transport(err)) => {
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
        if let Some(tr) = &self.transport {
            let setup = tr.rest.fetch_account_setup().await?;
            setup.validate_for_activation(self.config.max_leverage_cap)?;
            if let Err(err) = tr.ws.connect().await {
                warn!(error = %err, "Futures WS connect failed during activate; REST fallback remains active");
            }
            return Ok(ActivationReport {
                venue: Venue::UsdsFutures,
                server_time_offset_ms: 0,
                subscription_id: None,
                symbols_loaded: 0,
            });
        }
        if let Some(setup) = &self.account_setup {
            setup.validate_for_activation(self.config.max_leverage_cap)?;
        }
        Ok(ActivationReport {
            venue: Venue::UsdsFutures,
            server_time_offset_ms: 0,
            subscription_id: None,
            symbols_loaded: 0,
        })
    }

    async fn deactivate(&self) -> Result<(), GatewayError> {
        // Hard rule #7: never cancel protective Algo orders on deactivate.
        if let Some(tr) = &self.transport {
            tr.ws.close().await;
        }
        Ok(())
    }
}
