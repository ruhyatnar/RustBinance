//! Order Finite State Machine (§5.3) with monotonic `filled_qty`, idempotent fill dedup,
//! and fee normalization into quote currency.

use std::collections::{BTreeMap, HashSet};

use bot_exchange::ExecEvent;
use bot_store::FillRecord;
use bot_types::{Order, OrderStatus, Price, Qty, Symbol, Venue};
use rust_decimal::Decimal;
use time::OffsetDateTime;

/// Maximum consecutive `order.status` queries before marking `UnknownNeedsQuery` as `Rejected(Lost)`.
pub const MAX_UNKNOWN_QUERY_ATTEMPTS: u32 = 3;

/// Outcome of applying an [`ExecEvent`] to [`OrderFsm`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsmApplyOutcome {
    /// Order was not tracked by this FSM (e.g. external/orphan order).
    UnknownOrder,
    /// Duplicate fill `(venue, symbol, trade_id)` ignored idempotently.
    DuplicateFillIgnored,
    /// Out-of-order stale event ignored without regressing state.
    StaleEventIgnored,
    /// Order state updated cleanly (with optional newly recorded fill).
    Updated {
        /// New order status.
        status: OrderStatus,
        /// Newly accepted fill record, if this event was a new trade execution.
        new_fill: Option<FillRecord>,
        /// Fee converted into quote currency (USD/USDT).
        fee_quote: Decimal,
    },
}

/// Pure, deterministic Order FSM enforcing §5.3 invariants.
#[derive(Debug, Default, Clone)]
pub struct OrderFsm {
    orders: BTreeMap<(Venue, String), Order>,
    seen_fills: HashSet<(Venue, Symbol, i64)>,
    unknown_query_attempts: BTreeMap<(Venue, String), u32>,
}

impl OrderFsm {
    /// Create an empty [`OrderFsm`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed FSM from recovered active orders and seen fills.
    #[must_use]
    pub fn from_recovered(
        orders: BTreeMap<(Venue, String), Order>,
        fills: &BTreeMap<(Venue, Symbol, i64), FillRecord>,
    ) -> Self {
        let seen_fills = fills.keys().cloned().collect();
        Self {
            orders,
            seen_fills,
            unknown_query_attempts: BTreeMap::new(),
        }
    }

    /// Register a newly intended order (must be called AFTER journaling `OrderIntended`).
    pub fn register_intended(&mut self, order: Order) {
        let key = (order.venue, order.client_id.clone());
        self.orders.insert(key, order);
    }

    /// Get a tracked order by `(venue, client_id)`.
    #[must_use]
    pub fn get(&self, venue: Venue, client_id: &str) -> Option<&Order> {
        self.orders.get(&(venue, client_id.to_owned()))
    }

    /// Iterate over all currently tracked orders.
    pub fn orders(&self) -> impl Iterator<Item = &Order> {
        self.orders.values()
    }

    /// Iterate over all non-terminal orders for a given venue.
    #[must_use]
    pub fn active_orders(&self, venue: Venue) -> Vec<&Order> {
        self.orders
            .values()
            .filter(|o| o.venue == venue && !is_terminal(o.status))
            .collect()
    }

    /// Transition an order into `UnknownNeedsQuery` after a timeout or 5xx response (§5.3).
    pub fn mark_unknown_needs_query(
        &mut self,
        venue: Venue,
        client_id: &str,
        now: OffsetDateTime,
    ) -> Option<OrderStatus> {
        let key = (venue, client_id.to_owned());
        let order = self.orders.get_mut(&key)?;
        if is_terminal(order.status) {
            return Some(order.status);
        }
        order.status = OrderStatus::UnknownNeedsQuery;
        order.updated_at = now;
        Some(order.status)
    }

    /// Record a failed query attempt (`UnknownOrder` from exchange) for an `UnknownNeedsQuery` order.
    ///
    /// After [`MAX_UNKNOWN_QUERY_ATTEMPTS`] (3) consecutive not-found responses, transitions
    /// the order to [`OrderStatus::Rejected`] (`Lost`) per §5.3.
    pub fn record_unknown_query_not_found(
        &mut self,
        venue: Venue,
        client_id: &str,
        now: OffsetDateTime,
    ) -> Option<OrderStatus> {
        let key = (venue, client_id.to_owned());
        let order = self.orders.get_mut(&key)?;
        if order.status != OrderStatus::UnknownNeedsQuery {
            return Some(order.status);
        }
        let attempts = self.unknown_query_attempts.entry(key).or_insert(0);
        *attempts = attempts.saturating_add(1);
        if *attempts >= MAX_UNKNOWN_QUERY_ATTEMPTS {
            order.status = OrderStatus::Rejected;
            order.updated_at = now;
        }
        Some(order.status)
    }

    /// Mark an order rejected (e.g. immediate exchange rejection `-2010`).
    pub fn mark_rejected(
        &mut self,
        venue: Venue,
        client_id: &str,
        now: OffsetDateTime,
    ) -> Option<OrderStatus> {
        let key = (venue, client_id.to_owned());
        let order = self.orders.get_mut(&key)?;
        if is_terminal(order.status) {
            return Some(order.status);
        }
        order.status = OrderStatus::Rejected;
        order.updated_at = now;
        Some(order.status)
    }

    /// Apply an [`ExecEvent`] idempotently, enforcing monotonic `filled_qty` and converting fees.
    ///
    /// `fee_asset_to_quote_rate` converts non-quote commission assets (e.g. BNB → USDT).
    /// When commission is in base asset, `exec.last_filled_price` is used automatically.
    #[allow(clippy::too_many_lines)]
    pub fn apply_exec_event(
        &mut self,
        exec: &ExecEvent,
        quote_asset: &str,
        fee_asset_to_quote_rate: Option<Decimal>,
    ) -> FsmApplyOutcome {
        // Resolve by client_order_id or orig_client_order_id (for CANCELED reports).
        let key = {
            let primary = (exec.venue, exec.client_order_id.clone());
            if self.orders.contains_key(&primary) {
                primary
            } else if let Some(orig) = &exec.orig_client_order_id {
                let fallback = (exec.venue, orig.clone());
                if self.orders.contains_key(&fallback) {
                    fallback
                } else {
                    return FsmApplyOutcome::UnknownOrder;
                }
            } else {
                return FsmApplyOutcome::UnknownOrder;
            }
        };

        let is_trade = exec.trade_id >= 0 && exec.last_filled_qty.as_decimal() > Decimal::ZERO;
        if is_trade {
            let fill_key = (exec.venue, exec.symbol.clone(), exec.trade_id);
            if self.seen_fills.contains(&fill_key) {
                return FsmApplyOutcome::DuplicateFillIgnored;
            }
            self.seen_fills.insert(fill_key);
        }

        let Some(order) = self.orders.get_mut(&key) else {
            return FsmApplyOutcome::UnknownOrder;
        };

        // Do not regress a terminal order to a non-terminal status on out-of-order events.
        if is_terminal(order.status) && !is_terminal(exec.status) {
            return FsmApplyOutcome::StaleEventIgnored;
        }

        order.exchange_id = Some(exec.order_id);
        self.unknown_query_attempts.remove(&key);

        let mut new_fill = None;
        let mut fee_quote = Decimal::ZERO;

        if is_trade {
            let prev_qty = order.filled_qty.as_decimal();
            let prev_avg = order
                .avg_fill_price
                .map_or(Decimal::ZERO, Price::as_decimal);
            let fill_qty = exec.last_filled_qty.as_decimal();
            let fill_price = exec.last_filled_price.as_decimal();

            // §5.3 invariant: filled_qty is strictly monotonic non-decreasing.
            let candidate_cum = exec
                .cumulative_filled_qty
                .as_decimal()
                .max(prev_qty.saturating_add(fill_qty));
            let new_cum = prev_qty.max(candidate_cum);

            if new_cum > Decimal::ZERO {
                if exec.cumulative_quote_qty > Decimal::ZERO
                    && exec.cumulative_filled_qty.as_decimal() >= new_cum
                {
                    if let Some(avg) = exec.cumulative_quote_qty.checked_div(new_cum) {
                        order.avg_fill_price = Some(Price(avg));
                    }
                } else {
                    let prev_notional = prev_qty.saturating_mul(prev_avg);
                    let add_notional = fill_qty.saturating_mul(fill_price);
                    let denom = prev_qty.saturating_add(fill_qty);
                    if denom > Decimal::ZERO
                        && let Some(avg) = prev_notional
                            .saturating_add(add_notional)
                            .checked_div(denom)
                    {
                        order.avg_fill_price = Some(Price(avg));
                    }
                }
            }
            order.filled_qty = Qty(new_cum);

            let fee_asset = exec
                .commission_asset
                .clone()
                .unwrap_or_else(|| quote_asset.to_owned());
            fee_quote = convert_fee_to_quote(
                exec.commission,
                &fee_asset,
                quote_asset,
                exec.symbol.as_str(),
                fill_price,
                fee_asset_to_quote_rate,
            );

            new_fill = Some(FillRecord {
                venue: exec.venue,
                trade_id: exec.trade_id,
                symbol: exec.symbol.clone(),
                client_id: order.client_id.clone(),
                price: exec.last_filled_price,
                qty: exec.last_filled_qty,
                fee: exec.commission,
                fee_asset,
                is_maker: exec.is_maker,
                ts_ms: exec.transact_time_ms,
            });
        } else if exec.cumulative_filled_qty.as_decimal() > order.filled_qty.as_decimal() {
            order.filled_qty = exec.cumulative_filled_qty;
        }

        // Advance status without regressing Filled -> PartiallyFilled on late events.
        if !(order.status == OrderStatus::Filled && exec.status == OrderStatus::PartiallyFilled) {
            order.status = exec.status;
        }
        if let Ok(ts) = OffsetDateTime::from_unix_timestamp_nanos(
            i128::from(exec.event_time_ms).saturating_mul(1_000_000),
        ) {
            order.updated_at = ts;
        }

        FsmApplyOutcome::Updated {
            status: order.status,
            new_fill,
            fee_quote,
        }
    }
}

/// Returns `true` if `status` is a terminal order state.
#[must_use]
pub const fn is_terminal(status: OrderStatus) -> bool {
    matches!(
        status,
        OrderStatus::Filled | OrderStatus::Canceled | OrderStatus::Rejected | OrderStatus::Expired
    )
}

/// Convert commission from `fee_asset` into quote currency (`quote_asset`).
#[must_use]
pub fn convert_fee_to_quote(
    commission: Decimal,
    fee_asset: &str,
    quote_asset: &str,
    symbol: &str,
    fill_price: Decimal,
    fee_asset_to_quote_rate: Option<Decimal>,
) -> Decimal {
    if commission <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    if fee_asset.eq_ignore_ascii_case(quote_asset) {
        return commission;
    }
    if let Some(rate) = fee_asset_to_quote_rate
        && rate > Decimal::ZERO
    {
        return commission.saturating_mul(rate);
    }
    // If fee was paid in the base asset (e.g. BTC on BTCUSDT), convert via fill_price.
    let base_prefix = symbol.strip_suffix(quote_asset).unwrap_or("");
    if !base_prefix.is_empty()
        && fee_asset.eq_ignore_ascii_case(base_prefix)
        && fill_price > Decimal::ZERO
    {
        return commission.saturating_mul(fill_price);
    }
    commission
}

#[cfg(test)]
mod tests {
    use bot_exchange::ExecEvent;
    use bot_types::{Order, OrderKind, OrderPurpose, OrderStatus, Price, Qty, Side, Symbol, Venue};
    use rust_decimal::Decimal;
    use time::OffsetDateTime;

    use super::{FsmApplyOutcome, OrderFsm};

    fn sample_order() -> Order {
        Order {
            venue: Venue::Spot,
            client_id: "b01r1S-000001-ENT".into(),
            exchange_id: None,
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            price: Some(Price(Decimal::from(100_u32))),
            qty: Qty(Decimal::from(2_u32)),
            filled_qty: Qty(Decimal::ZERO),
            avg_fill_price: None,
            status: OrderStatus::PendingNew,
            position_id: None,
            purpose: OrderPurpose::Entry,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn fill_exec(trade_id: i64, last_qty: u32, cum_qty: u32, status: OrderStatus) -> ExecEvent {
        ExecEvent {
            venue: Venue::Spot,
            event_time_ms: 1_700_000_000_000,
            transact_time_ms: 1_700_000_000_000,
            symbol: Symbol("BTCUSDT".into()),
            client_order_id: "b01r1S-000001-ENT".into(),
            orig_client_order_id: None,
            order_id: 500,
            order_list_id: -1,
            trade_id,
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            exec_type: "TRADE".into(),
            status,
            reject_reason: "NONE".into(),
            order_price: Price(Decimal::from(100_u32)),
            order_qty: Qty(Decimal::from(2_u32)),
            last_filled_qty: Qty(Decimal::from(last_qty)),
            cumulative_filled_qty: Qty(Decimal::from(cum_qty)),
            last_filled_price: Price(Decimal::from(100_u32)),
            cumulative_quote_qty: Decimal::from(cum_qty * 100),
            commission: Decimal::new(1, 1),
            commission_asset: Some("USDT".into()),
            is_maker: true,
        }
    }

    #[test]
    fn duplicate_fill_is_idempotent_and_unknown_resolves_after_3_misses() {
        let mut fsm = OrderFsm::new();
        fsm.register_intended(sample_order());

        let f1 = fill_exec(42, 1, 1, OrderStatus::PartiallyFilled);
        let r1 = fsm.apply_exec_event(&f1, "USDT", None);
        assert!(matches!(
            r1,
            FsmApplyOutcome::Updated {
                status: OrderStatus::PartiallyFilled,
                new_fill: Some(_),
                ..
            }
        ));
        assert_eq!(
            fsm.get(Venue::Spot, "b01r1S-000001-ENT")
                .expect("order")
                .filled_qty
                .as_decimal(),
            Decimal::ONE
        );

        // Duplicate trade_id=42 must be ignored without changing filled_qty!
        let r_dup = fsm.apply_exec_event(&f1, "USDT", None);
        assert_eq!(r_dup, FsmApplyOutcome::DuplicateFillIgnored);
        assert_eq!(
            fsm.get(Venue::Spot, "b01r1S-000001-ENT")
                .expect("order")
                .filled_qty
                .as_decimal(),
            Decimal::ONE
        );

        // UnknownNeedsQuery -> 3 not-found queries -> Rejected
        let mut ord2 = sample_order();
        ord2.client_id = "b01r1S-000002-ENT".into();
        fsm.register_intended(ord2);
        fsm.mark_unknown_needs_query(Venue::Spot, "b01r1S-000002-ENT", OffsetDateTime::UNIX_EPOCH);
        assert_eq!(
            fsm.record_unknown_query_not_found(
                Venue::Spot,
                "b01r1S-000002-ENT",
                OffsetDateTime::UNIX_EPOCH
            ),
            Some(OrderStatus::UnknownNeedsQuery)
        );
        assert_eq!(
            fsm.record_unknown_query_not_found(
                Venue::Spot,
                "b01r1S-000002-ENT",
                OffsetDateTime::UNIX_EPOCH
            ),
            Some(OrderStatus::UnknownNeedsQuery)
        );
        assert_eq!(
            fsm.record_unknown_query_not_found(
                Venue::Spot,
                "b01r1S-000002-ENT",
                OffsetDateTime::UNIX_EPOCH
            ),
            Some(OrderStatus::Rejected)
        );
    }
}
