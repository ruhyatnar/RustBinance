//! Passive entry execution with bounded chasing, `-2010` post-only retry, TTL timeout,
//! and journal-first `OrderIntended` persistence (§5.1, §5.4).

use std::time::Duration;

use bot_exchange::{
    CancelOrderRequest, CancelReplaceMode, CancelReplaceRequest, ExchangeErrorKind,
    ExchangeGateway, GatewayError, GatewayEvent, PlaceOrderRequest,
};
use bot_store::{JournalEnvelope, JournalEvent, SqliteStore};
use bot_types::{
    InstrumentRules, Order, OrderKind, OrderPurpose, OrderStatus, Price, Qty, Side, Symbol, Venue,
    floor_to_tick,
};
use rust_decimal::Decimal;
use thiserror::Error;
use time::OffsetDateTime;
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use crate::client_id::format_client_order_id;
use crate::fsm::{FsmApplyOutcome, OrderFsm};

/// Minimum viable partial fill fraction (30% per §5.4).
pub const MIN_VIABLE_FILL_RATIO: Decimal = Decimal::from_parts(3, 0, 0, false, 1);

/// Errors returned by [`execute_passive_entry`].
#[derive(Debug, Error)]
pub enum EntryExecError {
    /// Persistence failure while journaling `OrderIntended` before submission.
    #[error("journal persistence error: {0}")]
    Store(#[from] bot_store::StoreError),
    /// Exchange gateway error.
    #[error("exchange gateway error: {0}")]
    Gateway(#[from] GatewayError),
    /// Invalid `clientOrderId` generation.
    #[error("client order id error: {0}")]
    ClientId(#[from] crate::client_id::ClientIdError),
    /// Invalid price or tick rounding.
    #[error("rules error: {0}")]
    Rules(#[from] bot_types::RulesError),
}

/// Book top-of-book snapshot fed into passive entry chasing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopOfBook {
    /// Best bid price.
    pub bid: Price,
    /// Best ask price.
    pub ask: Price,
    /// Timestamp (ms) of this quote.
    pub ts_ms: i64,
}

/// Parameters for §5.4 passive `LIMIT_MAKER` entry with bounded chasing.
#[derive(Debug, Clone)]
pub struct PassiveEntryPlan {
    /// Target venue.
    pub venue: Venue,
    /// Target symbol.
    pub symbol: Symbol,
    /// Bot prefix for `clientOrderId` (e.g. `"b01"`).
    pub bot_id: String,
    /// Daemon run ID.
    pub run_id: u64,
    /// Position UUID associated with this entry.
    pub position_id: Uuid,
    /// Approved quantity (already sized & floored by `bot-risk`).
    pub qty: Qty,
    /// Maximum entry price allowed (slippage cap — never chased past this).
    pub limit_price_max: Price,
    /// Maximum total entry lifetime before canceling remaining quantity.
    pub ttl: Duration,
    /// Maximum number of `cancelReplace` chases allowed.
    pub max_replaces: u32,
    /// Instrument rules (`tick_size`, `min_notional`, etc.).
    pub rules: InstrumentRules,
    /// Quote asset ticker (default `"USDT"`).
    pub quote_asset: String,
}

/// Outcome of [`execute_passive_entry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassiveEntryOutcome {
    /// Entry filled (fully or viable partial fill >= 30% & >= `min_notional`).
    Filled {
        /// Total filled base quantity.
        filled_qty: Qty,
        /// Volume-weighted average entry fill price.
        avg_fill_price: Price,
        /// Total fees paid in quote currency.
        fees_quote: Decimal,
        /// Number of `cancelReplace` chases performed.
        replaces_used: u32,
        /// Final active `clientOrderId`.
        last_client_id: String,
    },
    /// Partial fill below `min_viable_fill` and below `min_notional` → recorded as dust.
    DustPartial {
        /// Small filled quantity that cannot be sold cleanly.
        filled_qty: Qty,
        /// Average fill price.
        avg_fill_price: Price,
        /// Total fees paid in quote currency.
        fees_quote: Decimal,
    },
    /// Entry expired or rejected with zero fill (missed trade is OK per §5.4).
    Missed {
        /// Human-readable reason.
        reason: String,
    },
}

/// Execute a §5.4 passive Spot Long entry (`LIMIT_MAKER`) with journal-first persistence,
/// event-driven chasing via `cancelReplace`, `-2010` retry on next book tick, and TTL cancel.
///
/// # Errors
/// Returns [`EntryExecError`] on fatal store/rule errors or unrecoverable gateway failures.
#[allow(clippy::too_many_lines)]
pub async fn execute_passive_entry<G: ExchangeGateway + ?Sized>(
    gateway: &G,
    store: &mut SqliteStore,
    fsm: &mut OrderFsm,
    plan: &PassiveEntryPlan,
    book_rx: &mut watch::Receiver<TopOfBook>,
    exec_rx: &mut mpsc::Receiver<GatewayEvent>,
) -> Result<PassiveEntryOutcome, EntryExecError> {
    let deadline = tokio::time::Instant::now() + plan.ttl;
    let mut replaces_used: u32 = 0;
    let mut active_cid: Option<String> = None;
    let mut active_price: Option<Price> = None;
    let mut total_filled = Decimal::ZERO;
    let mut total_notional = Decimal::ZERO;
    let mut total_fees = Decimal::ZERO;
    let mut waiting_for_next_book_after_2010 = false;

    // Initial placement attempt using current top-of-book bid (capped at limit_price_max).
    let init_book = *book_rx.borrow();
    if init_book.bid.as_decimal() > Decimal::ZERO
        && init_book.bid.as_decimal() <= plan.limit_price_max.as_decimal()
    {
        let price = floor_to_tick(init_book.bid, plan.rules.tick_size)?;
        match place_journaled_maker(gateway, store, fsm, plan, price, plan.qty, init_book.ts_ms)
            .await
        {
            Ok(cid) => {
                active_cid = Some(cid);
                active_price = Some(price);
            }
            Err(EntryExecError::Gateway(GatewayError::ExchangeRejected {
                kind: ExchangeErrorKind::NewOrderRejected,
                code: -2010,
                ..
            })) => {
                waiting_for_next_book_after_2010 = true;
            }
            Err(err) => return Err(err),
        }
    } else {
        return Ok(PassiveEntryOutcome::Missed {
            reason: "initial best_bid exceeds limit_price_max or is non-positive".into(),
        });
    }

    loop {
        if total_filled >= plan.qty.as_decimal() {
            break;
        }

        tokio::select! {
            () = tokio::time::sleep_until(deadline) => {
                if let Some(cid) = active_cid.as_deref() {
                    let res = gateway.cancel_order(CancelOrderRequest {
                        symbol: plan.symbol.clone(),
                        orig_client_order_id: Some(cid.to_owned()),
                        order_id: None,
                        new_client_order_id: None,
                    }).await;
                    if let Err(err) = res
                        && !cancel_error_is_benign(&err)
                    {
                        // Order may still be live on the exchange: surface to caller/reconcile.
                        return Err(EntryExecError::Gateway(err));
                    }
                }
                break;
            }
            maybe_ev = exec_rx.recv() => {
                let Some(ev) = maybe_ev else {
                    break;
                };
                if let GatewayEvent::Exec(exec) = ev
                    && let FsmApplyOutcome::Updated {
                        status,
                        new_fill,
                        fee_quote,
                    } = fsm.apply_exec_event(&exec, &plan.quote_asset, None)
                {
                    if let Some(fill) = new_fill {
                        let fq = fill.qty.as_decimal();
                        let fp = fill.price.as_decimal();
                        total_filled = total_filled.saturating_add(fq);
                        total_notional = total_notional.saturating_add(fq.saturating_mul(fp));
                        total_fees = total_fees.saturating_add(fee_quote);
                        store.upsert_fill(&fill)?;
                        let env = JournalEnvelope::new(JournalEvent::FillRecorded { fill });
                        store.append_event(exec.event_time_ms, &env)?;
                    }
                    if active_cid.as_deref() == Some(exec.client_order_id.as_str())
                        && status == OrderStatus::Filled
                    {
                        break;
                    }
                }
            }
            changed = book_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                let book = *book_rx.borrow();
                if book.bid.as_decimal() <= Decimal::ZERO
                    || book.bid.as_decimal() > plan.limit_price_max.as_decimal()
                {
                    continue;
                }
                let target_price = floor_to_tick(book.bid, plan.rules.tick_size)?;
                let remaining = plan.qty.as_decimal().saturating_sub(total_filled);
                if remaining <= Decimal::ZERO {
                    break;
                }

                // Retry after -2010 rejection strictly on next book update (no sleep loop).
                if waiting_for_next_book_after_2010 {
                    match place_journaled_maker(
                        gateway,
                        store,
                        fsm,
                        plan,
                        target_price,
                        Qty(remaining),
                        book.ts_ms,
                    ).await {
                        Ok(cid) => {
                            active_cid = Some(cid);
                            active_price = Some(target_price);
                            waiting_for_next_book_after_2010 = false;
                        }
                        Err(EntryExecError::Gateway(GatewayError::ExchangeRejected {
                            kind: ExchangeErrorKind::NewOrderRejected,
                            code: -2010,
                            ..
                        })) => {}
                        Err(err) => return Err(err),
                    }
                    continue;
                }

                // Bounded chasing: only chase upward when best_bid rises >= 1 tick and <= limit_price_max.
                if let (Some(cur_cid), Some(cur_price)) = (active_cid.clone(), active_price)
                    && replaces_used < plan.max_replaces
                    && target_price.as_decimal() > cur_price.as_decimal()
                    && target_price.as_decimal().saturating_sub(cur_price.as_decimal()) >= plan.rules.tick_size
                {
                    let seq = store.next_order_seq()?;
                    let new_cid = format_client_order_id(
                        &plan.bot_id,
                        plan.run_id,
                        plan.venue,
                        seq,
                        OrderPurpose::Entry,
                    )?;
                    let new_ord = make_entry_order(plan, &new_cid, target_price, Qty(remaining), book.ts_ms);
                    // Journal-first before cancelReplace!
                    let env = JournalEnvelope::new(JournalEvent::OrderIntended { order: new_ord.clone() });
                    store.append_event(book.ts_ms, &env)?;
                    store.upsert_order(&new_ord)?;
                    fsm.register_intended(new_ord);

                    match gateway.cancel_replace(CancelReplaceRequest {
                        symbol: plan.symbol.clone(),
                        side: Side::Buy,
                        kind: OrderKind::LimitMaker,
                        cancel_replace_mode: CancelReplaceMode::StopOnFailure,
                        cancel_orig_client_order_id: Some(cur_cid),
                        cancel_order_id: None,
                        new_client_order_id: new_cid.clone(),
                        quantity: Qty(remaining),
                        price: Some(target_price),
                        time_in_force: None,
                    }).await {
                        Ok(ack) if ack.new_order_result == "SUCCESS" => {
                            replaces_used = replaces_used.saturating_add(1);
                            active_cid = Some(new_cid);
                            active_price = Some(target_price);
                        }
                        Ok(_) => {}
                        Err(GatewayError::ExchangeRejected {
                            kind: ExchangeErrorKind::NewOrderRejected,
                            code: -2010,
                            ..
                        }) => {
                            fsm.mark_rejected(plan.venue, &new_cid, ms_to_dt(book.ts_ms));
                            waiting_for_next_book_after_2010 = true;
                            active_cid = None;
                            active_price = None;
                        }
                        Err(err) => {
                            // On network / unknown gateway error, mark the replacement order as needing
                            // query so reconciliation / operator knows state, and don't assume previous order canceled.
                            fsm.mark_unknown_needs_query(plan.venue, &new_cid, ms_to_dt(book.ts_ms));
                            return Err(EntryExecError::Gateway(err));
                        }
                    }
                }
            }
        }
    }

    // Drain any in-flight execution reports that arrived right before/during cancel.
    while let Ok(ev) = exec_rx.try_recv() {
        if let GatewayEvent::Exec(exec) = ev
            && let FsmApplyOutcome::Updated {
                new_fill,
                fee_quote,
                ..
            } = fsm.apply_exec_event(&exec, &plan.quote_asset, None)
            && let Some(fill) = new_fill
        {
            let fq = fill.qty.as_decimal();
            let fp = fill.price.as_decimal();
            total_filled = total_filled.saturating_add(fq);
            total_notional = total_notional.saturating_add(fq.saturating_mul(fp));
            total_fees = total_fees.saturating_add(fee_quote);
            store.upsert_fill(&fill)?;
            let env = JournalEnvelope::new(JournalEvent::FillRecorded { fill });
            store.append_event(exec.event_time_ms, &env)?;
        }
    }

    if total_filled <= Decimal::ZERO {
        return Ok(PassiveEntryOutcome::Missed {
            reason: "entry TTL expired with zero fill".into(),
        });
    }

    let avg_px = total_notional
        .checked_div(total_filled)
        .unwrap_or(plan.limit_price_max.as_decimal());
    let fill_ratio = total_filled
        .checked_div(plan.qty.as_decimal())
        .unwrap_or(Decimal::ZERO);

    if fill_ratio < MIN_VIABLE_FILL_RATIO && total_notional < plan.rules.min_notional {
        return Ok(PassiveEntryOutcome::DustPartial {
            filled_qty: Qty(total_filled),
            avg_fill_price: Price(avg_px),
            fees_quote: total_fees,
        });
    }

    Ok(PassiveEntryOutcome::Filled {
        filled_qty: Qty(total_filled),
        avg_fill_price: Price(avg_px),
        fees_quote: total_fees,
        replaces_used,
        last_client_id: active_cid.unwrap_or_default(),
    })
}

/// `-2011` / `-2013` on cancel mean the order is already terminal or unknown (e.g. filled
/// just before the cancel): not a reason to abort. Anything else leaves the order state unknown.
fn cancel_error_is_benign(err: &GatewayError) -> bool {
    matches!(
        err,
        GatewayError::ExchangeRejected {
            kind: ExchangeErrorKind::CancelRejected | ExchangeErrorKind::OrderNotFound,
            ..
        }
    )
}

async fn place_journaled_maker<G: ExchangeGateway + ?Sized>(
    gateway: &G,
    store: &mut SqliteStore,
    fsm: &mut OrderFsm,
    plan: &PassiveEntryPlan,
    price: Price,
    qty: Qty,
    ts_ms: i64,
) -> Result<String, EntryExecError> {
    let seq = store.next_order_seq()?;
    let cid = format_client_order_id(
        &plan.bot_id,
        plan.run_id,
        plan.venue,
        seq,
        OrderPurpose::Entry,
    )?;
    let order = make_entry_order(plan, &cid, price, qty, ts_ms);

    // §5.1 Hard principle: journal OrderIntended BEFORE sending to exchange.
    let env = JournalEnvelope::new(JournalEvent::OrderIntended {
        order: order.clone(),
    });
    store.append_event(ts_ms, &env)?;
    store.upsert_order(&order)?;
    fsm.register_intended(order);

    match gateway
        .place_order(PlaceOrderRequest {
            symbol: plan.symbol.clone(),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            client_order_id: cid.clone(),
            quantity: Some(qty),
            quote_order_qty: None,
            price: Some(price),
            stop_price: None,
            time_in_force: None,
        })
        .await
    {
        Ok(_) => Ok(cid),
        Err(err) => {
            if matches!(err, GatewayError::UnknownNeedsQuery { .. }) {
                fsm.mark_unknown_needs_query(plan.venue, &cid, ms_to_dt(ts_ms));
            } else {
                fsm.mark_rejected(plan.venue, &cid, ms_to_dt(ts_ms));
            }
            Err(EntryExecError::Gateway(err))
        }
    }
}

fn make_entry_order(
    plan: &PassiveEntryPlan,
    cid: &str,
    price: Price,
    qty: Qty,
    ts_ms: i64,
) -> Order {
    let dt = ms_to_dt(ts_ms);
    Order {
        venue: plan.venue,
        client_id: cid.to_owned(),
        exchange_id: None,
        symbol: plan.symbol.clone(),
        side: Side::Buy,
        kind: OrderKind::LimitMaker,
        price: Some(price),
        qty,
        filled_qty: Qty(Decimal::ZERO),
        avg_fill_price: None,
        status: OrderStatus::PendingNew,
        position_id: Some(plan.position_id),
        purpose: OrderPurpose::Entry,
        created_at: dt,
        updated_at: dt,
    }
}

fn ms_to_dt(ms: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms).saturating_mul(1_000_000))
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

#[cfg(test)]
mod cancel_tests {
    use super::*;

    #[test]
    fn cancel_rejected_and_not_found_are_benign_others_are_not() {
        let rej = |kind, code| GatewayError::ExchangeRejected {
            status: 400,
            code,
            kind,
            message: String::new(),
        };
        assert!(cancel_error_is_benign(&rej(
            ExchangeErrorKind::CancelRejected,
            -2011
        )));
        assert!(cancel_error_is_benign(&rej(
            ExchangeErrorKind::OrderNotFound,
            -2013
        )));
        assert!(!cancel_error_is_benign(&rej(
            ExchangeErrorKind::TooManyRequests,
            -1003
        )));
        assert!(!cancel_error_is_benign(&GatewayError::InvalidRequest(
            "x".into()
        )));
    }
}
