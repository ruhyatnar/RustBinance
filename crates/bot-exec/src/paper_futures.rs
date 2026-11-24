//! Paper broker for USDⓈ-M Futures (`PaperFuturesBroker`): isolated margin, funding payments,
//! simple liquidation, and Algo SL/TP/Trailing orders for Long and Short (§4.5, M15).

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use bot_exchange::{
    AccountBalance, AccountSnapshot, ActivationReport, AlgoOrderAck, AlgoOrderType,
    CancelAlgoOrderAck, CancelAlgoOrderRequest, CancelOrderAck, CancelOrderListAck,
    CancelOrderListRequest, CancelOrderRequest, CancelReplaceAck, CancelReplaceRequest,
    ExchangeGateway, ExchangeInfoSnapshot, GatewayError, OcoOrderAck, OrderAck, OrderSnapshot,
    PlaceAlgoOrderRequest, PlaceOcoRequest, PlaceOrderRequest, QueryOrderRequest, VenueAdapter,
};
use bot_risk::futures::estimate_isolated_liquidation_price;
use bot_types::{OrderStatus, Price, Qty, Side, Symbol, Venue, VenueCaps};
use rust_decimal::Decimal;

/// Audit record of Algo operations executed against [`PaperFuturesBroker`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaperFuturesOp {
    /// Placed an Algo order (`algo_id`, `order_type`, `active_sl_count_after`).
    PlaceAlgo {
        /// Assigned algo ID.
        algo_id: i64,
        /// Algo order type.
        order_type: AlgoOrderType,
        /// Number of active `StopMarket` SL orders immediately after placement.
        active_sl_count_after: usize,
    },
    /// Canceled an Algo order (`algo_id`, `active_sl_count_after`).
    CancelAlgo {
        /// Canceled algo ID.
        algo_id: i64,
        /// Number of active `StopMarket` SL orders immediately after cancellation.
        active_sl_count_after: usize,
    },
}

/// Simulated isolated Futures position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperFuturesPosition {
    /// Direction (`Side::Buy` = Long, `Side::Sell` = Short).
    pub side: Side,
    /// Open quantity.
    pub qty: Qty,
    /// Average entry price.
    pub entry_price: Price,
    /// Isolated leverage.
    pub leverage: u32,
    /// Isolated margin allocated in USDT.
    pub isolated_margin: Decimal,
    /// Estimated liquidation price.
    pub liq_price: Price,
}

/// Event produced when [`PaperFuturesBroker::on_mark_price`] triggers a liquidation or Algo exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaperMarkOutcome {
    /// Position remains open; no trigger fired.
    None,
    /// Position was liquidated at `mark_price`.
    Liquidated {
        /// Realized PnL (isolated margin lost).
        realized_pnl: Decimal,
    },
    /// An Algo order (`StopMarket`, `TakeProfitMarket`, or `TrailingStopMarket`) closed the position.
    AlgoTriggered {
        /// Triggered algo ID.
        algo_id: i64,
        /// Algo order type.
        order_type: AlgoOrderType,
        /// Realized PnL in USDT.
        realized_pnl: Decimal,
    },
}

struct InnerState {
    equity_usdt: Decimal,
    leverage: u32,
    maint_margin_rate: Decimal,
    next_id: i64,
    positions: HashMap<Symbol, PaperFuturesPosition>,
    algo_orders: HashMap<i64, PlaceAlgoOrderRequest>,
    ops: Vec<PaperFuturesOp>,
    fail_next_place_algo: bool,
}

/// Deterministic USDⓈ-M Futures paper broker implementing [`ExchangeGateway`] and [`VenueAdapter`].
pub struct PaperFuturesBroker {
    state: Mutex<InnerState>,
}

impl PaperFuturesBroker {
    /// Create a new [`PaperFuturesBroker`] with starting USDT equity, leverage, and MMR.
    #[must_use]
    pub fn new(initial_equity_usdt: Decimal, leverage: u32, maint_margin_rate: Decimal) -> Self {
        Self {
            state: Mutex::new(InnerState {
                equity_usdt: initial_equity_usdt,
                leverage,
                maint_margin_rate,
                next_id: 1,
                positions: HashMap::new(),
                algo_orders: HashMap::new(),
                ops: Vec::new(),
                fail_next_place_algo: false,
            }),
        }
    }

    /// Inject a one-shot failure on the next `place_algo_order` call (for testing make-before-break).
    pub fn inject_next_place_algo_failure(&self) {
        if let Ok(mut g) = self.state.lock() {
            g.fail_next_place_algo = true;
        }
    }

    /// Snapshot of current USDT equity.
    #[must_use]
    pub fn equity_usdt(&self) -> Decimal {
        self.state
            .lock()
            .map_or(Decimal::ZERO, |g| g.equity_usdt)
    }

    /// Snapshot of open position for `symbol`, if any.
    #[must_use]
    pub fn position(&self, symbol: &Symbol) -> Option<PaperFuturesPosition> {
        self.state
            .lock()
            .ok()
            .and_then(|g| g.positions.get(symbol).cloned())
    }

    /// Count of currently active `StopMarket` SL orders for `symbol`.
    #[must_use]
    pub fn active_sl_count(&self, symbol: &Symbol) -> usize {
        self.state.lock().map_or(0, |g| {
            g.algo_orders
                .values()
                .filter(|o| o.symbol == *symbol && o.order_type == AlgoOrderType::StopMarket)
                .count()
        })
    }

    /// Return the recorded sequence of Algo operations for zero-gap verification.
    #[must_use]
    pub fn recorded_ops(&self) -> Vec<PaperFuturesOp> {
        self.state
            .lock()
            .map_or_else(|_| Vec::new(), |g| g.ops.clone())
    }

    /// Apply a periodic funding settlement (`funding_rate > 0`: Long pays Short).
    pub fn apply_funding(&self, symbol: &Symbol, mark_price: Price, funding_rate: Decimal) -> Decimal {
        let Ok(mut g) = self.state.lock() else {
            return Decimal::ZERO;
        };
        let Some(pos) = g.positions.get(symbol) else {
            return Decimal::ZERO;
        };
        let notional = pos.qty.0 * mark_price.0;
        let payment = match pos.side {
            Side::Buy => -(notional * funding_rate),
            Side::Sell => notional * funding_rate,
        };
        g.equity_usdt += payment;
        payment
    }

    /// Advance simulated mark price: checks liquidation first, then Algo SL/TP triggers.
    pub fn on_mark_price(&self, symbol: &Symbol, mark_price: Price) -> PaperMarkOutcome {
        let Ok(mut g) = self.state.lock() else {
            return PaperMarkOutcome::None;
        };
        let Some(pos) = g.positions.get(symbol).cloned() else {
            return PaperMarkOutcome::None;
        };

        // 1. Check isolated liquidation
        let liquidated = match pos.side {
            Side::Buy => mark_price.0 <= pos.liq_price.0,
            Side::Sell => mark_price.0 >= pos.liq_price.0,
        };
        if liquidated {
            g.positions.remove(symbol);
            g.algo_orders.retain(|_, o| o.symbol != *symbol);
            let loss = -pos.isolated_margin;
            g.equity_usdt += loss;
            return PaperMarkOutcome::Liquidated { realized_pnl: loss };
        }

        // 2. Check Algo SL/TP triggers
        let triggered = g.algo_orders.iter().find_map(|(&id, req)| {
            if req.symbol != *symbol {
                return None;
            }
            let trig = req.trigger_price?;
            let hit = match (pos.side, req.order_type) {
                (Side::Buy, AlgoOrderType::StopMarket) => mark_price.0 <= trig.0,
                (Side::Sell, AlgoOrderType::StopMarket) => mark_price.0 >= trig.0,
                (Side::Buy, AlgoOrderType::TakeProfitMarket) => mark_price.0 >= trig.0,
                (Side::Sell, AlgoOrderType::TakeProfitMarket) => mark_price.0 <= trig.0,
                _ => false,
            };
            hit.then_some((id, req.order_type))
        });

        if let Some((algo_id, order_type)) = triggered {
            g.positions.remove(symbol);
            g.algo_orders.retain(|_, o| o.symbol != *symbol);
            let pnl = match pos.side {
                Side::Buy => (mark_price.0 - pos.entry_price.0) * pos.qty.0,
                Side::Sell => (pos.entry_price.0 - mark_price.0) * pos.qty.0,
            };
            g.equity_usdt += pnl;
            return PaperMarkOutcome::AlgoTriggered {
                algo_id,
                order_type,
                realized_pnl: pnl,
            };
        }

        PaperMarkOutcome::None
    }
}

#[async_trait]
impl ExchangeGateway for PaperFuturesBroker {
    async fn place_order(&self, req: PlaceOrderRequest) -> Result<OrderAck, GatewayError> {
        let mut g = self
            .state
            .lock()
            .map_err(|_| GatewayError::InvalidRequest("paper futures lock poisoned".into()))?;
        let qty = req
            .quantity
            .ok_or_else(|| GatewayError::InvalidRequest("quantity required".into()))?;
        let price = req
            .price
            .ok_or_else(|| GatewayError::InvalidRequest("price required for paper fill".into()))?;
        let notional = price.0 * qty.0;
        let margin = notional / Decimal::from(g.leverage.max(1));
        let liq_price =
            estimate_isolated_liquidation_price(req.side, price, g.leverage, g.maint_margin_rate)
                .map_err(|e| GatewayError::InvalidRequest(e.to_string()))?;

        let order_id = g.next_id;
        let leverage = g.leverage;
        g.next_id += 1;

        g.positions.insert(
            req.symbol.clone(),
            PaperFuturesPosition {
                side: req.side,
                qty,
                entry_price: price,
                leverage,
                isolated_margin: margin,
                liq_price,
            },
        );

        Ok(OrderAck {
            venue: Venue::UsdsFutures,
            symbol: req.symbol,
            order_id,
            client_order_id: req.client_order_id,
            transact_time_ms: 0,
            status: OrderStatus::Filled,
            executed_qty: qty,
            cumulative_quote_qty: notional,
        })
    }

    async fn cancel_order(&self, req: CancelOrderRequest) -> Result<CancelOrderAck, GatewayError> {
        Ok(CancelOrderAck {
            venue: Venue::UsdsFutures,
            symbol: req.symbol,
            order_id: req.order_id.unwrap_or(0),
            orig_client_order_id: req.orig_client_order_id.unwrap_or_default(),
            status: OrderStatus::Canceled,
            executed_qty: Qty(Decimal::ZERO),
        })
    }

    async fn cancel_replace(
        &self,
        _req: CancelReplaceRequest,
    ) -> Result<CancelReplaceAck, GatewayError> {
        Err(GatewayError::Unsupported(
            "cancel_replace not supported on Futures".into(),
        ))
    }

    async fn place_oco(&self, _req: PlaceOcoRequest) -> Result<OcoOrderAck, GatewayError> {
        Err(GatewayError::Unsupported(
            "place_oco is Spot-only; use place_algo_order on Futures".into(),
        ))
    }

    async fn cancel_list(
        &self,
        _req: CancelOrderListRequest,
    ) -> Result<CancelOrderListAck, GatewayError> {
        Err(GatewayError::Unsupported(
            "cancel_list is Spot-only; use cancel_algo_order on Futures".into(),
        ))
    }

    async fn query_order(&self, _req: QueryOrderRequest) -> Result<OrderSnapshot, GatewayError> {
        Err(GatewayError::Unsupported("not implemented in paper futures".into()))
    }

    async fn open_orders(
        &self,
        _symbol: Option<&Symbol>,
    ) -> Result<Vec<OrderSnapshot>, GatewayError> {
        Ok(Vec::new())
    }

    async fn account_snapshot(&self) -> Result<AccountSnapshot, GatewayError> {
        let equity = self.equity_usdt();
        Ok(AccountSnapshot {
            venue: Venue::UsdsFutures,
            update_time_ms: 0,
            can_trade: true,
            balances: vec![AccountBalance {
                asset: "USDT".into(),
                free: equity,
                locked: Decimal::ZERO,
            }],
        })
    }

    async fn exchange_info(&self) -> Result<ExchangeInfoSnapshot, GatewayError> {
        Ok(ExchangeInfoSnapshot {
            venue: Venue::UsdsFutures,
            server_time_ms: 0,
            rules: Vec::new(),
            rate_limits: Vec::new(),
        })
    }

    async fn place_algo_order(
        &self,
        req: PlaceAlgoOrderRequest,
    ) -> Result<AlgoOrderAck, GatewayError> {
        let mut g = self
            .state
            .lock()
            .map_err(|_| GatewayError::InvalidRequest("paper futures lock poisoned".into()))?;
        if g.fail_next_place_algo {
            g.fail_next_place_algo = false;
            return Err(GatewayError::Transport("injected place_algo_order failure".into()));
        }
        let algo_id = g.next_id;
        g.next_id += 1;
        let order_type = req.order_type;
        let sym = req.symbol.clone();
        let cid = req.client_algo_id.clone();
        g.algo_orders.insert(algo_id, req);
        let sl_count = g
            .algo_orders
            .values()
            .filter(|o| o.symbol == sym && o.order_type == AlgoOrderType::StopMarket)
            .count();
        g.ops.push(PaperFuturesOp::PlaceAlgo {
            algo_id,
            order_type,
            active_sl_count_after: sl_count,
        });
        Ok(AlgoOrderAck {
            venue: Venue::UsdsFutures,
            symbol: sym,
            algo_id,
            client_algo_id: cid,
            status: "NEW".into(),
            transact_time_ms: 0,
        })
    }

    async fn cancel_algo_order(
        &self,
        req: CancelAlgoOrderRequest,
    ) -> Result<CancelAlgoOrderAck, GatewayError> {
        let mut g = self
            .state
            .lock()
            .map_err(|_| GatewayError::InvalidRequest("paper futures lock poisoned".into()))?;
        let target_id = if let Some(id) = req.algo_id {
            id
        } else if let Some(ref cid) = req.client_algo_id {
            g.algo_orders
                .iter()
                .find_map(|(&id, o)| (o.client_algo_id == *cid).then_some(id))
                .ok_or_else(|| GatewayError::InvalidRequest("algo order not found".into()))?
        } else {
            return Err(GatewayError::InvalidRequest("missing algo_id/client_algo_id".into()));
        };
        let removed = g
            .algo_orders
            .remove(&target_id)
            .ok_or_else(|| GatewayError::InvalidRequest("algo order not found".into()))?;
        let sl_count = g
            .algo_orders
            .values()
            .filter(|o| o.symbol == req.symbol && o.order_type == AlgoOrderType::StopMarket)
            .count();
        g.ops.push(PaperFuturesOp::CancelAlgo {
            algo_id: target_id,
            active_sl_count_after: sl_count,
        });
        Ok(CancelAlgoOrderAck {
            venue: Venue::UsdsFutures,
            symbol: req.symbol,
            algo_id: target_id,
            client_algo_id: removed.client_algo_id,
        })
    }
}

#[async_trait]
impl VenueAdapter for PaperFuturesBroker {
    fn venue(&self) -> Venue {
        Venue::UsdsFutures
    }

    fn caps(&self) -> VenueCaps {
        let lev = self.state.lock().map_or(1, |g| g.leverage);
        VenueCaps::usds_futures(lev)
    }

    async fn activate(&self) -> Result<ActivationReport, GatewayError> {
        Ok(ActivationReport {
            venue: Venue::UsdsFutures,
            server_time_offset_ms: 0,
            subscription_id: None,
            symbols_loaded: 0,
        })
    }

    async fn deactivate(&self) -> Result<(), GatewayError> {
        Ok(())
    }
}
