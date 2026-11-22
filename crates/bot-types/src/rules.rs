//! Pure exchange instrument filter rules, exact `Decimal` rounding, and order validation.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{InstrumentKey, Price, Qty, Symbol, Venue};

/// Errors produced by [`InstrumentRules`] validation or precision rounding.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RulesError {
    /// Instrument `tick_size` is zero or negative.
    #[error("tick_size must be positive, got {0}")]
    InvalidTickSize(Decimal),
    /// Instrument `step_size` is zero or negative.
    #[error("step_size must be positive, got {0}")]
    InvalidStepSize(Decimal),
    /// Instrument `min_qty` is zero or negative.
    #[error("min_qty must be positive, got {0}")]
    InvalidMinQty(Decimal),
    /// Instrument `max_qty` is smaller than `min_qty`.
    #[error("max_qty ({max_qty}) must be >= min_qty ({min_qty})")]
    InvalidMaxQty {
        /// Minimum allowed quantity.
        min_qty: Decimal,
        /// Maximum allowed quantity.
        max_qty: Decimal,
    },
    /// Instrument `min_notional` is negative.
    #[error("min_notional must be non-negative, got {0}")]
    NegativeMinNotional(Decimal),
    /// Input price or quantity is negative.
    #[error("value must be non-negative, got {0}")]
    NegativeValue(Decimal),
    /// Order price must be strictly positive.
    #[error("order price must be positive, got {0}")]
    NonPositivePrice(Decimal),
    /// Checked decimal arithmetic overflowed.
    #[error("decimal arithmetic overflow during rule calculation")]
    ArithmeticOverflow,
    /// Instrument is not currently in `TRADING` status.
    #[error("instrument {venue}:{symbol} is not in TRADING status")]
    NotTrading {
        /// Venue of the instrument.
        venue: Venue,
        /// Symbol of the instrument.
        symbol: Symbol,
    },
    /// Quantity is below the instrument `min_qty`.
    #[error("qty {qty} is below min_qty {min_qty}")]
    QtyBelowMin {
        /// Provided quantity.
        qty: Decimal,
        /// Required minimum quantity.
        min_qty: Decimal,
    },
    /// Quantity exceeds the instrument `max_qty`.
    #[error("qty {qty} exceeds max_qty {max_qty}")]
    QtyAboveMax {
        /// Provided quantity.
        qty: Decimal,
        /// Allowed maximum quantity.
        max_qty: Decimal,
    },
    /// Quantity is not an exact multiple of `step_size`.
    #[error("qty {qty} is not a multiple of step_size {step_size}")]
    QtyNotStepMultiple {
        /// Provided quantity.
        qty: Decimal,
        /// Instrument step size.
        step_size: Decimal,
    },
    /// Price is not an exact multiple of `tick_size`.
    #[error("price {price} is not a multiple of tick_size {tick_size}")]
    PriceNotTickMultiple {
        /// Provided price.
        price: Decimal,
        /// Instrument tick size.
        tick_size: Decimal,
    },
    /// Order notional (`price * qty`) is below `min_notional`.
    #[error("notional {notional} is below min_notional {min_notional}")]
    NotionalBelowMin {
        /// Computed notional (`price * qty`).
        notional: Decimal,
        /// Required minimum notional.
        min_notional: Decimal,
    },
}

/// Cached exchange symbol filters (`PRICE_FILTER`, `LOT_SIZE`, `NOTIONAL`/`MIN_NOTIONAL`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstrumentRules {
    /// Venue where these rules apply.
    pub venue: Venue,
    /// Exchange symbol string.
    pub symbol: Symbol,
    /// Minimum price increment (`PRICE_FILTER.tickSize`).
    #[serde(with = "rust_decimal::serde::str")]
    pub tick_size: Decimal,
    /// Minimum quantity increment (`LOT_SIZE.stepSize`).
    #[serde(with = "rust_decimal::serde::str")]
    pub step_size: Decimal,
    /// Minimum order quantity (`LOT_SIZE.minQty`).
    #[serde(with = "rust_decimal::serde::str")]
    pub min_qty: Decimal,
    /// Maximum order quantity (`LOT_SIZE.maxQty`).
    #[serde(with = "rust_decimal::serde::str")]
    pub max_qty: Decimal,
    /// Minimum order notional (Spot `NOTIONAL`/`MIN_NOTIONAL.minNotional`; Futures `MIN_NOTIONAL.notional`).
    #[serde(with = "rust_decimal::serde::str")]
    pub min_notional: Decimal,
    /// Whether the symbol currently has exchange status `TRADING`.
    pub status_trading: bool,
    /// Base asset decimal precision.
    pub base_precision: u32,
    /// Quote asset decimal precision.
    pub quote_precision: u32,
}

fn floor_decimal(
    value: Decimal,
    step: Decimal,
    invalid_step: impl FnOnce(Decimal) -> RulesError,
) -> Result<Decimal, RulesError> {
    if step <= Decimal::ZERO {
        return Err(invalid_step(step));
    }
    if value < Decimal::ZERO {
        return Err(RulesError::NegativeValue(value));
    }
    let norm_val = value.normalize();
    let norm_step = step.normalize();
    let rem = norm_val
        .checked_rem(norm_step)
        .ok_or(RulesError::ArithmeticOverflow)?;
    norm_val
        .checked_sub(rem)
        .map(|d| d.normalize())
        .ok_or(RulesError::ArithmeticOverflow)
}

/// Floors `qty` down to the nearest exact multiple of `step_size` (`result <= qty`).
///
/// # Errors
/// Returns [`RulesError::InvalidStepSize`] if `step_size <= 0`, [`RulesError::NegativeValue`] if
/// `qty < 0`, or [`RulesError::ArithmeticOverflow`] if checked decimal arithmetic overflows.
pub fn floor_to_step(qty: Qty, step_size: Decimal) -> Result<Qty, RulesError> {
    floor_decimal(qty.0, step_size, RulesError::InvalidStepSize).map(Qty)
}

/// Floors `price` down to the nearest exact multiple of `tick_size` (`result <= price`).
///
/// # Errors
/// Returns [`RulesError::InvalidTickSize`] if `tick_size <= 0`, [`RulesError::NegativeValue`] if
/// `price < 0`, or [`RulesError::ArithmeticOverflow`] if checked decimal arithmetic overflows.
pub fn floor_to_tick(price: Price, tick_size: Decimal) -> Result<Price, RulesError> {
    floor_decimal(price.0, tick_size, RulesError::InvalidTickSize).map(Price)
}

/// Rounds `price` to the nearest exact multiple of `tick_size` (midpoints round up).
///
/// # Errors
/// Returns [`RulesError::InvalidTickSize`] if `tick_size <= 0`, [`RulesError::NegativeValue`] if
/// `price < 0`, or [`RulesError::ArithmeticOverflow`] if checked decimal arithmetic overflows.
pub fn round_to_tick(price: Price, tick_size: Decimal) -> Result<Price, RulesError> {
    if tick_size <= Decimal::ZERO {
        return Err(RulesError::InvalidTickSize(tick_size));
    }
    if price.0 < Decimal::ZERO {
        return Err(RulesError::NegativeValue(price.0));
    }
    let norm_price = price.0.normalize();
    let norm_tick = tick_size.normalize();
    let rem = norm_price
        .checked_rem(norm_tick)
        .ok_or(RulesError::ArithmeticOverflow)?;
    let floored = norm_price
        .checked_sub(rem)
        .ok_or(RulesError::ArithmeticOverflow)?;
    let to_upper = norm_tick
        .checked_sub(rem)
        .ok_or(RulesError::ArithmeticOverflow)?;
    if rem >= to_upper {
        floored
            .checked_add(norm_tick)
            .map(|d| d.normalize())
            .ok_or(RulesError::ArithmeticOverflow)
            .map(Price)
    } else {
        Ok(Price(floored.normalize()))
    }
}

/// Ceils `price` up to the nearest exact multiple of `tick_size` (`result >= price`).
///
/// # Errors
/// Returns [`RulesError::InvalidTickSize`] if `tick_size <= 0`, [`RulesError::NegativeValue`] if
/// `price < 0`, or [`RulesError::ArithmeticOverflow`] if checked decimal arithmetic overflows.
pub fn ceil_to_tick(price: Price, tick_size: Decimal) -> Result<Price, RulesError> {
    if tick_size <= Decimal::ZERO {
        return Err(RulesError::InvalidTickSize(tick_size));
    }
    if price.0 < Decimal::ZERO {
        return Err(RulesError::NegativeValue(price.0));
    }
    let norm_price = price.0.normalize();
    let norm_tick = tick_size.normalize();
    let rem = norm_price
        .checked_rem(norm_tick)
        .ok_or(RulesError::ArithmeticOverflow)?;
    if rem.is_zero() {
        return Ok(Price(norm_price));
    }
    let floored = norm_price
        .checked_sub(rem)
        .ok_or(RulesError::ArithmeticOverflow)?;
    floored
        .checked_add(norm_tick)
        .map(|d| d.normalize())
        .ok_or(RulesError::ArithmeticOverflow)
        .map(Price)
}

impl InstrumentRules {
    /// Returns the composite [`InstrumentKey`] for this rule set.
    #[must_use]
    pub fn key(&self) -> InstrumentKey {
        InstrumentKey {
            venue: self.venue,
            symbol: self.symbol.clone(),
        }
    }

    /// Validates that the rule parameters themselves are well-formed.
    ///
    /// # Errors
    /// Returns a [`RulesError`] if any increment or bound is invalid.
    pub fn validate_rules(&self) -> Result<(), RulesError> {
        if self.tick_size <= Decimal::ZERO {
            return Err(RulesError::InvalidTickSize(self.tick_size));
        }
        if self.step_size <= Decimal::ZERO {
            return Err(RulesError::InvalidStepSize(self.step_size));
        }
        if self.min_qty <= Decimal::ZERO {
            return Err(RulesError::InvalidMinQty(self.min_qty));
        }
        if self.max_qty < self.min_qty {
            return Err(RulesError::InvalidMaxQty {
                min_qty: self.min_qty,
                max_qty: self.max_qty,
            });
        }
        if self.min_notional < Decimal::ZERO {
            return Err(RulesError::NegativeMinNotional(self.min_notional));
        }
        Ok(())
    }

    /// Floors `qty` down to this instrument's `step_size`.
    ///
    /// # Errors
    /// Propagates [`floor_to_step`] errors.
    pub fn floor_qty(&self, qty: Qty) -> Result<Qty, RulesError> {
        floor_to_step(qty, self.step_size)
    }

    /// Rounds `price` to this instrument's nearest `tick_size`.
    ///
    /// # Errors
    /// Propagates [`round_to_tick`] errors.
    pub fn round_price(&self, price: Price) -> Result<Price, RulesError> {
        round_to_tick(price, self.tick_size)
    }

    /// Floors `price` down to this instrument's `tick_size`.
    ///
    /// # Errors
    /// Propagates [`floor_to_tick`] errors.
    pub fn floor_price(&self, price: Price) -> Result<Price, RulesError> {
        floor_to_tick(price, self.tick_size)
    }

    /// Ceils `price` up to this instrument's `tick_size`.
    ///
    /// # Errors
    /// Propagates [`ceil_to_tick`] errors.
    pub fn ceil_price(&self, price: Price) -> Result<Price, RulesError> {
        ceil_to_tick(price, self.tick_size)
    }

    /// Validates `qty` against `min_qty`, `max_qty`, and `step_size`.
    ///
    /// # Errors
    /// Returns a [`RulesError`] if the rule set is invalid or `qty` violates any quantity filter.
    pub fn validate_qty(&self, qty: Qty) -> Result<(), RulesError> {
        self.validate_rules()?;
        if qty.0 < self.min_qty {
            return Err(RulesError::QtyBelowMin {
                qty: qty.0,
                min_qty: self.min_qty,
            });
        }
        if qty.0 > self.max_qty {
            return Err(RulesError::QtyAboveMax {
                qty: qty.0,
                max_qty: self.max_qty,
            });
        }
        let rem = qty
            .0
            .normalize()
            .checked_rem(self.step_size.normalize())
            .ok_or(RulesError::ArithmeticOverflow)?;
        if !rem.is_zero() {
            return Err(RulesError::QtyNotStepMultiple {
                qty: qty.0,
                step_size: self.step_size,
            });
        }
        Ok(())
    }

    /// Validates `price` is strictly positive and a multiple of `tick_size`.
    ///
    /// # Errors
    /// Returns a [`RulesError`] if the rule set is invalid or `price` violates `PRICE_FILTER`.
    pub fn validate_price(&self, price: Price) -> Result<(), RulesError> {
        self.validate_rules()?;
        if price.0 <= Decimal::ZERO {
            return Err(RulesError::NonPositivePrice(price.0));
        }
        let rem = price
            .0
            .normalize()
            .checked_rem(self.tick_size.normalize())
            .ok_or(RulesError::ArithmeticOverflow)?;
        if !rem.is_zero() {
            return Err(RulesError::PriceNotTickMultiple {
                price: price.0,
                tick_size: self.tick_size,
            });
        }
        Ok(())
    }

    /// Validates that `price * qty >= min_notional` (never upsizes orders).
    ///
    /// # Errors
    /// Returns [`RulesError::NotionalBelowMin`] if below threshold, or
    /// [`RulesError::ArithmeticOverflow`] if `price * qty` overflows.
    pub fn validate_notional(&self, price: Price, qty: Qty) -> Result<(), RulesError> {
        self.validate_rules()?;
        if price.0 <= Decimal::ZERO {
            return Err(RulesError::NonPositivePrice(price.0));
        }
        if qty.0 < Decimal::ZERO {
            return Err(RulesError::NegativeValue(qty.0));
        }
        let notional = price
            .0
            .checked_mul(qty.0)
            .ok_or(RulesError::ArithmeticOverflow)?;
        if notional < self.min_notional {
            return Err(RulesError::NotionalBelowMin {
                notional,
                min_notional: self.min_notional,
            });
        }
        Ok(())
    }

    /// Runs full pre-submission filter validation (`TRADING`, price, qty, notional).
    ///
    /// # Errors
    /// Returns the first [`RulesError`] violated by the order parameters.
    pub fn validate_order(&self, price: Price, qty: Qty) -> Result<(), RulesError> {
        self.validate_rules()?;
        if !self.status_trading {
            return Err(RulesError::NotTrading {
                venue: self.venue,
                symbol: self.symbol.clone(),
            });
        }
        self.validate_price(price)?;
        self.validate_qty(qty)?;
        self.validate_notional(price, qty)?;
        Ok(())
    }
}
