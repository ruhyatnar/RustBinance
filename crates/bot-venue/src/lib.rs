#![forbid(unsafe_code)]
//! `VenueManager`, `VenueAdapter` lifecycle, and runtime venue switching (§3.5).

pub mod error;
pub mod manager;

pub use error::SwitchError;
pub use manager::{SwitchConfig, SwitchPolicy, SwitchPrecheckContext, VenueManager, VenueSlot};
