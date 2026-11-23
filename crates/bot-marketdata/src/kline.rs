//! Per-`(Venue, Symbol, Timeframe)` kline buffer with closed-bar gating, gap detection,
//! REST backfill, and warm-up readiness tracking (§4.3, §6.5).

use std::collections::VecDeque;

use async_trait::async_trait;
use bot_types::{Kline, Symbol, Venue};

use crate::error::MarketDataError;
use crate::types::{ClosedKlineEvent, KlineKey, Timeframe};

/// Abstraction for fetching historical closed candles to backfill detected gaps.
#[async_trait]
pub trait KlineBackfillClient: Send + Sync {
    /// Fetch klines in `[start_time_ms, end_time_ms]` (inclusive) for `(venue, symbol, timeframe)`.
    async fn fetch_klines(
        &self,
        venue: Venue,
        symbol: &Symbol,
        timeframe: Timeframe,
        start_time_ms: i64,
        end_time_ms: i64,
        limit: u16,
    ) -> Result<Vec<Kline>, MarketDataError>;
}

/// Result of feeding a single streaming kline update into [`KlineBuffer`].
#[derive(Debug, Clone, PartialEq)]
pub enum KlineIngestResult {
    /// Candle is still open (`is_closed == false`); stored as forming only, NEVER triggers signals.
    Forming,
    /// Duplicate or older closed candle already recorded.
    Duplicate,
    /// Contiguous closed candle appended cleanly; ready to emit to strategy engine.
    Closed(ClosedKlineEvent),
    /// One or more bars are missing between the previous closed bar and this closed bar.
    GapDetected {
        /// First missing bar's expected `open_time_ms`.
        expected_open_ms: i64,
        /// `open_time_ms` of the newly arrived closed bar.
        actual_open_ms: i64,
        /// The newly arrived closed bar (held until backfill completes).
        pending_bar: Kline,
    },
}

/// Bounded ring buffer of closed klines for one `(Venue, Symbol, Timeframe)`.
#[derive(Debug, Clone)]
pub struct KlineBuffer {
    key: KlineKey,
    capacity: usize,
    warmup_bars: usize,
    closed_bars: VecDeque<Kline>,
    latest_forming: Option<Kline>,
    unresolved_gap: bool,
}

impl KlineBuffer {
    /// Create a new buffer requiring `warmup_bars` closed bars before [`Self::is_ready`] is `true`.
    #[must_use]
    pub fn new(key: KlineKey, warmup_bars: usize, capacity: usize) -> Self {
        let cap = capacity.max(warmup_bars).max(1);
        Self {
            key,
            capacity: cap,
            warmup_bars,
            closed_bars: VecDeque::with_capacity(cap),
            latest_forming: None,
            unresolved_gap: false,
        }
    }

    /// Composite `(Venue, Symbol, Timeframe)` key.
    #[must_use]
    pub const fn key(&self) -> &KlineKey {
        &self.key
    }

    /// Returns `true` when warm-up bar count is satisfied and no gap is unresolved (§6.5).
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.closed_bars.len() >= self.warmup_bars && !self.unresolved_gap
    }

    /// Number of closed bars currently buffered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.closed_bars.len()
    }

    /// Returns `true` if no closed bars have been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.closed_bars.is_empty()
    }

    /// Latest non-closed (forming) bar, useful for intrabar trailing/exit checks.
    #[must_use]
    pub const fn latest_forming(&self) -> Option<&Kline> {
        self.latest_forming.as_ref()
    }

    /// Ordered slice-like iterator over closed bars from oldest to newest.
    pub fn closed_bars(&self) -> impl Iterator<Item = &Kline> {
        self.closed_bars.iter()
    }

    /// Ingest a kline tick from the market stream.
    ///
    /// Non-closed candles (`is_closed == false`) only update `latest_forming` and NEVER
    /// emit a [`ClosedKlineEvent`].
    pub fn on_kline(&mut self, bar: Kline) -> KlineIngestResult {
        if !bar.is_closed {
            self.latest_forming = Some(bar);
            return KlineIngestResult::Forming;
        }

        let interval = self.key.timeframe.interval_ms();
        if let Some(last) = self.closed_bars.back() {
            if bar.open_time_ms <= last.open_time_ms {
                return KlineIngestResult::Duplicate;
            }
            let expected = last.open_time_ms.saturating_add(interval);
            if bar.open_time_ms > expected {
                self.unresolved_gap = true;
                return KlineIngestResult::GapDetected {
                    expected_open_ms: expected,
                    actual_open_ms: bar.open_time_ms,
                    pending_bar: bar,
                };
            }
        }

        self.push_closed_bar(bar.clone());
        self.latest_forming = None;
        KlineIngestResult::Closed(ClosedKlineEvent {
            venue: self.key.venue,
            symbol: self.key.symbol.clone(),
            timeframe: self.key.timeframe,
            kline: bar,
        })
    }

    /// Seed historical closed bars (e.g. initial warm-up) in chronological order.
    pub fn seed_history(&mut self, mut bars: Vec<Kline>) {
        bars.sort_by_key(|b| b.open_time_ms);
        for bar in bars {
            if bar.is_closed {
                if let Some(last) = self.closed_bars.back()
                    && bar.open_time_ms <= last.open_time_ms
                {
                    continue;
                }
                self.push_closed_bar(bar);
            }
        }
        self.unresolved_gap = false;
    }

    /// Resolve a detected gap by applying backfilled closed bars followed by `pending_bar`.
    ///
    /// Returns the ordered list of newly accepted [`ClosedKlineEvent`]s to emit downstream.
    pub fn apply_backfill(
        &mut self,
        mut backfill_bars: Vec<Kline>,
        pending_bar: Kline,
    ) -> Vec<ClosedKlineEvent> {
        backfill_bars.sort_by_key(|b| b.open_time_ms);
        backfill_bars.push(pending_bar);

        let interval = self.key.timeframe.interval_ms();
        let mut emitted = Vec::new();
        let mut still_has_gap = false;

        for bar in backfill_bars {
            if !bar.is_closed {
                continue;
            }
            if let Some(last) = self.closed_bars.back() {
                if bar.open_time_ms <= last.open_time_ms {
                    continue;
                }
                let expected = last.open_time_ms.saturating_add(interval);
                if bar.open_time_ms > expected {
                    still_has_gap = true;
                }
            }
            self.push_closed_bar(bar.clone());
            emitted.push(ClosedKlineEvent {
                venue: self.key.venue,
                symbol: self.key.symbol.clone(),
                timeframe: self.key.timeframe,
                kline: bar,
            });
        }

        self.unresolved_gap = still_has_gap;
        self.latest_forming = None;
        emitted
    }

    /// Convenience helper: ingest a bar and automatically call `backfill_client` on gap.
    ///
    /// # Errors
    /// Returns [`MarketDataError`] if REST backfill fails on a detected gap.
    pub async fn ingest_with_backfill<C: KlineBackfillClient + ?Sized>(
        &mut self,
        bar: Kline,
        backfill_client: &C,
    ) -> Result<Vec<ClosedKlineEvent>, MarketDataError> {
        match self.on_kline(bar) {
            KlineIngestResult::Forming | KlineIngestResult::Duplicate => Ok(Vec::new()),
            KlineIngestResult::Closed(ev) => Ok(vec![ev]),
            KlineIngestResult::GapDetected {
                expected_open_ms,
                actual_open_ms,
                pending_bar,
            } => {
                let end_ms = actual_open_ms.saturating_sub(1);
                let fetched = backfill_client
                    .fetch_klines(
                        self.key.venue,
                        &self.key.symbol,
                        self.key.timeframe,
                        expected_open_ms,
                        end_ms,
                        1000,
                    )
                    .await?;
                Ok(self.apply_backfill(fetched, pending_bar))
            }
        }
    }

    fn push_closed_bar(&mut self, bar: Kline) {
        if self.closed_bars.len() >= self.capacity {
            let _ = self.closed_bars.pop_front();
        }
        self.closed_bars.push_back(bar);
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use bot_types::{Kline, Symbol, Venue};

    use super::{KlineBackfillClient, KlineBuffer, KlineIngestResult};
    use crate::error::MarketDataError;
    use crate::types::{KlineKey, Timeframe};

    fn make_bar(open_ms: i64, tf: Timeframe, is_closed: bool) -> Kline {
        Kline {
            open_time_ms: open_ms,
            close_time_ms: open_ms + tf.interval_ms() - 1,
            open: 100.0,
            high: 105.0,
            low: 99.0,
            close: 102.0,
            volume: 10.0,
            quote_volume: 1020.0,
            trades: 50,
            taker_buy_base: 5.0,
            is_closed,
        }
    }

    struct MockBackfill(Vec<Kline>);

    #[async_trait]
    impl KlineBackfillClient for MockBackfill {
        async fn fetch_klines(
            &self,
            _venue: Venue,
            _symbol: &Symbol,
            _timeframe: Timeframe,
            start_time_ms: i64,
            end_time_ms: i64,
            _limit: u16,
        ) -> Result<Vec<Kline>, MarketDataError> {
            Ok(self
                .0
                .iter()
                .filter(|b| b.open_time_ms >= start_time_ms && b.open_time_ms <= end_time_ms)
                .cloned()
                .collect())
        }
    }

    #[test]
    fn non_closed_kline_never_emits_closed_event() {
        let key = KlineKey {
            venue: Venue::Spot,
            symbol: Symbol("BTCUSDT".into()),
            timeframe: Timeframe::H1,
        };
        let mut buf = KlineBuffer::new(key, 2, 10);
        let forming = make_bar(0, Timeframe::H1, false);
        assert_eq!(buf.on_kline(forming), KlineIngestResult::Forming);
        assert!(buf.is_empty());
        assert!(buf.latest_forming().is_some());
        assert!(!buf.is_ready());
    }

    #[tokio::test]
    async fn detects_gap_backfills_missing_bars_and_reaches_ready() {
        let tf = Timeframe::H1;
        let step = tf.interval_ms();
        let key = KlineKey {
            venue: Venue::Spot,
            symbol: Symbol("BTCUSDT".into()),
            timeframe: tf,
        };
        let mut buf = KlineBuffer::new(key, 4, 10);

        let mock = MockBackfill(vec![make_bar(step, tf, true), make_bar(2 * step, tf, true)]);

        // Bar 0 arrives normally.
        let r0 = buf
            .ingest_with_backfill(make_bar(0, tf, true), &mock)
            .await
            .expect("bar 0");
        assert_eq!(r0.len(), 1);
        assert!(!buf.is_ready());

        // Bar 3 arrives -> missing bars 1 and 2 -> backfill fills them in order!
        let r3 = buf
            .ingest_with_backfill(make_bar(3 * step, tf, true), &mock)
            .await
            .expect("bar 3 with backfill");
        let open_times: Vec<i64> = r3.iter().map(|e| e.kline.open_time_ms).collect();
        assert_eq!(open_times, vec![step, 2 * step, 3 * step]);
        assert_eq!(buf.len(), 4);
        assert!(buf.is_ready());
    }
}
