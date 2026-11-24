//! `VenueManager` state machine (`Disabled -> Activating -> Active -> Draining`).

use bot_types::{EngineMode, TradingMode, Venue, VenueState};
use serde::{Deserialize, Serialize};

use crate::error::SwitchError;

/// Policy for handling open positions on the old venue during `switch_venue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchPolicy {
    /// Transition old venue to `Draining` and keep managing open positions until flat.
    #[default]
    KeepManaging,
    /// Reject the switch if any position is still open on the old venue.
    RequireFlat,
    /// Close all open positions on the old venue (reduce-only) and disable it.
    Flatten,
}

/// Configuration for runtime venue switching guardrails (§3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchConfig {
    /// Minimum seconds between completed switches (default: 60).
    pub switch_cooldown_s: u64,
    /// Maximum completed switches per UTC day (default: 3).
    pub max_switches_per_day: u32,
    /// Default open-position policy (default: `KeepManaging`).
    pub default_policy: SwitchPolicy,
}

impl Default for SwitchConfig {
    fn default() -> Self {
        Self {
            switch_cooldown_s: 60,
            max_switches_per_day: 3,
            default_policy: SwitchPolicy::KeepManaging,
        }
    }
}

/// Per-venue lifecycle and readiness metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VenueSlot {
    /// Venue identifier.
    pub venue: Venue,
    /// Current lifecycle state.
    pub state: VenueState,
    /// Whether this venue is enabled in config.
    pub enabled: bool,
    /// Whether this venue's credentials are valid.
    pub credentials_valid: bool,
    /// Whether this venue's `live_confirmation_file` exists.
    pub live_confirmation_present: bool,
}

impl VenueSlot {
    const fn new(venue: Venue, active: bool) -> Self {
        Self {
            venue,
            state: if active {
                VenueState::Active
            } else {
                VenueState::Disabled
            },
            enabled: active,
            credentials_valid: active,
            live_confirmation_present: false,
        }
    }
}

/// Snapshot of engine state required to validate `switch_venue` preconditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SwitchPrecheckContext {
    /// Current global engine mode.
    pub engine_mode: EngineMode,
    /// Current trading mode (`Paper`, `Testnet`, `Live`).
    pub trading_mode: TradingMode,
    /// Whether the engine is currently syncing/reconciling.
    pub syncing: bool,
    /// Number of open positions on the currently active venue.
    pub old_open_positions: usize,
    /// Current wall-clock timestamp in milliseconds since Unix epoch.
    pub now_ms: i64,
    /// Current UTC day number (e.g. `epoch_secs / 86_400`).
    pub utc_day: u32,
}

/// Manages Spot and USDⓈ-M Futures lifecycle states and atomic switching (§3.5).
#[derive(Debug, Clone)]
pub struct VenueManager {
    active_venue: Venue,
    spot: VenueSlot,
    futures: VenueSlot,
    config: SwitchConfig,
    last_switch_ms: Option<i64>,
    switches_today_count: u32,
    switches_utc_day: u32,
}

impl VenueManager {
    /// Create a new `VenueManager` with `active_venue` in `Active` and the other in `Disabled`.
    #[must_use]
    pub const fn new(active_venue: Venue, config: SwitchConfig) -> Self {
        Self {
            active_venue,
            spot: VenueSlot::new(Venue::Spot, matches!(active_venue, Venue::Spot)),
            futures: VenueSlot::new(
                Venue::UsdsFutures,
                matches!(active_venue, Venue::UsdsFutures),
            ),
            config,
            last_switch_ms: None,
            switches_today_count: 0,
            switches_utc_day: 0,
        }
    }

    /// Return the currently active venue.
    #[must_use]
    pub const fn active_venue(&self) -> Venue {
        self.active_venue
    }

    /// Return the current lifecycle state of `venue`.
    #[must_use]
    pub const fn state(&self, venue: Venue) -> VenueState {
        self.slot(venue).state
    }

    /// Return the slot metadata for `venue`.
    #[must_use]
    pub const fn slot(&self, venue: Venue) -> &VenueSlot {
        match venue {
            Venue::Spot => &self.spot,
            Venue::UsdsFutures => &self.futures,
        }
    }

    /// Return the active switch configuration.
    #[must_use]
    pub const fn config(&self) -> &SwitchConfig {
        &self.config
    }

    /// Enable or disable a venue in config readiness.
    pub const fn set_venue_enabled(&mut self, venue: Venue, enabled: bool) {
        self.slot_mut(venue).enabled = enabled;
    }

    /// Mark whether a venue's credentials are valid.
    pub const fn set_credentials_valid(&mut self, venue: Venue, valid: bool) {
        self.slot_mut(venue).credentials_valid = valid;
    }

    /// Mark whether a venue's `live_confirmation_file` is present.
    pub const fn set_live_confirmation_present(&mut self, venue: Venue, present: bool) {
        self.slot_mut(venue).live_confirmation_present = present;
    }

    /// Number of completed switches on `utc_day`.
    #[must_use]
    pub const fn switches_today(&self, utc_day: u32) -> u32 {
        if self.switches_utc_day == utc_day {
            self.switches_today_count
        } else {
            0
        }
    }

    /// Validate all §3.5 preconditions without mutating state.
    ///
    /// # Errors
    /// Returns [`SwitchError`] if any safety precondition fails.
    pub fn precheck_switch(
        &self,
        target: Venue,
        policy: SwitchPolicy,
        ctx: &SwitchPrecheckContext,
    ) -> Result<Venue, SwitchError> {
        if !matches!(ctx.engine_mode, EngineMode::Ready | EngineMode::Paused) {
            return Err(SwitchError::InvalidEngineMode(ctx.engine_mode));
        }
        if ctx.syncing {
            return Err(SwitchError::SyncingInProgress);
        }
        if target == self.active_venue {
            return Err(SwitchError::AlreadyActive(target));
        }

        let target_slot = self.slot(target);
        if !target_slot.enabled {
            return Err(SwitchError::TargetDisabled(target));
        }
        if !target_slot.credentials_valid {
            return Err(SwitchError::InvalidCredentials(target));
        }
        if ctx.trading_mode == TradingMode::Live && !target_slot.live_confirmation_present {
            return Err(SwitchError::MissingLiveConfirmation(target));
        }

        if let Some(last_ms) = self.last_switch_ms {
            let elapsed_ms = ctx.now_ms.saturating_sub(last_ms);
            let cooldown_ms = i64::try_from(self.config.switch_cooldown_s)
                .unwrap_or(i64::MAX)
                .saturating_mul(1_000);
            if elapsed_ms < cooldown_ms {
                let wait_ms = cooldown_ms.saturating_sub(elapsed_ms);
                let remaining_s = u64::try_from((wait_ms + 999) / 1_000).unwrap_or(1);
                return Err(SwitchError::CooldownActive { remaining_s });
            }
        }

        if self.switches_today(ctx.utc_day) >= self.config.max_switches_per_day {
            return Err(SwitchError::DailyLimitReached {
                limit: self.config.max_switches_per_day,
            });
        }

        if policy == SwitchPolicy::RequireFlat && ctx.old_open_positions > 0 {
            return Err(SwitchError::OpenPositionsRequireFlat {
                count: ctx.old_open_positions,
            });
        }

        Ok(self.active_venue)
    }

    /// Begin a switch after validating preconditions.
    ///
    /// Transitions target to `Activating` and old venue to `Draining` (if `KeepManaging`
    /// with open positions) or `Disabled` (if flat or `Flatten`).
    ///
    /// # Errors
    /// Returns [`SwitchError`] if preconditions fail.
    pub fn begin_switch(
        &mut self,
        target: Venue,
        policy: SwitchPolicy,
        ctx: &SwitchPrecheckContext,
    ) -> Result<Venue, SwitchError> {
        let old = self.precheck_switch(target, policy, ctx)?;

        let old_next_state = match policy {
            SwitchPolicy::KeepManaging if ctx.old_open_positions > 0 => VenueState::Draining,
            SwitchPolicy::KeepManaging | SwitchPolicy::RequireFlat | SwitchPolicy::Flatten => {
                VenueState::Disabled
            }
        };
        self.slot_mut(old).state = old_next_state;
        self.slot_mut(target).state = VenueState::Activating;
        Ok(old)
    }

    /// Finalize a switch after target activation and per-venue reconciliation succeed.
    pub fn complete_switch(
        &mut self,
        target: Venue,
        _policy: SwitchPolicy,
        now_ms: i64,
        utc_day: u32,
    ) {
        self.slot_mut(target).state = VenueState::Active;
        self.active_venue = target;
        self.last_switch_ms = Some(now_ms);
        if self.switches_utc_day == utc_day {
            self.switches_today_count = self.switches_today_count.saturating_add(1);
        } else {
            self.switches_utc_day = utc_day;
            self.switches_today_count = 1;
        }
    }

    /// Roll back a failed target activation: target returns to `Disabled` and `old_venue`
    /// returns to `Active` (no half-switched state).
    pub fn rollback_switch(&mut self, old_venue: Venue, target: Venue) {
        self.slot_mut(target).state = VenueState::Disabled;
        self.slot_mut(old_venue).state = VenueState::Active;
        self.active_venue = old_venue;
    }

    /// Transition a `Draining` venue to `Disabled` once its open positions reach zero.
    /// Returns `true` if a transition occurred.
    pub fn mark_draining_flat(&mut self, venue: Venue) -> bool {
        let slot = self.slot_mut(venue);
        if slot.state == VenueState::Draining {
            slot.state = VenueState::Disabled;
            true
        } else {
            false
        }
    }

    const fn slot_mut(&mut self, venue: Venue) -> &mut VenueSlot {
        match venue {
            Venue::Spot => &mut self.spot,
            Venue::UsdsFutures => &mut self.futures,
        }
    }
}
