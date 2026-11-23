//! Local Spot order book with official Binance synchronization, gap invalidation,
//! top-N OBI EMA smoothing, and exact `Decimal` spread bps (§4.3).

use std::collections::{BTreeMap, VecDeque};

use async_trait::async_trait;
use bot_types::{Price, Qty, Symbol, Venue};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::error::MarketDataError;
use crate::types::{BookStatus, BookTelemetry, DepthDiffEvent, DepthLevel, DepthSnapshot};

/// Abstraction for fetching REST depth snapshots (`GET /api/v3/depth`).
#[async_trait]
pub trait DepthSnapshotClient: Send + Sync {
    /// Fetch a depth snapshot for `(venue, symbol)` with `limit` levels per side (e.g. 1000).
    async fn fetch_depth_snapshot(
        &self,
        venue: Venue,
        symbol: &Symbol,
        limit: u16,
    ) -> Result<DepthSnapshot, MarketDataError>;
}

/// Outcome of applying a REST [`DepthSnapshot`] to [`LocalOrderBook`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotSyncResult {
    /// Snapshot `lastUpdateId` is older than the first buffered diff's `U`; caller must re-fetch.
    SnapshotTooOld,
    /// Snapshot applied and buffered diffs replayed cleanly; book is now `Synced`.
    Synced,
    /// Gap encountered while replaying buffered diffs; book marked `Invalid`, caller must resync.
    GapDetected,
}

/// Outcome of applying a streaming [`DepthDiffEvent`] to [`LocalOrderBook`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffApplyResult {
    /// Book is not yet synced; diff was buffered and caller should fetch/apply a snapshot.
    BufferedNeedsSnapshot,
    /// Diff `u <= last_update_id`; safely ignored.
    IgnoredStale,
    /// Diff applied continuously; book remains `Synced`.
    Applied,
    /// Gap detected (`U > last_update_id + 1`); book marked `Invalid`, caller MUST resync.
    GapDetectedNeedsResync,
}

/// Synchronized local order book for a single `(Venue, Symbol)`.
#[derive(Debug, Clone)]
pub struct LocalOrderBook {
    venue: Venue,
    symbol: Symbol,
    status: BookStatus,
    last_update_id: u64,
    bids: BTreeMap<Price, Qty>,
    asks: BTreeMap<Price, Qty>,
    buffer: VecDeque<DepthDiffEvent>,
    max_buffer_len: usize,
    obi_top_n: usize,
    obi_ema_alpha: f64,
    obi_ema: Option<f64>,
}

impl LocalOrderBook {
    /// Default top-N depth levels for OBI calculation (§4.3: N=10).
    pub const DEFAULT_OBI_TOP_N: usize = 10;
    /// Default EMA alpha for 30s smoothing on 100ms updates (`2 / (300 + 1)`).
    pub const DEFAULT_OBI_EMA_ALPHA: f64 = 2.0 / 301.0;

    /// Create a new unsynchronized local order book (`BookStatus::Syncing`).
    #[must_use]
    pub fn new(venue: Venue, symbol: Symbol) -> Self {
        Self::with_obi_params(
            venue,
            symbol,
            Self::DEFAULT_OBI_TOP_N,
            Self::DEFAULT_OBI_EMA_ALPHA,
        )
    }

    /// Create a local order book with custom OBI depth `top_n` and EMA smoothing `alpha`.
    #[must_use]
    pub fn with_obi_params(
        venue: Venue,
        symbol: Symbol,
        obi_top_n: usize,
        obi_ema_alpha: f64,
    ) -> Self {
        Self {
            venue,
            symbol,
            status: BookStatus::Syncing,
            last_update_id: 0,
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            buffer: VecDeque::new(),
            max_buffer_len: 2048,
            obi_top_n: obi_top_n.max(1),
            obi_ema_alpha: obi_ema_alpha.clamp(0.0001, 1.0),
            obi_ema: None,
        }
    }

    /// Current sync health (`Syncing`, `Synced`, or `Invalid`).
    #[must_use]
    pub const fn status(&self) -> BookStatus {
        self.status
    }

    /// Latest applied Binance update ID (`0` when not synced).
    #[must_use]
    pub const fn last_update_id(&self) -> u64 {
        self.last_update_id
    }

    /// Number of currently buffered diffs awaiting snapshot sync.
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// Best bid `(price, qty)` — returns `None` if book is not `Synced` or has no bids.
    #[must_use]
    pub fn best_bid(&self) -> Option<(Price, Qty)> {
        if self.status != BookStatus::Synced {
            return None;
        }
        self.bids.iter().next_back().map(|(&p, &q)| (p, q))
    }

    /// Best ask `(price, qty)` — returns `None` if book is not `Synced` or has no asks.
    #[must_use]
    pub fn best_ask(&self) -> Option<(Price, Qty)> {
        if self.status != BookStatus::Synced {
            return None;
        }
        self.asks.iter().next().map(|(&p, &q)| (p, q))
    }

    /// Exact spread in basis points: `(ask - bid) / mid * 10_000`.
    /// Returns `None` whenever `status != BookStatus::Synced`.
    #[must_use]
    pub fn spread_bps(&self) -> Option<Decimal> {
        let (bid_p, _) = self.best_bid()?;
        let (ask_p, _) = self.best_ask()?;
        let bid = bid_p.as_decimal();
        let ask = ask_p.as_decimal();
        if bid <= Decimal::ZERO || ask < bid {
            return None;
        }
        let mid = bid.checked_add(ask)?.checked_div(Decimal::from(2))?;
        if mid <= Decimal::ZERO {
            return None;
        }
        ask.checked_sub(bid)?
            .checked_div(mid)?
            .checked_mul(Decimal::from(10_000))
    }

    /// Top-N Order Book Imbalance: `(ΣbidQty - ΣaskQty) / (ΣbidQty + ΣaskQty)`.
    /// Returns `None` whenever `status != BookStatus::Synced` (§4.3).
    #[must_use]
    pub fn obi_raw(&self) -> Option<f64> {
        if self.status != BookStatus::Synced {
            return None;
        }
        let bid_sum: f64 = self
            .bids
            .iter()
            .rev()
            .take(self.obi_top_n)
            .filter_map(|(_, q)| q.as_decimal().to_f64())
            .sum();
        let ask_sum: f64 = self
            .asks
            .iter()
            .take(self.obi_top_n)
            .filter_map(|(_, q)| q.as_decimal().to_f64())
            .sum();
        let denom = bid_sum + ask_sum;
        if denom <= 0.0 {
            return None;
        }
        Some((bid_sum - ask_sum) / denom)
    }

    /// EMA-smoothed top-N OBI. Returns `None` whenever `status != BookStatus::Synced`.
    #[must_use]
    pub const fn obi_ema(&self) -> Option<f64> {
        if !matches!(self.status, BookStatus::Synced) {
            return None;
        }
        self.obi_ema
    }

    /// Immutable access to bid levels (empty if not `Synced`).
    #[must_use]
    pub const fn bids(&self) -> &BTreeMap<Price, Qty> {
        &self.bids
    }

    /// Immutable access to ask levels (empty if not `Synced`).
    #[must_use]
    pub const fn asks(&self) -> &BTreeMap<Price, Qty> {
        &self.asks
    }

    /// Snapshot telemetry suitable for conflated `watch` channels.
    #[must_use]
    pub fn telemetry(&self) -> BookTelemetry {
        BookTelemetry {
            venue: self.venue,
            symbol: self.symbol.clone(),
            status: self.status,
            last_update_id: self.last_update_id,
            best_bid: self.best_bid().map(|(p, _)| p),
            best_ask: self.best_ask().map(|(p, _)| p),
            spread_bps: self.spread_bps(),
            obi_raw: self.obi_raw(),
            obi_ema: self.obi_ema(),
        }
    }

    /// Mark the book `Invalid`, clear levels, and reset OBI EMA (e.g. on WS disconnect).
    pub fn mark_invalid(&mut self) {
        self.status = BookStatus::Invalid;
        self.last_update_id = 0;
        self.bids.clear();
        self.asks.clear();
        self.obi_ema = None;
    }

    /// Apply a streaming [`DepthDiffEvent`] per official Binance Spot order book sync rules.
    pub fn apply_diff(&mut self, diff: DepthDiffEvent) -> DiffApplyResult {
        if self.status != BookStatus::Synced {
            self.push_buffer(diff);
            return DiffApplyResult::BufferedNeedsSnapshot;
        }

        if diff.final_update_id <= self.last_update_id {
            return DiffApplyResult::IgnoredStale;
        }

        let expected_next = self.last_update_id.saturating_add(1);
        if diff.first_update_id > expected_next {
            // Gap detected -> discard book, mark Invalid, keep triggering diff buffered for resync.
            self.mark_invalid();
            self.buffer.clear();
            self.push_buffer(diff);
            return DiffApplyResult::GapDetectedNeedsResync;
        }

        apply_levels(&mut self.bids, &diff.bids);
        apply_levels(&mut self.asks, &diff.asks);
        self.last_update_id = diff.final_update_id;
        self.update_obi_ema();
        DiffApplyResult::Applied
    }

    /// Apply a REST [`DepthSnapshot`] and replay any buffered diffs per official Binance procedure.
    pub fn apply_snapshot(&mut self, snapshot: &DepthSnapshot) -> SnapshotSyncResult {
        // Official rule: if snapshot.lastUpdateId < U of first buffered event, snapshot is too old.
        if let Some(first) = self.buffer.front()
            && snapshot.last_update_id < first.first_update_id
        {
            return SnapshotSyncResult::SnapshotTooOld;
        }

        self.bids.clear();
        self.asks.clear();
        apply_levels(&mut self.bids, &snapshot.bids);
        apply_levels(&mut self.asks, &snapshot.asks);
        self.last_update_id = snapshot.last_update_id;
        self.obi_ema = None;

        // Discard buffered events with u <= lastUpdateId.
        while let Some(front) = self.buffer.front() {
            if front.final_update_id <= self.last_update_id {
                let _ = self.buffer.pop_front();
            } else {
                break;
            }
        }

        // Replay remaining buffered events with strict continuity verification.
        let pending = std::mem::take(&mut self.buffer);
        for diff in pending {
            if diff.final_update_id <= self.last_update_id {
                continue;
            }
            let expected_next = self.last_update_id.saturating_add(1);
            if diff.first_update_id > expected_next {
                self.mark_invalid();
                self.push_buffer(diff);
                return SnapshotSyncResult::GapDetected;
            }
            apply_levels(&mut self.bids, &diff.bids);
            apply_levels(&mut self.asks, &diff.asks);
            self.last_update_id = diff.final_update_id;
        }

        self.status = BookStatus::Synced;
        self.update_obi_ema();
        SnapshotSyncResult::Synced
    }

    /// Convenience resync loop: fetches up to `max_attempts` snapshots until synced.
    ///
    /// # Errors
    /// Returns [`MarketDataError::SyncFailed`] if all attempts fail or remain stale.
    pub async fn sync_from_client<C: DepthSnapshotClient + ?Sized>(
        &mut self,
        client: &C,
        limit: u16,
        max_attempts: usize,
    ) -> Result<(), MarketDataError> {
        let attempts = max_attempts.max(1);
        for _ in 0..attempts {
            let snap = client
                .fetch_depth_snapshot(self.venue, &self.symbol, limit)
                .await?;
            if self.apply_snapshot(&snap) == SnapshotSyncResult::Synced {
                return Ok(());
            }
        }
        Err(MarketDataError::SyncFailed(format!(
            "failed to synchronize order book for {}:{} after {attempts} attempts",
            self.venue, self.symbol
        )))
    }

    fn push_buffer(&mut self, diff: DepthDiffEvent) {
        if self.buffer.len() >= self.max_buffer_len {
            let _ = self.buffer.pop_front();
        }
        self.buffer.push_back(diff);
    }

    fn update_obi_ema(&mut self) {
        if let Some(raw) = self.obi_raw() {
            self.obi_ema = Some(match self.obi_ema {
                Some(prev) => self.obi_ema_alpha * raw + (1.0 - self.obi_ema_alpha) * prev,
                None => raw,
            });
        }
    }
}

fn apply_levels(side: &mut BTreeMap<Price, Qty>, updates: &[DepthLevel]) {
    for lvl in updates {
        if lvl.qty.as_decimal().is_zero() {
            side.remove(&lvl.price);
        } else {
            side.insert(lvl.price, lvl.qty);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bot_types::{Price, Qty, Symbol, Venue};
    use rust_decimal::Decimal;

    use super::{DiffApplyResult, LocalOrderBook, SnapshotSyncResult};
    use crate::types::{BookStatus, DepthDiffEvent, DepthLevel, DepthSnapshot};

    fn lvl(p: &str, q: &str) -> DepthLevel {
        DepthLevel {
            price: Price(Decimal::from_str(p).expect("p")),
            qty: Qty(Decimal::from_str(q).expect("q")),
        }
    }

    fn diff(
        u_first: u64,
        u_final: u64,
        bids: Vec<DepthLevel>,
        asks: Vec<DepthLevel>,
    ) -> DepthDiffEvent {
        DepthDiffEvent {
            venue: Venue::Spot,
            symbol: Symbol("BTCUSDT".into()),
            event_time_ms: 1000,
            first_update_id: u_first,
            final_update_id: u_final,
            prev_final_update_id: None,
            bids,
            asks,
        }
    }

    #[test]
    fn sync_gap_invalidation_and_resync_match_reference() {
        let mut book = LocalOrderBook::new(Venue::Spot, Symbol("BTCUSDT".into()));
        assert_eq!(book.status(), BookStatus::Syncing);
        assert!(book.obi_raw().is_none());
        assert!(book.spread_bps().is_none());

        // Buffer two diffs before snapshot arrives.
        assert_eq!(
            book.apply_diff(diff(100, 102, vec![lvl("100.00", "2.0")], vec![])),
            DiffApplyResult::BufferedNeedsSnapshot
        );
        assert_eq!(
            book.apply_diff(diff(
                103,
                105,
                vec![lvl("100.00", "3.0")],
                vec![lvl("100.10", "1.0")]
            )),
            DiffApplyResult::BufferedNeedsSnapshot
        );

        // Too-old snapshot (lastUpdateId < 100) is rejected.
        let old_snap = DepthSnapshot {
            venue: Venue::Spot,
            symbol: Symbol("BTCUSDT".into()),
            last_update_id: 99,
            bids: vec![lvl("99.90", "1.0")],
            asks: vec![lvl("100.20", "1.0")],
        };
        assert_eq!(
            book.apply_snapshot(&old_snap),
            SnapshotSyncResult::SnapshotTooOld
        );

        // Valid snapshot at 102 -> discards [100..=102], replays [103..=105].
        let snap = DepthSnapshot {
            venue: Venue::Spot,
            symbol: Symbol("BTCUSDT".into()),
            last_update_id: 102,
            bids: vec![lvl("99.90", "1.0"), lvl("100.00", "2.0")],
            asks: vec![lvl("100.10", "5.0")],
        };
        assert_eq!(book.apply_snapshot(&snap), SnapshotSyncResult::Synced);
        assert_eq!(book.status(), BookStatus::Synced);
        assert_eq!(book.last_update_id(), 105);
        assert_eq!(
            book.best_bid(),
            Some((lvl("100.00", "3.0").price, lvl("100.00", "3.0").qty))
        );
        assert_eq!(
            book.best_ask(),
            Some((lvl("100.10", "1.0").price, lvl("100.10", "1.0").qty))
        );
        assert!(book.obi_raw().expect("obi") > 0.0);
        assert!(book.spread_bps().expect("spread") > Decimal::ZERO);

        // Sequence gap: expected 106, got 110 -> Invalid, OBI & spread become None!
        assert_eq!(
            book.apply_diff(diff(110, 112, vec![lvl("100.00", "0")], vec![])),
            DiffApplyResult::GapDetectedNeedsResync
        );
        assert_eq!(book.status(), BookStatus::Invalid);
        assert!(book.obi_raw().is_none());
        assert!(book.obi_ema().is_none());
        assert!(book.spread_bps().is_none());

        // Resync with fresh snapshot at 111 -> replays buffered [110..=112] removing 100.00!
        let resync_snap = DepthSnapshot {
            venue: Venue::Spot,
            symbol: Symbol("BTCUSDT".into()),
            last_update_id: 111,
            bids: vec![lvl("99.90", "4.0"), lvl("100.00", "1.0")],
            asks: vec![lvl("100.10", "2.0")],
        };
        assert_eq!(
            book.apply_snapshot(&resync_snap),
            SnapshotSyncResult::Synced
        );
        assert_eq!(book.last_update_id(), 112);
        assert_eq!(
            book.best_bid(),
            Some((lvl("99.90", "4.0").price, lvl("99.90", "4.0").qty))
        );
    }
}
