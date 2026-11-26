//! Spot REST client for clock synchronization (`GET /api/v3/time`) and `exchangeInfo` (§4.1).

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bot_types::{InstrumentRules, Kline, Symbol, Venue};
use reqwest::Client;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::auth::{Ed25519Credentials, signed_query};
use crate::clock::ClockSyncState;
use crate::error::GatewayError;
use crate::gateway::{AccountBalance, AccountSnapshot, ExchangeInfoSnapshot};
use crate::rate_limit::{RateLimitRule, RateLimitTracker};

/// Result of calling `GET /api/v3/time`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerTimeSync {
    /// Server UTC epoch timestamp in milliseconds.
    pub server_time_ms: i64,
    /// Local UTC epoch timestamp in milliseconds (midpoint of request).
    pub local_time_ms: i64,
    /// Clock offset (`server_time_ms - local_time_ms`).
    pub offset_ms: i64,
}

/// Minimal Spot REST client (used for bootstrap `time`, `exchangeInfo`, and fallback).
#[derive(Debug, Clone)]
pub struct SpotRestClient {
    base_url: String,
    http: Client,
    clock: ClockSyncState,
    rate_limits: Arc<RateLimitTracker>,
}

impl SpotRestClient {
    /// Production Spot REST base URL.
    pub const PRODUCTION_URL: &'static str = "https://api.binance.com";
    /// Testnet Spot REST base URL.
    pub const TESTNET_URL: &'static str = "https://testnet.binance.vision";

    /// Create a new Spot REST client targeting `base_url` with an isolated [`RateLimitTracker`].
    ///
    /// # Errors
    /// Returns [`GatewayError::Http`] if the underlying HTTP client fails to initialize.
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> Result<Self, GatewayError> {
        Self::with_rate_limits(base_url, timeout, Arc::new(RateLimitTracker::new()))
    }

    /// Create a new Spot REST client sharing the given [`RateLimitTracker`].
    ///
    /// # Errors
    /// Returns [`GatewayError::Http`] if the underlying HTTP client fails to initialize.
    pub fn with_rate_limits(
        base_url: impl Into<String>,
        timeout: Duration,
        rate_limits: Arc<RateLimitTracker>,
    ) -> Result<Self, GatewayError> {
        let http = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| GatewayError::Http(e.to_string()))?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            http,
            clock: ClockSyncState::new(),
            rate_limits,
        })
    }

    /// Return the shared clock synchronization state updated by [`Self::server_time`].
    #[must_use]
    pub const fn clock_state(&self) -> &ClockSyncState {
        &self.clock
    }

    /// Return the shared rate-limit tracker updated from HTTP response headers.
    #[must_use]
    pub const fn rate_limits(&self) -> &Arc<RateLimitTracker> {
        &self.rate_limits
    }

    /// Query `GET /api/v3/ping` to test Spot REST connectivity.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on HTTP failure, rate limit (429/418), or non-success status.
    pub async fn ping(&self) -> Result<(), GatewayError> {
        let url = format!("{}/api/v3/ping", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| GatewayError::Http(e.to_string()))?;
        let _body = check_rest_response(resp, now_unix_ms(), &self.rate_limits).await?;
        Ok(())
    }

    /// Query `GET /api/v3/time` and compute local-to-server clock offset.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on HTTP failure, rate limit (429/418), or malformed JSON.
    pub async fn server_time(&self) -> Result<ServerTimeSync, GatewayError> {
        let url = format!("{}/api/v3/time", self.base_url);
        let t0 = now_unix_ms();
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| GatewayError::Http(e.to_string()))?;
        let t1 = now_unix_ms();
        let local_mid = t0.saturating_add(t1.saturating_sub(t0) / 2);

        let body = check_rest_response(resp, local_mid, &self.rate_limits).await?;
        let parsed: RawServerTime =
            serde_json::from_str(&body).map_err(|e| GatewayError::WireFormat(e.to_string()))?;

        let sync = ServerTimeSync {
            server_time_ms: parsed.server_time,
            local_time_ms: local_mid,
            offset_ms: parsed.server_time.saturating_sub(local_mid),
        };
        self.clock.record(sync);
        Ok(sync)
    }

    /// Query `GET /api/v3/exchangeInfo` and parse symbol filter rules + rate limits.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on HTTP failure, rate limit (429/418), or malformed payload.
    pub async fn exchange_info(&self) -> Result<ExchangeInfoSnapshot, GatewayError> {
        let url = format!("{}/api/v3/exchangeInfo", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| GatewayError::Http(e.to_string()))?;
        let body = check_rest_response(resp, now_unix_ms(), &self.rate_limits).await?;
        parse_spot_exchange_info(&body)
    }

    /// Signed read-only `GET /api/v3/account` (weight 20). Never places orders.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on HTTP failure, Binance API error (e.g. `-2015`, `-1022`),
    /// or malformed payload.
    pub async fn account_snapshot(
        &self,
        creds: &Ed25519Credentials,
        recv_window_ms: u64,
    ) -> Result<AccountSnapshot, GatewayError> {
        let mut params = BTreeMap::new();
        params.insert("recvWindow".to_owned(), recv_window_ms.to_string());
        params.insert(
            "timestamp".to_owned(),
            self.clock.adjusted_now_ms(now_unix_ms()).to_string(),
        );
        let sig = creds.sign_params(&params);
        let url = format!(
            "{}/api/v3/account?{}",
            self.base_url,
            signed_query(&params, &sig)
        );
        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", creds.api_key())
            .send()
            .await
            .map_err(|e| GatewayError::Http(e.to_string()))?;
        let body = check_rest_response(resp, now_unix_ms(), &self.rate_limits).await?;
        parse_spot_account(&body)
    }

    /// Public `GET /api/v3/klines` (no auth). Last bar may still be forming.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on HTTP failure or malformed payload.
    pub async fn klines(
        &self,
        symbol: &str,
        interval: &str,
        limit: u16,
    ) -> Result<Vec<Kline>, GatewayError> {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("symbol", symbol)
            .append_pair("interval", interval)
            .append_pair("limit", &limit.to_string())
            .finish();
        let url = format!("{}/api/v3/klines?{query}", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| GatewayError::Http(e.to_string()))?;
        let body = check_rest_response(resp, now_unix_ms(), &self.rate_limits).await?;
        parse_rest_klines(&body, now_unix_ms())
    }
}

/// Parse a Spot `GET /api/v3/account` payload, keeping only non-zero balances.
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if the JSON structure is invalid.
pub fn parse_spot_account(raw: &str) -> Result<AccountSnapshot, GatewayError> {
    let v: RawSpotAccount =
        serde_json::from_str(raw).map_err(|e| GatewayError::WireFormat(e.to_string()))?;
    let balances = v
        .balances
        .into_iter()
        .filter_map(|b| {
            let free = Decimal::from_str(&b.free).ok()?;
            let locked = Decimal::from_str(&b.locked).ok()?;
            (!free.is_zero() || !locked.is_zero()).then_some(AccountBalance {
                asset: b.asset,
                free,
                locked,
            })
        })
        .collect();
    Ok(AccountSnapshot {
        venue: Venue::Spot,
        can_trade: v.can_trade,
        update_time_ms: v.update_time.unwrap_or_else(now_unix_ms),
        balances,
    })
}

/// Parse a Binance REST klines array (12-column rows, Spot and Futures share the format).
///
/// A bar is marked closed when its `close_time_ms < now_ms`.
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] on malformed rows.
pub fn parse_rest_klines(raw: &str, now_ms: i64) -> Result<Vec<Kline>, GatewayError> {
    let rows: Vec<Vec<serde_json::Value>> =
        serde_json::from_str(raw).map_err(|e| GatewayError::WireFormat(e.to_string()))?;
    rows.iter()
        .map(|row| {
            let bad = || GatewayError::WireFormat("malformed kline row".into());
            let int = |i: usize| {
                row.get(i)
                    .and_then(serde_json::Value::as_i64)
                    .ok_or_else(bad)
            };
            let num = |i: usize| {
                row.get(i)
                    .and_then(serde_json::Value::as_str)
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|v| v.is_finite())
                    .ok_or_else(bad)
            };
            let close_time_ms = int(6)?;
            Ok(Kline {
                open_time_ms: int(0)?,
                close_time_ms,
                open: num(1)?,
                high: num(2)?,
                low: num(3)?,
                close: num(4)?,
                volume: num(5)?,
                quote_volume: num(7)?,
                trades: row
                    .get(8)
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(bad)?,
                taker_buy_base: num(9)?,
                is_closed: close_time_ms < now_ms,
            })
        })
        .collect()
}

/// Parse a Binance Spot `exchangeInfo` JSON payload into [`ExchangeInfoSnapshot`].
///
/// Reads `PRICE_FILTER`, `LOT_SIZE`, and `NOTIONAL` (falling back to `MIN_NOTIONAL` per
/// `docs/verified-facts.md`). Symbols missing mandatory filters are skipped cleanly.
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if the JSON structure is invalid.
pub fn parse_spot_exchange_info(json_str: &str) -> Result<ExchangeInfoSnapshot, GatewayError> {
    let raw: RawExchangeInfo =
        serde_json::from_str(json_str).map_err(|e| GatewayError::WireFormat(e.to_string()))?;

    let mut rules = Vec::with_capacity(raw.symbols.len());
    for sym in raw.symbols {
        if let Some(parsed) = parse_symbol_rules(sym) {
            rules.push(parsed);
        }
    }

    Ok(ExchangeInfoSnapshot {
        venue: Venue::Spot,
        server_time_ms: raw.server_time,
        rules,
        rate_limits: raw.rate_limits,
    })
}

fn parse_symbol_rules(sym: RawSymbol) -> Option<InstrumentRules> {
    let mut tick_size: Option<Decimal> = None;
    let mut step_size: Option<Decimal> = None;
    let mut min_qty: Option<Decimal> = None;
    let mut max_qty: Option<Decimal> = None;
    let mut notional_primary: Option<Decimal> = None;
    let mut notional_legacy: Option<Decimal> = None;

    for filter in sym.filters {
        match filter.filter_type.as_str() {
            "PRICE_FILTER" => {
                tick_size = filter.tick_size.as_deref().and_then(parse_dec);
            }
            "LOT_SIZE" => {
                step_size = filter.step_size.as_deref().and_then(parse_dec);
                min_qty = filter.min_qty.as_deref().and_then(parse_dec);
                max_qty = filter.max_qty.as_deref().and_then(parse_dec);
            }
            "NOTIONAL" => {
                notional_primary = filter.min_notional.as_deref().and_then(parse_dec);
            }
            "MIN_NOTIONAL" => {
                notional_legacy = filter.min_notional.as_deref().and_then(parse_dec);
            }
            _ => {}
        }
    }

    let rules = InstrumentRules {
        venue: Venue::Spot,
        symbol: Symbol(sym.symbol),
        tick_size: tick_size?,
        step_size: step_size?,
        min_qty: min_qty?,
        max_qty: max_qty?,
        min_notional: notional_primary
            .or(notional_legacy)
            .unwrap_or(Decimal::ZERO),
        status_trading: sym.status == "TRADING",
        base_precision: sym.base_asset_precision,
        quote_precision: sym.quote_asset_precision,
    };

    if rules.validate_rules().is_ok() {
        Some(rules)
    } else {
        None
    }
}

fn parse_dec(s: &str) -> Option<Decimal> {
    Decimal::from_str(s).ok()
}

async fn check_rest_response(
    resp: reqwest::Response,
    now_ms: i64,
    rate_limits: &RateLimitTracker,
) -> Result<String, GatewayError> {
    let status = resp.status().as_u16();
    let retry_after_ms = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
        .map(|secs| now_ms.saturating_add(secs.saturating_mul(1000)));
    rate_limits.record_rest_headers(
        resp.headers()
            .iter()
            .filter_map(|(name, val)| val.to_str().ok().map(|v| (name.as_str(), v))),
    );
    if matches!(status, 418 | 429)
        && let Some(ra) = retry_after_ms
    {
        rate_limits.record_retry_after(ra, status == 418);
    }

    let body = resp
        .text()
        .await
        .map_err(|e| GatewayError::Http(e.to_string()))?;

    if (200..300).contains(&status) {
        return Ok(body);
    }

    let (code, msg) =
        serde_json::from_str::<RawRestError>(&body).map_or((None, body), |e| (Some(e.code), e.msg));

    Err(GatewayError::from_exchange_error(
        status,
        code,
        msg,
        retry_after_ms,
        None,
    ))
}

/// Current UTC Unix timestamp in milliseconds.
#[must_use]
pub fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawServerTime {
    server_time: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSpotAccount {
    can_trade: bool,
    update_time: Option<i64>,
    #[serde(default)]
    balances: Vec<RawSpotBalance>,
}

#[derive(Debug, Deserialize)]
struct RawSpotBalance {
    asset: String,
    free: String,
    locked: String,
}

#[derive(Debug, Deserialize)]
struct RawRestError {
    code: i64,
    msg: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawExchangeInfo {
    server_time: i64,
    #[serde(default)]
    rate_limits: Vec<RateLimitRule>,
    #[serde(default)]
    symbols: Vec<RawSymbol>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSymbol {
    symbol: String,
    status: String,
    base_asset_precision: u32,
    quote_asset_precision: u32,
    #[serde(default)]
    filters: Vec<RawFilter>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFilter {
    filter_type: String,
    tick_size: Option<String>,
    step_size: Option<String>,
    min_qty: Option<String>,
    max_qty: Option<String>,
    min_notional: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bot_types::Venue;
    use rust_decimal::Decimal;

    use super::parse_spot_exchange_info;
    use crate::rate_limit::{RateLimitInterval, RateLimitType};

    #[test]
    fn parses_spot_exchange_info_with_notional_and_legacy_fallback() {
        let golden = r#"{
            "timezone": "UTC",
            "serverTime": 1770736694138,
            "rateLimits": [
                { "rateLimitType": "REQUEST_WEIGHT", "interval": "MINUTE", "intervalNum": 1, "limit": 6000 },
                { "rateLimitType": "ORDERS", "interval": "SECOND", "intervalNum": 10, "limit": 100 }
            ],
            "symbols": [
                {
                    "symbol": "BTCUSDT",
                    "status": "TRADING",
                    "baseAssetPrecision": 8,
                    "quoteAssetPrecision": 8,
                    "filters": [
                        { "filterType": "PRICE_FILTER", "minPrice": "0.01", "maxPrice": "1000000.00", "tickSize": "0.01" },
                        { "filterType": "LOT_SIZE", "minQty": "0.00001", "maxQty": "9000.00", "stepSize": "0.00001" },
                        { "filterType": "NOTIONAL", "minNotional": "5.00000000", "applyMinToMarket": true }
                    ]
                },
                {
                    "symbol": "ETHUSDT",
                    "status": "BREAK",
                    "baseAssetPrecision": 8,
                    "quoteAssetPrecision": 8,
                    "filters": [
                        { "filterType": "PRICE_FILTER", "tickSize": "0.01" },
                        { "filterType": "LOT_SIZE", "minQty": "0.0001", "maxQty": "1000.0", "stepSize": "0.0001" },
                        { "filterType": "MIN_NOTIONAL", "minNotional": "10.0" }
                    ]
                }
            ]
        }"#;

        let snap = parse_spot_exchange_info(golden).expect("parse golden exchangeInfo");
        assert_eq!(snap.venue, Venue::Spot);
        assert_eq!(snap.server_time_ms, 1_770_736_694_138);
        assert_eq!(snap.rate_limits.len(), 2);
        assert_eq!(
            snap.rate_limits[0].rate_limit_type,
            RateLimitType::RequestWeight
        );
        assert_eq!(snap.rate_limits[1].interval, RateLimitInterval::Second);

        assert_eq!(snap.rules.len(), 2);
        let btc = &snap.rules[0];
        assert_eq!(btc.symbol.as_str(), "BTCUSDT");
        assert!(btc.status_trading);
        assert_eq!(btc.tick_size, Decimal::from_str("0.01").expect("dec"));
        assert_eq!(btc.step_size, Decimal::from_str("0.00001").expect("dec"));
        assert_eq!(
            btc.min_notional,
            Decimal::from_str("5.00000000").expect("dec")
        );

        let eth = &snap.rules[1];
        assert_eq!(eth.symbol.as_str(), "ETHUSDT");
        assert!(!eth.status_trading);
        assert_eq!(eth.min_notional, Decimal::from_str("10.0").expect("dec"));
    }

    #[tokio::test]
    async fn server_time_records_clock_state_and_ping_succeeds() {
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.expect("accept");
                let mut buf = [0u8; 1024];
                let n = sock.read(&mut buf).await.expect("read");
                let req = String::from_utf8_lossy(&buf[..n]);
                let (status, body) = if req.starts_with("GET /api/v3/time ") {
                    ("200 OK", r#"{"serverTime":1700000000123}"#)
                } else if req.starts_with("GET /api/v3/ping ") {
                    ("200 OK", "{}")
                } else {
                    ("404 Not Found", r#"{"code":-1,"msg":"unexpected path"}"#)
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                sock.write_all(resp.as_bytes()).await.expect("write");
            }
        });

        let client = super::SpotRestClient::new(format!("http://{addr}"), Duration::from_secs(2))
            .expect("client");
        assert_eq!(client.clock_state().offset_ms(), None);

        client.ping().await.expect("ping");
        let sync = client.server_time().await.expect("server_time");
        assert_eq!(sync.server_time_ms, 1_700_000_000_123);
        assert_eq!(client.clock_state().offset_ms(), Some(sync.offset_ms));
        assert_eq!(
            client.clock_state().last_sync_ms(),
            Some(sync.local_time_ms)
        );

        server.await.expect("server join");
    }

    #[tokio::test]
    async fn spot_rest_ingests_rate_limit_headers_into_shared_tracker() {
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        use crate::rate_limit::{RateLimitBucketKey, RateLimitRule, RateLimitTracker};

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await.expect("read");
            let body = "{}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-MBX-USED-WEIGHT-1M: 120\r\nX-MBX-ORDER-COUNT-10S: 45\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).await.expect("write");
        });

        let tracker = Arc::new(RateLimitTracker::new());
        tracker.apply_rules(&[RateLimitRule {
            rate_limit_type: RateLimitType::Orders,
            interval: RateLimitInterval::Second,
            interval_num: 10,
            limit: 50,
        }]);
        let client = super::SpotRestClient::with_rate_limits(
            format!("http://{addr}"),
            Duration::from_secs(2),
            Arc::clone(&tracker),
        )
        .expect("client");

        client.ping().await.expect("ping");
        let snap = client.rate_limits().snapshot();
        let weight = snap
            .get(&RateLimitBucketKey {
                rate_limit_type: RateLimitType::RequestWeight,
                interval: RateLimitInterval::Minute,
                interval_num: 1,
            })
            .expect("weight bucket");
        assert_eq!(weight.count, 120);
        let orders = snap
            .get(&RateLimitBucketKey {
                rate_limit_type: RateLimitType::Orders,
                interval: RateLimitInterval::Second,
                interval_num: 10,
            })
            .expect("orders bucket");
        assert_eq!(orders.count, 45);
        assert_eq!(orders.limit, 50);
        assert!(tracker.can_place_order(1_000, 90).is_err());

        server.await.expect("server join");
    }
}

#[cfg(test)]
mod probe_tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use ed25519_dalek::{Signature, SigningKey, Verifier};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{SpotRestClient, parse_rest_klines, parse_spot_account};
    use crate::auth::{Ed25519Credentials, canonical_payload};

    #[test]
    fn parse_spot_account_extracts_can_trade_and_non_zero_balances() {
        let raw = r#"{"canTrade":false,"canWithdraw":false,"canDeposit":true,"updateTime":123,
            "accountType":"SPOT","permissions":["SPOT"],
            "balances":[{"asset":"BTC","free":"0.5","locked":"0"},
                        {"asset":"ETH","free":"0","locked":"0.00000000"},
                        {"asset":"USDT","free":"0","locked":"12.5"}]}"#;
        let snap = parse_spot_account(raw).unwrap();
        assert!(!snap.can_trade);
        assert_eq!(snap.update_time_ms, 123);
        let assets: Vec<_> = snap.balances.iter().map(|b| b.asset.as_str()).collect();
        assert_eq!(assets, vec!["BTC", "USDT"]);
    }

    #[test]
    fn parse_rest_klines_parses_12_column_array() {
        let raw = r#"[[1000,"1.0","2.0","0.5","1.5","10",1999,"15",7,"4","6","0"],
                      [2000,"1.5","2.5","1.0","2.0","11",2999,"16",8,"5","7","0"]]"#;
        let bars = parse_rest_klines(raw, 2500).unwrap();
        assert_eq!(bars.len(), 2);
        let b0 = bars.first().unwrap();
        assert_eq!(b0.open_time_ms, 1000);
        assert_eq!(b0.close_time_ms, 1999);
        assert!((b0.high - 2.0).abs() < f64::EPSILON);
        assert!((b0.close - 1.5).abs() < f64::EPSILON);
        assert_eq!(b0.trades, 7);
        assert!(b0.is_closed);
        // Last bar still forming at now=2500 (close_time 2999).
        assert!(!bars.get(1).unwrap().is_closed);
        assert!(parse_rest_klines(r#"[[1000,"x"]]"#, 0).is_err());
        assert!(
            parse_rest_klines(
                r#"[[1000,"NaN","2.0","0.5","1.5","10",1999,"15",7,"4","6","0"]]"#,
                2500
            )
            .is_err(),
            "NaN OHLCV must be rejected"
        );
        assert!(
            parse_rest_klines(
                r#"[[1000,"1.0","Infinity","0.5","1.5","10",1999,"15",7,"4","6","0"]]"#,
                2500
            )
            .is_err(),
            "Infinity OHLCV must be rejected"
        );
    }

    #[tokio::test]
    async fn spot_account_snapshot_sends_signed_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(buf.get(..n).unwrap()).to_string();
            let body = r#"{"canTrade":true,"updateTime":5,"balances":[]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            req
        });

        let seed = [7u8; 32];
        let creds = Ed25519Credentials::from_seed("test-api-key".into(), seed).unwrap();
        let client = SpotRestClient::new(format!("http://{addr}"), Duration::from_secs(5)).unwrap();
        let snap = client.account_snapshot(&creds, 5000).await.unwrap();
        assert!(snap.can_trade);

        let req = server.await.unwrap();
        let first = req.lines().next().unwrap();
        assert!(first.starts_with("GET /api/v3/account?"), "{first}");
        assert!(
            req.to_ascii_lowercase()
                .contains("x-mbx-apikey: test-api-key")
        );
        let query = first
            .trim_start_matches("GET /api/v3/account?")
            .split(' ')
            .next()
            .unwrap();
        let mut params = BTreeMap::new();
        let mut sig = String::new();
        for kv in query.split('&') {
            let (k, v) = kv.split_once('=').unwrap();
            if k == "signature" {
                sig = v
                    .replace("%2B", "+")
                    .replace("%2F", "/")
                    .replace("%3D", "=");
            } else {
                params.insert(k.to_owned(), v.to_owned());
            }
        }
        assert_eq!(params.get("recvWindow").map(String::as_str), Some("5000"));
        assert!(params.contains_key("timestamp"));
        let sig_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, sig.as_bytes())
                .unwrap();
        let sig = Signature::from_slice(&sig_bytes).unwrap();
        SigningKey::from_bytes(&seed)
            .verifying_key()
            .verify(canonical_payload(&params).as_bytes(), &sig)
            .unwrap();
    }
}
