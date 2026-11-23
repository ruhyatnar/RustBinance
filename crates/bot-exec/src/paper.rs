//! Deterministic Paper Trading Broker implementing [`ExchangeGateway`] and [`VenueAdapter`] (M07).
//!
//! Simulates Spot balances, maker/taker fees, market slippage, `LIMIT_MAKER` `-2010` rejections,
//! `cancelReplace`, Dual-OCO order lists (`orderList.place.oco`), and `GatewayEvent` emission.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use bot_exchange::{
    AccountBalance, AccountSnapshot, ActivationReport, CancelOrderAck, CancelOrderListAck,
    CancelOrderListRequest, CancelOrderRequest, CancelReplaceAck, CancelReplaceMode,
    CancelReplaceRequest, ExchangeErrorKind, ExchangeGateway, ExchangeInfoSnapshot, ExecEvent,
    GatewayError, GatewayEvent, ListOrderItem, ListStatusEvent, OcoOrderAck, OrderAck,
    OrderSnapshot, PlaceOcoRequest, PlaceOrderRequest, QueryOrderRequest, VenueAdapter,
    validate_client_order_id,
};
use bot_types::{
    InstrumentRules, OrderKind, OrderStatus, Price, Qty, Side, Symbol, Venue, VenueCaps,
};
use rust_decimal::Decimal;
use tokio::sync::mpsc;

/// Configuration for [`PaperBroker`].
#[derive(Debug, Clone)]
pub struct PaperBrokerConfig {
    /// Venue simulated by this broker (default `Venue::Spot`).
    pub venue: Venue,
    /// Primary quote asset (default `"USDT"`).
    pub quote_asset: String,
    /// Initial free quote balance (default `10_000` USDT).
    pub initial_quote_balance: Decimal,
    /// Simulated maker fee in basis points (default `10` bps = 0.10%).
    pub maker_fee_bps: Decimal,
    /// Simulated taker fee in basis points (default `10` bps = 0.10%).
    pub taker_fee_bps: Decimal,
    /// Simulated market order slippage in basis points (default `5` bps = 0.05%).
    pub market_slippage_bps: Decimal,
    /// When `false` (§13.2 backtest mode), newly placed passive entry orders (`LimitMaker`/`Limit`)
    /// cannot fill on a price tick at the exact same timestamp as order placement (`ts_ms <= created_time_ms`).
    pub allow_same_bar_entry_fill: bool,
    /// When `true` (§13.2 backtest mode), `LimitMaker` fills only when price **strictly penetrates**
    /// the limit price (`ask < limit` for Buy, `bid > limit` for Sell) rather than merely touching (`<=`/`>=`).
    pub strict_limit_maker_penetration: bool,
    /// Multiplier applied to slippage (and optional stress testing: default `1`, stress test `2` per §13.2).
    pub slippage_stress_multiplier: Decimal,
    /// Instrument rules served by `exchange_info()`.
    pub rules: Vec<InstrumentRules>,
    /// Bounded user-data event channel capacity (default `4096`).
    pub event_channel_capacity: usize,
}

impl Default for PaperBrokerConfig {
    fn default() -> Self {
        Self {
            venue: Venue::Spot,
            quote_asset: "USDT".to_owned(),
            initial_quote_balance: Decimal::from(10_000_u32),
            maker_fee_bps: Decimal::from(10_u32),
            taker_fee_bps: Decimal::from(10_u32),
            market_slippage_bps: Decimal::from(5_u32),
            allow_same_bar_entry_fill: true,
            strict_limit_maker_penetration: false,
            slippage_stress_multiplier: Decimal::ONE,
            rules: Vec::new(),
            event_channel_capacity: 4096,
        }
    }
}

#[derive(Debug, Clone)]
struct PaperOrder {
    order_id: i64,
    order_list_id: i64,
    symbol: Symbol,
    client_order_id: String,
    side: Side,
    kind: OrderKind,
    price: Price,
    stop_price: Option<Price>,
    orig_qty: Qty,
    executed_qty: Qty,
    cumulative_quote_qty: Decimal,
    status: OrderStatus,
    created_time_ms: i64,
    update_time_ms: i64,
}

impl PaperOrder {
    fn to_snapshot(&self, venue: Venue) -> OrderSnapshot {
        OrderSnapshot {
            venue,
            symbol: self.symbol.clone(),
            order_id: self.order_id,
            order_list_id: self.order_list_id,
            client_order_id: self.client_order_id.clone(),
            side: self.side,
            kind: self.kind,
            price: self.price,
            orig_qty: self.orig_qty,
            executed_qty: self.executed_qty,
            cumulative_quote_qty: self.cumulative_quote_qty,
            status: self.status,
            update_time_ms: self.update_time_ms,
        }
    }
}

#[derive(Debug, Clone)]
struct PaperOco {
    order_list_id: i64,
    list_client_order_id: String,
    symbol: Symbol,
    above_client_id: String,
    below_client_id: String,
    active: bool,
}

#[derive(Debug)]
struct BrokerInner {
    active: bool,
    now_ms: i64,
    next_order_id: i64,
    next_list_id: i64,
    next_trade_id: i64,
    /// `(bid, ask)` per symbol.
    market_prices: BTreeMap<Symbol, (Price, Price)>,
    /// `(free, locked)` per asset ticker.
    balances: BTreeMap<String, (Decimal, Decimal)>,
    /// All orders keyed by `client_order_id`.
    orders: BTreeMap<String, PaperOrder>,
    /// OCO lists keyed by `order_list_id`.
    oco_lists: BTreeMap<i64, PaperOco>,
    /// Outbound user-data event sender.
    event_tx: Option<mpsc::Sender<GatewayEvent>>,
}

/// Deterministic in-memory Paper Broker behind [`ExchangeGateway`] + [`VenueAdapter`].
#[derive(Debug)]
pub struct PaperBroker {
    config: PaperBrokerConfig,
    inner: Mutex<BrokerInner>,
    event_rx: Mutex<Option<mpsc::Receiver<GatewayEvent>>>,
}

impl PaperBroker {
    /// Construct a new [`PaperBroker`] with initial balances and market rules.
    #[must_use]
    pub fn new(config: PaperBrokerConfig) -> Self {
        let mut balances = BTreeMap::new();
        balances.insert(
            config.quote_asset.clone(),
            (config.initial_quote_balance, Decimal::ZERO),
        );
        let (tx, rx) = mpsc::channel(config.event_channel_capacity.max(16));
        Self {
            config,
            inner: Mutex::new(BrokerInner {
                active: false,
                now_ms: 1_700_000_000_000,
                next_order_id: 1_000,
                next_list_id: 100,
                next_trade_id: 1,
                market_prices: BTreeMap::new(),
                balances,
                orders: BTreeMap::new(),
                oco_lists: BTreeMap::new(),
                event_tx: Some(tx),
            }),
            event_rx: Mutex::new(Some(rx)),
        }
    }

    /// Take the bounded user-data [`GatewayEvent`] receiver for the engine loop.
    #[must_use]
    pub fn take_event_rx(&self) -> Option<mpsc::Receiver<GatewayEvent>> {
        self.event_rx.lock().ok().and_then(|mut g| g.take())
    }

    /// Set the current simulated clock timestamp (`now_ms`) without evaluating fills.
    pub fn set_clock_ms(&self, ts_ms: i64) {
        if let Ok(mut st) = self.inner.lock() {
            st.now_ms = ts_ms;
        }
    }

    /// Set the current market `(bid, ask)` for `symbol` and evaluate resting orders & OCO legs.
    ///
    /// Returns all [`GatewayEvent`]s generated by this tick (they are also sent to `event_rx`).
    pub fn on_price_tick(
        &self,
        symbol: &Symbol,
        bid: Price,
        ask: Price,
        ts_ms: i64,
    ) -> Vec<GatewayEvent> {
        let Ok(mut st) = self.inner.lock() else {
            return Vec::new();
        };
        st.now_ms = ts_ms;
        st.market_prices.insert(symbol.clone(), (bid, ask));

        let mut events = Vec::new();
        let candidate_ids: Vec<String> = st
            .orders
            .values()
            .filter(|o| {
                o.symbol == *symbol
                    && matches!(o.status, OrderStatus::New | OrderStatus::PartiallyFilled)
            })
            .map(|o| o.client_order_id.clone())
            .collect();

        for cid in candidate_ids {
            let Some(ord) = st.orders.get(&cid).cloned() else {
                continue;
            };
            if !matches!(ord.status, OrderStatus::New | OrderStatus::PartiallyFilled) {
                continue;
            }
            // §13.2: No fill on the same bar/timestamp as signal placement for standalone passive entry orders.
            if !self.config.allow_same_bar_entry_fill
                && ord.order_list_id < 0
                && matches!(ord.kind, OrderKind::LimitMaker | OrderKind::Limit)
                && ts_ms <= ord.created_time_ms
            {
                continue;
            }

            let strict_pen =
                self.config.strict_limit_maker_penetration && ord.kind == OrderKind::LimitMaker;
            let trigger_fill = match ord.kind {
                OrderKind::LimitMaker | OrderKind::Limit => match ord.side {
                    // Passive Buy fills when market ask trades below (or at/below) limit price.
                    Side::Buy => {
                        let hit = if strict_pen {
                            ask.as_decimal() < ord.price.as_decimal()
                        } else {
                            ask.as_decimal() <= ord.price.as_decimal()
                        };
                        hit.then_some((ord.price, true))
                    }
                    // Passive Sell fills when market bid trades above (or at/above) limit price.
                    Side::Sell => {
                        let hit = if strict_pen {
                            bid.as_decimal() > ord.price.as_decimal()
                        } else {
                            bid.as_decimal() >= ord.price.as_decimal()
                        };
                        hit.then_some((ord.price, true))
                    }
                },
                OrderKind::StopLossLimit => {
                    let stop = ord.stop_price.unwrap_or(ord.price);
                    match ord.side {
                        Side::Sell => (bid.as_decimal() <= stop.as_decimal()).then_some((
                            self.apply_slippage(
                                Side::Sell,
                                Price(bid.as_decimal().min(stop.as_decimal())),
                            ),
                            false,
                        )),
                        Side::Buy => (ask.as_decimal() >= stop.as_decimal()).then_some((
                            self.apply_slippage(
                                Side::Buy,
                                Price(ask.as_decimal().max(stop.as_decimal())),
                            ),
                            false,
                        )),
                    }
                }
                OrderKind::TakeProfitLimit => {
                    let stop = ord.stop_price.unwrap_or(ord.price);
                    match ord.side {
                        Side::Sell => {
                            (bid.as_decimal() >= stop.as_decimal()).then_some((ord.price, true))
                        }
                        Side::Buy => {
                            (ask.as_decimal() <= stop.as_decimal()).then_some((ord.price, true))
                        }
                    }
                }
                OrderKind::Market | OrderKind::OcoLeg => None,
            };

            if let Some((fill_price, is_maker)) = trigger_fill {
                let mut generated =
                    self.execute_fill_locked(&mut st, &cid, fill_price, is_maker, ts_ms);
                events.append(&mut generated);
            }
        }

        if let Some(tx) = &st.event_tx {
            for ev in &events {
                let _ = tx.try_send(ev.clone());
            }
        }
        events
    }

    /// Inject an external/orphan open order directly into the paper exchange (for reconciler tests).
    pub fn inject_orphan_order(
        &self,
        symbol: Symbol,
        client_order_id: String,
        side: Side,
        kind: OrderKind,
        price: Price,
        qty: Qty,
    ) {
        if let Ok(mut st) = self.inner.lock() {
            let oid = st.next_order_id;
            st.next_order_id += 1;
            let now = st.now_ms;
            st.orders.insert(
                client_order_id.clone(),
                PaperOrder {
                    order_id: oid,
                    order_list_id: -1,
                    symbol,
                    client_order_id,
                    side,
                    kind,
                    price,
                    stop_price: None,
                    orig_qty: qty,
                    executed_qty: Qty(Decimal::ZERO),
                    cumulative_quote_qty: Decimal::ZERO,
                    status: OrderStatus::New,
                    created_time_ms: now,
                    update_time_ms: now,
                },
            );
        }
    }

    /// Inject an external/orphan asset balance directly into the paper exchange (for reconciler tests).
    pub fn inject_orphan_balance(&self, asset: &str, free: Decimal) {
        if let Ok(mut st) = self.inner.lock() {
            let entry = st
                .balances
                .entry(asset.to_owned())
                .or_insert((Decimal::ZERO, Decimal::ZERO));
            entry.0 = entry.0.saturating_add(free);
        }
    }

    /// Count currently open orders on the simulated exchange.
    #[must_use]
    pub fn open_order_count(&self) -> usize {
        self.inner.lock().map_or(0, |st| {
            st.orders
                .values()
                .filter(|o| matches!(o.status, OrderStatus::New | OrderStatus::PartiallyFilled))
                .count()
        })
    }

    /// Count currently active OCO lists on the simulated exchange.
    #[must_use]
    pub fn active_oco_count(&self) -> usize {
        self.inner
            .lock()
            .map_or(0, |st| st.oco_lists.values().filter(|l| l.active).count())
    }

    fn base_asset<'a>(&self, symbol: &'a Symbol) -> &'a str {
        symbol
            .as_str()
            .strip_suffix(&self.config.quote_asset)
            .unwrap_or(symbol.as_str())
    }

    fn apply_slippage(&self, side: Side, price: Price) -> Price {
        let effective_bps = self
            .config
            .market_slippage_bps
            .saturating_mul(self.config.slippage_stress_multiplier.max(Decimal::ZERO));
        let bps = effective_bps / Decimal::from(10_000_u32);
        let mult = match side {
            Side::Buy => Decimal::ONE + bps,
            Side::Sell => (Decimal::ONE - bps).max(Decimal::new(1, 4)),
        };
        Price(price.as_decimal().saturating_mul(mult))
    }

    fn fee_rate(&self, is_maker: bool) -> Decimal {
        let bps = if is_maker {
            self.config.maker_fee_bps
        } else {
            self.config.taker_fee_bps
        };
        bps / Decimal::from(10_000_u32)
    }

    #[allow(clippy::too_many_lines, clippy::similar_names)]
    fn execute_fill_locked(
        &self,
        st: &mut BrokerInner,
        client_order_id: &str,
        fill_price: Price,
        is_maker: bool,
        ts_ms: i64,
    ) -> Vec<GatewayEvent> {
        let Some(ord) = st.orders.get_mut(client_order_id) else {
            return Vec::new();
        };
        let rem_qty = ord
            .orig_qty
            .as_decimal()
            .saturating_sub(ord.executed_qty.as_decimal());
        if rem_qty <= Decimal::ZERO {
            return Vec::new();
        }

        let trade_id = st.next_trade_id;
        st.next_trade_id += 1;

        let notional = rem_qty.saturating_mul(fill_price.as_decimal());
        let fee = notional.saturating_mul(self.fee_rate(is_maker));

        ord.executed_qty = ord.orig_qty;
        ord.cumulative_quote_qty = ord.cumulative_quote_qty.saturating_add(notional);
        ord.status = OrderStatus::Filled;
        ord.update_time_ms = ts_ms;

        let ord_clone = ord.clone();
        let base = self.base_asset(&ord_clone.symbol).to_owned();
        let quote = self.config.quote_asset.clone();

        // Update simulated Spot balances.
        match ord_clone.side {
            Side::Buy => {
                let q = st
                    .balances
                    .entry(quote.clone())
                    .or_insert((Decimal::ZERO, Decimal::ZERO));
                let total_cost = notional.saturating_add(fee);
                if q.1 >= total_cost {
                    q.1 = q.1.saturating_sub(total_cost);
                } else {
                    let rem = total_cost.saturating_sub(q.1);
                    q.1 = Decimal::ZERO;
                    q.0 = q.0.saturating_sub(rem).max(Decimal::ZERO);
                }
                let b = st
                    .balances
                    .entry(base)
                    .or_insert((Decimal::ZERO, Decimal::ZERO));
                b.0 = b.0.saturating_add(rem_qty);
            }
            Side::Sell => {
                let b = st
                    .balances
                    .entry(base)
                    .or_insert((Decimal::ZERO, Decimal::ZERO));
                if b.1 >= rem_qty {
                    b.1 = b.1.saturating_sub(rem_qty);
                } else {
                    let rem = rem_qty.saturating_sub(b.1);
                    b.1 = Decimal::ZERO;
                    b.0 = b.0.saturating_sub(rem).max(Decimal::ZERO);
                }
                let q = st
                    .balances
                    .entry(quote.clone())
                    .or_insert((Decimal::ZERO, Decimal::ZERO));
                q.0 =
                    q.0.saturating_add(notional.saturating_sub(fee).max(Decimal::ZERO));
            }
        }

        let mut events = vec![GatewayEvent::Exec(ExecEvent {
            venue: self.config.venue,
            event_time_ms: ts_ms,
            transact_time_ms: ts_ms,
            symbol: ord_clone.symbol.clone(),
            client_order_id: ord_clone.client_order_id.clone(),
            orig_client_order_id: None,
            order_id: ord_clone.order_id,
            order_list_id: ord_clone.order_list_id,
            trade_id,
            side: ord_clone.side,
            kind: ord_clone.kind,
            exec_type: "TRADE".into(),
            status: OrderStatus::Filled,
            reject_reason: "NONE".into(),
            order_price: ord_clone.price,
            order_qty: ord_clone.orig_qty,
            last_filled_qty: Qty(rem_qty),
            cumulative_filled_qty: ord_clone.executed_qty,
            last_filled_price: fill_price,
            cumulative_quote_qty: ord_clone.cumulative_quote_qty,
            commission: fee,
            commission_asset: Some(quote),
            is_maker,
        })];

        // If part of an OCO list, auto-cancel the sibling leg and emit ListStatus ALL_DONE.
        if ord_clone.order_list_id >= 0
            && let Some(oco) = st.oco_lists.get_mut(&ord_clone.order_list_id)
            && oco.active
        {
            oco.active = false;
            let sibling_cid = if oco.above_client_id == ord_clone.client_order_id {
                oco.below_client_id.clone()
            } else {
                oco.above_client_id.clone()
            };
            let list_cid = oco.list_client_order_id.clone();
            let list_id = oco.order_list_id;
            let sym = oco.symbol.clone();

            let mut list_items = vec![ListOrderItem {
                symbol: ord_clone.symbol.clone(),
                order_id: ord_clone.order_id,
                client_order_id: ord_clone.client_order_id.clone(),
            }];

            if let Some(sib) = st.orders.get_mut(&sibling_cid) {
                sib.status = OrderStatus::Canceled;
                sib.update_time_ms = ts_ms;
                list_items.push(ListOrderItem {
                    symbol: sib.symbol.clone(),
                    order_id: sib.order_id,
                    client_order_id: sib.client_order_id.clone(),
                });
                events.push(GatewayEvent::Exec(ExecEvent {
                    venue: self.config.venue,
                    event_time_ms: ts_ms,
                    transact_time_ms: ts_ms,
                    symbol: sib.symbol.clone(),
                    client_order_id: sib.client_order_id.clone(),
                    orig_client_order_id: Some(sib.client_order_id.clone()),
                    order_id: sib.order_id,
                    order_list_id: list_id,
                    trade_id: -1,
                    side: sib.side,
                    kind: sib.kind,
                    exec_type: "CANCELED".into(),
                    status: OrderStatus::Canceled,
                    reject_reason: "NONE".into(),
                    order_price: sib.price,
                    order_qty: sib.orig_qty,
                    last_filled_qty: Qty(Decimal::ZERO),
                    cumulative_filled_qty: sib.executed_qty,
                    last_filled_price: Price(Decimal::ZERO),
                    cumulative_quote_qty: sib.cumulative_quote_qty,
                    commission: Decimal::ZERO,
                    commission_asset: None,
                    is_maker: false,
                }));
            }

            events.push(GatewayEvent::ListStatus(ListStatusEvent {
                venue: self.config.venue,
                event_time_ms: ts_ms,
                symbol: sym,
                order_list_id: list_id,
                list_client_order_id: list_cid,
                list_status_type: "ALL_DONE".into(),
                list_order_status: "ALL_DONE".into(),
                reject_reason: "NONE".into(),
                orders: list_items,
            }));
        }

        events
    }
}

#[async_trait]
impl ExchangeGateway for PaperBroker {
    async fn place_order(&self, req: PlaceOrderRequest) -> Result<OrderAck, GatewayError> {
        validate_client_order_id(&req.client_order_id)?;
        let mut st = self
            .inner
            .lock()
            .map_err(|_| GatewayError::Transport("paper broker lock poisoned".into()))?;
        let now = st.now_ms;
        let qty = req
            .quantity
            .ok_or_else(|| GatewayError::InvalidRequest("paper broker requires quantity".into()))?;
        let market = st.market_prices.get(&req.symbol).copied();

        // Check LIMIT_MAKER crossing -> Binance Spot -2010 rejection.
        if req.kind == OrderKind::LimitMaker
            && let (Some(limit_p), Some((bid, ask))) = (req.price, market)
        {
            let crosses = match req.side {
                Side::Buy => limit_p.as_decimal() >= ask.as_decimal(),
                Side::Sell => limit_p.as_decimal() <= bid.as_decimal(),
            };
            if crosses {
                return Err(GatewayError::ExchangeRejected {
                    status: 400,
                    code: -2010,
                    kind: ExchangeErrorKind::NewOrderRejected,
                    message: "Order would immediately match and take.".into(),
                });
            }
        }

        let oid = st.next_order_id;
        st.next_order_id += 1;
        let ref_price = req.price.unwrap_or_else(|| {
            market.map_or(Price(Decimal::ONE), |(bid, ask)| match req.side {
                Side::Buy => ask,
                Side::Sell => bid,
            })
        });

        let order = PaperOrder {
            order_id: oid,
            order_list_id: -1,
            symbol: req.symbol.clone(),
            client_order_id: req.client_order_id.clone(),
            side: req.side,
            kind: req.kind,
            price: ref_price,
            stop_price: req.stop_price,
            orig_qty: qty,
            executed_qty: Qty(Decimal::ZERO),
            cumulative_quote_qty: Decimal::ZERO,
            status: OrderStatus::New,
            created_time_ms: now,
            update_time_ms: now,
        };
        st.orders.insert(req.client_order_id.clone(), order);

        // Market orders fill immediately with slippage & taker fee.
        if req.kind == OrderKind::Market {
            let fill_px = self.apply_slippage(req.side, ref_price);
            let evs = self.execute_fill_locked(&mut st, &req.client_order_id, fill_px, false, now);
            if let Some(tx) = &st.event_tx {
                for ev in evs {
                    let _ = tx.try_send(ev);
                }
            }
        }

        let updated = st
            .orders
            .get(&req.client_order_id)
            .cloned()
            .ok_or_else(|| GatewayError::Transport("missing inserted paper order".into()))?;
        Ok(OrderAck {
            venue: self.config.venue,
            symbol: updated.symbol,
            order_id: updated.order_id,
            client_order_id: updated.client_order_id,
            transact_time_ms: now,
            status: updated.status,
            executed_qty: updated.executed_qty,
            cumulative_quote_qty: updated.cumulative_quote_qty,
        })
    }

    async fn cancel_order(&self, req: CancelOrderRequest) -> Result<CancelOrderAck, GatewayError> {
        let mut st = self
            .inner
            .lock()
            .map_err(|_| GatewayError::Transport("paper broker lock poisoned".into()))?;
        let now = st.now_ms;
        let target_cid = req
            .orig_client_order_id
            .clone()
            .or_else(|| {
                req.order_id.and_then(|oid| {
                    st.orders
                        .values()
                        .find(|o| o.order_id == oid)
                        .map(|o| o.client_order_id.clone())
                })
            })
            .ok_or_else(|| GatewayError::InvalidRequest("missing cancel identifier".into()))?;

        let ord = st
            .orders
            .get_mut(&target_cid)
            .ok_or_else(|| GatewayError::ExchangeRejected {
                status: 400,
                code: -2011,
                kind: ExchangeErrorKind::CancelRejected,
                message: "Unknown order sent.".into(),
            })?;
        if !matches!(ord.status, OrderStatus::New | OrderStatus::PartiallyFilled) {
            return Err(GatewayError::ExchangeRejected {
                status: 400,
                code: -2011,
                kind: ExchangeErrorKind::CancelRejected,
                message: "Order already terminal.".into(),
            });
        }
        ord.status = OrderStatus::Canceled;
        ord.update_time_ms = now;
        let snap = ord.clone();

        if let Some(tx) = &st.event_tx {
            let _ = tx.try_send(GatewayEvent::Exec(ExecEvent {
                venue: self.config.venue,
                event_time_ms: now,
                transact_time_ms: now,
                symbol: snap.symbol.clone(),
                client_order_id: req
                    .new_client_order_id
                    .unwrap_or_else(|| snap.client_order_id.clone()),
                orig_client_order_id: Some(snap.client_order_id.clone()),
                order_id: snap.order_id,
                order_list_id: snap.order_list_id,
                trade_id: -1,
                side: snap.side,
                kind: snap.kind,
                exec_type: "CANCELED".into(),
                status: OrderStatus::Canceled,
                reject_reason: "NONE".into(),
                order_price: snap.price,
                order_qty: snap.orig_qty,
                last_filled_qty: Qty(Decimal::ZERO),
                cumulative_filled_qty: snap.executed_qty,
                last_filled_price: Price(Decimal::ZERO),
                cumulative_quote_qty: snap.cumulative_quote_qty,
                commission: Decimal::ZERO,
                commission_asset: None,
                is_maker: false,
            }));
        }

        Ok(CancelOrderAck {
            venue: self.config.venue,
            symbol: snap.symbol,
            order_id: snap.order_id,
            orig_client_order_id: snap.client_order_id,
            status: OrderStatus::Canceled,
            executed_qty: snap.executed_qty,
        })
    }

    async fn cancel_replace(
        &self,
        req: CancelReplaceRequest,
    ) -> Result<CancelReplaceAck, GatewayError> {
        let cancel_res = self
            .cancel_order(CancelOrderRequest {
                symbol: req.symbol.clone(),
                orig_client_order_id: req.cancel_orig_client_order_id.clone(),
                order_id: req.cancel_order_id,
                new_client_order_id: None,
            })
            .await;

        if cancel_res.is_err() && req.cancel_replace_mode == CancelReplaceMode::StopOnFailure {
            return Ok(CancelReplaceAck {
                venue: self.config.venue,
                cancel_result: "FAILURE".into(),
                new_order_result: "NOT_ATTEMPTED".into(),
                new_order: None,
            });
        }

        let place_res = self
            .place_order(PlaceOrderRequest {
                symbol: req.symbol,
                side: req.side,
                kind: req.kind,
                client_order_id: req.new_client_order_id,
                quantity: Some(req.quantity),
                quote_order_qty: None,
                price: req.price,
                stop_price: None,
                time_in_force: req.time_in_force,
            })
            .await;

        match place_res {
            Ok(ack) => Ok(CancelReplaceAck {
                venue: self.config.venue,
                cancel_result: if cancel_res.is_ok() {
                    "SUCCESS".into()
                } else {
                    "FAILURE".into()
                },
                new_order_result: "SUCCESS".into(),
                new_order: Some(ack),
            }),
            Err(err) => Err(err),
        }
    }

    #[allow(clippy::similar_names)]
    async fn place_oco(&self, req: PlaceOcoRequest) -> Result<OcoOrderAck, GatewayError> {
        validate_client_order_id(&req.list_client_order_id)?;
        let mut st = self
            .inner
            .lock()
            .map_err(|_| GatewayError::Transport("paper broker lock poisoned".into()))?;
        let now = st.now_ms;
        let list_id = st.next_list_id;
        st.next_list_id += 1;

        let above_oid = st.next_order_id;
        let below_oid = st.next_order_id + 1;
        st.next_order_id += 2;

        let above_cid = req
            .above_client_order_id
            .clone()
            .unwrap_or_else(|| format!("{}_A", req.list_client_order_id));
        let below_cid = req
            .below_client_order_id
            .clone()
            .unwrap_or_else(|| format!("{}_B", req.list_client_order_id));

        let above_order = PaperOrder {
            order_id: above_oid,
            order_list_id: list_id,
            symbol: req.symbol.clone(),
            client_order_id: above_cid.clone(),
            side: req.side,
            kind: req.above_type,
            price: req.above_price.unwrap_or(Price(Decimal::ZERO)),
            stop_price: req.above_stop_price,
            orig_qty: req.quantity,
            executed_qty: Qty(Decimal::ZERO),
            cumulative_quote_qty: Decimal::ZERO,
            status: OrderStatus::New,
            created_time_ms: now,
            update_time_ms: now,
        };
        let below_order = PaperOrder {
            order_id: below_oid,
            order_list_id: list_id,
            symbol: req.symbol.clone(),
            client_order_id: below_cid.clone(),
            side: req.side,
            kind: req.below_type,
            price: req.below_price.unwrap_or(Price(Decimal::ZERO)),
            stop_price: req.below_stop_price,
            orig_qty: req.quantity,
            executed_qty: Qty(Decimal::ZERO),
            cumulative_quote_qty: Decimal::ZERO,
            status: OrderStatus::New,
            created_time_ms: now,
            update_time_ms: now,
        };

        st.orders.insert(above_cid.clone(), above_order);
        st.orders.insert(below_cid.clone(), below_order);
        st.oco_lists.insert(
            list_id,
            PaperOco {
                order_list_id: list_id,
                list_client_order_id: req.list_client_order_id.clone(),
                symbol: req.symbol.clone(),
                above_client_id: above_cid.clone(),
                below_client_id: below_cid.clone(),
                active: true,
            },
        );

        let items = vec![
            ListOrderItem {
                symbol: req.symbol.clone(),
                order_id: above_oid,
                client_order_id: above_cid,
            },
            ListOrderItem {
                symbol: req.symbol.clone(),
                order_id: below_oid,
                client_order_id: below_cid,
            },
        ];

        if let Some(tx) = &st.event_tx {
            let _ = tx.try_send(GatewayEvent::ListStatus(ListStatusEvent {
                venue: self.config.venue,
                event_time_ms: now,
                symbol: req.symbol.clone(),
                order_list_id: list_id,
                list_client_order_id: req.list_client_order_id.clone(),
                list_status_type: "EXEC_STARTED".into(),
                list_order_status: "EXECUTING".into(),
                reject_reason: "NONE".into(),
                orders: items.clone(),
            }));
        }

        Ok(OcoOrderAck {
            venue: self.config.venue,
            order_list_id: list_id,
            list_client_order_id: req.list_client_order_id,
            list_order_status: "EXECUTING".into(),
            symbol: req.symbol,
            transaction_time_ms: now,
            orders: items,
        })
    }

    async fn cancel_list(
        &self,
        req: CancelOrderListRequest,
    ) -> Result<CancelOrderListAck, GatewayError> {
        let mut st = self
            .inner
            .lock()
            .map_err(|_| GatewayError::Transport("paper broker lock poisoned".into()))?;
        let now = st.now_ms;
        let list_id = req
            .order_list_id
            .or_else(|| {
                req.list_client_order_id.as_deref().and_then(|lcid| {
                    st.oco_lists
                        .values()
                        .find(|l| l.list_client_order_id == lcid)
                        .map(|l| l.order_list_id)
                })
            })
            .ok_or_else(|| GatewayError::ExchangeRejected {
                status: 400,
                code: -2011,
                kind: ExchangeErrorKind::CancelRejected,
                message: "Unknown order list.".into(),
            })?;

        let (above_cid, below_cid, lcid, sym) = {
            let oco =
                st.oco_lists
                    .get_mut(&list_id)
                    .ok_or_else(|| GatewayError::ExchangeRejected {
                        status: 400,
                        code: -2011,
                        kind: ExchangeErrorKind::CancelRejected,
                        message: "Unknown order list.".into(),
                    })?;
            oco.active = false;
            (
                oco.above_client_id.clone(),
                oco.below_client_id.clone(),
                oco.list_client_order_id.clone(),
                oco.symbol.clone(),
            )
        };

        for cid in [&above_cid, &below_cid] {
            if let Some(ord) = st.orders.get_mut(cid)
                && matches!(ord.status, OrderStatus::New | OrderStatus::PartiallyFilled)
            {
                ord.status = OrderStatus::Canceled;
                ord.update_time_ms = now;
            }
        }

        Ok(CancelOrderListAck {
            venue: self.config.venue,
            order_list_id: list_id,
            list_client_order_id: lcid,
            list_order_status: "ALL_DONE".into(),
            symbol: sym,
        })
    }

    async fn query_order(&self, req: QueryOrderRequest) -> Result<OrderSnapshot, GatewayError> {
        let st = self
            .inner
            .lock()
            .map_err(|_| GatewayError::Transport("paper broker lock poisoned".into()))?;
        let found = if let Some(cid) = &req.orig_client_order_id {
            st.orders.get(cid)
        } else if let Some(oid) = req.order_id {
            st.orders.values().find(|o| o.order_id == oid)
        } else {
            None
        };
        found
            .map(|o| o.to_snapshot(self.config.venue))
            .ok_or_else(|| GatewayError::ExchangeRejected {
                status: 400,
                code: -2013,
                kind: ExchangeErrorKind::OrderNotFound,
                message: "Order does not exist.".into(),
            })
    }

    async fn open_orders(
        &self,
        symbol: Option<&Symbol>,
    ) -> Result<Vec<OrderSnapshot>, GatewayError> {
        let st = self
            .inner
            .lock()
            .map_err(|_| GatewayError::Transport("paper broker lock poisoned".into()))?;
        Ok(st
            .orders
            .values()
            .filter(|o| {
                matches!(o.status, OrderStatus::New | OrderStatus::PartiallyFilled)
                    && symbol.is_none_or(|s| o.symbol == *s)
            })
            .map(|o| o.to_snapshot(self.config.venue))
            .collect())
    }

    async fn account_snapshot(&self) -> Result<AccountSnapshot, GatewayError> {
        let st = self
            .inner
            .lock()
            .map_err(|_| GatewayError::Transport("paper broker lock poisoned".into()))?;
        let balances = st
            .balances
            .iter()
            .map(|(asset, &(free, locked))| AccountBalance {
                asset: asset.clone(),
                free,
                locked,
            })
            .collect();
        Ok(AccountSnapshot {
            venue: self.config.venue,
            can_trade: true,
            update_time_ms: st.now_ms,
            balances,
        })
    }

    async fn exchange_info(&self) -> Result<ExchangeInfoSnapshot, GatewayError> {
        let now_ms = self.inner.lock().map_or(0, |s| s.now_ms);
        Ok(ExchangeInfoSnapshot {
            venue: self.config.venue,
            server_time_ms: now_ms,
            rules: self.config.rules.clone(),
            rate_limits: Vec::new(),
        })
    }
}

#[async_trait]
impl VenueAdapter for PaperBroker {
    fn venue(&self) -> Venue {
        self.config.venue
    }

    fn caps(&self) -> VenueCaps {
        match self.config.venue {
            Venue::Spot => VenueCaps::spot(),
            Venue::UsdsFutures => VenueCaps::usds_futures(3),
        }
    }

    async fn activate(&self) -> Result<ActivationReport, GatewayError> {
        if let Ok(mut st) = self.inner.lock() {
            st.active = true;
        }
        Ok(ActivationReport {
            venue: self.config.venue,
            server_time_offset_ms: 0,
            symbols_loaded: self.config.rules.len(),
            subscription_id: Some(0),
        })
    }

    async fn deactivate(&self) -> Result<(), GatewayError> {
        // Hard rule #7: deactivate NEVER cancels protective OCO/SL orders.
        if let Ok(mut st) = self.inner.lock() {
            st.active = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bot_exchange::{
        ExchangeGateway, GatewayError, PlaceOcoRequest, PlaceOrderRequest, TimeInForce,
        VenueAdapter,
    };
    use bot_types::{OrderKind, OrderStatus, Price, Qty, Side, Symbol};
    use rust_decimal::Decimal;

    use super::{PaperBroker, PaperBrokerConfig};

    #[tokio::test]
    async fn paper_broker_enforces_limit_maker_rejection_and_oco_sibling_cancel() {
        let broker = PaperBroker::new(PaperBrokerConfig::default());
        broker.activate().await.expect("activate");
        let sym = Symbol("BTCUSDT".into());
        broker.on_price_tick(
            &sym,
            Price(Decimal::from(100_u32)),
            Price(Decimal::from(101_u32)),
            1_000,
        );

        // LIMIT_MAKER Buy at 101 (>= ask) rejected with -2010
        let err = broker
            .place_order(PlaceOrderRequest {
                symbol: sym.clone(),
                side: Side::Buy,
                kind: OrderKind::LimitMaker,
                client_order_id: "b01r1S-000001-ENT".into(),
                quantity: Some(Qty(Decimal::from(2_u32))),
                quote_order_qty: None,
                price: Some(Price(Decimal::from(101_u32))),
                stop_price: None,
                time_in_force: None,
            })
            .await
            .expect_err("crossing LIMIT_MAKER must reject");
        assert!(matches!(
            err,
            GatewayError::ExchangeRejected { code: -2010, .. }
        ));

        // Passive Buy at 100 succeeds, fills when ask drops to 100
        let ack = broker
            .place_order(PlaceOrderRequest {
                symbol: sym.clone(),
                side: Side::Buy,
                kind: OrderKind::LimitMaker,
                client_order_id: "b01r1S-000002-ENT".into(),
                quantity: Some(Qty(Decimal::from(2_u32))),
                quote_order_qty: None,
                price: Some(Price(Decimal::from(100_u32))),
                stop_price: None,
                time_in_force: None,
            })
            .await
            .expect("passive buy");
        assert_eq!(ack.status, OrderStatus::New);

        let evs = broker.on_price_tick(
            &sym,
            Price(Decimal::from(99_u32)),
            Price(Decimal::from(100_u32)),
            2_000,
        );
        assert_eq!(evs.len(), 1);

        // Place OCO (TP 105, SL 95) -> trigger TP at bid 105 -> SL auto-canceled!
        let _oco = broker
            .place_oco(PlaceOcoRequest {
                symbol: sym.clone(),
                side: Side::Sell,
                list_client_order_id: "b01r1S-000003-TP1".into(),
                quantity: Qty(Decimal::from(2_u32)),
                above_type: OrderKind::LimitMaker,
                above_client_order_id: Some("b01r1S-000004-TP1".into()),
                above_price: Some(Price(Decimal::from(105_u32))),
                above_stop_price: None,
                above_time_in_force: None,
                below_type: OrderKind::StopLossLimit,
                below_client_order_id: Some("b01r1S-000005-SL".into()),
                below_price: Some(Price(Decimal::from(94_u32))),
                below_stop_price: Some(Price(Decimal::from(95_u32))),
                below_time_in_force: Some(TimeInForce::Gtc),
                below_trailing_delta: None,
            })
            .await
            .expect("place_oco");
        assert_eq!(broker.active_oco_count(), 1);
        assert_eq!(broker.open_order_count(), 2);

        let tp_evs = broker.on_price_tick(
            &sym,
            Price(Decimal::from(105_u32)),
            Price(Decimal::from(106_u32)),
            3_000,
        );
        assert_eq!(tp_evs.len(), 3); // TP fill + SL cancel + ListStatus ALL_DONE
        assert_eq!(broker.active_oco_count(), 0);
        assert_eq!(broker.open_order_count(), 0);
    }
}
