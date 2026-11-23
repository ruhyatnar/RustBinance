//! Startup and periodic reconciliation against exchange truth (§7.6, §8.3).

use std::collections::{BTreeMap, HashSet};

use bot_config::OrphanPolicy;
use bot_exchange::{
    CancelOrderRequest, ExchangeErrorKind, ExchangeGateway, GatewayError, PlaceOrderRequest,
    QueryOrderRequest,
};
use bot_store::{JournalEnvelope, JournalEvent, PersistedEngineState, SqliteStore};
use bot_types::{
    Direction, InstrumentRules, OrderKind, OrderPurpose, OrderStatus, Position, PositionState,
    Price, Qty, Side, StrategyId, Symbol, Venue, floor_to_step,
};
use rust_decimal::Decimal;
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::client_id::format_client_order_id;
use crate::fsm::{OrderFsm, is_terminal};
use crate::protection::{DualOcoManager, ProtectionError};

/// Summary report produced by [`reconcile_venue`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Local active orders resolved via `order.status` query.
    pub resolved_orders: usize,
    /// Orphan exchange orders canceled (`Flatten`) or adopted (`AdoptWithStop`).
    pub orphan_orders_handled: usize,
    /// Orphan base balances flattened or adopted with protective OCO.
    pub orphan_balances_handled: usize,
    /// Open positions whose missing exchange protection was reinstalled.
    pub protections_reinstalled: usize,
    /// True if any material discrepancy was detected (caller may enter `ReduceOnly`).
    pub material_mismatch: bool,
}

/// Errors returned by [`reconcile_venue`].
#[derive(Debug, Error)]
pub enum ReconcileError {
    /// SQLite store error.
    #[error("store error: {0}")]
    Store(#[from] bot_store::StoreError),
    /// Exchange gateway error.
    #[error("gateway error: {0}")]
    Gateway(#[from] GatewayError),
    /// Protection installation error.
    #[error("protection error: {0}")]
    Protection(#[from] ProtectionError),
    /// Client ID formatting error.
    #[error("client id error: {0}")]
    ClientId(#[from] crate::client_id::ClientIdError),
    /// Filter rounding error.
    #[error("rules error: {0}")]
    Rules(#[from] bot_types::RulesError),
}

/// Configuration parameters for [`reconcile_venue`].
#[derive(Debug, Clone)]
pub struct ReconcileConfig {
    /// Target venue.
    pub venue: Venue,
    /// Quote asset ticker (default `"USDT"`).
    pub quote_asset: String,
    /// Policy for exchange orders/balances missing from local journal (§7.6, §8.3).
    pub orphan_policy: OrphanPolicy,
    /// Bot prefix for generating emergency/adoption order IDs.
    pub bot_id: String,
    /// Daemon run ID.
    pub run_id: u64,
    /// TP1 close ratio when installing/adopting Dual-OCO.
    pub tp1_close_ratio: Decimal,
    /// Default stop distance % when adopting an orphan balance (e.g. `2.0` = 2%).
    pub adopt_stop_pct: Decimal,
}

/// Run §8.3 startup/periodic reconciliation for a single venue:
/// 1. Compare local active orders vs exchange `open_orders` → query missing by `clientOrderId`.
/// 2. Detect orphan exchange orders not in local state → apply `orphan_policy`.
/// 3. Detect orphan base balances (> dust / `min_notional`) without open position → apply `orphan_policy`.
/// 4. Verify every open position has active protection on exchange; reinstall if missing.
///
/// # Errors
/// Returns [`ReconcileError`] on store or exchange errors.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn reconcile_venue<G: ExchangeGateway + ?Sized>(
    gateway: &G,
    store: &mut SqliteStore,
    state: &mut PersistedEngineState,
    fsm: &mut OrderFsm,
    oco_mgr: &mut DualOcoManager,
    rules_by_symbol: &BTreeMap<Symbol, InstrumentRules>,
    ref_prices: &BTreeMap<Symbol, Price>,
    cfg: &ReconcileConfig,
    now_ms: i64,
) -> Result<ReconcileReport, ReconcileError> {
    let mut report = ReconcileReport::default();

    let exchange_open = gateway.open_orders(None).await?;
    let account = gateway.account_snapshot().await?;

    let exchange_cids: HashSet<String> = exchange_open
        .iter()
        .map(|o| o.client_order_id.clone())
        .collect();

    // 1. Local active orders missing from exchange open_orders → query by clientOrderId.
    let local_active: Vec<(String, Symbol)> = state
        .orders
        .values()
        .filter(|o| o.venue == cfg.venue && !is_terminal(o.status))
        .map(|o| (o.client_id.clone(), o.symbol.clone()))
        .collect();

    for (cid, sym) in local_active {
        if exchange_cids.contains(&cid) {
            continue;
        }
        match gateway
            .query_order(QueryOrderRequest {
                symbol: sym,
                orig_client_order_id: Some(cid.clone()),
                order_id: None,
            })
            .await
        {
            Ok(snap) => {
                let updated_at = OffsetDateTime::from_unix_timestamp_nanos(
                    i128::from(snap.update_time_ms.max(now_ms)).saturating_mul(1_000_000),
                )
                .unwrap_or(OffsetDateTime::UNIX_EPOCH);
                let avg = if snap.executed_qty.as_decimal() > Decimal::ZERO
                    && snap.cumulative_quote_qty > Decimal::ZERO
                {
                    snap.cumulative_quote_qty
                        .checked_div(snap.executed_qty.as_decimal())
                        .map(Price)
                } else {
                    None
                };
                let env = JournalEnvelope::new(JournalEvent::OrderStatusUpdated {
                    venue: cfg.venue,
                    client_id: cid.clone(),
                    exchange_id: Some(snap.order_id),
                    status: snap.status,
                    filled_qty: snap.executed_qty,
                    avg_fill_price: avg,
                    updated_at,
                });
                let seq = store.append_event(now_ms, &env)?;
                state.apply_event(seq, &env);
                report.resolved_orders += 1;
            }
            Err(GatewayError::ExchangeRejected {
                kind: ExchangeErrorKind::OrderNotFound,
                ..
            }) => {
                let env = JournalEnvelope::new(JournalEvent::OrderStatusUpdated {
                    venue: cfg.venue,
                    client_id: cid.clone(),
                    exchange_id: None,
                    status: OrderStatus::Rejected,
                    filled_qty: Qty(Decimal::ZERO),
                    avg_fill_price: None,
                    updated_at: OffsetDateTime::UNIX_EPOCH,
                });
                let seq = store.append_event(now_ms, &env)?;
                state.apply_event(seq, &env);
                fsm.mark_rejected(cfg.venue, &cid, OffsetDateTime::UNIX_EPOCH);
                report.resolved_orders += 1;
            }
            Err(err) => return Err(ReconcileError::Gateway(err)),
        }
    }

    // 2. Orphan orders on exchange not known in local state or OCO manager.
    let known_oco_lists: HashSet<i64> = state
        .positions
        .keys()
        .filter_map(|pid| oco_mgr.get(*pid))
        .flat_map(|p| [p.oco_a_list_id, p.oco_b_list_id])
        .flatten()
        .collect();

    for ex_ord in &exchange_open {
        let known_local = state
            .orders
            .contains_key(&(cfg.venue, ex_ord.client_order_id.clone()))
            || (ex_ord.order_list_id >= 0 && known_oco_lists.contains(&ex_ord.order_list_id));
        if known_local {
            continue;
        }
        report.material_mismatch = true;
        report.orphan_orders_handled += 1;

        if cfg.orphan_policy == OrphanPolicy::Flatten {
            let _ = gateway
                .cancel_order(CancelOrderRequest {
                    symbol: ex_ord.symbol.clone(),
                    orig_client_order_id: Some(ex_ord.client_order_id.clone()),
                    order_id: Some(ex_ord.order_id),
                    new_client_order_id: None,
                })
                .await;
        }
    }

    // 3. Orphan base balances (> min_notional) without an open local position.
    let open_symbols: HashSet<Symbol> = state
        .positions
        .values()
        .filter(|p| p.venue == cfg.venue && p.state != PositionState::Closed)
        .map(|p| p.symbol.clone())
        .collect();

    for bal in &account.balances {
        if bal.asset.eq_ignore_ascii_case(&cfg.quote_asset) {
            continue;
        }
        let total_base = bal.free.saturating_add(bal.locked);
        if total_base <= Decimal::ZERO {
            continue;
        }
        let sym = Symbol(format!("{}{}", bal.asset, cfg.quote_asset));
        if open_symbols.contains(&sym) {
            continue;
        }
        let Some(rules) = rules_by_symbol.get(&sym) else {
            continue;
        };
        let Some(&ref_px) = ref_prices.get(&sym) else {
            continue;
        };
        let qty = floor_to_step(Qty(total_base), rules.step_size)?;
        let notional = qty.as_decimal().saturating_mul(ref_px.as_decimal());
        if qty.as_decimal() < rules.min_qty || notional < rules.min_notional {
            // Dust below min_notional: log/ignore cleanly per §7.6.
            continue;
        }

        report.material_mismatch = true;
        report.orphan_balances_handled += 1;

        match cfg.orphan_policy {
            OrphanPolicy::Flatten => {
                let seq = store.next_order_seq()?;
                let cid = format_client_order_id(
                    &cfg.bot_id,
                    cfg.run_id,
                    cfg.venue,
                    seq,
                    OrderPurpose::ManualFlatten,
                )?;
                let _ = gateway
                    .place_order(PlaceOrderRequest {
                        symbol: sym,
                        side: Side::Sell,
                        kind: OrderKind::Market,
                        client_order_id: cid,
                        quantity: Some(qty),
                        quote_order_qty: None,
                        price: None,
                        stop_price: None,
                        time_in_force: None,
                    })
                    .await?;
            }
            OrphanPolicy::AdoptWithStop => {
                let stop_factor = Decimal::ONE - (cfg.adopt_stop_pct / Decimal::from(100_u32));
                let stop_px = Price(ref_px.as_decimal().saturating_mul(stop_factor));
                let r_dist = (ref_px.as_decimal() - stop_px.as_decimal()).abs();
                let tp1_px = Price(ref_px.as_decimal().saturating_add(r_dist));
                let tp2_px = Price(
                    ref_px
                        .as_decimal()
                        .saturating_add(r_dist.saturating_mul(Decimal::from(2_u32))),
                );
                let pos = Position {
                    id: Uuid::now_v7(),
                    symbol: sym.clone(),
                    venue: cfg.venue,
                    direction: Direction::Long,
                    strategy: StrategyId::TrendPullback,
                    entry_price: ref_px,
                    qty_initial: qty,
                    qty_open: qty,
                    r_distance: r_dist,
                    leverage: None,
                    liq_price: None,
                    stop_price: stop_px,
                    tp1_price: tp1_px,
                    tp2_price: Some(tp2_px),
                    best_price: ref_px,
                    tp1_done: false,
                    breakeven_done: false,
                    fees_paid_quote: Decimal::ZERO,
                    realized_pnl_quote: Decimal::ZERO,
                    state: PositionState::OpenProtected,
                    opened_at: OffsetDateTime::UNIX_EPOCH,
                };
                let env = JournalEnvelope::new(JournalEvent::PositionOpened {
                    position: pos.clone(),
                });
                let seq = store.append_event(now_ms, &env)?;
                state.apply_event(seq, &env);
                store.upsert_position(&pos, None, None)?;
                oco_mgr
                    .install_dual_oco(
                        gateway,
                        store,
                        &pos,
                        rules,
                        cfg.tp1_close_ratio,
                        &cfg.bot_id,
                        cfg.run_id,
                        now_ms,
                    )
                    .await?;
            }
        }
    }

    // 4. Ensure every open local position has active exchange-side protection (§8.3 #5).
    let open_positions: Vec<Position> = state
        .positions
        .values()
        .filter(|p| {
            p.venue == cfg.venue
                && matches!(
                    p.state,
                    PositionState::OpenProtected | PositionState::OpenUnprotected
                )
        })
        .cloned()
        .collect();

    for pos in open_positions {
        let has_exchange_sell = exchange_open
            .iter()
            .any(|o| o.symbol == pos.symbol && o.side == Side::Sell && o.order_list_id >= 0);
        if !has_exchange_sell && let Some(rules) = rules_by_symbol.get(&pos.symbol) {
            oco_mgr
                .install_dual_oco(
                    gateway,
                    store,
                    &pos,
                    rules,
                    cfg.tp1_close_ratio,
                    &cfg.bot_id,
                    cfg.run_id,
                    now_ms,
                )
                .await?;
            report.protections_reinstalled += 1;
        }
    }

    Ok(report)
}
