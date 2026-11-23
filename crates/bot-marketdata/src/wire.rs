//! Spot Market Streams URL builder (`<= 1024` streams/conn) and JSON frame parser (§4.3).

use std::str::FromStr;

use bot_types::{Kline, Price, Qty, Symbol, Venue};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::error::MarketDataError;
use crate::types::{BookTickerEvent, DepthDiffEvent, DepthLevel, MiniTickerItem, Timeframe};

/// Official Binance Spot maximum streams per WebSocket connection.
pub const MAX_STREAMS_PER_CONNECTION: usize = 1024;

/// Subscription plan for a single Spot combined market stream connection.
#[derive(Debug, Clone, Default)]
pub struct StreamPlan {
    streams: Vec<String>,
}

impl StreamPlan {
    /// Create an empty stream plan.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add `{symbol}@kline_{interval}` (symbol lowercased per Binance spec).
    pub fn add_kline(&mut self, symbol: &Symbol, timeframe: Timeframe) -> &mut Self {
        let s = format!(
            "{}@kline_{}",
            symbol.as_str().to_ascii_lowercase(),
            timeframe.as_binance_str()
        );
        self.push_unique(s);
        self
    }

    /// Add `{symbol}@bookTicker`.
    pub fn add_book_ticker(&mut self, symbol: &Symbol) -> &mut Self {
        let s = format!("{}@bookTicker", symbol.as_str().to_ascii_lowercase());
        self.push_unique(s);
        self
    }

    /// Add `{symbol}@depth@100ms` (only for symbols being evaluated or with open positions).
    pub fn add_depth_100ms(&mut self, symbol: &Symbol) -> &mut Self {
        let s = format!("{}@depth@100ms", symbol.as_str().to_ascii_lowercase());
        self.push_unique(s);
        self
    }

    /// Add `!miniTicker@arr` for the gainers scanner (replacement for deprecated `!ticker@arr`).
    pub fn add_all_mini_tickers(&mut self) -> &mut Self {
        self.push_unique("!miniTicker@arr".to_owned());
        self
    }

    /// Number of streams in this plan.
    #[must_use]
    pub fn len(&self) -> usize {
        self.streams.len()
    }

    /// Returns `true` if no streams have been added.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    /// Build the combined stream WebSocket URL (`{base_url}/stream?streams=...`).
    ///
    /// # Errors
    /// Returns [`MarketDataError::InvalidStream`] if empty, or
    /// [`MarketDataError::TooManyStreams`] if stream count exceeds `max_streams`.
    pub fn build_combined_url(
        &self,
        base_url: &str,
        max_streams: usize,
    ) -> Result<String, MarketDataError> {
        if self.streams.is_empty() {
            return Err(MarketDataError::InvalidStream(
                "stream plan must contain at least one stream".into(),
            ));
        }
        let limit = max_streams.min(MAX_STREAMS_PER_CONNECTION);
        if self.streams.len() > limit {
            return Err(MarketDataError::TooManyStreams {
                count: self.streams.len(),
                max: limit,
            });
        }
        let base = base_url.trim_end_matches('/');
        let joined = self.streams.join("/");
        Ok(format!("{base}/stream?streams={joined}"))
    }

    fn push_unique(&mut self, stream: String) {
        if !self.streams.contains(&stream) {
            self.streams.push(stream);
        }
    }
}

/// Parsed inbound Spot market stream message.
#[derive(Debug, Clone, PartialEq)]
pub enum MarketStreamMessage {
    /// Kline tick (closed or forming).
    Kline {
        /// Venue.
        venue: Venue,
        /// Symbol.
        symbol: Symbol,
        /// Timeframe.
        timeframe: Timeframe,
        /// OHLCV bar.
        kline: Kline,
    },
    /// Best bid/ask tick.
    BookTicker(BookTickerEvent),
    /// Incremental order book diff.
    DepthDiff(DepthDiffEvent),
    /// All-market 24h mini tickers (`!miniTicker@arr`).
    MiniTickerBatch(Vec<MiniTickerItem>),
    /// Server shutdown notice (`serverShutdown`).
    ServerShutdown {
        /// Event time (UTC ms).
        event_time_ms: i64,
        /// Planned shutdown time (UTC ms), if provided.
        shutdown_time_ms: Option<i64>,
    },
}

/// Parse a raw JSON text frame from a Spot market stream (combined or single-stream).
///
/// # Errors
/// Returns [`MarketDataError::WireFormat`] if the JSON cannot be parsed or is unrecognized.
pub fn parse_market_stream_frame(
    text: &str,
    venue: Venue,
    received_at_ms: i64,
) -> Result<MarketStreamMessage, MarketDataError> {
    let val: Value =
        serde_json::from_str(text).map_err(|e| MarketDataError::WireFormat(e.to_string()))?;

    // Unwrap combined-stream envelope `{"stream":"...","data":...}` if present.
    let (stream_name, payload) = match (val.get("stream").and_then(Value::as_str), val.get("data"))
    {
        (Some(name), Some(data)) => (Some(name), data),
        _ => (None, &val),
    };

    // `!miniTicker@arr` payload is a JSON array.
    if let Some(arr) = payload.as_array() {
        let mut items = Vec::with_capacity(arr.len());
        for entry in arr {
            items.push(parse_mini_ticker(entry, venue)?);
        }
        return Ok(MarketStreamMessage::MiniTickerBatch(items));
    }

    // Check `e` event discriminator first.
    if let Some(ev_type) = payload.get("e").and_then(Value::as_str) {
        return match ev_type {
            "kline" => parse_kline_event(payload, venue),
            "depthUpdate" => Ok(MarketStreamMessage::DepthDiff(parse_depth_diff(
                payload, venue,
            )?)),
            "24hrMiniTicker" => Ok(MarketStreamMessage::MiniTickerBatch(vec![
                parse_mini_ticker(payload, venue)?,
            ])),
            "serverShutdown" => Ok(MarketStreamMessage::ServerShutdown {
                event_time_ms: req_i64(payload, "E")?,
                shutdown_time_ms: payload.get("T").and_then(Value::as_i64),
            }),
            other => Err(MarketDataError::WireFormat(format!(
                "unrecognized market stream event `{other}`"
            ))),
        };
    }

    // `bookTicker` does not carry an `"e"` field on Spot; identified by stream suffix or `(u, s, b, B, a, A)`.
    if stream_name.is_some_and(|n| n.ends_with("@bookTicker"))
        || (payload.get("u").is_some()
            && payload.get("s").is_some()
            && payload.get("b").is_some()
            && payload.get("a").is_some())
    {
        return Ok(MarketStreamMessage::BookTicker(parse_book_ticker(
            payload,
            venue,
            received_at_ms,
        )?));
    }

    Err(MarketDataError::WireFormat(
        "unrecognized market stream frame".into(),
    ))
}

fn parse_kline_event(v: &Value, venue: Venue) -> Result<MarketStreamMessage, MarketDataError> {
    let k = v
        .get("k")
        .ok_or_else(|| MarketDataError::WireFormat("kline event missing `k` object".into()))?;
    let interval = req_str(k, "i")?;
    let timeframe = Timeframe::from_binance_str(interval).ok_or_else(|| {
        MarketDataError::WireFormat(format!("unsupported kline interval `{interval}`"))
    })?;
    let symbol = Symbol(req_str(k, "s")?.to_owned());
    let kline = Kline {
        open_time_ms: req_i64(k, "t")?,
        close_time_ms: req_i64(k, "T")?,
        open: req_f64_str(k, "o")?,
        high: req_f64_str(k, "h")?,
        low: req_f64_str(k, "l")?,
        close: req_f64_str(k, "c")?,
        volume: req_f64_str(k, "v")?,
        quote_volume: req_f64_str(k, "q")?,
        trades: req_u64(k, "n")?,
        taker_buy_base: req_f64_str(k, "V")?,
        is_closed: k
            .get("x")
            .and_then(Value::as_bool)
            .ok_or_else(|| MarketDataError::WireFormat("kline missing boolean `x`".into()))?,
    };
    Ok(MarketStreamMessage::Kline {
        venue,
        symbol,
        timeframe,
        kline,
    })
}

fn parse_book_ticker(
    v: &Value,
    venue: Venue,
    received_at_ms: i64,
) -> Result<BookTickerEvent, MarketDataError> {
    Ok(BookTickerEvent {
        venue,
        symbol: Symbol(req_str(v, "s")?.to_owned()),
        update_id: req_u64(v, "u")?,
        bid_price: Price(req_dec(v, "b")?),
        bid_qty: Qty(req_dec(v, "B")?),
        ask_price: Price(req_dec(v, "a")?),
        ask_qty: Qty(req_dec(v, "A")?),
        received_at_ms,
    })
}

fn parse_depth_diff(v: &Value, venue: Venue) -> Result<DepthDiffEvent, MarketDataError> {
    Ok(DepthDiffEvent {
        venue,
        symbol: Symbol(req_str(v, "s")?.to_owned()),
        event_time_ms: req_i64(v, "E")?,
        first_update_id: req_u64(v, "U")?,
        final_update_id: req_u64(v, "u")?,
        prev_final_update_id: v.get("pu").and_then(Value::as_u64),
        bids: parse_levels(v, "b")?,
        asks: parse_levels(v, "a")?,
    })
}

fn parse_mini_ticker(v: &Value, venue: Venue) -> Result<MiniTickerItem, MarketDataError> {
    Ok(MiniTickerItem {
        venue,
        symbol: Symbol(req_str(v, "s")?.to_owned()),
        event_time_ms: req_i64(v, "E")?,
        close: Price(req_dec(v, "c")?),
        open: Price(req_dec(v, "o")?),
        high: Price(req_dec(v, "h")?),
        low: Price(req_dec(v, "l")?),
        base_volume: req_dec(v, "v")?,
        quote_volume: req_dec(v, "q")?,
    })
}

pub(crate) fn parse_levels(v: &Value, field: &str) -> Result<Vec<DepthLevel>, MarketDataError> {
    let arr = v
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| MarketDataError::WireFormat(format!("missing level array `{field}`")))?;
    let mut out = Vec::with_capacity(arr.len());
    for pair in arr {
        let tuple = pair
            .as_array()
            .ok_or_else(|| MarketDataError::WireFormat("depth level is not array".into()))?;
        let p_str = tuple
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| MarketDataError::WireFormat("depth level missing price".into()))?;
        let q_str = tuple
            .get(1)
            .and_then(Value::as_str)
            .ok_or_else(|| MarketDataError::WireFormat("depth level missing qty".into()))?;
        let price = Decimal::from_str(p_str)
            .map_err(|_| MarketDataError::WireFormat(format!("invalid price `{p_str}`")))?;
        let qty = Decimal::from_str(q_str)
            .map_err(|_| MarketDataError::WireFormat(format!("invalid qty `{q_str}`")))?;
        out.push(DepthLevel {
            price: Price(price),
            qty: Qty(qty),
        });
    }
    Ok(out)
}

fn req_str<'a>(v: &'a Value, field: &str) -> Result<&'a str, MarketDataError> {
    v.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| MarketDataError::WireFormat(format!("missing string `{field}`")))
}

fn req_i64(v: &Value, field: &str) -> Result<i64, MarketDataError> {
    v.get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| MarketDataError::WireFormat(format!("missing i64 `{field}`")))
}

fn req_u64(v: &Value, field: &str) -> Result<u64, MarketDataError> {
    v.get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| MarketDataError::WireFormat(format!("missing u64 `{field}`")))
}

fn req_dec(v: &Value, field: &str) -> Result<Decimal, MarketDataError> {
    let s = req_str(v, field)?;
    Decimal::from_str(s)
        .map_err(|_| MarketDataError::WireFormat(format!("invalid decimal `{s}` in `{field}`")))
}

fn req_f64_str(v: &Value, field: &str) -> Result<f64, MarketDataError> {
    let s = req_str(v, field)?;
    s.parse::<f64>()
        .map_err(|_| MarketDataError::WireFormat(format!("invalid f64 `{s}` in `{field}`")))
}

#[cfg(test)]
mod tests {
    use bot_types::{Symbol, Venue};

    use super::{
        MAX_STREAMS_PER_CONNECTION, MarketStreamMessage, StreamPlan, parse_market_stream_frame,
    };
    use crate::error::MarketDataError;
    use crate::types::Timeframe;

    #[test]
    fn enforces_max_streams_per_connection() {
        let mut plan = StreamPlan::new();
        for i in 0..=MAX_STREAMS_PER_CONNECTION {
            plan.add_book_ticker(&Symbol(format!("SYM{i}USDT")));
        }
        let err = plan
            .build_combined_url(
                "wss://stream.testnet.binance.vision",
                MAX_STREAMS_PER_CONNECTION,
            )
            .expect_err("must reject > 1024 streams");
        assert!(matches!(err, MarketDataError::TooManyStreams { .. }));
    }

    #[test]
    fn parses_combined_kline_book_ticker_depth_and_mini_ticker() {
        let kline_json = r#"{
            "stream": "btcusdt@kline_1h",
            "data": {
                "e": "kline",
                "E": 1700000000000,
                "s": "BTCUSDT",
                "k": {
                    "t": 1700000000000,
                    "T": 1700003599999,
                    "s": "BTCUSDT",
                    "i": "1h",
                    "o": "60000.0",
                    "c": "60500.0",
                    "h": "60600.0",
                    "l": "59900.0",
                    "v": "123.45",
                    "n": 420,
                    "x": true,
                    "q": "7400000.0",
                    "V": "60.0"
                }
            }
        }"#;
        match parse_market_stream_frame(kline_json, Venue::Spot, 10).expect("kline") {
            MarketStreamMessage::Kline {
                symbol,
                timeframe: Timeframe::H1,
                kline,
                ..
            } => {
                assert_eq!(symbol.as_str(), "BTCUSDT");
                assert!(kline.is_closed);
            }
            other => panic!("unexpected {other:?}"),
        }

        let bt_json = r#"{
            "stream": "btcusdt@bookTicker",
            "data": {"u": 400900217, "s": "BTCUSDT", "b": "60000.00", "B": "1.5", "a": "60006.00", "A": "2.0"}
        }"#;
        match parse_market_stream_frame(bt_json, Venue::Spot, 500).expect("bookTicker") {
            MarketStreamMessage::BookTicker(bt) => {
                assert_eq!(bt.symbol.as_str(), "BTCUSDT");
                assert!(bt.spread_bps().is_some());
            }
            other => panic!("unexpected {other:?}"),
        }

        let mini_json = r#"{
            "stream": "!miniTicker@arr",
            "data": [{"e":"24hrMiniTicker","E":1700000000000,"s":"BTCUSDT","c":"60000.0","o":"58000.0","h":"61000.0","l":"57500.0","v":"1000.0","q":"60000000.0"}]
        }"#;
        match parse_market_stream_frame(mini_json, Venue::Spot, 500).expect("miniTicker") {
            MarketStreamMessage::MiniTickerBatch(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].symbol.as_str(), "BTCUSDT");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
