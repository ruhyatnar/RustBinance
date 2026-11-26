//! Spot Dual-OCO protection manager (§4.4) and software `HardStopWatcher` (§5.5).

use std::collections::BTreeMap;

use bot_exchange::{
    CancelOrderListRequest, ExchangeGateway, GatewayError, PlaceOcoRequest, TimeInForce,
};
use bot_store::{JournalEnvelope, JournalEvent, SqliteStore};
use bot_types::{
    Direction, InstrumentRules, OrderKind, OrderPurpose, Position, Price, Qty, Side, Venue,
    floor_to_step, floor_to_tick,
};
use rust_decimal::Decimal;
use thiserror::Error;
use uuid::Uuid;

use crate::client_id::format_client_order_id;

/// Default limit price offset below stop price for `STOP_LOSS_LIMIT` (30 bps = 0.30%).
pub const DEFAULT_STOP_LIMIT_OFFSET_BPS: Decimal = Decimal::from_parts(30, 0, 0, false, 0);

/// Errors returned by [`DualOcoManager`].
#[derive(Debug, Error)]
pub enum ProtectionError {
    /// Store journaling error.
    #[error("store error: {0}")]
    Store(#[from] bot_store::StoreError),
    /// Exchange gateway error.
    #[error("gateway error: {0}")]
    Gateway(#[from] GatewayError),
    /// Client ID formatting error.
    #[error("client id error: {0}")]
    ClientId(#[from] crate::client_id::ClientIdError),
    /// Filter rounding error.
    #[error("rules error: {0}")]
    Rules(#[from] bot_types::RulesError),
}

/// Active OCO leg tracking for a single Spot position (§4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionProtection {
    /// OCO-A (`tp1_close_ratio` of qty: TP1 + SL), if active.
    pub oco_a_list_id: Option<i64>,
    /// OCO-A list client order ID.
    pub oco_a_list_cid: Option<String>,
    /// OCO-B (runner qty: TP2 + SL), if active.
    pub oco_b_list_id: Option<i64>,
    /// OCO-B list client order ID.
    pub oco_b_list_cid: Option<String>,
    /// Last stop price installed on exchange OCOs.
    pub installed_stop: Price,
    /// Consecutive protection reinstall failures (2× → `EmergencyExit` per §5.5 #1).
    pub reinstall_failures: u32,
    /// SQLite journal sequence number of the latest `PositionStopMoved` event written for this protection.
    pub last_journal_seq: Option<i64>,
}

impl PositionProtection {
    /// Returns `true` if at least one protective OCO list is currently active.
    #[must_use]
    pub const fn has_active_protection(&self) -> bool {
        self.oco_a_list_id.is_some() || self.oco_b_list_id.is_some()
    }
}

/// Manages Dual-OCO installation, TP1 → BE replacement, and monotonic trailing updates (§4.4).
#[derive(Debug, Default)]
pub struct DualOcoManager {
    protections: BTreeMap<Uuid, PositionProtection>,
}

impl DualOcoManager {
    /// Construct an empty [`DualOcoManager`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up protection state for `position_id`.
    #[must_use]
    pub fn get(&self, position_id: Uuid) -> Option<&PositionProtection> {
        self.protections.get(&position_id)
    }

    /// Mutable access to protection state for `position_id`.
    pub fn get_mut(&mut self, position_id: Uuid) -> Option<&mut PositionProtection> {
        self.protections.get_mut(&position_id)
    }

    /// Remove protection tracking when a position is closed.
    pub fn remove(&mut self, position_id: Uuid) -> Option<PositionProtection> {
        self.protections.remove(&position_id)
    }

    /// Record a protection reinstall failure even if no prior protection entry existed (§5.5 #1).
    pub fn record_reinstall_failure(&mut self, position_id: Uuid, fallback_stop: Price) -> u32 {
        let entry = self
            .protections
            .entry(position_id)
            .or_insert(PositionProtection {
                oco_a_list_id: None,
                oco_a_list_cid: None,
                oco_b_list_id: None,
                oco_b_list_cid: None,
                installed_stop: fallback_stop,
                reinstall_failures: 0,
                last_journal_seq: None,
            });
        entry.reinstall_failures = entry.reinstall_failures.saturating_add(1);
        entry.reinstall_failures
    }

    /// Mark OCO-A or OCO-B completed when a `ListStatus` (`ALL_DONE`) arrives from exchange.
    pub fn on_list_done(&mut self, order_list_id: i64) -> Option<(Uuid, bool)> {
        for (&pos_id, prot) in &mut self.protections {
            if prot.oco_a_list_id == Some(order_list_id) {
                prot.oco_a_list_id = None;
                prot.oco_a_list_cid = None;
                return Some((pos_id, true)); // true = was OCO-A (TP1 leg)
            }
            if prot.oco_b_list_id == Some(order_list_id) {
                prot.oco_b_list_id = None;
                prot.oco_b_list_cid = None;
                return Some((pos_id, false));
            }
        }
        None
    }

    /// Install Dual-OCO (OCO-A for TP1 + OCO-B for runner) for a freshly opened Spot position (§4.4).
    ///
    /// Falls back cleanly to a single OCO for the entire open quantity if splitting would
    /// violate `min_qty` or `min_notional`, ensuring the position is NEVER left unprotected.
    ///
    /// # Errors
    /// Returns [`ProtectionError`] on store, rounding, or gateway failure.
    #[allow(
        clippy::too_many_arguments,
        clippy::similar_names,
        clippy::too_many_lines
    )]
    pub async fn install_dual_oco<G: ExchangeGateway + ?Sized>(
        &mut self,
        gateway: &G,
        store: &mut SqliteStore,
        pos: &Position,
        rules: &InstrumentRules,
        tp1_close_ratio: Decimal,
        bot_id: &str,
        run_id: u64,
        ts_ms: i64,
    ) -> Result<PositionProtection, ProtectionError> {
        let total_qty = pos.qty_open.as_decimal();
        let stop_px = floor_to_tick(pos.stop_price, rules.tick_size)?;
        let tp1_px = floor_to_tick(pos.tp1_price, rules.tick_size)?;
        let tp2_px = floor_to_tick(
            pos.tp2_price.unwrap_or_else(|| {
                Price(
                    pos.tp1_price
                        .as_decimal()
                        .saturating_mul(Decimal::from(2_u32)),
                )
            }),
            rules.tick_size,
        )?;

        let raw_a = total_qty.saturating_mul(tp1_close_ratio);
        let qty_a = floor_to_step(Qty(raw_a), rules.step_size)?.as_decimal();
        let qty_b =
            floor_to_step(Qty(total_qty.saturating_sub(qty_a)), rules.step_size)?.as_decimal();

        let can_split = !pos.tp1_done
            && qty_a >= rules.min_qty
            && qty_b >= rules.min_qty
            && qty_a.saturating_mul(stop_px.as_decimal()) >= rules.min_notional
            && qty_b.saturating_mul(stop_px.as_decimal()) >= rules.min_notional;

        // Journal intent before placing exchange-side protection (§4.4).
        let journal_env = JournalEnvelope::new(JournalEvent::PositionStopMoved {
            venue: pos.venue,
            position_id: pos.id,
            new_stop_price: stop_px,
            breakeven_done: pos.breakeven_done,
        });
        let journal_seq = store.append_event(ts_ms, &journal_env)?;

        let prot = if can_split {
            let (aid, acid) = place_single_oco(
                gateway,
                store,
                pos.venue,
                &pos.symbol,
                Qty(qty_a),
                tp1_px,
                stop_px,
                rules.tick_size,
                OrderPurpose::TakeProfit1,
                bot_id,
                run_id,
            )
            .await?;

            // Persist OCO-A immediately so it is never orphaned if OCO-B placement fails.
            self.protections.insert(
                pos.id,
                PositionProtection {
                    oco_a_list_id: Some(aid),
                    oco_a_list_cid: Some(acid.clone()),
                    oco_b_list_id: None,
                    oco_b_list_cid: None,
                    installed_stop: stop_px,
                    reinstall_failures: 0,
                    last_journal_seq: Some(journal_seq),
                },
            );

            let (bid, bcid) = place_single_oco(
                gateway,
                store,
                pos.venue,
                &pos.symbol,
                Qty(qty_b),
                tp2_px,
                stop_px,
                rules.tick_size,
                OrderPurpose::TakeProfit2,
                bot_id,
                run_id,
            )
            .await?;

            PositionProtection {
                oco_a_list_id: Some(aid),
                oco_a_list_cid: Some(acid),
                oco_b_list_id: Some(bid),
                oco_b_list_cid: Some(bcid),
                installed_stop: stop_px,
                reinstall_failures: 0,
                last_journal_seq: Some(journal_seq),
            }
        } else {
            let tp_px = if pos.tp1_done { tp2_px } else { tp1_px };
            let purpose = if pos.tp1_done {
                OrderPurpose::TakeProfit2
            } else {
                OrderPurpose::TakeProfit1
            };
            let (bid, bcid) = place_single_oco(
                gateway,
                store,
                pos.venue,
                &pos.symbol,
                pos.qty_open,
                tp_px,
                stop_px,
                rules.tick_size,
                purpose,
                bot_id,
                run_id,
            )
            .await?;

            PositionProtection {
                oco_a_list_id: None,
                oco_a_list_cid: None,
                oco_b_list_id: Some(bid),
                oco_b_list_cid: Some(bcid),
                installed_stop: stop_px,
                reinstall_failures: 0,
                last_journal_seq: Some(journal_seq),
            }
        };

        self.protections.insert(pos.id, prot.clone());
        Ok(prot)
    }

    /// Replace OCO-B with a new monotonic stop price (e.g. Break-Even after TP1 or Trailing ratchet).
    ///
    /// Journals `PositionStopMoved` BEFORE canceling & placing the replacement OCO (§4.4).
    ///
    /// # Errors
    /// Returns [`ProtectionError`] if journaling, cancellation, or replacement fails.
    #[allow(clippy::too_many_arguments, clippy::similar_names)]
    pub async fn replace_runner_stop<G: ExchangeGateway + ?Sized>(
        &mut self,
        gateway: &G,
        store: &mut SqliteStore,
        pos: &Position,
        new_stop: Price,
        breakeven_done: bool,
        rules: &InstrumentRules,
        bot_id: &str,
        run_id: u64,
        ts_ms: i64,
    ) -> Result<PositionProtection, ProtectionError> {
        let stop_px = floor_to_tick(new_stop, rules.tick_size)?;
        let tp2_px = floor_to_tick(
            pos.tp2_price.unwrap_or_else(|| {
                Price(
                    pos.tp1_price
                        .as_decimal()
                        .saturating_mul(Decimal::from(2_u32)),
                )
            }),
            rules.tick_size,
        )?;

        let old_oco_b = self.protections.get(&pos.id).and_then(|existing| {
            existing
                .oco_b_list_id
                .map(|id| (id, existing.oco_b_list_cid.clone()))
        });
        if let Some((old_id, old_cid)) = old_oco_b
            && gateway
                .cancel_list(CancelOrderListRequest {
                    symbol: pos.symbol.clone(),
                    order_list_id: Some(old_id),
                    list_client_order_id: old_cid,
                })
                .await
                .is_ok()
            && let Some(existing) = self.protections.get_mut(&pos.id)
        {
            existing.oco_b_list_id = None;
            existing.oco_b_list_cid = None;
        }

        let (bid, bcid) = place_single_oco(
            gateway,
            store,
            pos.venue,
            &pos.symbol,
            pos.qty_open,
            tp2_px,
            stop_px,
            rules.tick_size,
            OrderPurpose::TakeProfit2,
            bot_id,
            run_id,
        )
        .await?;

        let env = JournalEnvelope::new(JournalEvent::PositionStopMoved {
            venue: pos.venue,
            position_id: pos.id,
            new_stop_price: stop_px,
            breakeven_done,
        });
        let journal_seq = store.append_event(ts_ms, &env)?;

        let entry = self
            .protections
            .entry(pos.id)
            .or_insert(PositionProtection {
                oco_a_list_id: None,
                oco_a_list_cid: None,
                oco_b_list_id: None,
                oco_b_list_cid: None,
                installed_stop: stop_px,
                reinstall_failures: 0,
                last_journal_seq: Some(journal_seq),
            });
        entry.oco_b_list_id = Some(bid);
        entry.oco_b_list_cid = Some(bcid);
        entry.installed_stop = stop_px;
        entry.reinstall_failures = 0;
        entry.last_journal_seq = Some(journal_seq);

        Ok(entry.clone())
    }
}

#[allow(clippy::too_many_arguments)]
async fn place_single_oco<G: ExchangeGateway + ?Sized>(
    gateway: &G,
    store: &mut SqliteStore,
    venue: Venue,
    symbol: &bot_types::Symbol,
    qty: Qty,
    tp_price: Price,
    stop_price: Price,
    tick_size: Decimal,
    tp_purpose: OrderPurpose,
    bot_id: &str,
    run_id: u64,
) -> Result<(i64, String), ProtectionError> {
    let list_seq = store.next_order_seq()?;
    let tp_seq = store.next_order_seq()?;
    let sl_seq = store.next_order_seq()?;

    let list_cid = format_client_order_id(bot_id, run_id, venue, list_seq, tp_purpose)?;
    let tp_cid = format_client_order_id(bot_id, run_id, venue, tp_seq, tp_purpose)?;
    let sl_cid = format_client_order_id(bot_id, run_id, venue, sl_seq, OrderPurpose::ProtectStop)?;

    let offset = Decimal::ONE - (DEFAULT_STOP_LIMIT_OFFSET_BPS / Decimal::from(10_000_u32));
    let sl_limit = floor_to_tick(
        Price(stop_price.as_decimal().saturating_mul(offset)),
        tick_size,
    )?;

    let ack = gateway
        .place_oco(PlaceOcoRequest {
            symbol: symbol.clone(),
            side: Side::Sell,
            list_client_order_id: list_cid.clone(),
            quantity: qty,
            above_type: OrderKind::LimitMaker,
            above_client_order_id: Some(tp_cid),
            above_price: Some(tp_price),
            above_stop_price: None,
            above_time_in_force: None,
            below_type: OrderKind::StopLossLimit,
            below_client_order_id: Some(sl_cid),
            below_price: Some(sl_limit),
            below_stop_price: Some(stop_price),
            below_time_in_force: Some(TimeInForce::Gtc),
            below_trailing_delta: None,
        })
        .await?;

    Ok((ack.order_list_id, list_cid))
}

/// Software `HardStopWatcher` (§4.4, §5.5 #2): ensures no open position stays breached
/// past `stop_price` (or unprotected) for longer than `hard_stop_grace_ms`.
#[derive(Debug, Default, Clone)]
pub struct HardStopWatcher {
    /// Timestamp (ms) when `stop_price` was first breached per `position_id`.
    breached_since_ms: BTreeMap<Uuid, i64>,
    /// Timestamp (ms) when a position was first observed without exchange protection.
    unprotected_since_ms: BTreeMap<Uuid, i64>,
}

impl HardStopWatcher {
    /// Create a fresh [`HardStopWatcher`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Remove tracking for a closed position.
    pub fn clear(&mut self, position_id: Uuid) {
        self.breached_since_ms.remove(&position_id);
        self.unprotected_since_ms.remove(&position_id);
    }

    /// Evaluate whether `price` has breached `stop_price` beyond `grace_ms` (default 1500 ms).
    ///
    /// Returns `true` when an emergency market exit MUST be triggered immediately.
    pub fn check_stop_breach(
        &mut self,
        position_id: Uuid,
        direction: Direction,
        price: Price,
        stop_price: Price,
        now_ms: i64,
        grace_ms: u64,
    ) -> bool {
        let breached = match direction {
            Direction::Long => price.as_decimal() <= stop_price.as_decimal(),
            Direction::Short => price.as_decimal() >= stop_price.as_decimal(),
        };
        if !breached {
            self.breached_since_ms.remove(&position_id);
            return false;
        }
        let since = *self.breached_since_ms.entry(position_id).or_insert(now_ms);
        let elapsed = now_ms.saturating_sub(since);
        elapsed >= i64::try_from(grace_ms).unwrap_or(i64::MAX)
    }

    /// Track unprotected exposure duration; returns `true` if unprotected for `>= grace_ms`.
    pub fn check_unprotected_duration(
        &mut self,
        position_id: Uuid,
        has_protection: bool,
        now_ms: i64,
        grace_ms: u64,
    ) -> bool {
        if has_protection {
            self.unprotected_since_ms.remove(&position_id);
            return false;
        }
        let since = *self
            .unprotected_since_ms
            .entry(position_id)
            .or_insert(now_ms);
        let elapsed = now_ms.saturating_sub(since);
        elapsed >= i64::try_from(grace_ms).unwrap_or(i64::MAX)
    }
}
