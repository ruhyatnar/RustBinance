//! Real [`IpcBackend`] and [`TelemetryHub`] bridge wired directly to [`EngineHandle`].

use std::sync::Arc;

use async_trait::async_trait;
use bot_config::BotConfig;
use bot_engine::{EngineFullSnapshot, EngineHandle, TelemetrySnapshot};
use bot_exchange::{RateLimitInterval, RateLimitTracker, RateLimitType};
use bot_ipc::protocol::{
    DeltaPayload, EngineSummary, OrderView, PositionView, RateStatus, RiskLimitsSummary,
    RiskSummary, ServerMessage, SnapshotPayload, VenuesSummary,
};
use bot_ipc::{IpcBackend, IpcError, TelemetryHub};
use bot_risk::ModeTrigger;
use bot_types::{Direction, EngineMode, Position, Price, TradingMode, Venue};
use rust_decimal::Decimal;
use time::OffsetDateTime;
use tokio::task::JoinHandle;
use uuid::Uuid;

const DEFAULT_SPOT_WEIGHT_LIMIT: u32 = 6000;
const DEFAULT_FUTURES_WEIGHT_LIMIT: u32 = 2400;

fn unix_now_ms() -> i64 {
    let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();
    i64::try_from(nanos / 1_000_000).unwrap_or(0)
}

/// Shared per-venue [`RateLimitTracker`] handles exposed to IPC snapshots and telemetry deltas.
#[derive(Debug, Default, Clone)]
pub struct VenueRateTrackers {
    /// Spot adapter rate-limit tracker (present in `Testnet` / `Live` when Spot is enabled).
    pub spot: Option<Arc<RateLimitTracker>>,
    /// USDⓈ-M Futures adapter rate-limit tracker (present in `Testnet` / `Live` when Futures is enabled).
    pub futures: Option<Arc<RateLimitTracker>>,
}

impl VenueRateTrackers {
    /// Read the current [`RateStatus`] for `venue` from its tracked buckets.
    #[must_use]
    pub fn rate_status_for(&self, venue: Venue) -> RateStatus {
        let default_limit = match venue {
            Venue::Spot => DEFAULT_SPOT_WEIGHT_LIMIT,
            Venue::UsdsFutures => DEFAULT_FUTURES_WEIGHT_LIMIT,
        };
        let tracker = match venue {
            Venue::Spot => self.spot.as_ref(),
            Venue::UsdsFutures => self.futures.as_ref(),
        };
        let Some(tracker) = tracker else {
            return RateStatus {
                weight_used: 0,
                weight_limit: default_limit,
                orders_10s: 0,
            };
        };

        let buckets = tracker.snapshot();
        let mut weight_used: Option<u64> = None;
        let mut weight_limit: Option<u64> = None;
        let mut weight_exact = false;
        let mut orders_10s: Option<u64> = None;
        let mut orders_exact = false;

        for (key, state) in &buckets {
            match key.rate_limit_type {
                RateLimitType::RequestWeight => {
                    let is_1m = key.interval == RateLimitInterval::Minute && key.interval_num == 1;
                    if is_1m || !weight_exact {
                        weight_used = Some(state.count);
                        if state.limit > 0 {
                            weight_limit = Some(state.limit);
                        }
                        if is_1m {
                            weight_exact = true;
                        }
                    }
                }
                RateLimitType::Orders => {
                    let is_10s =
                        key.interval == RateLimitInterval::Second && key.interval_num == 10;
                    if is_10s || !orders_exact {
                        orders_10s = Some(state.count);
                        if is_10s {
                            orders_exact = true;
                        }
                    }
                }
                RateLimitType::RawRequests | RateLimitType::Unknown => {}
            }
        }

        RateStatus {
            weight_used: weight_used.map_or(0, |v| u32::try_from(v).unwrap_or(u32::MAX)),
            weight_limit: weight_limit
                .map_or(default_limit, |v| u32::try_from(v).unwrap_or(u32::MAX)),
            orders_10s: orders_10s.map_or(0, |v| u32::try_from(v).unwrap_or(u32::MAX)),
        }
    }
}

/// Resolve the mark price for a position, preferring a live reference price when available
/// and falling back to `position.best_price`.
#[must_use]
pub const fn resolve_mark_price(position: &Position, ref_price: Option<Price>) -> Price {
    match ref_price {
        Some(p) => p,
        None => position.best_price,
    }
}

/// Compute direction-aware `(profit_r, unrealized_quote)` for an open position using exact [`Decimal`].
#[must_use]
pub fn compute_position_pnl(position: &Position, mark_price: Price) -> (Decimal, Decimal) {
    let entry = position.entry_price.as_decimal();
    let mark = mark_price.as_decimal();
    let price_diff = match position.direction {
        Direction::Long => mark.saturating_sub(entry),
        Direction::Short => entry.saturating_sub(mark),
    };
    let unrealized_quote = price_diff.saturating_mul(position.qty_open.as_decimal());
    let profit_r = if position.r_distance > Decimal::ZERO {
        price_diff
            .checked_div(position.r_distance)
            .map_or(Decimal::ZERO, |r| r.round_dp(4))
    } else {
        Decimal::ZERO
    };
    (profit_r, unrealized_quote)
}

fn compute_uptime_s(started_at_ms: i64, now_ms: i64) -> u64 {
    u64::try_from(now_ms.saturating_sub(started_at_ms).max(0) / 1000).unwrap_or(0)
}

/// [`IpcBackend`] implementation backed by the live [`EngineHandle`] of [`bot_engine::EngineActor`].
pub struct EngineIpcBackend {
    engine_instance_id: Uuid,
    config: BotConfig,
    handle: EngineHandle,
    started_at_ms: i64,
    rate_trackers: VenueRateTrackers,
}

impl EngineIpcBackend {
    /// Construct a new IPC backend wired to `handle`.
    #[must_use]
    pub fn new(config: BotConfig, handle: EngineHandle) -> Self {
        Self {
            engine_instance_id: Uuid::now_v7(),
            config,
            handle,
            started_at_ms: unix_now_ms(),
            rate_trackers: VenueRateTrackers::default(),
        }
    }

    /// Attach real per-venue [`RateLimitTracker`] handles for live [`RateStatus`] reporting.
    #[must_use]
    pub fn with_rate_trackers(mut self, rate_trackers: VenueRateTrackers) -> Self {
        self.rate_trackers = rate_trackers;
        self
    }

    fn map_full_snapshot(&self, full: &EngineFullSnapshot, now_ms: i64) -> SnapshotPayload {
        let uptime_s = compute_uptime_s(full.started_at_ms, now_ms);
        let positions = full
            .positions
            .iter()
            .map(|p| {
                let opened_ms =
                    i64::try_from(p.opened_at.unix_timestamp_nanos() / 1_000_000).unwrap_or(now_ms);
                let age_s =
                    u64::try_from(now_ms.saturating_sub(opened_ms).max(0) / 1000).unwrap_or(0);
                // TODO(verifier): pass `full.ref_prices.get(&(p.venue, p.symbol.clone())).copied()`
                // once `exec-engine-fixer` adds `ref_prices` to `EngineFullSnapshot`.
                let mark = resolve_mark_price(p, None);
                let (profit_r, unrealized_quote) = compute_position_pnl(p, mark);
                PositionView {
                    id: p.id,
                    venue: p.venue,
                    symbol: p.symbol.as_str().to_owned(),
                    strategy: format!("{:?}", p.strategy),
                    state: format!("{:?}", p.state),
                    entry: p.entry_price.as_decimal(),
                    mark: mark.as_decimal(),
                    qty: p.qty_open.as_decimal(),
                    stop: p.stop_price.as_decimal(),
                    tp1: p.tp1_price.as_decimal(),
                    tp2: p
                        .tp2_price
                        .map_or_else(|| p.tp1_price.as_decimal(), bot_types::Price::as_decimal),
                    profit_r,
                    unrealized_quote,
                    age_s,
                    tp1_done: p.tp1_done,
                    be_done: p.breakeven_done,
                }
            })
            .collect();

        let orders = full
            .orders
            .iter()
            .map(|o| OrderView {
                client_id: o.client_id.clone(),
                venue: o.venue,
                symbol: o.symbol.as_str().to_owned(),
                side: format!("{:?}", o.side),
                kind: format!("{:?}", o.kind),
                purpose: format!("{:?}", o.purpose),
                status: format!("{:?}", o.status),
                price: o.price.map(bot_types::Price::as_decimal),
                qty: o.qty.as_decimal(),
                filled_qty: o.filled_qty.as_decimal(),
            })
            .collect();

        SnapshotPayload {
            venues: VenuesSummary {
                active: full.telemetry.venue,
                spot: full.telemetry.spot_state,
                futures: full.telemetry.futures_state,
                last_switch_ms: u64::try_from(full.last_switch_ms).unwrap_or(0),
                switches_today: full.switches_today,
            },
            engine: EngineSummary {
                mode: full.telemetry.mode,
                build_mode: self.config.general.mode,
                uptime_s,
                ws_latency_ms: u64::from(full.health.ws_latency_ms),
                clock_drift_ms: full.health.clock_drift_ms,
                rate: self.rate_trackers.rate_status_for(full.telemetry.venue),
            },
            risk: RiskSummary {
                equity: full.telemetry.equity_total_usd,
                day_pnl_pct: full.day_pnl_pct,
                week_pnl_pct: full.week_pnl_pct,
                drawdown_pct: full.drawdown_pct,
                heat_pct: full.heat_pct,
                heat_max_pct: full.config.risk.portfolio_heat_max_pct,
                open_positions: u32::try_from(full.telemetry.open_positions).unwrap_or(u32::MAX),
                max_positions: full.config.risk.max_open_positions,
                btc_guard: full.btc_guard.clone(),
                limits: RiskLimitsSummary {
                    daily_halt_pct: full.config.risk.daily_loss_halt_pct,
                    kill_pct: full.config.risk.drawdown_kill_pct,
                },
            },
            positions,
            orders,
            signals_shadow: Vec::new(),
            alerts: Vec::new(),
        }
    }

    fn fallback_snapshot(&self, snap: &TelemetrySnapshot, now_ms: i64) -> SnapshotPayload {
        SnapshotPayload {
            venues: VenuesSummary {
                active: snap.venue,
                spot: snap.spot_state,
                futures: snap.futures_state,
                last_switch_ms: 0,
                switches_today: 0,
            },
            engine: EngineSummary {
                mode: snap.mode,
                build_mode: self.config.general.mode,
                uptime_s: compute_uptime_s(self.started_at_ms, now_ms),
                ws_latency_ms: 0,
                clock_drift_ms: 0,
                rate: self.rate_trackers.rate_status_for(snap.venue),
            },
            risk: RiskSummary {
                equity: snap.equity_total_usd,
                day_pnl_pct: Decimal::ZERO,
                week_pnl_pct: Decimal::ZERO,
                drawdown_pct: Decimal::ZERO,
                heat_pct: Decimal::ZERO,
                heat_max_pct: self.config.risk.portfolio_heat_max_pct,
                open_positions: u32::try_from(snap.open_positions).unwrap_or(u32::MAX),
                max_positions: self.config.risk.max_open_positions,
                btc_guard: "OK".to_owned(),
                limits: RiskLimitsSummary {
                    daily_halt_pct: self.config.risk.daily_loss_halt_pct,
                    kill_pct: self.config.risk.drawdown_kill_pct,
                },
            },
            positions: Vec::new(),
            orders: Vec::new(),
            signals_shadow: Vec::new(),
            alerts: Vec::new(),
        }
    }
}

#[async_trait]
impl IpcBackend for EngineIpcBackend {
    fn engine_instance_id(&self) -> Uuid {
        self.engine_instance_id
    }

    fn build_mode(&self) -> TradingMode {
        self.config.general.mode
    }

    async fn snapshot(&self) -> SnapshotPayload {
        let now_ms = unix_now_ms();
        match self.handle.full_snapshot(now_ms).await {
            Ok(full) => self.map_full_snapshot(&full, now_ms),
            Err(_) => self.fallback_snapshot(&self.handle.telemetry(), now_ms),
        }
    }

    async fn engine_mode(&self) -> EngineMode {
        self.handle.telemetry().mode
    }

    async fn pause(&self) -> Result<String, IpcError> {
        let current = self.handle.telemetry().mode;
        if current == EngineMode::Killed {
            return Err(IpcError::Forbidden(
                "cannot pause from Killed mode; use reset_kill --confirm first".to_owned(),
            ));
        }
        let next = self
            .handle
            .apply_mode_trigger(
                ModeTrigger::ManualPause {
                    reason: "ipc pause".to_owned(),
                },
                unix_now_ms(),
            )
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?;
        Ok(format!("engine mode -> {next:?}"))
    }

    async fn resume(&self) -> Result<String, IpcError> {
        let current = self.handle.telemetry().mode;
        if current != EngineMode::Paused && current != EngineMode::Ready {
            return Err(IpcError::Forbidden(format!(
                "cannot resume from {current:?} mode"
            )));
        }
        let next = self
            .handle
            .apply_mode_trigger(ModeTrigger::ManualResumeFromPause, unix_now_ms())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?;
        Ok(format!("engine mode -> {next:?}"))
    }

    async fn set_param(&self, key: &str, value: &str) -> Result<String, IpcError> {
        self.handle
            .set_param(key.to_owned(), value.to_owned())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?
            .map_err(|err| {
                if err.to_ascii_lowercase().contains("loosen") {
                    IpcError::LoosenRejected(err)
                } else {
                    IpcError::BadRequest(err)
                }
            })
    }

    async fn disable_strategy(&self, id: &str) -> Result<String, IpcError> {
        self.handle
            .disable_strategy(id.to_owned())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?
            .map_err(IpcError::BadRequest)
    }

    async fn close_position(&self, position_id: Uuid) -> Result<String, IpcError> {
        self.handle
            .close_position(position_id, unix_now_ms())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?
            .map_err(IpcError::NotFound)
    }

    async fn panic_flatten(&self) -> Result<String, IpcError> {
        let (next, flattened) = self
            .handle
            .panic_flatten(unix_now_ms())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?;
        Ok(format!(
            "panic flatten complete ({flattened} positions closed); engine mode -> {next:?}"
        ))
    }

    async fn reset_kill(&self) -> Result<String, IpcError> {
        let next = self
            .handle
            .apply_mode_trigger(
                ModeTrigger::ManualResetKill { confirmed: true },
                unix_now_ms(),
            )
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?;
        Ok(format!("kill reset; engine mode -> {next:?}"))
    }

    async fn ack_alert(&self, id: &str) -> Result<String, IpcError> {
        Ok(format!("alert '{id}' acknowledged"))
    }

    async fn switch_venue(
        &self,
        target: bot_types::Venue,
        policy: bot_ipc::protocol::OnOpenPositionsArg,
        actor: &str,
    ) -> Result<String, IpcError> {
        let mapped = match policy {
            bot_ipc::protocol::OnOpenPositionsArg::Drain => bot_venue::SwitchPolicy::KeepManaging,
            bot_ipc::protocol::OnOpenPositionsArg::Refuse => bot_venue::SwitchPolicy::RequireFlat,
            bot_ipc::protocol::OnOpenPositionsArg::Flatten => bot_venue::SwitchPolicy::Flatten,
        };
        let report = self
            .handle
            .switch_venue(target, mapped, actor.to_owned(), unix_now_ms())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?
            .map_err(IpcError::BadRequest)?;
        Ok(format!(
            "switched {:?} -> {:?} (old venue {:?}, canceled {} entry orders)",
            report.from, report.to, report.old_venue_state, report.canceled_entry_orders
        ))
    }
}

/// Build a non-destructive [`DeltaPayload`] from live [`EngineFullSnapshot`] data so UI
/// clients never receive zeroed `uptime_s`, `ws_latency_ms`, `clock_drift_ms`,
/// `last_switch_ms`, or `switches_today` values that overwrite snapshot state.
#[must_use]
fn build_delta_from_full(
    full: &EngineFullSnapshot,
    build_mode: TradingMode,
    rate_trackers: &VenueRateTrackers,
    now_ms: i64,
) -> DeltaPayload {
    DeltaPayload {
        venues: Some(VenuesSummary {
            active: full.telemetry.venue,
            spot: full.telemetry.spot_state,
            futures: full.telemetry.futures_state,
            last_switch_ms: u64::try_from(full.last_switch_ms).unwrap_or(0),
            switches_today: full.switches_today,
        }),
        engine: Some(EngineSummary {
            mode: full.telemetry.mode,
            build_mode,
            uptime_s: compute_uptime_s(full.started_at_ms, now_ms),
            ws_latency_ms: u64::from(full.health.ws_latency_ms),
            clock_drift_ms: full.health.clock_drift_ms,
            rate: rate_trackers.rate_status_for(full.telemetry.venue),
        }),
        risk: Some(RiskSummary {
            equity: full.telemetry.equity_total_usd,
            day_pnl_pct: full.day_pnl_pct,
            week_pnl_pct: full.week_pnl_pct,
            drawdown_pct: full.drawdown_pct,
            heat_pct: full.heat_pct,
            heat_max_pct: full.config.risk.portfolio_heat_max_pct,
            open_positions: u32::try_from(full.telemetry.open_positions).unwrap_or(u32::MAX),
            max_positions: full.config.risk.max_open_positions,
            btc_guard: full.btc_guard.clone(),
            limits: RiskLimitsSummary {
                daily_halt_pct: full.config.risk.daily_loss_halt_pct,
                kill_pct: full.config.risk.drawdown_kill_pct,
            },
        }),
        ..DeltaPayload::default()
    }
}

/// Spawn a lightweight bridge task that forwards [`EngineActor`] telemetry updates to [`TelemetryHub`].
///
/// Fetches live [`EngineFullSnapshot`] values on each telemetry change so deltas carry real
/// `uptime_s`, `ws_latency_ms`, `clock_drift_ms`, `last_switch_ms`, `switches_today`, and
/// `RateStatus` instead of zeroed placeholders.
#[must_use]
pub fn spawn_telemetry_bridge(
    handle: EngineHandle,
    hub: Arc<TelemetryHub>,
    build_mode: TradingMode,
    started_at_ms: i64,
    rate_trackers: VenueRateTrackers,
) -> JoinHandle<()> {
    let mut rx = handle.telemetry_rx();
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let snap = rx.borrow_and_update().clone();
            let now_ms = unix_now_ms();
            let delta = match handle.full_snapshot(now_ms).await {
                Ok(full) => build_delta_from_full(&full, build_mode, &rate_trackers, now_ms),
                Err(_) => DeltaPayload {
                    // Omit `venues` when full snapshot is unavailable so we never overwrite
                    // `last_switch_ms` / `switches_today` with zeros.
                    venues: None,
                    engine: Some(EngineSummary {
                        mode: snap.mode,
                        build_mode,
                        uptime_s: compute_uptime_s(started_at_ms, now_ms),
                        ws_latency_ms: 0,
                        clock_drift_ms: 0,
                        rate: rate_trackers.rate_status_for(snap.venue),
                    }),
                    ..DeltaPayload::default()
                },
            };
            hub.publish(ServerMessage::Delta(delta), now_ms).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bot_engine::EngineActor;
    use bot_exchange::{RateLimitReading, VenueAdapter};
    use bot_exec::{PaperBroker, PaperBrokerConfig};
    use bot_store::SqliteStore;
    use bot_types::{PositionState, Qty, StrategyId, Symbol};

    fn sample_position(direction: Direction) -> Position {
        Position {
            id: Uuid::now_v7(),
            symbol: Symbol("ETHUSDT".to_owned()),
            venue: Venue::Spot,
            direction,
            strategy: StrategyId::TrendPullback,
            entry_price: Price(Decimal::from(2000_u32)),
            qty_initial: Qty(Decimal::from(2_u32)),
            qty_open: Qty(Decimal::from(2_u32)),
            r_distance: Decimal::from(50_u32),
            leverage: None,
            liq_price: None,
            stop_price: Price(Decimal::from(1950_u32)),
            tp1_price: Price(Decimal::from(2075_u32)),
            tp2_price: Some(Price(Decimal::from(2150_u32))),
            best_price: Price(Decimal::from(2100_u32)),
            tp1_done: false,
            breakeven_done: false,
            fees_paid_quote: Decimal::ZERO,
            realized_pnl_quote: Decimal::ZERO,
            state: PositionState::OpenProtected,
            opened_at: OffsetDateTime::now_utc(),
        }
    }

    #[test]
    fn compute_position_pnl_is_direction_aware_and_exact_decimal() {
        let long_pos = sample_position(Direction::Long);
        // Mark 2100 vs entry 2000: +100 * 2 = +200 quote, +100 / 50 = +2.0 R
        let (r, u) = compute_position_pnl(&long_pos, Price(Decimal::from(2100_u32)));
        assert_eq!(r, Decimal::from(2_u32));
        assert_eq!(u, Decimal::from(200_u32));

        // Mark 1975 vs entry 2000: -25 * 2 = -50 quote, -25 / 50 = -0.5 R
        let (r_loss, u_loss) = compute_position_pnl(&long_pos, Price(Decimal::from(1975_u32)));
        assert_eq!(r_loss, Decimal::new(-5, 1));
        assert_eq!(u_loss, Decimal::from(-50_i32));

        // Short position: mark 1900 vs entry 2000 -> +100 * 2 = +200 quote, +2.0 R
        let short_pos = sample_position(Direction::Short);
        let (sr, su) = compute_position_pnl(&short_pos, Price(Decimal::from(1900_u32)));
        assert_eq!(sr, Decimal::from(2_u32));
        assert_eq!(su, Decimal::from(200_u32));

        // Short adverse move: mark 2050 vs entry 2000 -> -50 * 2 = -100 quote, -1.0 R
        let (short_r_loss, short_quote_loss) =
            compute_position_pnl(&short_pos, Price(Decimal::from(2050_u32)));
        assert_eq!(short_r_loss, Decimal::from(-1_i32));
        assert_eq!(short_quote_loss, Decimal::from(-100_i32));

        // Zero r_distance must never panic or divide by zero.
        let mut zero_r = sample_position(Direction::Long);
        zero_r.r_distance = Decimal::ZERO;
        let (zr, zu) = compute_position_pnl(&zero_r, Price(Decimal::from(2050_u32)));
        assert_eq!(zr, Decimal::ZERO);
        assert_eq!(zu, Decimal::from(100_u32));
    }

    #[test]
    fn venue_rate_trackers_read_live_weight_and_orders_10s() {
        let spot_tracker = Arc::new(RateLimitTracker::new());
        spot_tracker.record_readings(&[
            RateLimitReading {
                rate_limit_type: RateLimitType::RequestWeight,
                interval: RateLimitInterval::Minute,
                interval_num: 1,
                limit: 6000,
                count: 342,
            },
            RateLimitReading {
                rate_limit_type: RateLimitType::Orders,
                interval: RateLimitInterval::Second,
                interval_num: 10,
                limit: 100,
                count: 7,
            },
        ]);

        let trackers = VenueRateTrackers {
            spot: Some(spot_tracker),
            futures: None,
        };
        let spot_status = trackers.rate_status_for(Venue::Spot);
        assert_eq!(spot_status.weight_used, 342);
        assert_eq!(spot_status.weight_limit, 6000);
        assert_eq!(spot_status.orders_10s, 7);

        let fut_status = trackers.rate_status_for(Venue::UsdsFutures);
        assert_eq!(fut_status.weight_used, 0);
        assert_eq!(fut_status.weight_limit, 2400);
        assert_eq!(fut_status.orders_10s, 0);
    }

    #[tokio::test]
    async fn engine_ipc_backend_mutates_real_engine_actor_and_preserves_live_delta_metrics() {
        let cfg = BotConfig::default();
        let store = SqliteStore::open_in_memory().expect("in-memory sqlite");
        let broker = Arc::new(PaperBroker::new(PaperBrokerConfig::default()));
        broker.activate().await.expect("activate paper broker");
        let event_rx = broker.take_event_rx().expect("event rx");

        let started_at_ms = unix_now_ms().saturating_sub(42_000);
        let actor = EngineActor::bootstrap(
            cfg.clone(),
            "b01".to_owned(),
            store,
            broker,
            Decimal::from(10_000_u32),
            started_at_ms,
        )
        .await
        .expect("bootstrap engine");
        let (handle, join) = actor.spawn(event_rx);

        let spot_tracker = Arc::new(RateLimitTracker::new());
        spot_tracker.record_readings(&[
            RateLimitReading {
                rate_limit_type: RateLimitType::RequestWeight,
                interval: RateLimitInterval::Minute,
                interval_num: 1,
                limit: 6000,
                count: 120,
            },
            RateLimitReading {
                rate_limit_type: RateLimitType::Orders,
                interval: RateLimitInterval::Second,
                interval_num: 10,
                limit: 100,
                count: 3,
            },
        ]);
        let trackers = VenueRateTrackers {
            spot: Some(spot_tracker),
            futures: None,
        };

        let backend =
            EngineIpcBackend::new(cfg, handle.clone()).with_rate_trackers(trackers.clone());
        let hub = Arc::new(TelemetryHub::new(32));
        let (_cid, mut sub_rx) = hub.subscribe().await;
        let bridge = spawn_telemetry_bridge(
            handle.clone(),
            Arc::clone(&hub),
            TradingMode::Paper,
            started_at_ms,
            trackers,
        );

        assert_eq!(backend.engine_mode().await, EngineMode::Ready);

        // Tighten-only set_param must succeed for tightening and reject loosening!
        assert!(
            backend
                .set_param("risk.max_open_positions", "2")
                .await
                .is_ok()
        );
        assert!(
            backend
                .set_param("risk.max_open_positions", "9")
                .await
                .is_err()
        );
        let snap = backend.snapshot().await;
        assert_eq!(snap.risk.max_positions, 2);
        assert!(snap.engine.uptime_s >= 40, "snapshot uptime_s must be real");
        assert_eq!(snap.engine.rate.weight_used, 120);
        assert_eq!(snap.engine.rate.orders_10s, 3);

        // disable_strategy wired directly to EngineActor.
        assert!(backend.disable_strategy("breakout").await.is_ok());
        assert!(backend.disable_strategy("unknown").await.is_err());

        backend.pause().await.expect("pause");
        assert_eq!(handle.telemetry().mode, EngineMode::Paused);

        backend.panic_flatten().await.expect("panic");
        assert_eq!(handle.telemetry().mode, EngineMode::Killed);

        let frame = tokio::time::timeout(std::time::Duration::from_secs(2), sub_rx.recv())
            .await
            .expect("telemetry delta timeout")
            .expect("telemetry delta frame");
        match frame.body {
            ServerMessage::Delta(delta) => {
                let eng = delta.engine.expect("delta engine summary");
                assert!(
                    eng.uptime_s >= 40,
                    "delta uptime_s must not be zeroed (got {})",
                    eng.uptime_s
                );
                assert_eq!(eng.rate.weight_used, 120);
                assert_eq!(eng.rate.orders_10s, 3);
                assert!(delta.risk.is_some(), "delta should carry live risk summary");
            }
            other => panic!("expected Delta, got {other:?}"),
        }

        bridge.abort();
        let _ = handle.shutdown(unix_now_ms()).await;
        let _ = join.await;
    }
}
