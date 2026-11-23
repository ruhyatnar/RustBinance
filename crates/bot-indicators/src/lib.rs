#![forbid(unsafe_code)]
//! Incremental O(1), allocation-free, I/O-free technical indicators (§6.3).
//!
//! `f64` is used strictly within this indicator crate per project rules; monetary values
//! elsewhere remain `rust_decimal::Decimal`.

pub mod adx;
pub mod atr;
pub mod donchian;
pub mod ema;
pub mod rsi;
pub mod sma;
pub mod vwap;

pub use adx::{Adx, AdxValue};
pub use atr::{Atr, true_range};
pub use bot_types::Kline as Bar;
pub use donchian::{Donchian, DonchianValue};
pub use ema::Ema;
pub use rsi::Rsi;
pub use sma::VolumeSma;
pub use vwap::SessionVwap;

/// Unified incremental indicator interface (§6.3).
pub trait Indicator {
    /// Indicator output type.
    type Output;

    /// Feed one closed bar and return the updated value once warm-up is satisfied.
    fn update(&mut self, bar: &Bar) -> Option<Self::Output>;

    /// Minimum number of closed bars required before returning `Some`.
    fn warmup_bars(&self) -> usize;

    /// Returns `true` when warm-up has been satisfied.
    fn is_ready(&self) -> bool;
}

impl Indicator for Ema {
    type Output = f64;
    fn update(&mut self, bar: &Bar) -> Option<Self::Output> {
        Self::update(self, bar)
    }
    fn warmup_bars(&self) -> usize {
        Self::warmup_bars(self)
    }
    fn is_ready(&self) -> bool {
        Self::is_ready(self)
    }
}

impl Indicator for Rsi {
    type Output = f64;
    fn update(&mut self, bar: &Bar) -> Option<Self::Output> {
        Self::update(self, bar)
    }
    fn warmup_bars(&self) -> usize {
        Self::warmup_bars(self)
    }
    fn is_ready(&self) -> bool {
        Self::is_ready(self)
    }
}

impl Indicator for Atr {
    type Output = f64;
    fn update(&mut self, bar: &Bar) -> Option<Self::Output> {
        Self::update(self, bar)
    }
    fn warmup_bars(&self) -> usize {
        Self::warmup_bars(self)
    }
    fn is_ready(&self) -> bool {
        Self::is_ready(self)
    }
}

impl Indicator for Adx {
    type Output = AdxValue;
    fn update(&mut self, bar: &Bar) -> Option<Self::Output> {
        Self::update(self, bar)
    }
    fn warmup_bars(&self) -> usize {
        Self::warmup_bars(self)
    }
    fn is_ready(&self) -> bool {
        Self::is_ready(self)
    }
}

impl Indicator for Donchian {
    type Output = DonchianValue;
    fn update(&mut self, bar: &Bar) -> Option<Self::Output> {
        Self::update(self, bar)
    }
    fn warmup_bars(&self) -> usize {
        Self::warmup_bars(self)
    }
    fn is_ready(&self) -> bool {
        Self::is_ready(self)
    }
}

impl Indicator for SessionVwap {
    type Output = f64;
    fn update(&mut self, bar: &Bar) -> Option<Self::Output> {
        Self::update(self, bar)
    }
    fn warmup_bars(&self) -> usize {
        Self::warmup_bars(self)
    }
    fn is_ready(&self) -> bool {
        Self::is_ready(self)
    }
}

impl Indicator for VolumeSma {
    type Output = f64;
    fn update(&mut self, bar: &Bar) -> Option<Self::Output> {
        Self::update(self, bar)
    }
    fn warmup_bars(&self) -> usize {
        Self::warmup_bars(self)
    }
    fn is_ready(&self) -> bool {
        Self::is_ready(self)
    }
}
