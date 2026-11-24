#![forbid(unsafe_code)]
//! Pre-trade Gatekeeper, Spot position sizing, Kelly reducer, combined equity counters,
//! engine mode circuit breakers, and BTC Crash Guard (§7.1–§7.7).

pub mod btc_guard;
pub mod counters;
pub mod cross_venue;
pub mod futures;
pub mod gatekeeper;
pub mod kelly;
pub mod mode;
pub mod sizing;

pub use btc_guard::{
    BTC_GUARD_MIN_ACTIVE_MS, BTC_GUARD_NO_NEW_LOW_RELEASE_MS, BTC_GUARD_RETRIGGER_COOLDOWN_MS,
    BtcCrashGuard, BtcGuardAction, BtcMarketSample, BtcSpikeGuard, BtcSpikeSample,
};
pub use cross_venue::{CrossVenueBudget, CrossVenuePosition, CrossVenueSameSymbolPolicy};
pub use counters::{
    CombinedEquityCounters, EquityCheckOutcome, ModeTriggerKind, PROFIT_LOCK_REDUCE_TRIGGER_PCT,
    PROFIT_LOCK_STOP_TRIGGER_PCT,
};
pub use gatekeeper::{
    DEFAULT_MAX_STOP_PCT, DEFAULT_MIN_STOP_PCT, EntryIntent, GateDecision, Gatekeeper,
    GatekeeperContext, MAX_CLOCK_DRIFT_MS, MAX_MARKET_DATA_AGE_MS, MAX_ORDERS_10S_PCT,
    MAX_REQUEST_WEIGHT_PCT, RejectReason, STALE_DATA_REDUCE_ONLY_MS, TightenOnlyError,
    compute_portfolio_heat_and_exposure,
};
pub use kelly::{KELLY_MIN_TRADES, KellyDecision, KellyInput, evaluate_kelly};
pub use mode::{
    EngineModeMachine, ModeTrigger, PAUSE_STABLE_RELEASE_MS, REDUCE_ONLY_HEALTHY_RELEASE_MS,
};
pub use sizing::{
    DEFAULT_FEE_BUFFER_FRACTION, FREE_QUOTE_USABLE_FRACTION, SizingError, SpotSizingInput,
    SpotSizingOutput, ratchet_stop_monotonic, size_spot_position,
};
