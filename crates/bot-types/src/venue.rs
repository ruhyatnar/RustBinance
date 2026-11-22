//! Venue lifecycle state and capability descriptors (§3.5).

use serde::{Deserialize, Serialize};

/// Per-venue lifecycle state within the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VenueState {
    /// Venue is inactive and disconnected.
    Disabled,
    /// Venue session and reconciliation are starting up.
    Activating,
    /// Venue is live and accepting new entries.
    Active,
    /// No new entries; existing positions and protections remain managed until flat.
    Draining,
}

/// Exchange-side protective exit scheme per venue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProtectionScheme {
    /// Spot dual OCO legs (TP1 + SL and runner).
    DualOco,
    /// Futures conditional reduce-only algo orders.
    AlgoReduceOnly,
}

/// Static capability flags describing what a venue supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VenueCaps {
    /// Whether short positions are supported.
    pub can_short: bool,
    /// Hard exchange leverage ceiling (`1` on Spot).
    pub max_leverage: u32,
    /// Whether periodic funding payments apply.
    pub has_funding: bool,
    /// Exchange-side protection mechanism used by this venue.
    pub protection: ProtectionScheme,
}

impl VenueCaps {
    /// Standard Spot venue capabilities.
    #[must_use]
    pub const fn spot() -> Self {
        Self {
            can_short: false,
            max_leverage: 1,
            has_funding: false,
            protection: ProtectionScheme::DualOco,
        }
    }

    /// Standard USDⓈ-M Futures venue capabilities with a configured leverage ceiling.
    #[must_use]
    pub const fn usds_futures(max_leverage: u32) -> Self {
        Self {
            can_short: true,
            max_leverage,
            has_funding: true,
            protection: ProtectionScheme::AlgoReduceOnly,
        }
    }
}
