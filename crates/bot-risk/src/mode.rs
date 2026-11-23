//! Engine mode state machine and circuit breaker transitions (§7.4).

use bot_types::EngineMode;
use serde::{Deserialize, Serialize};

/// Minimum continuous stability duration before auto-exiting `Paused` (§7.4: ≥ 3 s).
pub const PAUSE_STABLE_RELEASE_MS: i64 = 3_000;
/// Minimum continuous healthy duration before auto-exiting `ReduceOnly` (§7.4: ≥ 60 s).
pub const REDUCE_ONLY_HEALTHY_RELEASE_MS: i64 = 60_000;

/// Events that can transition the global [`EngineMode`] (§7.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModeTrigger {
    /// Initial sync/reconciliation started (`Starting → Syncing`).
    SyncStarted,
    /// Initial sync/reconciliation completed cleanly (`Syncing → Ready`).
    SyncComplete,
    /// Temporary reconnect or reconciliation needed (`Ready → Paused`).
    ReconnectOrSyncNeeded {
        /// Human-readable reason.
        reason: String,
    },
    /// Operator manual pause (`Ready → Paused`).
    ManualPause {
        /// Reason.
        reason: String,
    },
    /// Stable stream/clock tick; exits `Paused → Ready` after ≥ 3 s continuous stability.
    StableTick {
        /// Current UTC ms.
        now_ms: i64,
    },
    /// Market data stale > 30 s (`Ready → ReduceOnly`).
    DataStaleOver30s,
    /// Reconnect failed > 30 s (`Ready → ReduceOnly`).
    ReconnectFailedOver30s,
    /// Clock drift exceeded safe limit (`Ready → ReduceOnly`).
    ClockDriftExceeded,
    /// Minor reconciliation discrepancy (`Ready → ReduceOnly`).
    MinorReconcileMismatch,
    /// Quote stablecoin depeg > 0.5% (`Ready → ReduceOnly`).
    StablecoinDepeg,
    /// Healthy operational tick; exits `ReduceOnly → Ready` after ≥ 60 s continuous health.
    HealthyTick {
        /// Current UTC ms.
        now_ms: i64,
    },
    /// Daily realized+unrealized loss threshold hit (`Ready/ReduceOnly → Halted`).
    DailyLossHit,
    /// Weekly loss threshold hit (`Ready/ReduceOnly → Halted`).
    WeeklyLossHit,
    /// User-data bounded channel full (`→ Halted`).
    UserDataBufferFull,
    /// Internal state anomaly (`→ Halted`).
    StateAnomaly {
        /// Description.
        reason: String,
    },
    /// 3 consecutive order submission errors (`→ Halted`).
    ConsecutiveOrderErrors {
        /// Consecutive error count.
        count: u32,
    },
    /// Fatal SQLite disk/IO/corruption fault (`→ Halted`).
    StorageFatalError {
        /// Description.
        reason: String,
    },
    /// UTC 00:00 rollover: auto-resets `Halted → Ready` ONLY when halted exclusively by daily loss.
    UtcDayRollover,
    /// Operator manual resume from `Halted` (`Halted → Ready`).
    ManualResumeFromHalt,
    /// Peak-to-trough drawdown kill threshold hit (`Any → Killed`).
    DrawdownKillHit,
    /// Operator hold-to-confirm Panic (`Any → Killed`).
    ManualPanic,
    /// Unrecoverable loss of exchange-side protection (`Any → Killed`).
    UnrecoverableProtectionLoss,
    /// Manual kill reset (`botctl reset-kill --confirm`): ONLY way to exit `Killed`.
    ManualResetKill {
        /// Must be `true` to confirm reset.
        confirmed: bool,
    },
}

/// Global [`EngineMode`] state machine enforcing §7.4 transition invariants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineModeMachine {
    mode: EngineMode,
    reason: String,
    since_ms: i64,
    stable_since_ms: Option<i64>,
    healthy_since_ms: Option<i64>,
    halted_by_daily_loss_only: bool,
    manual_pause: bool,
}

impl EngineModeMachine {
    /// Start a new machine in [`EngineMode::Starting`].
    #[must_use]
    pub fn new(now_ms: i64) -> Self {
        Self {
            mode: EngineMode::Starting,
            reason: "boot".into(),
            since_ms: now_ms,
            stable_since_ms: None,
            healthy_since_ms: None,
            halted_by_daily_loss_only: false,
            manual_pause: false,
        }
    }

    /// Current global mode.
    #[must_use]
    pub const fn mode(&self) -> EngineMode {
        self.mode
    }

    /// Human-readable reason for the current mode.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Whether new entries are permitted (`EngineMode::Ready` only).
    #[must_use]
    pub const fn allows_new_entries(&self) -> bool {
        matches!(self.mode, EngineMode::Ready)
    }

    /// Whether existing open positions & protections continue to be managed.
    #[must_use]
    pub const fn allows_position_management(&self) -> bool {
        !matches!(self.mode, EngineMode::Starting | EngineMode::Killed)
    }

    /// Apply a [`ModeTrigger`] and return the resulting [`EngineMode`].
    ///
    /// Hard safety rule: once in [`EngineMode::Killed`], **only**
    /// `ModeTrigger::ManualResetKill { confirmed: true }` can transition out (to `Paused`).
    #[allow(clippy::too_many_lines)]
    pub fn apply(&mut self, trigger: ModeTrigger, now_ms: i64) -> EngineMode {
        // Killed is a hard latching state; only explicit confirmed manual reset exits it.
        if self.mode == EngineMode::Killed {
            if let ModeTrigger::ManualResetKill { confirmed: true } = trigger {
                self.transition(
                    EngineMode::Paused,
                    "manual kill reset confirmed".into(),
                    now_ms,
                );
                self.manual_pause = true;
            }
            return self.mode;
        }

        match trigger {
            // Any -> Killed
            ModeTrigger::DrawdownKillHit => {
                self.transition(
                    EngineMode::Killed,
                    "drawdown_kill_pct breached".into(),
                    now_ms,
                );
            }
            ModeTrigger::ManualPanic => {
                self.transition(EngineMode::Killed, "manual panic command".into(), now_ms);
            }
            ModeTrigger::UnrecoverableProtectionLoss => {
                self.transition(
                    EngineMode::Killed,
                    "unrecoverable loss of exchange protection".into(),
                    now_ms,
                );
            }
            // Any non-Killed -> Halted
            ModeTrigger::DailyLossHit => {
                let was_other_halt =
                    self.mode == EngineMode::Halted && !self.halted_by_daily_loss_only;
                self.transition(
                    EngineMode::Halted,
                    "daily_loss_halt_pct breached".into(),
                    now_ms,
                );
                self.halted_by_daily_loss_only = !was_other_halt;
            }
            ModeTrigger::WeeklyLossHit => {
                self.transition(
                    EngineMode::Halted,
                    "weekly_loss_halt_pct breached".into(),
                    now_ms,
                );
                self.halted_by_daily_loss_only = false;
            }
            ModeTrigger::UserDataBufferFull => {
                self.transition(
                    EngineMode::Halted,
                    "user-data event buffer full".into(),
                    now_ms,
                );
                self.halted_by_daily_loss_only = false;
            }
            ModeTrigger::StateAnomaly { reason } => {
                self.transition(
                    EngineMode::Halted,
                    format!("state anomaly: {reason}"),
                    now_ms,
                );
                self.halted_by_daily_loss_only = false;
            }
            ModeTrigger::ConsecutiveOrderErrors { count } if count >= 3 => {
                self.transition(
                    EngineMode::Halted,
                    format!("{count} consecutive order errors"),
                    now_ms,
                );
                self.halted_by_daily_loss_only = false;
            }
            ModeTrigger::ManualResetKill { .. } | ModeTrigger::ConsecutiveOrderErrors { .. } => {}
            ModeTrigger::StorageFatalError { reason } => {
                self.transition(
                    EngineMode::Halted,
                    format!("fatal storage error: {reason}"),
                    now_ms,
                );
                self.halted_by_daily_loss_only = false;
            }

            // Halt exits
            ModeTrigger::UtcDayRollover => {
                if self.mode == EngineMode::Halted && self.halted_by_daily_loss_only {
                    self.halted_by_daily_loss_only = false;
                    self.transition(
                        EngineMode::Ready,
                        "UTC 00:00 rollover cleared daily loss halt".into(),
                        now_ms,
                    );
                }
            }
            ModeTrigger::ManualResumeFromHalt => {
                if self.mode == EngineMode::Halted {
                    self.halted_by_daily_loss_only = false;
                    self.transition(EngineMode::Ready, "manual resume from halt".into(), now_ms);
                }
            }

            // ReduceOnly triggers (cannot downgrade from Halted)
            ModeTrigger::DataStaleOver30s => {
                self.enter_reduce_only("market data stale > 30s", now_ms);
            }
            ModeTrigger::ReconnectFailedOver30s => {
                self.enter_reduce_only("reconnect failed > 30s", now_ms);
            }
            ModeTrigger::ClockDriftExceeded => {
                self.enter_reduce_only("clock drift exceeded limit", now_ms);
            }
            ModeTrigger::MinorReconcileMismatch => {
                self.enter_reduce_only("minor reconciliation mismatch", now_ms);
            }
            ModeTrigger::StablecoinDepeg => {
                self.enter_reduce_only("quote stablecoin depeg > 0.5%", now_ms);
            }
            ModeTrigger::HealthyTick { now_ms: tick_ms } => {
                if self.mode == EngineMode::ReduceOnly {
                    let start = *self.healthy_since_ms.get_or_insert(tick_ms);
                    if tick_ms.saturating_sub(start) >= REDUCE_ONLY_HEALTHY_RELEASE_MS {
                        self.transition(
                            EngineMode::Ready,
                            "healthy >= 60s; exited ReduceOnly".into(),
                            tick_ms,
                        );
                    }
                }
            }

            // Startup & Pause
            ModeTrigger::SyncStarted => {
                if self.mode == EngineMode::Starting {
                    self.transition(EngineMode::Syncing, "initial sync started".into(), now_ms);
                }
            }
            ModeTrigger::SyncComplete => {
                if self.mode == EngineMode::Syncing {
                    self.transition(EngineMode::Ready, "initial sync complete".into(), now_ms);
                }
            }
            ModeTrigger::ReconnectOrSyncNeeded { reason } => {
                if self.mode == EngineMode::Ready {
                    self.transition(EngineMode::Paused, reason, now_ms);
                    self.manual_pause = false;
                }
            }
            ModeTrigger::ManualPause { reason } => {
                if matches!(self.mode, EngineMode::Ready | EngineMode::Paused) {
                    self.transition(EngineMode::Paused, reason, now_ms);
                    self.manual_pause = true;
                }
            }
            ModeTrigger::StableTick { now_ms: tick_ms } => {
                if self.mode == EngineMode::Paused && !self.manual_pause {
                    let start = *self.stable_since_ms.get_or_insert(tick_ms);
                    if tick_ms.saturating_sub(start) >= PAUSE_STABLE_RELEASE_MS {
                        self.transition(
                            EngineMode::Ready,
                            "stable >= 3s; exited Paused".into(),
                            tick_ms,
                        );
                    }
                }
            }
        }

        self.mode
    }

    fn enter_reduce_only(&mut self, reason: &str, now_ms: i64) {
        if matches!(
            self.mode,
            EngineMode::Ready | EngineMode::Paused | EngineMode::ReduceOnly
        ) {
            self.transition(EngineMode::ReduceOnly, reason.to_owned(), now_ms);
        }
    }

    fn transition(&mut self, next: EngineMode, reason: String, now_ms: i64) {
        self.mode = next;
        self.reason = reason;
        self.since_ms = now_ms;
        self.stable_since_ms = None;
        self.healthy_since_ms = None;
        if next != EngineMode::Paused {
            self.manual_pause = false;
        }
    }
}
