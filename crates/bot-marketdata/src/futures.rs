//! USDⓈ-M Futures market data: `@markPrice@1s` parser and `pu`-chained order book sync (§4.3).

use std::collections::BTreeMap;
use std::str::FromStr;

use bot_types::{Price, Qty, Symbol, Venue};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::MarketDataError;
use crate::types::BookStatus;

/// Parsed `@markPrice@1s` tick for USDⓈ-M Futures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkPriceTick {
    /// Venue (`Venue::UsdsFutures`).
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Mark price.
    pub mark_price: Price,
    /// Index price.
    pub index_price: Price,
    /// Current funding rate (e.g. `0.00010000` = 0.01%).
    pub funding_rate: Decimal,
    /// Next funding settlement timestamp (UTC ms).
    pub next_funding_time_ms: i64,
    /// Event timestamp (UTC ms).
    pub event_time_ms: i64,
}

fn parse_dec_field(v: &Value, key: &str) -> Result<Decimal, MarketDataError> {
    let s = v
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| MarketDataError::WireFormat(format!("missing decimal string '{key}'")))?;
    Decimal::from_str(s)
        .map_err(|e| MarketDataError::WireFormat(format!("invalid decimal '{key}': {e}")))
}

/// Parse a Binance USDⓈ-M Futures `@markPrice@1s` JSON payload.
///
/// # Errors
/// Returns [`MarketDataError::WireFormat`] if the JSON is malformed or missing required fields.
pub fn parse_futures_mark_price(raw: &str) -> Result<MarkPriceTick, MarketDataError> {
    let root: Value = serde_json::from_str(raw)
        .map_err(|e| MarketDataError::WireFormat(format!("invalid markPrice JSON: {e}")))?;
    let event = root
        .get("e")
        .and_then(Value::as_str)
        .ok_or_else(|| MarketDataError::WireFormat("missing 'e'".into()))?;
    if event != "markPriceUpdate" {
        return Err(MarketDataError::WireFormat(format!(
            "expected markPriceUpdate, got '{event}'"
        )));
    }
    let sym = root
        .get("s")
        .and_then(Value::as_str)
        .ok_or_else(|| MarketDataError::WireFormat("missing 's'".into()))?;
    Ok(MarkPriceTick {
        venue: Venue::UsdsFutures,
        symbol: Symbol(sym.to_owned()),
        mark_price: Price(parse_dec_field(&root, "p")?),
        index_price: Price(parse_dec_field(&root, "i")?),
        funding_rate: parse_dec_field(&root, "r")?,
        next_funding_time_ms: root.get("T").and_then(Value::as_i64).unwrap_or(0),
        event_time_ms: root.get("E").and_then(Value::as_i64).unwrap_or(0),
    })
}

/// USDⓈ-M Futures incremental depth update carrying `pu` (`prev_final_update_id`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuturesDepthDelta {
    /// Symbol.
    pub symbol: Symbol,
    /// First update ID (`U`).
    pub first_update_id: u64,
    /// Final update ID (`u`).
    pub final_update_id: u64,
    /// Previous final update ID (`pu`) — must chain strictly to prior `u`.
    pub prev_final_update_id: u64,
    /// Bid updates `(price, qty)` where `qty == 0` removes the level.
    pub bids: Vec<(Price, Qty)>,
    /// Ask updates `(price, qty)` where `qty == 0` removes the level.
    pub asks: Vec<(Price, Qty)>,
}

/// Result of applying a Futures depth delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuturesDepthApplyResult {
    /// Stale event (`u < snapshot_last_update_id`) dropped safely.
    DroppedStale,
    /// Delta applied cleanly; book remains synchronized.
    Applied,
    /// Sequence gap detected (`pu != last_u`); book marked `Invalid` and caller must resync.
    GapDetectedNeedsResync,
}

/// Synchronized local order book for USDⓈ-M Futures using the official `pu` chain (§4.3).
#[derive(Debug, Clone)]
pub struct FuturesOrderBookSync {
    symbol: Symbol,
    status: BookStatus,
    last_update_id: Option<u64>,
    awaiting_first_post_snapshot: bool,
    bids: BTreeMap<Decimal, Decimal>,
    asks: BTreeMap<Decimal, Decimal>,
}

impl FuturesOrderBookSync {
    /// Create a new uninitialized Futures order book for `symbol`.
    #[must_use]
    pub fn new(symbol: Symbol) -> Self {
        Self {
            symbol,
            status: BookStatus::Invalid,
            last_update_id: None,
            awaiting_first_post_snapshot: false,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
        }
    }

    /// Current book synchronization status.
    #[must_use]
    pub const fn status(&self) -> BookStatus {
        self.status
    }

    /// Symbol tracked by this book.
    #[must_use]
    pub fn symbol(&self) -> &Symbol {
        &self.symbol
    }

    /// Best bid price, if any.
    #[must_use]
    pub fn best_bid(&self) -> Option<Price> {
        self.bids.keys().next_back().copied().map(Price)
    }

    /// Best ask price, if any.
    #[must_use]
    pub fn best_ask(&self) -> Option<Price> {
        self.asks.keys().next().copied().map(Price)
    }

    /// Load a REST depth snapshot (`GET /fapi/v1/depth`) and arm `pu` chain validation.
    pub fn apply_snapshot(
        &mut self,
        last_update_id: u64,
        bids: &[(Price, Qty)],
        asks: &[(Price, Qty)],
    ) {
        self.bids.clear();
        self.asks.clear();
        for &(p, q) in bids {
            if q.0 > Decimal::ZERO {
                self.bids.insert(p.0, q.0);
            }
        }
        for &(p, q) in asks {
            if q.0 > Decimal::ZERO {
                self.asks.insert(p.0, q.0);
            }
        }
        self.last_update_id = Some(last_update_id);
        self.awaiting_first_post_snapshot = true;
        self.status = BookStatus::Synced;
    }

    /// Apply a Futures `@depth` delta enforcing the official `pu` chain rule.
    pub fn apply_delta(&mut self, delta: &FuturesDepthDelta) -> FuturesDepthApplyResult {
        let Some(last_u) = self.last_update_id else {
            self.status = BookStatus::Invalid;
            return FuturesDepthApplyResult::GapDetectedNeedsResync;
        };

        if self.awaiting_first_post_snapshot {
            if delta.final_update_id < last_u {
                return FuturesDepthApplyResult::DroppedStale;
            }
            if delta.first_update_id <= last_u && delta.final_update_id >= last_u {
                self.awaiting_first_post_snapshot = false;
            } else {
                self.status = BookStatus::Invalid;
                return FuturesDepthApplyResult::GapDetectedNeedsResync;
            }
        } else if delta.prev_final_update_id != last_u {
            self.status = BookStatus::Invalid;
            return FuturesDepthApplyResult::GapDetectedNeedsResync;
        }

        for &(p, q) in &delta.bids {
            if q.0 <= Decimal::ZERO {
                self.bids.remove(&p.0);
            } else {
                self.bids.insert(p.0, q.0);
            }
        }
        for &(p, q) in &delta.asks {
            if q.0 <= Decimal::ZERO {
                self.asks.remove(&p.0);
            } else {
                self.asks.insert(p.0, q.0);
            }
        }
        self.last_update_id = Some(delta.final_update_id);
        self.status = BookStatus::Synced;
        FuturesDepthApplyResult::Applied
    }
}
