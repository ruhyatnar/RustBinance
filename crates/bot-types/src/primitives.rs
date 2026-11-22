//! Core venue-agnostic primitives and identifiers.

use std::fmt;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Exchange trading symbol (e.g. `"BTCUSDT"`).
///
/// Never use `Symbol` alone as a state map key; always pair it with [`Venue`] via [`InstrumentKey`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Symbol(pub String);

impl Symbol {
    /// Returns the symbol string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Exact decimal price newtype; serialized as a string to preserve precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Price(#[serde(with = "rust_decimal::serde::str")] pub Decimal);

impl Price {
    /// Returns the inner `Decimal` value.
    #[must_use]
    pub const fn as_decimal(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Exact decimal quantity newtype; serialized as a string to preserve precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Qty(#[serde(with = "rust_decimal::serde::str")] pub Decimal);

impl Qty {
    /// Returns the inner `Decimal` value.
    #[must_use]
    pub const fn as_decimal(self) -> Decimal {
        self.0
    }
}

impl fmt::Display for Qty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Supported trading venues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Venue {
    /// Binance Spot venue.
    #[serde(rename = "spot")]
    Spot,
    /// Binance USDⓈ-M Futures venue.
    #[serde(rename = "futures", alias = "usds_futures")]
    UsdsFutures,
}

impl fmt::Display for Venue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spot => f.write_str("spot"),
            Self::UsdsFutures => f.write_str("futures"),
        }
    }
}

/// Composite key uniquely identifying an instrument across venues.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InstrumentKey {
    /// Venue where this instrument trades.
    pub venue: Venue,
    /// Exchange symbol string.
    pub symbol: Symbol,
}

/// Order side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    /// Buy order.
    Buy,
    /// Sell order.
    Sell,
}

/// Position direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    /// Long exposure (supported on Spot and Futures).
    Long,
    /// Short exposure (supported on Futures only).
    Short,
}

/// Execution mode orthogonal to [`Venue`]. Always defaults to [`TradingMode::Paper`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingMode {
    /// Simulated execution with no real orders sent.
    #[default]
    Paper,
    /// Binance testnet execution.
    Testnet,
    /// Production execution (gated by per-venue `live_confirmation_file`).
    Live,
}

/// Strategy identifiers supported by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StrategyId {
    /// Trend pullback strategy (S1).
    TrendPullback,
    /// Donchian breakout strategy (S2).
    Breakout,
    /// Gainers momentum strategy (S3).
    GainersMomentum,
    /// VWAP mean-reversion strategy (S4).
    VwapReversion,
}

/// Global engine lifecycle and circuit-breaker mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EngineMode {
    /// Bootstrapping components.
    Starting,
    /// Reconciling exchange state before allowing entries.
    Syncing,
    /// Healthy; entries and position management enabled.
    Ready,
    /// Entries paused; existing positions and protections continue to be managed.
    Paused,
    /// Only position-reducing actions are permitted.
    ReduceOnly,
    /// Circuit breaker tripped; entries blocked while protections remain active.
    Halted,
    /// Emergency stop; requires manual reset.
    Killed,
}
