//! Binance Spot WS API wire envelopes, request builders, and user-data event parsers.

use std::str::FromStr;

use bot_types::{OrderKind, OrderStatus, Price, Qty, Side, Symbol, Venue};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::error::GatewayError;
use crate::gateway::{
    AccountBalance, AccountEvent, AccountSnapshot, CancelOrderAck, CancelOrderListAck,
    CancelOrderListRequest, CancelOrderRequest, CancelReplaceAck, CancelReplaceRequest, ExecEvent,
    GatewayEvent, ListOrderItem, ListStatusEvent, OcoOrderAck, OrderAck, OrderSnapshot,
    PlaceOcoRequest, PlaceOrderRequest, QueryOrderRequest, validate_client_order_id,
};
use crate::rate_limit::RateLimitReading;

/// Generate a unique UUID v7 correlation ID for every WS API request (§4.1).
#[must_use]
pub fn next_request_id() -> String {
    Uuid::now_v7().to_string()
}

/// Outbound Spot WS API JSON-RPC request frame.
#[derive(Debug, Clone, Serialize)]
pub struct WsRequest {
    /// Unique request ID (UUID v7).
    pub id: String,
    /// Spot WS API method name (e.g. `"session.logon"`, `"order.place"`).
    pub method: String,
    /// Optional method parameters object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// Inbound Spot WS API JSON-RPC response frame.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WsResponse {
    /// Correlation ID matching the outbound [`WsRequest::id`].
    pub id: String,
    /// HTTP-equivalent status code (200, 400, 409, 418, 429, 5xx).
    pub status: u16,
    /// Method result payload when `status == 200`.
    pub result: Option<Value>,
    /// Error payload when `status != 200`.
    pub error: Option<WsErrorBody>,
    /// Optional per-response rate limit readings.
    #[serde(default)]
    pub rate_limits: Vec<RateLimitReading>,
}

/// Error body inside a non-200 [`WsResponse`].
#[derive(Debug, Clone, Deserialize)]
pub struct WsErrorBody {
    /// Binance numeric error code (e.g. `-1003`, `-2010`, `-1021`).
    pub code: i64,
    /// Human-readable error message.
    pub msg: String,
    /// Optional structured error data (contains `retryAfter` on 429/418).
    pub data: Option<Value>,
}

impl WsResponse {
    /// Extract `error.data.retryAfter` (UTC epoch ms) if present.
    #[must_use]
    pub fn retry_after_ms(&self) -> Option<i64> {
        self.error
            .as_ref()?
            .data
            .as_ref()?
            .get("retryAfter")?
            .as_i64()
    }

    /// Convert a non-200 WS API response into a typed [`GatewayError`].
    ///
    /// # Errors
    /// Always returns `Err(GatewayError)` if `status != 200`.
    pub fn into_result(self, client_order_id: Option<String>) -> Result<Value, GatewayError> {
        if self.status == 200 {
            return Ok(self.result.unwrap_or(Value::Null));
        }
        let retry_after = self.retry_after_ms();
        let (code, msg) = match self.error {
            Some(err) => (Some(err.code), err.msg),
            None => (None, format!("WS API status {}", self.status)),
        };
        Err(GatewayError::from_exchange_error(
            self.status,
            code,
            msg,
            retry_after,
            client_order_id,
        ))
    }
}

/// Parsed inbound WS text frame: either a correlated RPC response or an asynchronous push event.
#[derive(Debug, Clone)]
pub enum InboundFrame {
    /// Correlated response to an outbound request.
    Response(WsResponse),
    /// Push event from user data stream or server lifecycle (`serverShutdown`).
    Event {
        /// Optional subscription ID (`{"subscriptionId": 0, "event": ...}`).
        subscription_id: Option<i64>,
        /// Parsed domain event.
        event: GatewayEvent,
    },
}

/// Parse a raw JSON text frame from the Spot WS API connection.
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if the frame is not valid JSON or lacks required fields.
pub fn parse_inbound_frame(text: &str) -> Result<InboundFrame, GatewayError> {
    let val: Value =
        serde_json::from_str(text).map_err(|e| GatewayError::WireFormat(e.to_string()))?;

    // Correlated RPC response always has `id` and `status`.
    if val.get("id").is_some() && val.get("status").is_some() {
        let resp: WsResponse =
            serde_json::from_value(val).map_err(|e| GatewayError::WireFormat(e.to_string()))?;
        return Ok(InboundFrame::Response(resp));
    }

    // Wrapped WS API user-data/lifecycle event: `{"subscriptionId": 0, "event": { "e": ... }}`
    // or bare `{"event": { "e": "serverShutdown", ... }}` or bare `{"e": ...}`.
    let subscription_id = val.get("subscriptionId").and_then(Value::as_i64);
    let event_obj = val.get("event").unwrap_or(&val);
    let event = parse_spot_event(event_obj)?;
    Ok(InboundFrame::Event {
        subscription_id,
        event,
    })
}

/// Parse a Spot event JSON object (`executionReport`, `outboundAccountPosition`,
/// `balanceUpdate`, `listStatus`, `eventStreamTerminated`, `serverShutdown`).
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] on unknown event type or invalid decimal/field types.
pub fn parse_spot_event(event_val: &Value) -> Result<GatewayEvent, GatewayError> {
    let event_type = event_val
        .get("e")
        .and_then(Value::as_str)
        .ok_or_else(|| GatewayError::WireFormat("missing event type field `e`".into()))?;

    match event_type {
        "executionReport" => Ok(GatewayEvent::Exec(parse_execution_report(event_val)?)),
        "outboundAccountPosition" => Ok(GatewayEvent::Account(parse_outbound_account_position(
            event_val,
        )?)),
        "balanceUpdate" => Ok(GatewayEvent::Account(parse_balance_update(event_val)?)),
        "listStatus" => Ok(GatewayEvent::ListStatus(parse_list_status(event_val)?)),
        "eventStreamTerminated" => {
            let event_time_ms = req_i64(event_val, "E")?;
            Ok(GatewayEvent::StreamTerminated {
                venue: Venue::Spot,
                event_time_ms,
            })
        }
        "serverShutdown" => {
            let event_time_ms = req_i64(event_val, "E")?;
            let shutdown_time_ms = event_val.get("T").and_then(Value::as_i64);
            Ok(GatewayEvent::ServerShutdown {
                venue: Venue::Spot,
                event_time_ms,
                shutdown_time_ms,
            })
        }
        other => Err(GatewayError::WireFormat(format!(
            "unrecognized Spot user-data event type `{other}`"
        ))),
    }
}

fn parse_execution_report(v: &Value) -> Result<ExecEvent, GatewayError> {
    let side = parse_side(req_str(v, "S")?)?;
    let kind = parse_order_kind(req_str(v, "o")?)?;
    let status = parse_order_status(req_str(v, "X")?)?;
    let orig_cid = v
        .get("C")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let commission_asset = v
        .get("N")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    Ok(ExecEvent {
        venue: Venue::Spot,
        event_time_ms: req_i64(v, "E")?,
        transact_time_ms: req_i64(v, "T")?,
        symbol: Symbol(req_str(v, "s")?.to_owned()),
        client_order_id: req_str(v, "c")?.to_owned(),
        orig_client_order_id: orig_cid,
        order_id: req_i64(v, "i")?,
        order_list_id: v.get("g").and_then(Value::as_i64).unwrap_or(-1),
        trade_id: v.get("t").and_then(Value::as_i64).unwrap_or(-1),
        side,
        kind,
        exec_type: req_str(v, "x")?.to_owned(),
        status,
        reject_reason: v
            .get("r")
            .and_then(Value::as_str)
            .unwrap_or("NONE")
            .to_owned(),
        order_price: Price(req_dec(v, "p")?),
        order_qty: Qty(req_dec(v, "q")?),
        last_filled_qty: Qty(req_dec(v, "l")?),
        cumulative_filled_qty: Qty(req_dec(v, "z")?),
        last_filled_price: Price(req_dec(v, "L")?),
        cumulative_quote_qty: opt_dec(v, "Z")?.unwrap_or(Decimal::ZERO),
        commission: opt_dec(v, "n")?.unwrap_or(Decimal::ZERO),
        commission_asset,
        is_maker: v.get("m").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn parse_outbound_account_position(v: &Value) -> Result<AccountEvent, GatewayError> {
    let event_time_ms = req_i64(v, "E")?;
    let last_update_ms = req_i64(v, "u")?;
    let raw_balances = v
        .get("B")
        .and_then(Value::as_array)
        .ok_or_else(|| GatewayError::WireFormat("outboundAccountPosition missing `B`".into()))?;

    let mut balances = Vec::with_capacity(raw_balances.len());
    for entry in raw_balances {
        balances.push(AccountBalance {
            asset: req_str(entry, "a")?.to_owned(),
            free: req_dec(entry, "f")?,
            locked: req_dec(entry, "l")?,
        });
    }

    Ok(AccountEvent::PositionUpdate {
        venue: Venue::Spot,
        event_time_ms,
        last_update_ms,
        balances,
    })
}

fn parse_balance_update(v: &Value) -> Result<AccountEvent, GatewayError> {
    Ok(AccountEvent::BalanceDelta {
        venue: Venue::Spot,
        event_time_ms: req_i64(v, "E")?,
        asset: req_str(v, "a")?.to_owned(),
        delta: req_dec(v, "d")?,
        clear_time_ms: req_i64(v, "T")?,
    })
}

fn parse_list_status(v: &Value) -> Result<ListStatusEvent, GatewayError> {
    let raw_orders = v
        .get("O")
        .and_then(Value::as_array)
        .ok_or_else(|| GatewayError::WireFormat("listStatus missing `O`".into()))?;
    let mut orders = Vec::with_capacity(raw_orders.len());
    for o in raw_orders {
        orders.push(ListOrderItem {
            symbol: Symbol(req_str(o, "s")?.to_owned()),
            order_id: req_i64(o, "i")?,
            client_order_id: req_str(o, "c")?.to_owned(),
        });
    }
    Ok(ListStatusEvent {
        venue: Venue::Spot,
        event_time_ms: req_i64(v, "E")?,
        symbol: Symbol(req_str(v, "s")?.to_owned()),
        order_list_id: req_i64(v, "g")?,
        list_client_order_id: req_str(v, "C")?.to_owned(),
        list_status_type: req_str(v, "l")?.to_owned(),
        list_order_status: req_str(v, "L")?.to_owned(),
        reject_reason: v
            .get("r")
            .and_then(Value::as_str)
            .unwrap_or("NONE")
            .to_owned(),
        orders,
    })
}

/// Build JSON params for `order.place`.
///
/// # Errors
/// Returns [`GatewayError::InvalidRequest`] if `clientOrderId` or order parameters are invalid.
pub fn build_place_order_params(
    req: &PlaceOrderRequest,
    timestamp_ms: i64,
    recv_window_ms: Option<u64>,
) -> Result<Value, GatewayError> {
    validate_client_order_id(&req.client_order_id)?;
    let mut m = Map::new();
    m.insert(
        "symbol".into(),
        Value::String(req.symbol.as_str().to_owned()),
    );
    m.insert(
        "side".into(),
        Value::String(side_to_binance(req.side).into()),
    );
    m.insert(
        "type".into(),
        Value::String(order_kind_to_binance(req.kind)?.into()),
    );
    m.insert(
        "newClientOrderId".into(),
        Value::String(req.client_order_id.clone()),
    );
    if let Some(qty) = req.quantity {
        m.insert(
            "quantity".into(),
            Value::String(qty.as_decimal().to_string()),
        );
    }
    if let Some(qoq) = req.quote_order_qty {
        m.insert("quoteOrderQty".into(), Value::String(qoq.to_string()));
    }
    if let Some(price) = req.price {
        m.insert(
            "price".into(),
            Value::String(price.as_decimal().to_string()),
        );
    }
    if let Some(stop) = req.stop_price {
        m.insert(
            "stopPrice".into(),
            Value::String(stop.as_decimal().to_string()),
        );
    }
    if let Some(tif) = req.time_in_force {
        m.insert(
            "timeInForce".into(),
            Value::String(tif.as_binance_str().into()),
        );
    }
    m.insert("newOrderRespType".into(), Value::String("RESULT".into()));
    insert_timing(&mut m, timestamp_ms, recv_window_ms);
    Ok(Value::Object(m))
}

/// Build JSON params for `order.cancel`.
///
/// # Errors
/// Returns [`GatewayError::InvalidRequest`] if neither `orig_client_order_id` nor `order_id` is set.
pub fn build_cancel_order_params(
    req: &CancelOrderRequest,
    timestamp_ms: i64,
    recv_window_ms: Option<u64>,
) -> Result<Value, GatewayError> {
    if req.orig_client_order_id.is_none() && req.order_id.is_none() {
        return Err(GatewayError::InvalidRequest(
            "cancel_order requires orig_client_order_id or order_id".into(),
        ));
    }
    let mut m = Map::new();
    m.insert(
        "symbol".into(),
        Value::String(req.symbol.as_str().to_owned()),
    );
    if let Some(cid) = &req.orig_client_order_id {
        validate_client_order_id(cid)?;
        m.insert("origClientOrderId".into(), Value::String(cid.clone()));
    }
    if let Some(oid) = req.order_id {
        m.insert("orderId".into(), Value::Number(oid.into()));
    }
    if let Some(new_cid) = &req.new_client_order_id {
        validate_client_order_id(new_cid)?;
        m.insert("newClientOrderId".into(), Value::String(new_cid.clone()));
    }
    insert_timing(&mut m, timestamp_ms, recv_window_ms);
    Ok(Value::Object(m))
}

/// Build JSON params for `order.cancelReplace`.
///
/// # Errors
/// Returns [`GatewayError::InvalidRequest`] if identifiers or order kind are invalid.
pub fn build_cancel_replace_params(
    req: &CancelReplaceRequest,
    timestamp_ms: i64,
    recv_window_ms: Option<u64>,
) -> Result<Value, GatewayError> {
    if req.cancel_orig_client_order_id.is_none() && req.cancel_order_id.is_none() {
        return Err(GatewayError::InvalidRequest(
            "cancel_replace requires cancel_orig_client_order_id or cancel_order_id".into(),
        ));
    }
    validate_client_order_id(&req.new_client_order_id)?;
    let mut m = Map::new();
    m.insert(
        "symbol".into(),
        Value::String(req.symbol.as_str().to_owned()),
    );
    m.insert(
        "cancelReplaceMode".into(),
        Value::String(req.cancel_replace_mode.as_binance_str().into()),
    );
    if let Some(cid) = &req.cancel_orig_client_order_id {
        validate_client_order_id(cid)?;
        m.insert("cancelOrigClientOrderId".into(), Value::String(cid.clone()));
    }
    if let Some(oid) = req.cancel_order_id {
        m.insert("cancelOrderId".into(), Value::Number(oid.into()));
    }
    m.insert(
        "side".into(),
        Value::String(side_to_binance(req.side).into()),
    );
    m.insert(
        "type".into(),
        Value::String(order_kind_to_binance(req.kind)?.into()),
    );
    m.insert(
        "newClientOrderId".into(),
        Value::String(req.new_client_order_id.clone()),
    );
    m.insert(
        "quantity".into(),
        Value::String(req.quantity.as_decimal().to_string()),
    );
    if let Some(price) = req.price {
        m.insert(
            "price".into(),
            Value::String(price.as_decimal().to_string()),
        );
    }
    if let Some(tif) = req.time_in_force {
        m.insert(
            "timeInForce".into(),
            Value::String(tif.as_binance_str().into()),
        );
    }
    m.insert("newOrderRespType".into(), Value::String("RESULT".into()));
    insert_timing(&mut m, timestamp_ms, recv_window_ms);
    Ok(Value::Object(m))
}

/// Build JSON params for `orderList.place.oco`.
///
/// # Errors
/// Returns [`GatewayError::InvalidRequest`] if client IDs or order kinds are invalid.
pub fn build_place_oco_params(
    req: &PlaceOcoRequest,
    timestamp_ms: i64,
    recv_window_ms: Option<u64>,
) -> Result<Value, GatewayError> {
    validate_client_order_id(&req.list_client_order_id)?;
    let mut m = Map::new();
    m.insert(
        "symbol".into(),
        Value::String(req.symbol.as_str().to_owned()),
    );
    m.insert(
        "listClientOrderId".into(),
        Value::String(req.list_client_order_id.clone()),
    );
    m.insert(
        "side".into(),
        Value::String(side_to_binance(req.side).into()),
    );
    m.insert(
        "quantity".into(),
        Value::String(req.quantity.as_decimal().to_string()),
    );
    m.insert(
        "aboveType".into(),
        Value::String(order_kind_to_binance(req.above_type)?.into()),
    );
    if let Some(cid) = &req.above_client_order_id {
        validate_client_order_id(cid)?;
        m.insert("aboveClientOrderId".into(), Value::String(cid.clone()));
    }
    if let Some(p) = req.above_price {
        m.insert(
            "abovePrice".into(),
            Value::String(p.as_decimal().to_string()),
        );
    }
    if let Some(sp) = req.above_stop_price {
        m.insert(
            "aboveStopPrice".into(),
            Value::String(sp.as_decimal().to_string()),
        );
    }
    if let Some(tif) = req.above_time_in_force {
        m.insert(
            "aboveTimeInForce".into(),
            Value::String(tif.as_binance_str().into()),
        );
    }
    m.insert(
        "belowType".into(),
        Value::String(order_kind_to_binance(req.below_type)?.into()),
    );
    if let Some(cid) = &req.below_client_order_id {
        validate_client_order_id(cid)?;
        m.insert("belowClientOrderId".into(), Value::String(cid.clone()));
    }
    if let Some(p) = req.below_price {
        m.insert(
            "belowPrice".into(),
            Value::String(p.as_decimal().to_string()),
        );
    }
    if let Some(sp) = req.below_stop_price {
        m.insert(
            "belowStopPrice".into(),
            Value::String(sp.as_decimal().to_string()),
        );
    }
    if let Some(tif) = req.below_time_in_force {
        m.insert(
            "belowTimeInForce".into(),
            Value::String(tif.as_binance_str().into()),
        );
    }
    if let Some(td) = req.below_trailing_delta {
        m.insert("belowTrailingDelta".into(), Value::Number(td.into()));
    }
    m.insert("newOrderRespType".into(), Value::String("RESULT".into()));
    insert_timing(&mut m, timestamp_ms, recv_window_ms);
    Ok(Value::Object(m))
}

/// Build JSON params for `orderList.cancel`.
///
/// # Errors
/// Returns [`GatewayError::InvalidRequest`] if neither `order_list_id` nor `list_client_order_id` is set.
pub fn build_cancel_list_params(
    req: &CancelOrderListRequest,
    timestamp_ms: i64,
    recv_window_ms: Option<u64>,
) -> Result<Value, GatewayError> {
    if req.order_list_id.is_none() && req.list_client_order_id.is_none() {
        return Err(GatewayError::InvalidRequest(
            "cancel_list requires order_list_id or list_client_order_id".into(),
        ));
    }
    let mut m = Map::new();
    m.insert(
        "symbol".into(),
        Value::String(req.symbol.as_str().to_owned()),
    );
    if let Some(lid) = req.order_list_id {
        m.insert("orderListId".into(), Value::Number(lid.into()));
    }
    if let Some(lcid) = &req.list_client_order_id {
        validate_client_order_id(lcid)?;
        m.insert("listClientOrderId".into(), Value::String(lcid.clone()));
    }
    insert_timing(&mut m, timestamp_ms, recv_window_ms);
    Ok(Value::Object(m))
}

/// Build JSON params for `order.status`.
///
/// # Errors
/// Returns [`GatewayError::InvalidRequest`] if neither `orig_client_order_id` nor `order_id` is set.
pub fn build_query_order_params(
    req: &QueryOrderRequest,
    timestamp_ms: i64,
    recv_window_ms: Option<u64>,
) -> Result<Value, GatewayError> {
    if req.orig_client_order_id.is_none() && req.order_id.is_none() {
        return Err(GatewayError::InvalidRequest(
            "query_order requires orig_client_order_id or order_id".into(),
        ));
    }
    let mut m = Map::new();
    m.insert(
        "symbol".into(),
        Value::String(req.symbol.as_str().to_owned()),
    );
    if let Some(cid) = &req.orig_client_order_id {
        validate_client_order_id(cid)?;
        m.insert("origClientOrderId".into(), Value::String(cid.clone()));
    }
    if let Some(oid) = req.order_id {
        m.insert("orderId".into(), Value::Number(oid.into()));
    }
    insert_timing(&mut m, timestamp_ms, recv_window_ms);
    Ok(Value::Object(m))
}

/// Build JSON params for `openOrders.status`.
#[must_use]
pub fn build_open_orders_params(
    symbol: Option<&Symbol>,
    timestamp_ms: i64,
    recv_window_ms: Option<u64>,
) -> Value {
    let mut m = Map::new();
    if let Some(sym) = symbol {
        m.insert("symbol".into(), Value::String(sym.as_str().to_owned()));
    }
    insert_timing(&mut m, timestamp_ms, recv_window_ms);
    Value::Object(m)
}

/// Build JSON params for `account.status`.
#[must_use]
pub fn build_account_status_params(timestamp_ms: i64, recv_window_ms: Option<u64>) -> Value {
    let mut m = Map::new();
    m.insert("omitZeroBalances".into(), Value::Bool(true));
    insert_timing(&mut m, timestamp_ms, recv_window_ms);
    Value::Object(m)
}

/// Parse `order.place` result JSON into [`OrderAck`].
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing or malformed.
pub fn parse_order_ack(v: &Value) -> Result<OrderAck, GatewayError> {
    let status = match v.get("status").and_then(Value::as_str) {
        Some(s) => parse_order_status(s)?,
        None => OrderStatus::New,
    };
    Ok(OrderAck {
        venue: Venue::Spot,
        symbol: Symbol(req_str(v, "symbol")?.to_owned()),
        order_id: req_i64(v, "orderId")?,
        client_order_id: req_str(v, "clientOrderId")?.to_owned(),
        transact_time_ms: v.get("transactTime").and_then(Value::as_i64).unwrap_or(0),
        status,
        executed_qty: Qty(opt_dec(v, "executedQty")?.unwrap_or(Decimal::ZERO)),
        cumulative_quote_qty: opt_dec(v, "cummulativeQuoteQty")?.unwrap_or(Decimal::ZERO),
    })
}

/// Parse `order.cancel` result JSON into [`CancelOrderAck`].
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing or malformed.
pub fn parse_cancel_order_ack(v: &Value) -> Result<CancelOrderAck, GatewayError> {
    Ok(CancelOrderAck {
        venue: Venue::Spot,
        symbol: Symbol(req_str(v, "symbol")?.to_owned()),
        order_id: req_i64(v, "orderId")?,
        orig_client_order_id: req_str(v, "origClientOrderId")?.to_owned(),
        status: parse_order_status(req_str(v, "status")?)?,
        executed_qty: Qty(opt_dec(v, "executedQty")?.unwrap_or(Decimal::ZERO)),
    })
}

/// Parse `order.cancelReplace` result JSON into [`CancelReplaceAck`].
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing or malformed.
pub fn parse_cancel_replace_ack(v: &Value) -> Result<CancelReplaceAck, GatewayError> {
    let cancel_result = req_str(v, "cancelResult")?.to_owned();
    let new_order_result = req_str(v, "newOrderResult")?.to_owned();
    let new_order = if new_order_result == "SUCCESS" {
        match v.get("newOrderResponse") {
            Some(resp) if !resp.is_null() => Some(parse_order_ack(resp)?),
            _ => None,
        }
    } else {
        None
    };
    Ok(CancelReplaceAck {
        venue: Venue::Spot,
        cancel_result,
        new_order_result,
        new_order,
    })
}

/// Parse `orderList.place.oco` result JSON into [`OcoOrderAck`].
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing or malformed.
pub fn parse_oco_ack(v: &Value) -> Result<OcoOrderAck, GatewayError> {
    let raw_orders = v
        .get("orders")
        .and_then(Value::as_array)
        .ok_or_else(|| GatewayError::WireFormat("OCO response missing `orders`".into()))?;
    let mut orders = Vec::with_capacity(raw_orders.len());
    for o in raw_orders {
        orders.push(ListOrderItem {
            symbol: Symbol(req_str(o, "symbol")?.to_owned()),
            order_id: req_i64(o, "orderId")?,
            client_order_id: req_str(o, "clientOrderId")?.to_owned(),
        });
    }
    Ok(OcoOrderAck {
        venue: Venue::Spot,
        order_list_id: req_i64(v, "orderListId")?,
        list_client_order_id: req_str(v, "listClientOrderId")?.to_owned(),
        list_order_status: req_str(v, "listOrderStatus")?.to_owned(),
        symbol: Symbol(req_str(v, "symbol")?.to_owned()),
        transaction_time_ms: v
            .get("transactionTime")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        orders,
    })
}

/// Parse `orderList.cancel` result JSON into [`CancelOrderListAck`].
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing or malformed.
pub fn parse_cancel_list_ack(v: &Value) -> Result<CancelOrderListAck, GatewayError> {
    Ok(CancelOrderListAck {
        venue: Venue::Spot,
        order_list_id: req_i64(v, "orderListId")?,
        list_client_order_id: req_str(v, "listClientOrderId")?.to_owned(),
        list_order_status: req_str(v, "listOrderStatus")?.to_owned(),
        symbol: Symbol(req_str(v, "symbol")?.to_owned()),
    })
}

/// Parse a single order JSON object from `order.status` or `openOrders.status`.
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing or malformed.
pub fn parse_order_snapshot(v: &Value) -> Result<OrderSnapshot, GatewayError> {
    Ok(OrderSnapshot {
        venue: Venue::Spot,
        symbol: Symbol(req_str(v, "symbol")?.to_owned()),
        order_id: req_i64(v, "orderId")?,
        order_list_id: v.get("orderListId").and_then(Value::as_i64).unwrap_or(-1),
        client_order_id: req_str(v, "clientOrderId")?.to_owned(),
        side: parse_side(req_str(v, "side")?)?,
        kind: parse_order_kind(req_str(v, "type")?)?,
        price: Price(opt_dec(v, "price")?.unwrap_or(Decimal::ZERO)),
        orig_qty: Qty(req_dec(v, "origQty")?),
        executed_qty: Qty(opt_dec(v, "executedQty")?.unwrap_or(Decimal::ZERO)),
        cumulative_quote_qty: opt_dec(v, "cummulativeQuoteQty")?.unwrap_or(Decimal::ZERO),
        status: parse_order_status(req_str(v, "status")?)?,
        update_time_ms: v.get("updateTime").and_then(Value::as_i64).unwrap_or(0),
    })
}

/// Parse `account.status` result JSON into [`AccountSnapshot`].
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if required fields are missing or malformed.
pub fn parse_account_snapshot(v: &Value) -> Result<AccountSnapshot, GatewayError> {
    let raw_balances = v
        .get("balances")
        .and_then(Value::as_array)
        .ok_or_else(|| GatewayError::WireFormat("account.status missing `balances`".into()))?;
    let mut balances = Vec::with_capacity(raw_balances.len());
    for b in raw_balances {
        balances.push(AccountBalance {
            asset: req_str(b, "asset")?.to_owned(),
            free: req_dec(b, "free")?,
            locked: req_dec(b, "locked")?,
        });
    }
    Ok(AccountSnapshot {
        venue: Venue::Spot,
        can_trade: v.get("canTrade").and_then(Value::as_bool).unwrap_or(false),
        update_time_ms: v.get("updateTime").and_then(Value::as_i64).unwrap_or(0),
        balances,
    })
}

fn insert_timing(m: &mut Map<String, Value>, timestamp_ms: i64, recv_window_ms: Option<u64>) {
    m.insert("timestamp".into(), Value::Number(timestamp_ms.into()));
    if let Some(rw) = recv_window_ms {
        m.insert("recvWindow".into(), Value::Number(rw.into()));
    }
}

const fn side_to_binance(side: Side) -> &'static str {
    match side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    }
}

fn order_kind_to_binance(kind: OrderKind) -> Result<&'static str, GatewayError> {
    match kind {
        OrderKind::LimitMaker => Ok("LIMIT_MAKER"),
        OrderKind::Limit => Ok("LIMIT"),
        OrderKind::Market => Ok("MARKET"),
        OrderKind::StopLossLimit => Ok("STOP_LOSS_LIMIT"),
        OrderKind::TakeProfitLimit => Ok("TAKE_PROFIT_LIMIT"),
        OrderKind::OcoLeg => Err(GatewayError::InvalidRequest(
            "OrderKind::OcoLeg cannot be sent directly as a Spot wire order type".into(),
        )),
    }
}

fn parse_side(s: &str) -> Result<Side, GatewayError> {
    match s {
        "BUY" => Ok(Side::Buy),
        "SELL" => Ok(Side::Sell),
        other => Err(GatewayError::WireFormat(format!("unknown side `{other}`"))),
    }
}

fn parse_order_kind(s: &str) -> Result<OrderKind, GatewayError> {
    match s {
        "LIMIT_MAKER" => Ok(OrderKind::LimitMaker),
        "LIMIT" => Ok(OrderKind::Limit),
        "MARKET" => Ok(OrderKind::Market),
        "STOP_LOSS_LIMIT" | "STOP_LOSS" => Ok(OrderKind::StopLossLimit),
        "TAKE_PROFIT_LIMIT" | "TAKE_PROFIT" => Ok(OrderKind::TakeProfitLimit),
        other => Err(GatewayError::WireFormat(format!(
            "unknown order type `{other}`"
        ))),
    }
}

fn parse_order_status(s: &str) -> Result<OrderStatus, GatewayError> {
    match s {
        "NEW" => Ok(OrderStatus::New),
        "PARTIALLY_FILLED" => Ok(OrderStatus::PartiallyFilled),
        "FILLED" => Ok(OrderStatus::Filled),
        "CANCELED" | "PENDING_CANCEL" => Ok(OrderStatus::Canceled),
        "REJECTED" => Ok(OrderStatus::Rejected),
        "EXPIRED" | "EXPIRED_IN_MATCH" => Ok(OrderStatus::Expired),
        other => Err(GatewayError::WireFormat(format!(
            "unknown order status `{other}`"
        ))),
    }
}

fn req_str<'a>(v: &'a Value, field: &str) -> Result<&'a str, GatewayError> {
    v.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| GatewayError::WireFormat(format!("missing string field `{field}`")))
}

fn req_i64(v: &Value, field: &str) -> Result<i64, GatewayError> {
    v.get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| GatewayError::WireFormat(format!("missing i64 field `{field}`")))
}

fn req_dec(v: &Value, field: &str) -> Result<Decimal, GatewayError> {
    let raw = req_str(v, field)?;
    Decimal::from_str(raw)
        .map_err(|_| GatewayError::WireFormat(format!("invalid decimal `{raw}` in `{field}`")))
}

fn opt_dec(v: &Value, field: &str) -> Result<Option<Decimal>, GatewayError> {
    match v.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Decimal::from_str(s)
            .map(Some)
            .map_err(|_| GatewayError::WireFormat(format!("invalid decimal `{s}` in `{field}`"))),
        Some(_) => Err(GatewayError::WireFormat(format!(
            "expected decimal string in `{field}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bot_types::{OrderKind, OrderStatus, Side, Venue};
    use rust_decimal::Decimal;

    use super::{InboundFrame, parse_inbound_frame};
    use crate::gateway::{AccountEvent, GatewayEvent};

    #[test]
    fn parses_wrapped_execution_report_event() {
        let raw = r#"{
            "subscriptionId": 0,
            "event": {
                "e": "executionReport",
                "E": 1770736694200,
                "s": "BTCUSDT",
                "c": "b01r1S-000001-ENT",
                "S": "BUY",
                "o": "LIMIT_MAKER",
                "f": "GTC",
                "q": "0.01000000",
                "p": "65000.00",
                "P": "0.00",
                "g": -1,
                "C": "",
                "x": "TRADE",
                "X": "FILLED",
                "r": "NONE",
                "i": 987654321,
                "l": "0.01000000",
                "z": "0.01000000",
                "L": "65000.00",
                "n": "0.65000000",
                "N": "USDT",
                "T": 1770736694199,
                "t": 123456,
                "m": true,
                "Z": "650.00000000"
            }
        }"#;

        let frame = parse_inbound_frame(raw).expect("parse executionReport");
        match frame {
            InboundFrame::Event {
                subscription_id: Some(0),
                event: GatewayEvent::Exec(ev),
            } => {
                assert_eq!(ev.venue, Venue::Spot);
                assert_eq!(ev.symbol.as_str(), "BTCUSDT");
                assert_eq!(ev.client_order_id, "b01r1S-000001-ENT");
                assert_eq!(ev.order_id, 987_654_321);
                assert_eq!(ev.trade_id, 123_456);
                assert_eq!(ev.side, Side::Buy);
                assert_eq!(ev.kind, OrderKind::LimitMaker);
                assert_eq!(ev.status, OrderStatus::Filled);
                assert_eq!(ev.commission, Decimal::from_str("0.65000000").expect("dec"));
                assert_eq!(ev.commission_asset.as_deref(), Some("USDT"));
                assert!(ev.is_maker);
            }
            other => panic!("unexpected frame: {other:?}"),
        }
    }

    #[test]
    fn parses_outbound_account_position_and_server_shutdown() {
        let acct = r#"{
            "subscriptionId": 0,
            "event": {
                "e": "outboundAccountPosition",
                "E": 1770736695000,
                "u": 1770736694999,
                "B": [
                    { "a": "USDT", "f": "1000.50", "l": "250.00" }
                ]
            }
        }"#;
        match parse_inbound_frame(acct).expect("parse acct") {
            InboundFrame::Event {
                event:
                    GatewayEvent::Account(AccountEvent::PositionUpdate {
                        venue: Venue::Spot,
                        balances,
                        ..
                    }),
                ..
            } => {
                assert_eq!(balances.len(), 1);
                assert_eq!(balances[0].asset, "USDT");
                assert_eq!(balances[0].free, Decimal::from_str("1000.50").expect("dec"));
            }
            other => panic!("unexpected: {other:?}"),
        }

        let shutdown = r#"{"event":{"e":"serverShutdown","E":1770736700000,"T":1770737300000}}"#;
        match parse_inbound_frame(shutdown).expect("parse shutdown") {
            InboundFrame::Event {
                event:
                    GatewayEvent::ServerShutdown {
                        venue: Venue::Spot,
                        event_time_ms: 1_770_736_700_000,
                        shutdown_time_ms: Some(1_770_737_300_000),
                    },
                ..
            } => {}
            other => panic!("unexpected: {other:?}"),
        }
    }
}
