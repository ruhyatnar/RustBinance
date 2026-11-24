//! USDⓈ-M Futures `FuturesAdapter` with safe pre-activation account checks and isolated `/fapi`
//! rate-limit tracking (§4.5).

use std::sync::Arc;

use async_trait::async_trait;
use bot_types::{Symbol, Venue, VenueCaps};
use serde::{Deserialize, Serialize};

use crate::error::GatewayError;
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

/// USDⓈ-M Futures adapter implementing [`ExchangeGateway`] and [`VenueAdapter`] (§4.5).
pub struct FuturesAdapter {
    config: FuturesAdapterConfig,
    /// Isolated `/fapi` rate-limit tracker (completely separate from Spot).
    rate_limits: Arc<RateLimitTracker>,
    /// Last inspected account setup snapshot (used for safe activation validation).
    account_setup: FuturesAccountSetup,
}

impl FuturesAdapter {
    /// Construct a new [`FuturesAdapter`] with its own isolated `/fapi` rate-limit tracker.
    #[must_use]
    pub fn new(config: FuturesAdapterConfig, account_setup: FuturesAccountSetup) -> Self {
        Self {
            config,
            rate_limits: Arc::new(RateLimitTracker::new()),
            account_setup,
        }
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
}

#[async_trait]
impl ExchangeGateway for FuturesAdapter {
    async fn place_order(&self, _req: PlaceOrderRequest) -> Result<OrderAck, GatewayError> {
        Err(GatewayError::Unsupported(
            "live Futures REST/WS order placement is disabled in M15 (use PaperFuturesBroker or testnet-it)".into(),
        ))
    }

    async fn cancel_order(&self, _req: CancelOrderRequest) -> Result<CancelOrderAck, GatewayError> {
        Err(GatewayError::Unsupported(
            "live Futures cancel_order is disabled in M15".into(),
        ))
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

    async fn query_order(&self, _req: QueryOrderRequest) -> Result<OrderSnapshot, GatewayError> {
        Err(GatewayError::Unsupported(
            "live Futures query_order is disabled in M15".into(),
        ))
    }

    async fn open_orders(
        &self,
        _symbol: Option<&Symbol>,
    ) -> Result<Vec<OrderSnapshot>, GatewayError> {
        Err(GatewayError::Unsupported(
            "live Futures open_orders is disabled in M15".into(),
        ))
    }

    async fn account_snapshot(&self) -> Result<AccountSnapshot, GatewayError> {
        Err(GatewayError::Unsupported(
            "live Futures account_snapshot is disabled in M15".into(),
        ))
    }

    async fn exchange_info(&self) -> Result<ExchangeInfoSnapshot, GatewayError> {
        Err(GatewayError::Unsupported(
            "live Futures exchange_info is disabled in M15".into(),
        ))
    }

    async fn place_algo_order(
        &self,
        req: PlaceAlgoOrderRequest,
    ) -> Result<AlgoOrderAck, GatewayError> {
        let _params = crate::futures::wire::build_place_algo_order_params(&req)?;
        Err(GatewayError::Unsupported(
            "live Futures place_algo_order transport is disabled in M15".into(),
        ))
    }

    async fn cancel_algo_order(
        &self,
        req: CancelAlgoOrderRequest,
    ) -> Result<CancelAlgoOrderAck, GatewayError> {
        let _params = crate::futures::wire::build_cancel_algo_order_params(&req)?;
        Err(GatewayError::Unsupported(
            "live Futures cancel_algo_order transport is disabled in M15".into(),
        ))
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
        self.account_setup
            .validate_for_activation(self.config.max_leverage_cap)?;
        Ok(ActivationReport {
            venue: Venue::UsdsFutures,
            server_time_offset_ms: 0,
            subscription_id: None,
            symbols_loaded: 0,
        })
    }

    async fn deactivate(&self) -> Result<(), GatewayError> {
        // Hard rule #7: never cancel protective Algo orders on deactivate.
        Ok(())
    }
}
