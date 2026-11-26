//! USDⓈ-M Futures wire parsers: `exchangeInfo` (filter `notional`), User Data events
//! (`ORDER_TRADE_UPDATE`, `ACCOUNT_UPDATE`, `ALGO_UPDATE`, `MARGIN_CALL`,
//! `ACCOUNT_CONFIG_UPDATE`, `listenKeyExpired`), and Algo order request builders (§4.5).

use std::str::FromStr;

use bot_types::{InstrumentRules, OrderKind, OrderStatus, Price, Qty, Side, Symbol, Venue};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::GatewayError;
use crate::gateway::{
    AccountBalance, AccountEvent, CancelAlgoOrderRequest, ExchangeInfoSnapshot, ExecEvent,
    GatewayEvent, PlaceAlgoOrderRequest, validate_client_order_id,
};

/// Parsed USDⓈ-M Futures User Data Stream events (`FUTURES_WSS.yaml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FuturesUserEvent {
    /// `ORDER_TRADE_UPDATE`: order/fill lifecycle event.
    OrderTradeUpdate {
        /// Venue (`Venue::UsdsFutures`).
        venue: Venue,
        /// Event time (UTC ms).
        event_time_ms: i64,
        /// Symbol.
        symbol: Symbol,
        /// Client order ID.
        client_order_id: String,
        /// Exchange order ID.
        order_id: i64,
        /// Execution type (`NEW`, `TRADE`, `CANCELED`, `EXPIRED`, etc.).
        execution_type: String,
        /// Order status (`NEW`, `PARTIALLY_FILLED`, `FILLED`, `CANCELED`, etc.).
        order_status: String,
        /// Side.
        side: Side,
        /// Last filled quantity.
        last_filled_qty: Qty,
        /// Cumulative filled quantity.
        cum_filled_qty: Qty,
        /// Last filled price.
        last_filled_price: Price,
        /// Realized `PnL`.
        realized_pnl: Decimal,
    },
    /// `ACCOUNT_UPDATE`: balance and position update.
    AccountUpdate {
        /// Venue (`Venue::UsdsFutures`).
        venue: Venue,
        /// Event time (UTC ms).
        event_time_ms: i64,
        /// Reason (`ORDER`, `FUNDING_FEE`, `DEPOSIT`, etc.).
        reason: String,
    },
    /// `ALGO_UPDATE`: conditional Algo order status update.
    AlgoUpdate {
        /// Venue (`Venue::UsdsFutures`).
        venue: Venue,
        /// Event time (UTC ms).
        event_time_ms: i64,
        /// Symbol.
        symbol: Symbol,
        /// Exchange algo ID.
        algo_id: i64,
        /// Client algo ID.
        client_algo_id: String,
        /// Algo status (`NEW`, `TRIGGERED`, `CANCELED`, `EXPIRED`, `FINISHED`).
        algo_status: String,
    },
    /// `MARGIN_CALL`: margin call warning for isolated/cross positions.
    MarginCall {
        /// Venue (`Venue::UsdsFutures`).
        venue: Venue,
        /// Event time (UTC ms).
        event_time_ms: i64,
        /// Number of positions in margin call.
        position_count: usize,
    },
    /// `ACCOUNT_CONFIG_UPDATE`: leverage or multi-assets config change.
    AccountConfigUpdate {
        /// Venue (`Venue::UsdsFutures`).
        venue: Venue,
        /// Event time (UTC ms).
        event_time_ms: i64,
    },
    /// `listenKeyExpired`: User Data Stream listenKey expired.
    ListenKeyExpired {
        /// Venue (`Venue::UsdsFutures`).
        venue: Venue,
        /// Event time (UTC ms).
        event_time_ms: i64,
    },
}

fn parse_dec(v: &Value, field: &str) -> Result<Decimal, GatewayError> {
    let s = v.as_str().ok_or_else(|| {
        GatewayError::InvalidRequest(format!("missing decimal string field '{field}'"))
    })?;
    Decimal::from_str(s)
        .map_err(|e| GatewayError::InvalidRequest(format!("invalid decimal '{field}': {e}")))
}

fn parse_side(s: &str) -> Result<Side, GatewayError> {
    match s {
        "BUY" => Ok(Side::Buy),
        "SELL" => Ok(Side::Sell),
        other => Err(GatewayError::InvalidRequest(format!(
            "unknown side '{other}'"
        ))),
    }
}

/// Parse `GET /fapi/v1/exchangeInfo` JSON into a venue-agnostic [`ExchangeInfoSnapshot`].
///
/// Unlike Spot (`minNotional`), USDⓈ-M Futures uses the `notional` field in `MIN_NOTIONAL`.
///
/// # Errors
/// Returns [`GatewayError::InvalidRequest`] if required fields are missing or malformed.
pub fn parse_futures_exchange_info(raw: &str) -> Result<ExchangeInfoSnapshot, GatewayError> {
    let root: Value = serde_json::from_str(raw)
        .map_err(|e| GatewayError::InvalidRequest(format!("invalid exchangeInfo JSON: {e}")))?;
    let server_time_ms = root.get("serverTime").and_then(Value::as_i64).unwrap_or(0);
    let symbols_arr = root
        .get("symbols")
        .and_then(Value::as_array)
        .ok_or_else(|| GatewayError::InvalidRequest("missing 'symbols' array".into()))?;

    let mut rules = Vec::with_capacity(symbols_arr.len());
    for sym in symbols_arr {
        let symbol_str = sym
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| GatewayError::InvalidRequest("symbol missing 'symbol' field".into()))?;
        let status = sym.get("status").and_then(Value::as_str).unwrap_or("BREAK");
        let base_precision = sym
            .get("quantityPrecision")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(8);
        let quote_precision = sym
            .get("pricePrecision")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(8);

        let mut tick_size = Decimal::ZERO;
        let mut step_size = Decimal::ZERO;
        let mut min_qty = Decimal::ZERO;
        let mut max_qty = Decimal::ZERO;
        let mut min_notional = Decimal::ZERO;

        if let Some(filters) = sym.get("filters").and_then(Value::as_array) {
            for f in filters {
                match f.get("filterType").and_then(Value::as_str) {
                    Some("PRICE_FILTER") => {
                        if let Some(v) = f.get("tickSize") {
                            tick_size = parse_dec(v, "tickSize")?;
                        }
                    }
                    Some("LOT_SIZE") => {
                        if let Some(v) = f.get("stepSize") {
                            step_size = parse_dec(v, "stepSize")?;
                        }
                        if let Some(v) = f.get("minQty") {
                            min_qty = parse_dec(v, "minQty")?;
                        }
                        if let Some(v) = f.get("maxQty") {
                            max_qty = parse_dec(v, "maxQty")?;
                        }
                    }
                    Some("MIN_NOTIONAL") => {
                        if let Some(v) = f.get("notional").or_else(|| f.get("minNotional")) {
                            min_notional = parse_dec(v, "notional")?;
                        }
                    }
                    _ => {}
                }
            }
        }

        rules.push(InstrumentRules {
            venue: Venue::UsdsFutures,
            symbol: Symbol(symbol_str.to_owned()),
            tick_size,
            step_size,
            min_qty,
            max_qty,
            min_notional,
            status_trading: status == "TRADING",
            base_precision,
            quote_precision,
        });
    }

    Ok(ExchangeInfoSnapshot {
        venue: Venue::UsdsFutures,
        server_time_ms,
        rules,
        rate_limits: Vec::new(),
    })
}

/// Parse a USDⓈ-M Futures User Data Stream JSON frame.
///
/// # Errors
/// Returns [`GatewayError::InvalidRequest`] if the frame is not valid JSON or has an unknown event type.
#[allow(clippy::too_many_lines)]
pub fn parse_futures_user_event(raw: &str) -> Result<FuturesUserEvent, GatewayError> {
    let root: Value = serde_json::from_str(raw).map_err(|e| {
        GatewayError::InvalidRequest(format!("invalid futures user event JSON: {e}"))
    })?;
    let event_type = root
        .get("e")
        .and_then(Value::as_str)
        .ok_or_else(|| GatewayError::InvalidRequest("missing 'e' event type".into()))?;
    let event_time_ms = root.get("E").and_then(Value::as_i64).unwrap_or(0);

    match event_type {
        "ORDER_TRADE_UPDATE" => {
            let o = root.get("o").ok_or_else(|| {
                GatewayError::InvalidRequest("ORDER_TRADE_UPDATE missing 'o'".into())
            })?;
            let sym_str = o
                .get("s")
                .and_then(Value::as_str)
                .ok_or_else(|| GatewayError::InvalidRequest("missing 'o.s'".into()))?;
            let side = parse_side(
                o.get("S")
                    .and_then(Value::as_str)
                    .ok_or_else(|| GatewayError::InvalidRequest("missing 'o.S'".into()))?,
            )?;
            let last_qty = Qty(parse_dec(
                o.get("l")
                    .ok_or_else(|| GatewayError::InvalidRequest("missing 'o.l'".into()))?,
                "o.l",
            )?);
            let cum_qty = Qty(parse_dec(
                o.get("z")
                    .ok_or_else(|| GatewayError::InvalidRequest("missing 'o.z'".into()))?,
                "o.z",
            )?);
            let last_price = Price(parse_dec(
                o.get("L")
                    .ok_or_else(|| GatewayError::InvalidRequest("missing 'o.L'".into()))?,
                "o.L",
            )?);
            let realized_pnl = o
                .get("rp")
                .map(|v| parse_dec(v, "o.rp"))
                .transpose()?
                .unwrap_or(Decimal::ZERO);

            Ok(FuturesUserEvent::OrderTradeUpdate {
                venue: Venue::UsdsFutures,
                event_time_ms,
                symbol: Symbol(sym_str.to_owned()),
                client_order_id: o
                    .get("c")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                order_id: o.get("i").and_then(Value::as_i64).unwrap_or(0),
                execution_type: o
                    .get("x")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                order_status: o
                    .get("X")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                side,
                last_filled_qty: last_qty,
                cum_filled_qty: cum_qty,
                last_filled_price: last_price,
                realized_pnl,
            })
        }
        "ACCOUNT_UPDATE" => {
            let reason = root
                .get("a")
                .and_then(|a| a.get("m"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(FuturesUserEvent::AccountUpdate {
                venue: Venue::UsdsFutures,
                event_time_ms,
                reason,
            })
        }
        "ALGO_UPDATE" => {
            let o = root
                .get("o")
                .ok_or_else(|| GatewayError::InvalidRequest("ALGO_UPDATE missing 'o'".into()))?;
            let sym_str = o
                .get("s")
                .and_then(Value::as_str)
                .ok_or_else(|| GatewayError::InvalidRequest("ALGO_UPDATE missing 'o.s'".into()))?;
            Ok(FuturesUserEvent::AlgoUpdate {
                venue: Venue::UsdsFutures,
                event_time_ms,
                symbol: Symbol(sym_str.to_owned()),
                algo_id: o.get("aid").and_then(Value::as_i64).unwrap_or(0),
                client_algo_id: o
                    .get("caid")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                algo_status: o
                    .get("X")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
        }
        "MARGIN_CALL" => {
            let position_count = root.get("p").and_then(Value::as_array).map_or(0, Vec::len);
            Ok(FuturesUserEvent::MarginCall {
                venue: Venue::UsdsFutures,
                event_time_ms,
                position_count,
            })
        }
        "ACCOUNT_CONFIG_UPDATE" => Ok(FuturesUserEvent::AccountConfigUpdate {
            venue: Venue::UsdsFutures,
            event_time_ms,
        }),
        "listenKeyExpired" => Ok(FuturesUserEvent::ListenKeyExpired {
            venue: Venue::UsdsFutures,
            event_time_ms,
        }),
        other => Err(GatewayError::InvalidRequest(format!(
            "unsupported futures user event '{other}'"
        ))),
    }
}

/// Parse a USDⓈ-M Futures User Data Stream JSON frame into a venue-agnostic [`GatewayEvent`].
///
/// Maps:
/// - `ORDER_TRADE_UPDATE` -> `Some(GatewayEvent::Exec(_))`
/// - `ACCOUNT_UPDATE` -> `Some(GatewayEvent::Account(AccountEvent::PositionUpdate { .. }))`
/// - `ALGO_UPDATE` -> `Some(GatewayEvent::Exec(_))` (`exec_type = "ALGO_UPDATE"`)
/// - `listenKeyExpired` -> `Some(GatewayEvent::StreamTerminated { .. })`
/// - `MARGIN_CALL` / `ACCOUNT_CONFIG_UPDATE` -> `Ok(None)`
///
/// # Errors
/// Returns [`GatewayError::WireFormat`] if the frame is malformed or contains an unrecognized event.
#[allow(clippy::too_many_lines)]
pub fn parse_futures_user_gateway_event(raw: &str) -> Result<Option<GatewayEvent>, GatewayError> {
    let root: Value = serde_json::from_str(raw)
        .map_err(|e| GatewayError::WireFormat(format!("invalid futures user event JSON: {e}")))?;
    let event_type = root
        .get("e")
        .and_then(Value::as_str)
        .ok_or_else(|| GatewayError::WireFormat("missing 'e' event type".into()))?;
    let event_time_ms = root.get("E").and_then(Value::as_i64).unwrap_or(0);

    match event_type {
        "ORDER_TRADE_UPDATE" => {
            let o = root
                .get("o")
                .ok_or_else(|| GatewayError::WireFormat("ORDER_TRADE_UPDATE missing 'o'".into()))?;
            let sym_str = o
                .get("s")
                .and_then(Value::as_str)
                .ok_or_else(|| GatewayError::WireFormat("missing 'o.s'".into()))?;
            let side = parse_side(
                o.get("S")
                    .and_then(Value::as_str)
                    .ok_or_else(|| GatewayError::WireFormat("missing 'o.S'".into()))?,
            )
            .map_err(|e| GatewayError::WireFormat(e.to_string()))?;
            let kind =
                parse_futures_order_kind(o.get("o").and_then(Value::as_str).unwrap_or("LIMIT"));
            let status_str = o.get("X").and_then(Value::as_str).unwrap_or("NEW");
            let status = parse_futures_order_status(status_str);
            let order_price = o
                .get("p")
                .and_then(Value::as_str)
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or(Decimal::ZERO);
            let order_qty = o
                .get("q")
                .and_then(Value::as_str)
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or(Decimal::ZERO);
            let last_filled_qty = parse_dec(
                o.get("l")
                    .ok_or_else(|| GatewayError::WireFormat("missing 'o.l'".into()))?,
                "o.l",
            )
            .map_err(|e| GatewayError::WireFormat(e.to_string()))?;
            let cum_filled_qty = parse_dec(
                o.get("z")
                    .ok_or_else(|| GatewayError::WireFormat("missing 'o.z'".into()))?,
                "o.z",
            )
            .map_err(|e| GatewayError::WireFormat(e.to_string()))?;
            let last_filled_price = parse_dec(
                o.get("L")
                    .ok_or_else(|| GatewayError::WireFormat("missing 'o.L'".into()))?,
                "o.L",
            )
            .map_err(|e| GatewayError::WireFormat(e.to_string()))?;
            let avg_price = o
                .get("ap")
                .and_then(Value::as_str)
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or(last_filled_price);
            let commission = o
                .get("n")
                .and_then(Value::as_str)
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or(Decimal::ZERO);
            let commission_asset = o
                .get("N")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);

            Ok(Some(GatewayEvent::Exec(ExecEvent {
                venue: Venue::UsdsFutures,
                event_time_ms,
                transact_time_ms: o.get("T").and_then(Value::as_i64).unwrap_or(event_time_ms),
                symbol: Symbol(sym_str.to_owned()),
                client_order_id: o
                    .get("c")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                orig_client_order_id: None,
                order_id: o.get("i").and_then(Value::as_i64).unwrap_or(0),
                order_list_id: -1,
                trade_id: o.get("t").and_then(Value::as_i64).unwrap_or(-1),
                side,
                kind,
                exec_type: o
                    .get("x")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                status,
                reject_reason: "NONE".to_owned(),
                order_price: Price(order_price),
                order_qty: Qty(order_qty),
                last_filled_qty: Qty(last_filled_qty),
                cumulative_filled_qty: Qty(cum_filled_qty),
                last_filled_price: Price(last_filled_price),
                cumulative_quote_qty: cum_filled_qty.saturating_mul(avg_price),
                commission,
                commission_asset,
                is_maker: o.get("m").and_then(Value::as_bool).unwrap_or(false),
            })))
        }
        "ACCOUNT_UPDATE" => {
            let last_update_ms = root
                .get("T")
                .and_then(Value::as_i64)
                .unwrap_or(event_time_ms);
            let mut balances = Vec::new();
            if let Some(raw_balances) = root
                .get("a")
                .and_then(|a| a.get("B"))
                .and_then(Value::as_array)
            {
                for entry in raw_balances {
                    let Some(asset) = entry.get("a").and_then(Value::as_str) else {
                        continue;
                    };
                    let wb = entry
                        .get("wb")
                        .and_then(Value::as_str)
                        .and_then(|s| Decimal::from_str(s).ok())
                        .unwrap_or(Decimal::ZERO);
                    let cw = entry
                        .get("cw")
                        .and_then(Value::as_str)
                        .and_then(|s| Decimal::from_str(s).ok());
                    let free = cw.unwrap_or(wb);
                    let locked = cw.map_or(Decimal::ZERO, |cross| {
                        wb.saturating_sub(cross).max(Decimal::ZERO)
                    });
                    balances.push(AccountBalance {
                        asset: asset.to_owned(),
                        free,
                        locked,
                    });
                }
            }
            Ok(Some(GatewayEvent::Account(AccountEvent::PositionUpdate {
                venue: Venue::UsdsFutures,
                event_time_ms,
                last_update_ms,
                balances,
            })))
        }
        "ALGO_UPDATE" => {
            let o = root
                .get("o")
                .ok_or_else(|| GatewayError::WireFormat("ALGO_UPDATE missing 'o'".into()))?;
            let sym_str = o
                .get("s")
                .and_then(Value::as_str)
                .ok_or_else(|| GatewayError::WireFormat("ALGO_UPDATE missing 'o.s'".into()))?;
            let side = match o.get("S").and_then(Value::as_str) {
                Some("SELL") => Side::Sell,
                _ => Side::Buy,
            };
            let kind = parse_futures_order_kind(
                o.get("o").and_then(Value::as_str).unwrap_or("STOP_MARKET"),
            );
            let algo_status = o.get("X").and_then(Value::as_str).unwrap_or("NEW");
            let qty = o
                .get("q")
                .and_then(Value::as_str)
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or(Decimal::ZERO);
            let trigger_price = o
                .get("tp")
                .or_else(|| o.get("p"))
                .and_then(Value::as_str)
                .and_then(|s| Decimal::from_str(s).ok())
                .unwrap_or(Decimal::ZERO);

            Ok(Some(GatewayEvent::Exec(ExecEvent {
                venue: Venue::UsdsFutures,
                event_time_ms,
                transact_time_ms: o.get("T").and_then(Value::as_i64).unwrap_or(event_time_ms),
                symbol: Symbol(sym_str.to_owned()),
                client_order_id: o
                    .get("caid")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                orig_client_order_id: None,
                order_id: o.get("aid").and_then(Value::as_i64).unwrap_or(0),
                order_list_id: -1,
                trade_id: -1,
                side,
                kind,
                exec_type: "ALGO_UPDATE".to_owned(),
                status: parse_futures_algo_status(algo_status),
                reject_reason: "NONE".to_owned(),
                order_price: Price(trigger_price),
                order_qty: Qty(qty),
                last_filled_qty: Qty(Decimal::ZERO),
                cumulative_filled_qty: Qty(Decimal::ZERO),
                last_filled_price: Price(Decimal::ZERO),
                cumulative_quote_qty: Decimal::ZERO,
                commission: Decimal::ZERO,
                commission_asset: None,
                is_maker: false,
            })))
        }
        "listenKeyExpired" => Ok(Some(GatewayEvent::StreamTerminated {
            venue: Venue::UsdsFutures,
            event_time_ms,
        })),
        "MARGIN_CALL" | "ACCOUNT_CONFIG_UPDATE" => Ok(None),
        other => Err(GatewayError::WireFormat(format!(
            "unsupported futures user event '{other}'"
        ))),
    }
}

fn parse_futures_order_kind(raw: &str) -> OrderKind {
    match raw {
        "MARKET" => OrderKind::Market,
        "STOP" | "STOP_MARKET" | "TRAILING_STOP_MARKET" => OrderKind::StopLossLimit,
        "TAKE_PROFIT" | "TAKE_PROFIT_MARKET" => OrderKind::TakeProfitLimit,
        _ => OrderKind::Limit,
    }
}

fn parse_futures_order_status(raw: &str) -> OrderStatus {
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

fn parse_futures_algo_status(raw: &str) -> OrderStatus {
    match raw {
        "NEW" | "TRIGGERING" => OrderStatus::New,
        "TRIGGERED" | "FINISHED" => OrderStatus::Filled,
        "CANCELED" => OrderStatus::Canceled,
        "REJECTED" => OrderStatus::Rejected,
        "EXPIRED" => OrderStatus::Expired,
        _ => OrderStatus::UnknownNeedsQuery,
    }
}

/// Build wire parameter map for `/algoOrder.place` (`FUTURES_WS.yaml`).
///
/// # Errors
/// Returns [`GatewayError::InvalidRequest`] if `client_algo_id` is invalid or `close_position`
/// conflicts with `quantity`.
pub fn build_place_algo_order_params(
    req: &PlaceAlgoOrderRequest,
) -> Result<Map<String, Value>, GatewayError> {
    validate_client_order_id(&req.client_algo_id)?;
    if req.close_position && req.quantity.is_some() {
        return Err(GatewayError::InvalidRequest(
            "closePosition=true cannot be combined with quantity".into(),
        ));
    }
    let mut map = Map::new();
    map.insert("algoType".into(), Value::String("CONDITIONAL".into()));
    map.insert("symbol".into(), Value::String(req.symbol.0.clone()));
    map.insert(
        "side".into(),
        Value::String(match req.side {
            Side::Buy => "BUY".into(),
            Side::Sell => "SELL".into(),
        }),
    );
    map.insert(
        "type".into(),
        Value::String(req.order_type.as_binance_str().to_owned()),
    );
    map.insert(
        "clientAlgoId".into(),
        Value::String(req.client_algo_id.clone()),
    );
    map.insert(
        "workingType".into(),
        Value::String(req.working_type.as_binance_str().to_owned()),
    );
    if let Some(qty) = req.quantity {
        map.insert("quantity".into(), Value::String(qty.0.to_string()));
    }
    if let Some(trigger) = req.trigger_price {
        map.insert("triggerPrice".into(), Value::String(trigger.0.to_string()));
    }
    if let Some(activate) = req.activate_price {
        map.insert(
            "activatePrice".into(),
            Value::String(activate.0.to_string()),
        );
    }
    if let Some(cb) = req.callback_rate {
        map.insert("callbackRate".into(), Value::String(cb.to_string()));
    }
    if req.close_position {
        map.insert("closePosition".into(), Value::String("true".into()));
    }
    if req.reduce_only {
        map.insert("reduceOnly".into(), Value::String("true".into()));
    }
    if req.price_protect {
        map.insert("priceProtect".into(), Value::String("true".into()));
    }
    Ok(map)
}

/// Build wire parameter map for `/algoOrder.cancel` (`FUTURES_WS.yaml`).
///
/// # Errors
/// Returns [`GatewayError::InvalidRequest`] if neither `algo_id` nor `client_algo_id` is set.
pub fn build_cancel_algo_order_params(
    req: &CancelAlgoOrderRequest,
) -> Result<Map<String, Value>, GatewayError> {
    if req.algo_id.is_none() && req.client_algo_id.is_none() {
        return Err(GatewayError::InvalidRequest(
            "cancel_algo_order requires algo_id or client_algo_id".into(),
        ));
    }
    let mut map = Map::new();
    map.insert("symbol".into(), Value::String(req.symbol.0.clone()));
    if let Some(id) = req.algo_id {
        map.insert("algoId".into(), Value::Number(id.into()));
    }
    if let Some(cid) = &req.client_algo_id {
        validate_client_order_id(cid)?;
        map.insert("clientAlgoId".into(), Value::String(cid.clone()));
    }
    Ok(map)
}
