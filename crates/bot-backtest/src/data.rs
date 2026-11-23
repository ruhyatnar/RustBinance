//! Historical kline downloader (`data.binance.vision`), 12-column CSV parser/writer
//! (handling millisecond and 2025+ microsecond timestamps), and gap validation (M09 #1).

use bot_marketdata::Timeframe;
use bot_types::{Kline, Symbol, Venue};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Archive frequency on `https://data.binance.vision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataVisionFrequency {
    /// Monthly aggregated archive (`YYYY-MM`).
    Monthly,
    /// Daily archive (`YYYY-MM-DD`).
    Daily,
}

impl DataVisionFrequency {
    /// Path segment on `data.binance.vision` (`"monthly"` or `"daily"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Monthly => "monthly",
            Self::Daily => "daily",
        }
    }
}

/// Errors encountered while parsing or validating historical kline archives.
#[derive(Debug, Error)]
pub enum DataError {
    /// CSV row does not have at least 6 required columns (`open_time..=volume`).
    #[error("invalid kline CSV line {line}: expected >= 6 columns, got {cols}")]
    InsufficientColumns {
        /// 1-indexed line number.
        line: usize,
        /// Column count found.
        cols: usize,
    },
    /// Failed to parse integer or floating-point field in CSV row.
    #[error("invalid kline CSV field on line {line}: {reason}")]
    ParseField {
        /// 1-indexed line number.
        line: usize,
        /// Field error description.
        reason: String,
    },
}

/// A detected timestamp gap between two consecutive closed bars.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KlineGap {
    /// Expected `open_time_ms` after the preceding bar.
    pub expected_open_ms: i64,
    /// Actual `open_time_ms` of the next bar found.
    pub actual_open_ms: i64,
    /// Number of missing bars in this gap.
    pub missing_bars: usize,
}

/// Result of validating a chronological kline series for continuity and duplicates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapValidationReport {
    /// Total closed bars inspected.
    pub total_bars: usize,
    /// Number of duplicate or non-monotonic bars encountered.
    pub duplicate_or_out_of_order: usize,
    /// Ordered list of detected timestamp gaps.
    pub gaps: Vec<KlineGap>,
}

impl GapValidationReport {
    /// Returns `true` if there are zero missing bars and zero duplicate/out-of-order bars.
    #[must_use]
    pub fn is_gap_free(&self) -> bool {
        self.gaps.is_empty() && self.duplicate_or_out_of_order == 0
    }
}

/// Build the official `https://data.binance.vision` ZIP archive URL for `(venue, symbol, timeframe, freq, period)`.
///
/// Verified pattern (`docs/verified-facts.md` M09):
/// - Spot: `https://data.binance.vision/data/spot/{monthly|daily}/klines/{SYMBOL}/{INTERVAL}/{SYMBOL}-{INTERVAL}-{PERIOD}.zip`
/// - USDⓈ-M Futures: `https://data.binance.vision/data/futures/um/{monthly|daily}/klines/{SYMBOL}/{INTERVAL}/{SYMBOL}-{INTERVAL}-{PERIOD}.zip`
#[must_use]
pub fn build_data_vision_kline_url(
    venue: Venue,
    symbol: &Symbol,
    timeframe: Timeframe,
    freq: DataVisionFrequency,
    period: &str,
) -> String {
    let market_path = match venue {
        Venue::Spot => "spot",
        Venue::UsdsFutures => "futures/um",
    };
    let sym = symbol.as_str();
    let tf = timeframe.as_binance_str();
    let freq_str = freq.as_str();
    format!(
        "https://data.binance.vision/data/{market_path}/{freq_str}/klines/{sym}/{tf}/{sym}-{tf}-{period}.zip"
    )
}

/// Build the accompanying `.zip.CHECKSUM` URL on `https://data.binance.vision`.
#[must_use]
pub fn build_data_vision_checksum_url(
    venue: Venue,
    symbol: &Symbol,
    timeframe: Timeframe,
    freq: DataVisionFrequency,
    period: &str,
) -> String {
    format!(
        "{}.CHECKSUM",
        build_data_vision_kline_url(venue, symbol, timeframe, freq, period)
    )
}

/// Normalize a Binance public-data timestamp to UTC milliseconds.
///
/// Since January 2025, Spot archives on `data.binance.vision` use 16-digit microsecond
/// timestamps (`>= 10^15`), whereas pre-2025 archives and REST `/api/v3/klines` use
/// 13-digit millisecond timestamps (`~1.7 * 10^12`).
#[must_use]
pub const fn normalize_binance_timestamp_ms(raw_ts: i64) -> i64 {
    if raw_ts >= 100_000_000_000_000 {
        raw_ts / 1_000
    } else {
        raw_ts
    }
}

/// Parse a Binance 12-column kline CSV string into chronological closed [`Kline`]s.
///
/// Automatically skips optional header rows (e.g. `open_time,open,...`) and normalizes
/// both millisecond and microsecond (2025+) timestamps.
///
/// # Errors
/// Returns [`DataError`] if any non-empty data line has fewer than 6 columns or invalid numbers.
pub fn parse_binance_kline_csv(csv_text: &str) -> Result<Vec<Kline>, DataError> {
    let mut bars = Vec::new();
    for (idx, raw_line) in csv_text.lines().enumerate() {
        let line_num = idx + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        // Skip optional header row if first char is alphabetic.
        if line.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            continue;
        }

        let cols: Vec<&str> = line.split(',').map(str::trim).collect();
        let Some(&c0) = cols.first() else {
            continue;
        };
        let (Some(&c1), Some(&c2), Some(&c3), Some(&c4), Some(&c5)) = (
            cols.get(1),
            cols.get(2),
            cols.get(3),
            cols.get(4),
            cols.get(5),
        ) else {
            return Err(DataError::InsufficientColumns {
                line: line_num,
                cols: cols.len(),
            });
        };

        let raw_open_ts = c0.parse::<i64>().map_err(|e| DataError::ParseField {
            line: line_num,
            reason: format!("open_time: {e}"),
        })?;
        let open_time_ms = normalize_binance_timestamp_ms(raw_open_ts);

        let open = parse_f64(c1, line_num, "open")?;
        let high = parse_f64(c2, line_num, "high")?;
        let low = parse_f64(c3, line_num, "low")?;
        let close = parse_f64(c4, line_num, "close")?;
        let volume = parse_f64(c5, line_num, "volume")?;

        let close_time_ms = if let Some(&c6) = cols.get(6) {
            let raw_close = c6.parse::<i64>().map_err(|e| DataError::ParseField {
                line: line_num,
                reason: format!("close_time: {e}"),
            })?;
            normalize_binance_timestamp_ms(raw_close)
        } else {
            open_time_ms
        };

        let quote_volume = if let Some(&c7) = cols.get(7) {
            parse_f64(c7, line_num, "quote_volume")?
        } else {
            volume * close
        };

        let trades = if let Some(&c8) = cols.get(8) {
            c8.parse::<u64>().map_err(|e| DataError::ParseField {
                line: line_num,
                reason: format!("trades: {e}"),
            })?
        } else {
            0
        };

        let taker_buy_base = if let Some(&c9) = cols.get(9) {
            parse_f64(c9, line_num, "taker_buy_base")?
        } else {
            0.0
        };

        bars.push(Kline {
            open_time_ms,
            close_time_ms,
            open,
            high,
            low,
            close,
            volume,
            quote_volume,
            trades,
            taker_buy_base,
            is_closed: true,
        });
    }
    Ok(bars)
}

/// Serialize a slice of [`Kline`]s into standard 12-column Binance CSV format.
#[must_use]
pub fn write_binance_kline_csv(bars: &[Kline], include_header: bool) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if include_header {
        out.push_str(
            "open_time,open,high,low,close,volume,close_time,quote_volume,count,taker_buy_volume,taker_buy_quote_volume,ignore\n",
        );
    }
    for b in bars {
        let taker_quote = b.taker_buy_base * b.close;
        let _ = writeln!(
            out,
            "{},{:.8},{:.8},{:.8},{:.8},{:.8},{},{:.8},{},{:.8},{:.8},0",
            b.open_time_ms,
            b.open,
            b.high,
            b.low,
            b.close,
            b.volume,
            b.close_time_ms,
            b.quote_volume,
            b.trades,
            b.taker_buy_base,
            taker_quote
        );
    }
    out
}

/// Validate a sequence of closed klines for timestamp gaps or duplicates at `timeframe`.
#[must_use]
pub fn validate_kline_series_gaps(bars: &[Kline], timeframe: Timeframe) -> GapValidationReport {
    let interval_ms = timeframe.interval_ms();
    let mut duplicate_or_out_of_order = 0;
    let mut gaps = Vec::new();
    let mut prev_open: Option<i64> = None;

    for bar in bars {
        if let Some(prev) = prev_open {
            if bar.open_time_ms <= prev {
                duplicate_or_out_of_order += 1;
                continue;
            }
            let expected = prev.saturating_add(interval_ms);
            if bar.open_time_ms > expected && interval_ms > 0 {
                let diff = bar.open_time_ms.saturating_sub(expected);
                let missing = usize::try_from(diff / interval_ms).unwrap_or(1).max(1);
                gaps.push(KlineGap {
                    expected_open_ms: expected,
                    actual_open_ms: bar.open_time_ms,
                    missing_bars: missing,
                });
            }
        }
        prev_open = Some(bar.open_time_ms);
    }

    GapValidationReport {
        total_bars: bars.len(),
        duplicate_or_out_of_order,
        gaps,
    }
}

fn parse_f64(raw: &str, line: usize, field: &str) -> Result<f64, DataError> {
    let v = raw.parse::<f64>().map_err(|e| DataError::ParseField {
        line,
        reason: format!("{field}: {e}"),
    })?;
    if !v.is_finite() {
        return Err(DataError::ParseField {
            line,
            reason: format!("{field}: non-finite float"),
        });
    }
    Ok(v)
}
