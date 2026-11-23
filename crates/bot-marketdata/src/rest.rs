//! Spot REST market data client for kline backfill (`GET /api/v3/klines`)
//! and depth snapshots (`GET /api/v3/depth`).

use std::time::Duration;

use async_trait::async_trait;
use bot_types::{Kline, Symbol, Venue};
use reqwest::Client;
use serde_json::Value;

use crate::error::MarketDataError;
use crate::kline::KlineBackfillClient;
use crate::orderbook::DepthSnapshotClient;
use crate::types::{DepthSnapshot, Timeframe};
use crate::wire::parse_levels;

/// Spot REST client for kline backfill and order book depth snapshots.
#[derive(Debug, Clone)]
pub struct SpotMarketRestClient {
    base_url: String,
    http: Client,
}

impl SpotMarketRestClient {
    /// Default Spot Testnet REST URL.
    pub const TESTNET_URL: &'static str = "https://testnet.binance.vision";

    /// Create a new REST market data client.
    ///
    /// # Errors
    /// Returns [`MarketDataError::Http`] if the HTTP client fails to initialize.
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> Result<Self, MarketDataError> {
        let http = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| MarketDataError::Http(e.to_string()))?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            http,
        })
    }
}

#[async_trait]
impl KlineBackfillClient for SpotMarketRestClient {
    async fn fetch_klines(
        &self,
        _venue: Venue,
        symbol: &Symbol,
        timeframe: Timeframe,
        start_time_ms: i64,
        end_time_ms: i64,
        limit: u16,
    ) -> Result<Vec<Kline>, MarketDataError> {
        let clamped_limit = limit.clamp(1, 1000);
        let url = format!(
            "{}/api/v3/klines?symbol={}&interval={}&startTime={}&endTime={}&limit={}",
            self.base_url,
            symbol.as_str(),
            timeframe.as_binance_str(),
            start_time_ms,
            end_time_ms,
            clamped_limit
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| MarketDataError::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| MarketDataError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(MarketDataError::Http(format!(
                "GET /api/v3/klines returned {status}: {body}"
            )));
        }
        parse_rest_klines(&body)
    }
}

#[async_trait]
impl DepthSnapshotClient for SpotMarketRestClient {
    async fn fetch_depth_snapshot(
        &self,
        venue: Venue,
        symbol: &Symbol,
        limit: u16,
    ) -> Result<DepthSnapshot, MarketDataError> {
        let clamped_limit = limit.clamp(1, 5000);
        let url = format!(
            "{}/api/v3/depth?symbol={}&limit={}",
            self.base_url,
            symbol.as_str(),
            clamped_limit
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| MarketDataError::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| MarketDataError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(MarketDataError::Http(format!(
                "GET /api/v3/depth returned {status}: {body}"
            )));
        }
        parse_rest_depth_snapshot(&body, venue, symbol.clone())
    }
}

/// Parse Binance Spot `GET /api/v3/klines` JSON array-of-arrays into closed [`Kline`]s.
///
/// # Errors
/// Returns [`MarketDataError::WireFormat`] if the payload is malformed.
pub fn parse_rest_klines(json_str: &str) -> Result<Vec<Kline>, MarketDataError> {
    let val: Value =
        serde_json::from_str(json_str).map_err(|e| MarketDataError::WireFormat(e.to_string()))?;
    let rows = val
        .as_array()
        .ok_or_else(|| MarketDataError::WireFormat("klines response is not array".into()))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let cols = row
            .as_array()
            .ok_or_else(|| MarketDataError::WireFormat("kline row is not array".into()))?;
        out.push(Kline {
            open_time_ms: col_i64(cols, 0)?,
            open: col_f64_str(cols, 1)?,
            high: col_f64_str(cols, 2)?,
            low: col_f64_str(cols, 3)?,
            close: col_f64_str(cols, 4)?,
            volume: col_f64_str(cols, 5)?,
            close_time_ms: col_i64(cols, 6)?,
            quote_volume: col_f64_str(cols, 7)?,
            trades: col_u64(cols, 8)?,
            taker_buy_base: col_f64_str(cols, 9)?,
            is_closed: true,
        });
    }
    Ok(out)
}

/// Parse Binance Spot `GET /api/v3/depth` JSON into [`DepthSnapshot`].
///
/// # Errors
/// Returns [`MarketDataError::WireFormat`] if required fields are missing or invalid.
pub fn parse_rest_depth_snapshot(
    json_str: &str,
    venue: Venue,
    symbol: Symbol,
) -> Result<DepthSnapshot, MarketDataError> {
    let val: Value =
        serde_json::from_str(json_str).map_err(|e| MarketDataError::WireFormat(e.to_string()))?;
    let last_update_id = val
        .get("lastUpdateId")
        .and_then(Value::as_u64)
        .ok_or_else(|| MarketDataError::WireFormat("depth snapshot missing lastUpdateId".into()))?;
    Ok(DepthSnapshot {
        venue,
        symbol,
        last_update_id,
        bids: parse_levels(&val, "bids")?,
        asks: parse_levels(&val, "asks")?,
    })
}

fn col_i64(cols: &[Value], idx: usize) -> Result<i64, MarketDataError> {
    cols.get(idx)
        .and_then(Value::as_i64)
        .ok_or_else(|| MarketDataError::WireFormat(format!("kline col {idx} missing i64")))
}

fn col_u64(cols: &[Value], idx: usize) -> Result<u64, MarketDataError> {
    cols.get(idx)
        .and_then(Value::as_u64)
        .ok_or_else(|| MarketDataError::WireFormat(format!("kline col {idx} missing u64")))
}

fn col_f64_str(cols: &[Value], idx: usize) -> Result<f64, MarketDataError> {
    let s = cols
        .get(idx)
        .and_then(Value::as_str)
        .ok_or_else(|| MarketDataError::WireFormat(format!("kline col {idx} missing str")))?;
    s.parse::<f64>()
        .map_err(|_| MarketDataError::WireFormat(format!("kline col {idx} invalid f64 `{s}`")))
}
