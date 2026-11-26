//! USDⓈ-M Futures REST client (`/fapi/v1` & `/fapi/v2`) for bootstrap metadata,
//! pre-activation account safety checks, and fallback execution (§4.5).

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bot_types::{OrderKind, OrderStatus, Price, Qty, Side, Symbol, Venue};
use reqwest::{Client, Method, Response};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::auth::{Ed25519Credentials, canonical_payload};
use crate::clock::ClockSyncState;
use crate::error::GatewayError;
use crate::futures::adapter::{FuturesAccountSetup, FuturesMarginType};
use crate::futures::wire::{
    build_cancel_algo_order_params, build_place_algo_order_params, parse_futures_exchange_info,
};
use crate::gateway::{
    AccountBalance, AccountSnapshot, AlgoOrderAck, CancelAlgoOrderAck, CancelAlgoOrderRequest,
    CancelOrderAck, CancelOrderRequest, ExchangeInfoSnapshot, OrderAck, OrderSnapshot,
    PlaceAlgoOrderRequest, PlaceOrderRequest, QueryOrderRequest,
};
use crate::rate_limit::RateLimitTracker;
use crate::spot::ServerTimeSync;

/// USDⓈ-M Futures REST client (`/fapi`) with Ed25519 request signing and isolated rate-limit tracking.
#[derive(Clone)]
pub struct FuturesRestClient {
    base_url: String,
    http: Client,
    creds: Option<Arc<Ed25519Credentials>>,
    rate_limits: Arc<RateLimitTracker>,
    clock: ClockSyncState,
}

impl std::fmt::Debug for FuturesRestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FuturesRestClient")
            .field("base_url", &self.base_url)
            .field("has_creds", &self.creds.is_some())
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

impl FuturesRestClient {
    /// Production USDⓈ-M Futures REST base URL.
    pub const PRODUCTION_URL: &'static str = "https://fapi.binance.com";
    /// Testnet USDⓈ-M Futures REST base URL.
    pub const TESTNET_URL: &'static str = "https://testnet.binancefuture.com";

    /// Create a new Futures REST client targeting `base_url`.
    ///
    /// # Errors
    /// Returns [`GatewayError::Http`] if the HTTP client fails to build.
    pub fn new(
        base_url: impl Into<String>,
        timeout: Duration,
        creds: Option<Arc<Ed25519Credentials>>,
        rate_limits: Arc<RateLimitTracker>,
    ) -> Result<Self, GatewayError> {
        let http = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| GatewayError::Http(e.to_string()))?;
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            http,
            creds,
            rate_limits,
            clock: ClockSyncState::new(),
        })
    }

    /// Return the shared clock synchronization state updated by [`Self::server_time`].
    #[must_use]
    pub const fn clock_state(&self) -> &ClockSyncState {
        &self.clock
    }

    /// Query `GET /fapi/v1/ping` to test USDⓈ-M Futures REST connectivity.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on HTTP failure, rate limit (429/418), or non-success status.
    pub async fn ping(&self) -> Result<(), GatewayError> {
        let url = format!("{}/fapi/v1/ping", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| GatewayError::Http(e.to_string()))?;
        let _body = self.check_response(resp).await?;
        Ok(())
    }

    /// Query `GET /fapi/v1/time` and record the measured local-to-server clock offset.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on HTTP failure, rate limit (429/418), or malformed JSON.
    pub async fn server_time(&self) -> Result<ServerTimeSync, GatewayError> {
        let url = format!("{}/fapi/v1/time", self.base_url);
        let t0 = now_unix_ms();
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| GatewayError::Http(e.to_string()))?;
        let t1 = now_unix_ms();
        let local_mid = t0.saturating_add(t1.saturating_sub(t0) / 2);

        let body = self.check_response(resp).await?;
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

    /// Query `GET /fapi/v1/exchangeInfo` and parse symbol rules + rate limits.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on HTTP failure, rate limit, or malformed payload.
    pub async fn exchange_info(&self) -> Result<ExchangeInfoSnapshot, GatewayError> {
        let url = format!("{}/fapi/v1/exchangeInfo", self.base_url);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| GatewayError::Http(e.to_string()))?;
        let body = self.check_response(resp).await?;
        parse_futures_exchange_info(&body)
    }

    /// Fetch live pre-activation Futures account configuration from REST-only endpoints:
    /// `/fapi/v1/positionSide/dual`, `/fapi/v1/multiAssetsMargin`, and `/fapi/v2/positionRisk`.
    ///
    /// # Errors
    /// Returns [`GatewayError`] if credentials are missing or any REST query fails.
    pub async fn fetch_account_setup(&self) -> Result<FuturesAccountSetup, GatewayError> {
        let dual_body = self
            .signed_request(Method::GET, "/fapi/v1/positionSide/dual", BTreeMap::new())
            .await?;
        let dual: RawDualSide = serde_json::from_str(&dual_body)
            .map_err(|e| GatewayError::WireFormat(format!("positionSide/dual: {e}")))?;

        let multi_body = self
            .signed_request(Method::GET, "/fapi/v1/multiAssetsMargin", BTreeMap::new())
            .await?;
        let multi: RawMultiAssets = serde_json::from_str(&multi_body)
            .map_err(|e| GatewayError::WireFormat(format!("multiAssetsMargin: {e}")))?;

        let risk_body = self
            .signed_request(Method::GET, "/fapi/v2/positionRisk", BTreeMap::new())
            .await?;
        let positions: Vec<RawPositionRisk> = serde_json::from_str(&risk_body)
            .map_err(|e| GatewayError::WireFormat(format!("positionRisk: {e}")))?;

        let mut has_open_positions = false;
        let mut margin_type = FuturesMarginType::Isolated;
        let mut max_leverage: u32 = 1;

        for pos in &positions {
            let amt = Decimal::from_str(&pos.position_amt).unwrap_or(Decimal::ZERO);
            if !amt.is_zero() {
                has_open_positions = true;
            }
            if !pos.margin_type.eq_ignore_ascii_case("isolated") {
                margin_type = FuturesMarginType::Crossed;
            }
            if let Ok(lev) = pos.leverage.parse::<u32>() {
                max_leverage = max_leverage.max(lev);
            }
        }

        Ok(FuturesAccountSetup {
            dual_side_position: dual.dual_side_position,
            multi_assets_margin: multi.multi_assets_margin,
            margin_type,
            leverage: max_leverage,
            has_open_positions,
        })
    }

    /// Fallback `POST /fapi/v1/order`.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on signing, HTTP, or API error.
    pub async fn place_order(&self, req: &PlaceOrderRequest) -> Result<OrderAck, GatewayError> {
        let params = build_place_order_params(req);
        let body = self
            .signed_request(Method::POST, "/fapi/v1/order", params)
            .await?;
        parse_order_ack(&body)
    }

    /// Fallback `DELETE /fapi/v1/order`.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on signing, HTTP, or API error.
    pub async fn cancel_order(
        &self,
        req: &CancelOrderRequest,
    ) -> Result<CancelOrderAck, GatewayError> {
        let mut params = BTreeMap::new();
        params.insert("symbol".to_owned(), req.symbol.as_str().to_owned());
        if let Some(id) = req.order_id {
            params.insert("orderId".to_owned(), id.to_string());
        }
        if let Some(cid) = &req.orig_client_order_id {
            params.insert("origClientOrderId".to_owned(), cid.clone());
        }
        let body = self
            .signed_request(Method::DELETE, "/fapi/v1/order", params)
            .await?;
        parse_cancel_ack(&body)
    }

    /// Fallback `GET /fapi/v1/order`.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on signing, HTTP, or API error.
    pub async fn query_order(
        &self,
        req: &QueryOrderRequest,
    ) -> Result<OrderSnapshot, GatewayError> {
        let mut params = BTreeMap::new();
        params.insert("symbol".to_owned(), req.symbol.as_str().to_owned());
        if let Some(id) = req.order_id {
            params.insert("orderId".to_owned(), id.to_string());
        }
        if let Some(cid) = &req.orig_client_order_id {
            params.insert("origClientOrderId".to_owned(), cid.clone());
        }
        let body = self
            .signed_request(Method::GET, "/fapi/v1/order", params)
            .await?;
        parse_order_snapshot(&body)
    }

    /// Fallback `GET /fapi/v1/openOrders`.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on signing, HTTP, or API error.
    pub async fn open_orders(
        &self,
        symbol: Option<&Symbol>,
    ) -> Result<Vec<OrderSnapshot>, GatewayError> {
        let mut params = BTreeMap::new();
        if let Some(sym) = symbol {
            params.insert("symbol".to_owned(), sym.as_str().to_owned());
        }
        let body = self
            .signed_request(Method::GET, "/fapi/v1/openOrders", params)
            .await?;
        let raw: Vec<serde_json::Value> =
            serde_json::from_str(&body).map_err(|e| GatewayError::WireFormat(e.to_string()))?;
        raw.into_iter()
            .map(|v| parse_order_snapshot(&v.to_string()))
            .collect()
    }

    /// Fallback `GET /fapi/v2/account`.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on signing, HTTP, or API error.
    pub async fn account_snapshot(&self) -> Result<AccountSnapshot, GatewayError> {
        let body = self
            .signed_request(Method::GET, "/fapi/v2/account", BTreeMap::new())
            .await?;
        parse_account_snapshot(&body)
    }

    /// Fallback `POST /fapi/v1/algoOrder`.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on signing, HTTP, or API error.
    pub async fn place_algo_order(
        &self,
        req: &PlaceAlgoOrderRequest,
    ) -> Result<AlgoOrderAck, GatewayError> {
        let params = json_map_to_btree(build_place_algo_order_params(req)?);
        let body = self
            .signed_request(Method::POST, "/fapi/v1/algoOrder", params)
            .await?;
        parse_algo_order_ack(&body)
    }

    /// Fallback `DELETE /fapi/v1/algoOrder`.
    ///
    /// # Errors
    /// Returns [`GatewayError`] on signing, HTTP, or API error.
    pub async fn cancel_algo_order(
        &self,
        req: &CancelAlgoOrderRequest,
    ) -> Result<CancelAlgoOrderAck, GatewayError> {
        let params = json_map_to_btree(build_cancel_algo_order_params(req)?);
        let body = self
            .signed_request(Method::DELETE, "/fapi/v1/algoOrder", params)
            .await?;
        parse_cancel_algo_ack(&body, &req.symbol)
    }

    async fn signed_request(
        &self,
        method: Method,
        path: &str,
        mut params: BTreeMap<String, String>,
    ) -> Result<String, GatewayError> {
        let creds = self.creds.as_ref().ok_or_else(|| {
            GatewayError::KeyLoad(
                "FuturesRestClient requires Ed25519Credentials for signed endpoints".into(),
            )
        })?;
        params.insert("timestamp".to_owned(), now_unix_ms().to_string());
        let sig = creds.sign_params(&params);
        let query = format!("{}&signature={sig}", canonical_payload(&params));
        let url = format!("{}{path}?{query}", self.base_url);

        let resp = self
            .http
            .request(method, &url)
            .header("X-MBX-APIKEY", creds.api_key())
            .send()
            .await
            .map_err(|e| GatewayError::Http(e.to_string()))?;
        self.check_response(resp).await
    }

    async fn check_response(&self, resp: Response) -> Result<String, GatewayError> {
        let status = resp.status();
        let now = now_unix_ms();
        let retry_after_ms = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .map(|secs| now.saturating_add(secs.saturating_mul(1000)));
        if let Some(weight) = resp
            .headers()
            .get("x-mbx-used-weight-1m")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
        {
            self.rate_limits
                .record_readings(&[crate::rate_limit::RateLimitReading {
                    rate_limit_type: crate::rate_limit::RateLimitType::RequestWeight,
                    interval: crate::rate_limit::RateLimitInterval::Minute,
                    interval_num: 1,
                    limit: 2400,
                    count: weight,
                }]);
        }
        let body = resp
            .text()
            .await
            .map_err(|e| GatewayError::Http(e.to_string()))?;

        if status.is_success() {
            return Ok(body);
        }
        let (code, msg) = serde_json::from_str::<RawBinanceError>(&body)
            .map_or((None, body), |e| (Some(e.code), e.msg));
        Err(GatewayError::from_exchange_error(
            status.as_u16(),
            code,
            msg,
            retry_after_ms,
            None,
        ))
    }
}

/// Convert a JSON object map into a string `BTreeMap` for canonical signing.
#[must_use]
pub fn json_map_to_btree(
    map: serde_json::Map<String, serde_json::Value>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in map {
        let s = match v {
            serde_json::Value::String(s) => s,
            other => other.to_string(),
        };
        out.insert(k, s);
    }
    out
}

/// Build signed query/WS params for a standard Futures order.
#[must_use]
pub fn build_place_order_params(req: &PlaceOrderRequest) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    params.insert("symbol".to_owned(), req.symbol.as_str().to_owned());
    params.insert(
        "side".to_owned(),
        match req.side {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        }
        .to_owned(),
    );
    let (kind_str, tif) = match req.kind {
        OrderKind::LimitMaker => ("LIMIT", Some("GTX")),
        OrderKind::Limit | OrderKind::OcoLeg => ("LIMIT", Some("GTC")),
        OrderKind::Market => ("MARKET", None),
        OrderKind::StopLossLimit => ("STOP", Some("GTC")),
        OrderKind::TakeProfitLimit => ("TAKE_PROFIT", Some("GTC")),
    };
    params.insert("type".to_owned(), kind_str.to_owned());
    if let Some(tif_val) = tif {
        params.insert("timeInForce".to_owned(), tif_val.to_owned());
    }
    if let Some(qty) = req.quantity {
        params.insert("quantity".to_owned(), qty.0.to_string());
    }
    if let Some(price) = req.price {
        params.insert("price".to_owned(), price.0.to_string());
    }
    if let Some(stop_price) = req.stop_price {
        params.insert("stopPrice".to_owned(), stop_price.0.to_string());
    }
    params.insert("newClientOrderId".to_owned(), req.client_order_id.clone());
    params
}

/// Parse a Futures order acknowledgement JSON payload.
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing.
pub fn parse_order_ack(raw: &str) -> Result<OrderAck, GatewayError> {
    let v: RawOrderAck =
        serde_json::from_str(raw).map_err(|e| GatewayError::WireFormat(e.to_string()))?;
    Ok(OrderAck {
        venue: Venue::UsdsFutures,
        symbol: Symbol(v.symbol),
        order_id: v.order_id,
        client_order_id: v.client_order_id,
        status: parse_order_status(&v.status),
        transact_time_ms: v.update_time.unwrap_or_else(now_unix_ms),
        executed_qty: Qty(v
            .executed_qty
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(Decimal::ZERO)),
        cumulative_quote_qty: v
            .cum_quote
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(Decimal::ZERO),
    })
}

/// Parse a Futures cancel acknowledgement JSON payload.
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing.
pub fn parse_cancel_ack(raw: &str) -> Result<CancelOrderAck, GatewayError> {
    let v: RawOrderAck =
        serde_json::from_str(raw).map_err(|e| GatewayError::WireFormat(e.to_string()))?;
    Ok(CancelOrderAck {
        venue: Venue::UsdsFutures,
        symbol: Symbol(v.symbol),
        order_id: v.order_id,
        orig_client_order_id: v.client_order_id,
        status: parse_order_status(&v.status),
        executed_qty: Qty(v
            .executed_qty
            .as_deref()
            .and_then(|s| Decimal::from_str(s).ok())
            .unwrap_or(Decimal::ZERO)),
    })
}

/// Parse a Futures order snapshot JSON payload.
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing.
pub fn parse_order_snapshot(raw: &str) -> Result<OrderSnapshot, GatewayError> {
    let v: RawOrderSnapshot =
        serde_json::from_str(raw).map_err(|e| GatewayError::WireFormat(e.to_string()))?;
    Ok(OrderSnapshot {
        venue: Venue::UsdsFutures,
        symbol: Symbol(v.symbol),
        order_id: v.order_id,
        order_list_id: -1,
        client_order_id: v.client_order_id,
        side: if v.side.eq_ignore_ascii_case("SELL") {
            Side::Sell
        } else {
            Side::Buy
        },
        kind: OrderKind::Limit,
        status: parse_order_status(&v.status),
        price: Price(Decimal::from_str(&v.price).unwrap_or(Decimal::ZERO)),
        orig_qty: Qty(Decimal::from_str(&v.orig_qty).unwrap_or(Decimal::ZERO)),
        executed_qty: Qty(Decimal::from_str(&v.executed_qty).unwrap_or(Decimal::ZERO)),
        cumulative_quote_qty: Decimal::ZERO,
        update_time_ms: v.update_time.unwrap_or_else(now_unix_ms),
    })
}

/// Parse a Futures `/fapi/v2/account` snapshot JSON payload.
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if malformed.
pub fn parse_account_snapshot(raw: &str) -> Result<AccountSnapshot, GatewayError> {
    let v: RawAccount =
        serde_json::from_str(raw).map_err(|e| GatewayError::WireFormat(e.to_string()))?;
    let balances = v
        .assets
        .into_iter()
        .map(|a| AccountBalance {
            asset: a.asset,
            free: Decimal::from_str(&a.available_balance).unwrap_or(Decimal::ZERO),
            locked: Decimal::from_str(&a.initial_margin).unwrap_or(Decimal::ZERO),
        })
        .collect();
    Ok(AccountSnapshot {
        venue: Venue::UsdsFutures,
        can_trade: v.can_trade,
        update_time_ms: v.update_time.unwrap_or_else(now_unix_ms),
        balances,
    })
}

/// Parse a Futures Algo Order placement ack JSON payload.
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing.
pub fn parse_algo_order_ack(raw: &str) -> Result<AlgoOrderAck, GatewayError> {
    let v: RawAlgoAck =
        serde_json::from_str(raw).map_err(|e| GatewayError::WireFormat(e.to_string()))?;
    Ok(AlgoOrderAck {
        venue: Venue::UsdsFutures,
        symbol: Symbol(v.symbol),
        algo_id: v.algo_id,
        client_algo_id: v.client_algo_id,
        status: v.algo_status,
        transact_time_ms: v.create_time.unwrap_or_else(now_unix_ms),
    })
}

/// Parse a Futures Algo Order cancel ack JSON payload.
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing.
pub fn parse_cancel_algo_ack(
    raw: &str,
    fallback_symbol: &Symbol,
) -> Result<CancelAlgoOrderAck, GatewayError> {
    let v: RawCancelAlgoAck =
        serde_json::from_str(raw).map_err(|e| GatewayError::WireFormat(e.to_string()))?;
    Ok(CancelAlgoOrderAck {
        venue: Venue::UsdsFutures,
        symbol: v.symbol.map_or_else(|| fallback_symbol.clone(), Symbol),
        algo_id: v.algo_id,
        client_algo_id: v.client_algo_id,
    })
}

fn parse_order_status(raw: &str) -> OrderStatus {
    match raw {
        "NEW" => OrderStatus::New,
        "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
        "FILLED" => OrderStatus::Filled,
        "CANCELED" => OrderStatus::Canceled,
        "REJECTED" => OrderStatus::Rejected,
        "EXPIRED" | "EXPIRED_IN_MATCH" => OrderStatus::Expired,
        _ => OrderStatus::UnknownNeedsQuery,
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[derive(Deserialize)]
struct RawBinanceError {
    code: i64,
    msg: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDualSide {
    dual_side_position: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMultiAssets {
    multi_assets_margin: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPositionRisk {
    position_amt: String,
    margin_type: String,
    leverage: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOrderAck {
    symbol: String,
    order_id: i64,
    client_order_id: String,
    status: String,
    update_time: Option<i64>,
    executed_qty: Option<String>,
    cum_quote: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOrderSnapshot {
    symbol: String,
    order_id: i64,
    client_order_id: String,
    side: String,
    status: String,
    price: String,
    orig_qty: String,
    executed_qty: String,
    update_time: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAccount {
    can_trade: bool,
    update_time: Option<i64>,
    #[serde(default)]
    assets: Vec<RawAccountAsset>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAccountAsset {
    asset: String,
    available_balance: String,
    initial_margin: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAlgoAck {
    symbol: String,
    algo_id: i64,
    client_algo_id: String,
    algo_status: String,
    create_time: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawServerTime {
    server_time: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCancelAlgoAck {
    symbol: Option<String>,
    algo_id: i64,
    client_algo_id: String,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::FuturesRestClient;
    use crate::rate_limit::RateLimitTracker;

    #[tokio::test]
    async fn futures_server_time_records_clock_state_and_ping_succeeds() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.expect("accept");
                let mut buf = [0u8; 1024];
                let n = sock.read(&mut buf).await.expect("read");
                let req = String::from_utf8_lossy(&buf[..n]);
                let (status, body) = if req.starts_with("GET /fapi/v1/time ") {
                    ("200 OK", r#"{"serverTime":1700000000456}"#)
                } else if req.starts_with("GET /fapi/v1/ping ") {
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

        let client = FuturesRestClient::new(
            format!("http://{addr}"),
            Duration::from_secs(2),
            None,
            Arc::new(RateLimitTracker::new()),
        )
        .expect("client");
        assert_eq!(client.clock_state().offset_ms(), None);

        client.ping().await.expect("ping");
        let sync = client.server_time().await.expect("server_time");
        assert_eq!(sync.server_time_ms, 1_700_000_000_456);
        assert_eq!(client.clock_state().offset_ms(), Some(sync.offset_ms));
        assert_eq!(
            client.clock_state().last_sync_ms(),
            Some(sync.local_time_ms)
        );

        server.await.expect("server join");
    }
}
