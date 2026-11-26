//! Pre-live exchange probes (`binance-botctl probe signed|dry-run-order|ws-logon`, M19).
//!
//! Strictly read-only / non-destructive: zero orders are ever transmitted to Binance,
//! and secret credentials never appear in output or error messages.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use bot_config::{BotConfig, ConfigSourcePaths, load_config, load_from_paths};
use bot_exchange::auth::canonical_payload;
use bot_exchange::futures::rest::{
    build_place_order_params as build_futures_order_params, json_map_to_btree,
};
use bot_exchange::futures::wire::build_place_algo_order_params;
use bot_exchange::spot::wire::{
    build_place_oco_params as build_spot_oco_params,
    build_place_order_params as build_spot_order_params,
};
use bot_exchange::{
    AlgoOrderType, Ed25519Credentials, ExchangeErrorKind, FuturesRestClient, FuturesWsSession,
    GatewayError, PlaceAlgoOrderRequest, PlaceOcoRequest, PlaceOrderRequest, RateLimitTracker,
    SpotRestClient, SpotSessionConfig, SpotWsSession, TimeInForce, WorkingType,
};
use bot_types::{
    InstrumentRules, OrderKind, Price, Qty, RulesError, Side, Symbol, TradingMode, Venue,
};
use ed25519_dalek::{Signature, VerifyingKey};
use rust_decimal::Decimal;
use serde::Serialize;

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Return an actionable remediation hint for common Binance authentication/clock errors.
#[must_use]
pub const fn actionable_hint_for_error(err: &GatewayError) -> Option<&'static str> {
    match err {
        GatewayError::TimestampOutOfWindow { .. }
        | GatewayError::ExchangeRejected {
            kind: ExchangeErrorKind::TimestampOutOfWindow,
            ..
        } => Some(
            "Timestamp outside recvWindow (-1021): synchronize host clock via NTP/chronyd (`chronyc tracking`) or check max_clock_drift_ms",
        ),
        GatewayError::InvalidAuth { .. }
        | GatewayError::ExchangeRejected {
            kind: ExchangeErrorKind::InvalidApiKeyOrPermission,
            ..
        } => Some(
            "Invalid API key, IP whitelist, or permission (-2015): verify key matches environment (testnet vs live), IP allowlist, and venue trading permission",
        ),
        GatewayError::ExchangeRejected {
            kind: ExchangeErrorKind::ApiKeyFormatInvalid,
            ..
        } => Some(
            "API key format invalid (-2014): check api_key_file for stray whitespace, quotes, or wrong key copy",
        ),
        GatewayError::ExchangeRejected {
            kind: ExchangeErrorKind::InvalidSignature,
            ..
        } => Some(
            "Signature invalid (-1022): verify the registered Ed25519 public key on Binance matches `binance-botctl key fingerprint`",
        ),
        _ => None,
    }
}

/// Options for [`run_signed_probe`].
#[derive(Debug, Clone)]
pub struct SignedProbeOptions {
    /// Target venue (`Spot` or `UsdsFutures`).
    pub venue: Venue,
    /// Optional explicit TOML configuration path.
    pub config: Option<PathBuf>,
    /// Optional REST base URL override (primarily for tests).
    pub rest_url_override: Option<String>,
}

/// Result of `binance-botctl probe signed`.
#[derive(Debug, Clone, Serialize)]
pub struct SignedProbeReport {
    /// Target venue.
    pub venue: Venue,
    /// Endpoint probed (no query parameters or secrets).
    pub endpoint: String,
    /// Local Ed25519 sign + strict verify self-test result.
    pub local_sign_verify_ok: bool,
    /// Measured server minus local clock offset in ms.
    pub clock_offset_ms: i64,
    /// Whether `|clock_offset_ms| <= max_clock_drift_ms`.
    pub clock_within_limit: bool,
    /// Account `canTrade` permission flag when account read succeeded.
    pub can_trade: Option<bool>,
    /// Count of non-zero asset balances observed.
    pub non_zero_balances: usize,
    /// Overall probe pass/fail.
    pub ok: bool,
    /// Error summary when failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Actionable remediation hint when failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl SignedProbeReport {
    /// Render as concise human-readable text.
    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = format!(
            "probe signed ({:?}) [{}]\n  endpoint: {}\n  local sign+verify: {}\n  clock offset: {}ms (within limit: {})\n  canTrade: {:?} | non-zero balances: {}",
            self.venue,
            if self.ok { "PASS" } else { "FAIL" },
            self.endpoint,
            self.local_sign_verify_ok,
            self.clock_offset_ms,
            self.clock_within_limit,
            self.can_trade,
            self.non_zero_balances,
        );
        if let Some(err) = &self.error {
            out.push_str("\n  error: ");
            out.push_str(err);
        }
        if let Some(hint) = &self.hint {
            out.push_str("\n  hint: ");
            out.push_str(hint);
        }
        out
    }
}

pub(crate) fn load_probe_config(config: Option<&Path>) -> Result<BotConfig> {
    if let Some(path) = config {
        if !path.is_file() {
            bail!("config file not found: {}", path.display());
        }
        let conf_d_dir = path
            .parent()
            .map(|p| p.join("conf.d"))
            .filter(|d| d.is_dir());
        load_from_paths(&ConfigSourcePaths {
            base_file: Some(path.to_path_buf()),
            conf_d_dir,
            env_prefix: Some("BOT__".to_owned()),
        })
        .with_context(|| format!("failed to load config {}", path.display()))
    } else {
        load_config().context("failed to load default configuration")
    }
}

pub(crate) fn load_venue_credentials(cfg: &BotConfig, venue: Venue) -> Result<Ed25519Credentials> {
    let (api_key_file, private_key_path) = match venue {
        Venue::Spot => (
            &cfg.venues.spot.api_key_file,
            &cfg.venues.spot.private_key_path,
        ),
        Venue::UsdsFutures => (
            &cfg.venues.futures.api_key_file,
            &cfg.venues.futures.private_key_path,
        ),
    };
    let raw_key = std::fs::read_to_string(api_key_file)
        .with_context(|| format!("failed to read API key file {api_key_file}"))?;
    let api_key = raw_key.trim().to_owned();
    if api_key.is_empty() {
        bail!("API key file {api_key_file} is empty");
    }
    Ed25519Credentials::from_pkcs8_pem_file(api_key, Path::new(private_key_path))
        .map_err(|e| anyhow::anyhow!("failed to load Ed25519 credentials: {e}"))
}

pub(crate) fn verify_local_ed25519_signing(creds: &Ed25519Credentials) -> Result<()> {
    let mut sample = BTreeMap::new();
    sample.insert("recvWindow".to_owned(), "5000".to_owned());
    sample.insert("timestamp".to_owned(), "1700000000000".to_owned());
    let sig_b64 = creds.sign_params(&sample);
    let sig_bytes = BASE64_STANDARD
        .decode(sig_b64.as_bytes())
        .context("local signature base64 decode failed")?;
    let sig = Signature::from_slice(&sig_bytes).context("invalid Ed25519 signature length")?;
    let vk = VerifyingKey::from_bytes(&creds.public_key_bytes())
        .context("invalid Ed25519 verifying key")?;
    vk.verify_strict(canonical_payload(&sample).as_bytes(), &sig)
        .context("local Ed25519 strict verification failed")
}

/// Run the read-only signed account probe for Spot (`GET /api/v3/account`) or
/// Futures (`GET /fapi/v2/account`). Zero orders are sent.
///
/// # Errors
/// Returns an error if configuration or local credentials cannot be loaded.
pub async fn run_signed_probe(opts: &SignedProbeOptions) -> Result<SignedProbeReport> {
    let cfg = load_probe_config(opts.config.as_deref())?;
    let creds = load_venue_credentials(&cfg, opts.venue)?;
    verify_local_ed25519_signing(&creds)?;

    let max_drift = i64::try_from(cfg.binance.max_clock_drift_ms).unwrap_or(i64::MAX);
    let base_url = resolve_rest_base_url(&cfg, opts.venue, opts.rest_url_override.as_deref());
    let endpoint = match opts.venue {
        Venue::Spot => format!("{base_url}/api/v3/account"),
        Venue::UsdsFutures => format!("{base_url}/fapi/v2/account"),
    };

    let (clock_offset_ms, account_res) = match opts.venue {
        Venue::Spot => {
            let client = SpotRestClient::new(&base_url, PROBE_TIMEOUT)
                .map_err(|e| anyhow::anyhow!("SpotRestClient init failed: {e}"))?;
            let sync = match client.server_time().await {
                Ok(s) => s,
                Err(err) => {
                    return Ok(failed_probe(opts.venue, endpoint, 0, false, &err));
                }
            };
            let res = client
                .account_snapshot(&creds, cfg.binance.recv_window_ms)
                .await;
            (sync.offset_ms, res)
        }
        Venue::UsdsFutures => {
            let client = FuturesRestClient::new(
                &base_url,
                PROBE_TIMEOUT,
                Some(Arc::new(creds)),
                Arc::new(RateLimitTracker::new()),
            )
            .map_err(|e| anyhow::anyhow!("FuturesRestClient init failed: {e}"))?;
            let sync = match client.server_time().await {
                Ok(s) => s,
                Err(err) => {
                    return Ok(failed_probe(opts.venue, endpoint, 0, false, &err));
                }
            };
            let res = client.account_snapshot().await;
            (sync.offset_ms, res)
        }
    };

    let clock_within_limit = clock_offset_ms.saturating_abs() <= max_drift;
    match account_res {
        Ok(snap) => {
            let mut hint = None;
            let mut error = None;
            if !clock_within_limit {
                error = Some(format!(
                    "clock offset {clock_offset_ms}ms exceeds limit ±{max_drift}ms"
                ));
                hint =
                    Some("synchronize host clock via NTP/chronyd to avoid -1021 errors".to_owned());
            } else if !snap.can_trade {
                error = Some("account returned canTrade=false".to_owned());
                hint = Some(match opts.venue {
                    Venue::Spot => {
                        "canTrade=false: enable Spot trading permission on this Binance API key"
                            .to_owned()
                    }
                    Venue::UsdsFutures => {
                        "canTrade=false: enable USD-M Futures trading permission on this Binance API key"
                            .to_owned()
                    }
                });
            }
            let non_zero_balances = snap
                .balances
                .iter()
                .filter(|b| !b.free.is_zero() || !b.locked.is_zero())
                .count();
            let ok = clock_within_limit && snap.can_trade;
            Ok(SignedProbeReport {
                venue: opts.venue,
                endpoint,
                local_sign_verify_ok: true,
                clock_offset_ms,
                clock_within_limit,
                can_trade: Some(snap.can_trade),
                non_zero_balances,
                ok,
                error,
                hint,
            })
        }
        Err(err) => Ok(failed_probe(
            opts.venue,
            endpoint,
            clock_offset_ms,
            clock_within_limit,
            &err,
        )),
    }
}

fn failed_probe(
    venue: Venue,
    endpoint: String,
    clock_offset_ms: i64,
    clock_within_limit: bool,
    err: &GatewayError,
) -> SignedProbeReport {
    SignedProbeReport {
        venue,
        endpoint,
        local_sign_verify_ok: true,
        clock_offset_ms,
        clock_within_limit,
        can_trade: None,
        non_zero_balances: 0,
        ok: false,
        error: Some(err.to_string()),
        hint: actionable_hint_for_error(err).map(str::to_owned),
    }
}

pub(crate) fn resolve_rest_base_url(
    cfg: &BotConfig,
    venue: Venue,
    override_url: Option<&str>,
) -> String {
    if let Some(u) = override_url {
        return u.trim_end_matches('/').to_owned();
    }
    match (venue, cfg.general.mode) {
        (Venue::Spot, TradingMode::Live) => SpotRestClient::PRODUCTION_URL.to_owned(),
        (Venue::Spot, _) => SpotRestClient::TESTNET_URL.to_owned(),
        (Venue::UsdsFutures, TradingMode::Live) => FuturesRestClient::PRODUCTION_URL.to_owned(),
        (Venue::UsdsFutures, _) => FuturesRestClient::TESTNET_URL.to_owned(),
    }
}

/// Options for [`run_dry_run_order`].
#[derive(Debug, Clone)]
pub struct DryRunOrderOptions {
    /// Target venue.
    pub venue: Venue,
    /// Trading symbol (e.g. `BTCUSDT`).
    pub symbol: String,
    /// Optional synthetic entry price (defaults to a sensible fixture price).
    pub price: Option<Decimal>,
    /// Optional synthetic entry quantity (defaults to a sensible fixture qty).
    pub qty: Option<Decimal>,
    /// Use built-in offline `InstrumentRules` fixture without opening any network socket.
    pub offline: bool,
    /// Optional explicit TOML configuration path.
    pub config: Option<PathBuf>,
    /// Optional REST base URL override (tests).
    pub rest_url_override: Option<String>,
}

/// Result of `binance-botctl probe dry-run-order`. Never transmits any order.
#[derive(Debug, Clone, Serialize)]
pub struct DryRunOrderReport {
    /// Target venue.
    pub venue: Venue,
    /// Normalized symbol.
    pub symbol: String,
    /// Entry price quantized to `PRICE_FILTER.tickSize`.
    pub quantized_price: Decimal,
    /// Entry quantity quantized to `LOT_SIZE.stepSize`.
    pub quantized_qty: Decimal,
    /// Resulting notional (`quantized_price * quantized_qty`).
    pub notional_usd: Decimal,
    /// Canonical REST/WS entry payload preview (built via `bot-exchange`, never contains secrets).
    pub rest_entry_query_preview: String,
    /// Canonical protective exit payload preview (`orderList.place.oco` or `algoOrder.place`).
    pub exit_payload_preview: String,
    /// WS API entry method name.
    pub ws_entry_method: String,
    /// WS API protective exit method name (`orderList.place.oco` or `algoOrder.place`).
    pub ws_exit_method: String,
    /// Whether the locally generated entry Ed25519 signature passed strict verification.
    pub signature_verified_locally: bool,
    /// Whether the locally generated protective exit Ed25519 signature passed strict verification.
    pub exit_signature_verified_locally: bool,
    /// Always `false` — no order is ever transmitted over the network.
    pub transmitted: bool,
}

impl DryRunOrderReport {
    /// Render as concise human-readable text.
    #[must_use]
    pub fn render_text(&self) -> String {
        format!(
            "probe dry-run-order ({:?} {}) [PASS — NOT TRANSMITTED]\n  price: {} | qty: {} | notional: ${}\n  ws entry: {} | ws exit: {}\n  signatures verified locally: entry={}, exit={}\n  entry preview: {}\n  exit preview: {}",
            self.venue,
            self.symbol,
            self.quantized_price,
            self.quantized_qty,
            self.notional_usd,
            self.ws_entry_method,
            self.ws_exit_method,
            self.signature_verified_locally,
            self.exit_signature_verified_locally,
            self.rest_entry_query_preview,
            self.exit_payload_preview
        )
    }
}

fn offline_fixture_rules(venue: Venue, symbol: &str) -> InstrumentRules {
    let (step_size, min_notional, base_precision) = match venue {
        Venue::Spot => (Decimal::new(1, 5), Decimal::from(10), 5),
        Venue::UsdsFutures => (Decimal::new(1, 3), Decimal::from(5), 3),
    };
    InstrumentRules {
        venue,
        symbol: Symbol(symbol.to_owned()),
        tick_size: Decimal::new(1, 2),
        step_size,
        min_qty: step_size,
        max_qty: Decimal::from(1_000),
        min_notional,
        status_trading: true,
        base_precision,
        quote_precision: 2,
    }
}

async fn fetch_instrument_rules(
    opts: &DryRunOrderOptions,
    symbol: &str,
) -> Result<InstrumentRules> {
    if opts.offline {
        return Ok(offline_fixture_rules(opts.venue, symbol));
    }
    let cfg = load_probe_config(opts.config.as_deref())?;
    let base_url = resolve_rest_base_url(&cfg, opts.venue, opts.rest_url_override.as_deref());
    let info = match opts.venue {
        Venue::Spot => {
            let client = SpotRestClient::new(&base_url, PROBE_TIMEOUT)
                .map_err(|e| anyhow::anyhow!("SpotRestClient init failed: {e}"))?;
            client
                .exchange_info()
                .await
                .map_err(|e| anyhow::anyhow!("Spot exchangeInfo failed: {e}"))?
        }
        Venue::UsdsFutures => {
            let client = FuturesRestClient::new(
                &base_url,
                PROBE_TIMEOUT,
                None,
                Arc::new(RateLimitTracker::new()),
            )
            .map_err(|e| anyhow::anyhow!("FuturesRestClient init failed: {e}"))?;
            client
                .exchange_info()
                .await
                .map_err(|e| anyhow::anyhow!("Futures exchangeInfo failed: {e}"))?
        }
    };
    info.rules
        .into_iter()
        .find(|r| r.symbol.as_str() == symbol)
        .with_context(|| format!("symbol {symbol} not found in {:?} exchangeInfo", opts.venue))
}

/// Quantize a synthetic entry/exit order against `exchangeInfo` (or offline fixture) and
/// verify Ed25519 payload signing locally without transmitting any order.
///
/// # Errors
/// Returns an error if filter quantization/validation fails (`RulesError`) or exchange info fetch fails.
pub async fn run_dry_run_order(opts: &DryRunOrderOptions) -> Result<DryRunOrderReport> {
    let symbol = opts.symbol.trim().to_ascii_uppercase();
    let rules = fetch_instrument_rules(opts, &symbol).await?;

    let raw_price = opts.price.unwrap_or_else(|| Decimal::from(50_000));
    let raw_qty = opts.qty.unwrap_or_else(|| Decimal::new(1, 2));

    let quantized_price = rules.floor_price(Price(raw_price))?.0;
    let quantized_qty = rules.floor_qty(Qty(raw_qty))?.0;
    rules.validate_order(Price(quantized_price), Qty(quantized_qty))?;
    let notional = quantized_price
        .checked_mul(quantized_qty)
        .ok_or(RulesError::ArithmeticOverflow)?;

    let creds = if let Some(cfg_path) = opts.config.as_deref() {
        let cfg = load_probe_config(Some(cfg_path))?;
        load_venue_credentials(&cfg, opts.venue)?
    } else {
        Ed25519Credentials::from_seed("dry-run-ephemeral".to_owned(), [0x42_u8; 32])
            .map_err(|e| anyhow::anyhow!("ephemeral credential init failed: {e}"))?
    };

    let payloads = build_dry_run_payloads(opts.venue, &rules, quantized_price, quantized_qty)?;
    let entry_payload = sign_and_verify_map(&creds, &payloads.entry)?;
    let exit_payload = sign_and_verify_map(&creds, &payloads.exit)?;

    Ok(DryRunOrderReport {
        venue: opts.venue,
        symbol,
        quantized_price,
        quantized_qty,
        notional_usd: notional,
        rest_entry_query_preview: format!("{entry_payload}&signature=<verified-ed25519-sig>"),
        exit_payload_preview: format!("{exit_payload}&signature=<verified-ed25519-sig>"),
        ws_entry_method: "order.place".to_owned(),
        ws_exit_method: payloads.ws_exit_method.to_owned(),
        signature_verified_locally: true,
        exit_signature_verified_locally: true,
        transmitted: false,
    })
}

struct DryRunPayloads {
    entry: BTreeMap<String, String>,
    exit: BTreeMap<String, String>,
    ws_exit_method: &'static str,
}

fn build_dry_run_payloads(
    venue: Venue,
    rules: &InstrumentRules,
    entry_price: Decimal,
    qty: Decimal,
) -> Result<DryRunPayloads> {
    const SAMPLE_TS: i64 = 1_700_000_000_000;
    let sym = rules.symbol.clone();
    let tp_price = rules.floor_price(Price(entry_price * Decimal::new(102, 2)))?;
    let sl_stop = rules.floor_price(Price(entry_price * Decimal::new(99, 2)))?;
    let sl_limit = rules.floor_price(Price(entry_price * Decimal::new(988, 3)))?;

    match venue {
        Venue::Spot => {
            let entry_req = PlaceOrderRequest {
                symbol: sym.clone(),
                side: Side::Buy,
                kind: OrderKind::LimitMaker,
                client_order_id: "dryrun_spot_entry_1".to_owned(),
                quantity: Some(Qty(qty)),
                quote_order_qty: None,
                price: Some(Price(entry_price)),
                stop_price: None,
                time_in_force: None,
            };
            let entry_val = build_spot_order_params(&entry_req, SAMPLE_TS, Some(5_000))
                .map_err(|e| anyhow::anyhow!("Spot entry payload build failed: {e}"))?;
            let entry_map = entry_val
                .as_object()
                .cloned()
                .context("Spot entry params must be a JSON object")?;

            let oco_req = PlaceOcoRequest {
                symbol: sym,
                side: Side::Sell,
                list_client_order_id: "dryrun_spot_oco_1".to_owned(),
                quantity: Qty(qty),
                above_type: OrderKind::LimitMaker,
                above_client_order_id: Some("dryrun_spot_tp_1".to_owned()),
                above_price: Some(tp_price),
                above_stop_price: None,
                above_time_in_force: None,
                below_type: OrderKind::StopLossLimit,
                below_client_order_id: Some("dryrun_spot_sl_1".to_owned()),
                below_price: Some(sl_limit),
                below_stop_price: Some(sl_stop),
                below_time_in_force: Some(TimeInForce::Gtc),
                below_trailing_delta: None,
            };
            let exit_val = build_spot_oco_params(&oco_req, SAMPLE_TS, Some(5_000))
                .map_err(|e| anyhow::anyhow!("Spot OCO exit payload build failed: {e}"))?;
            let exit_map = exit_val
                .as_object()
                .cloned()
                .context("Spot OCO exit params must be a JSON object")?;

            Ok(DryRunPayloads {
                entry: json_map_to_btree(entry_map),
                exit: json_map_to_btree(exit_map),
                ws_exit_method: "orderList.place.oco",
            })
        }
        Venue::UsdsFutures => {
            let entry_req = PlaceOrderRequest {
                symbol: sym.clone(),
                side: Side::Buy,
                kind: OrderKind::LimitMaker,
                client_order_id: "dryrun_fut_entry_1".to_owned(),
                quantity: Some(Qty(qty)),
                quote_order_qty: None,
                price: Some(Price(entry_price)),
                stop_price: None,
                time_in_force: Some(TimeInForce::Gtx),
            };
            let mut entry_params = build_futures_order_params(&entry_req);
            entry_params.insert("timestamp".to_owned(), SAMPLE_TS.to_string());

            let algo_req = PlaceAlgoOrderRequest {
                symbol: sym,
                side: Side::Sell,
                order_type: AlgoOrderType::StopMarket,
                client_algo_id: "dryrun_fut_sl_1".to_owned(),
                quantity: None,
                trigger_price: Some(sl_stop),
                activate_price: None,
                callback_rate: None,
                working_type: WorkingType::MarkPrice,
                close_position: true,
                reduce_only: false,
                price_protect: true,
            };
            let exit_map = build_place_algo_order_params(&algo_req)
                .map_err(|e| anyhow::anyhow!("Futures algo exit payload build failed: {e}"))?;
            let mut exit_params = json_map_to_btree(exit_map);
            exit_params.insert("timestamp".to_owned(), SAMPLE_TS.to_string());

            Ok(DryRunPayloads {
                entry: entry_params,
                exit: exit_params,
                ws_exit_method: "algoOrder.place",
            })
        }
    }
}

fn sign_and_verify_map(
    creds: &Ed25519Credentials,
    params: &BTreeMap<String, String>,
) -> Result<String> {
    let sig_b64 = creds.sign_params(params);
    let sig_bytes = BASE64_STANDARD
        .decode(sig_b64.as_bytes())
        .context("dry-run signature base64 decode failed")?;
    let sig = Signature::from_slice(&sig_bytes).context("invalid Ed25519 signature")?;
    let vk = VerifyingKey::from_bytes(&creds.public_key_bytes())
        .context("invalid Ed25519 verifying key")?;
    let payload = canonical_payload(params);
    vk.verify_strict(payload.as_bytes(), &sig)
        .context("dry-run order signature failed strict verification")?;
    Ok(payload)
}

/// Options for [`run_ws_logon_probe`].
#[derive(Debug, Clone)]
pub struct WsLogonProbeOptions {
    /// Target venue.
    pub venue: Venue,
    /// Optional explicit TOML configuration path.
    pub config: Option<PathBuf>,
    /// Optional WebSocket API URL override (tests).
    pub ws_url_override: Option<String>,
}

/// Result of `binance-botctl probe ws-logon`.
#[derive(Debug, Clone, Serialize)]
pub struct WsLogonProbeReport {
    /// Target venue.
    pub venue: Venue,
    /// WebSocket API endpoint probed.
    pub endpoint: String,
    /// `"session.logon"` (Spot) or `"per_request_ed25519"` (Futures).
    pub auth_mode: String,
    /// Whether authentication succeeded.
    pub authenticated: bool,
    /// Whether the session closed cleanly.
    pub closed_cleanly: bool,
    /// Explanatory note (e.g. Futures per-request signing model).
    pub note: String,
    /// Error summary when authentication failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Actionable remediation hint when authentication failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl WsLogonProbeReport {
    /// Render as concise human-readable text.
    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = format!(
            "probe ws-logon ({:?}) [{}]\n  endpoint: {}\n  auth_mode: {}\n  closed_cleanly: {}\n  note: {}",
            self.venue,
            if self.authenticated && self.closed_cleanly {
                "PASS"
            } else {
                "FAIL"
            },
            self.endpoint,
            self.auth_mode,
            self.closed_cleanly,
            self.note
        );
        if let Some(err) = &self.error {
            out.push_str("\n  error: ");
            out.push_str(err);
        }
        if let Some(hint) = &self.hint {
            out.push_str("\n  hint: ");
            out.push_str(hint);
        }
        out
    }
}

/// Probe WS-API authentication and immediately close the session cleanly.
///
/// - Spot (`/ws-api/v3`): connects via [`SpotWsSession`] (`session.logon`) and shuts down.
/// - Futures (`/ws-fapi/v1`): connects via [`FuturesWsSession`], runs read-only signed
///   `v2/account.balance` (since Futures WS API has no `session.logon`), and closes.
///
/// # Errors
/// Returns an error if local configuration or credentials cannot be loaded.
pub async fn run_ws_logon_probe(opts: &WsLogonProbeOptions) -> Result<WsLogonProbeReport> {
    let cfg = load_probe_config(opts.config.as_deref())?;
    let creds = Arc::new(load_venue_credentials(&cfg, opts.venue)?);
    verify_local_ed25519_signing(&creds)?;

    let endpoint =
        opts.ws_url_override
            .clone()
            .unwrap_or_else(|| match (opts.venue, cfg.general.mode) {
                (Venue::Spot, TradingMode::Live) => {
                    "wss://ws-api.binance.com:443/ws-api/v3".to_owned()
                }
                (Venue::Spot, _) => "wss://ws-api.testnet.binance.vision:443/ws-api/v3".to_owned(),
                (Venue::UsdsFutures, TradingMode::Live) => {
                    "wss://ws-fapi.binance.com/ws-fapi/v1".to_owned()
                }
                (Venue::UsdsFutures, _) => "wss://testnet.binancefuture.com/ws-fapi/v1".to_owned(),
            });

    match opts.venue {
        Venue::Spot => Ok(probe_spot_ws_logon(&cfg, opts, endpoint, creds).await),
        Venue::UsdsFutures => Ok(probe_futures_ws_auth(endpoint, creds).await),
    }
}

async fn probe_spot_ws_logon(
    cfg: &BotConfig,
    opts: &WsLogonProbeOptions,
    endpoint: String,
    creds: Arc<Ed25519Credentials>,
) -> WsLogonProbeReport {
    let offset_ms = if opts.ws_url_override.is_none() {
        let rest_url = resolve_rest_base_url(cfg, Venue::Spot, None);
        match SpotRestClient::new(&rest_url, PROBE_TIMEOUT) {
            Ok(rest) => rest.server_time().await.map_or(0, |s| s.offset_ms),
            Err(_) => 0,
        }
    } else {
        0
    };
    let ws_cfg = SpotSessionConfig {
        ws_url: endpoint.clone(),
        request_timeout: PROBE_TIMEOUT,
        recv_window_ms: Some(cfg.binance.recv_window_ms),
        ..SpotSessionConfig::default()
    };
    match SpotWsSession::connect(
        ws_cfg,
        creds,
        Arc::new(RateLimitTracker::new()),
        Arc::new(AtomicI64::new(offset_ms)),
    )
    .await
    {
        Ok((session, _events, _sub_id)) => {
            session.shutdown().await;
            WsLogonProbeReport {
                venue: Venue::Spot,
                endpoint,
                auth_mode: "session.logon".to_owned(),
                authenticated: true,
                closed_cleanly: true,
                note: "Spot WS API session.logon succeeded and session shut down cleanly"
                    .to_owned(),
                error: None,
                hint: None,
            }
        }
        Err(err) => WsLogonProbeReport {
            venue: Venue::Spot,
            endpoint,
            auth_mode: "session.logon".to_owned(),
            authenticated: false,
            closed_cleanly: true,
            note: "Spot WS API session.logon failed".to_owned(),
            error: Some(err.to_string()),
            hint: actionable_hint_for_error(&err).map(str::to_owned),
        },
    }
}

async fn probe_futures_ws_auth(
    endpoint: String,
    creds: Arc<Ed25519Credentials>,
) -> WsLogonProbeReport {
    let session = FuturesWsSession::new(
        endpoint.clone(),
        creds,
        Arc::new(RateLimitTracker::new()),
        PROBE_TIMEOUT,
    );
    if let Err(err) = session.connect().await {
        return WsLogonProbeReport {
            venue: Venue::UsdsFutures,
            endpoint,
            auth_mode: "per_request_ed25519".to_owned(),
            authenticated: false,
            closed_cleanly: true,
            note: "Futures WS API (/ws-fapi/v1) handshake failed".to_owned(),
            error: Some(err.to_string()),
            hint: actionable_hint_for_error(&err).map(str::to_owned),
        };
    }
    let res = session.request("v2/account.balance", BTreeMap::new()).await;
    session.close().await;
    match res {
        Ok(_) => WsLogonProbeReport {
            venue: Venue::UsdsFutures,
            endpoint,
            auth_mode: "per_request_ed25519".to_owned(),
            authenticated: true,
            closed_cleanly: true,
            note: "USD-M Futures WS API (/ws-fapi/v1) has no session.logon; verified per-request Ed25519 auth via read-only v2/account.balance"
                .to_owned(),
            error: None,
            hint: None,
        },
        Err(err) => WsLogonProbeReport {
            venue: Venue::UsdsFutures,
            endpoint,
            auth_mode: "per_request_ed25519".to_owned(),
            authenticated: false,
            closed_cleanly: true,
            note: "USD-M Futures WS API (/ws-fapi/v1) signed v2/account.balance failed".to_owned(),
            error: Some(err.to_string()),
            hint: actionable_hint_for_error(&err).map(str::to_owned),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use bot_exchange::{ExchangeErrorKind, GatewayError};
    use bot_types::Venue;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{SignedProbeOptions, actionable_hint_for_error, run_signed_probe};
    use crate::keygen::generate_ed25519_keypair;

    fn make_venue_config(mode: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let spot_pair = generate_ed25519_keypair(dir.path(), "spot").unwrap();
        let fut_pair = generate_ed25519_keypair(dir.path(), "futures").unwrap();
        let spot_key = dir.path().join("spot_key.txt");
        let fut_key = dir.path().join("fut_key.txt");
        fs::write(&spot_key, "spot-test-key\n").unwrap();
        fs::write(&fut_key, "futures-test-key\n").unwrap();

        let cfg = dir.path().join("bot.toml");
        fs::write(
            &cfg,
            format!(
                "[general]\nmode = \"{mode}\"\n[binance]\nmax_clock_drift_ms = 5000\n[venues.spot]\nenabled = true\napi_key_file = \"{}\"\nprivate_key_path = \"{}\"\n[venues.futures]\nenabled = true\napi_key_file = \"{}\"\nprivate_key_path = \"{}\"\n",
                spot_key.display(),
                spot_pair.private_path.display(),
                fut_key.display(),
                fut_pair.private_path.display(),
            ),
        )
        .unwrap();
        (dir, cfg)
    }

    #[test]
    fn actionable_hints_cover_2014_2015_1021_1022() {
        let cases = [
            (ExchangeErrorKind::ApiKeyFormatInvalid, -2014, "format"),
            (
                ExchangeErrorKind::InvalidApiKeyOrPermission,
                -2015,
                "permission",
            ),
            (ExchangeErrorKind::TimestampOutOfWindow, -1021, "clock"),
            (ExchangeErrorKind::InvalidSignature, -1022, "ed25519"),
        ];
        for (kind, code, keyword) in cases {
            let err = GatewayError::from_exchange_error(400, Some(code), "err".into(), None, None);
            let hint = actionable_hint_for_error(&err).expect("hint expected");
            assert!(
                hint.to_ascii_lowercase().contains(keyword),
                "kind={kind:?} hint={hint}"
            );
        }
    }

    #[tokio::test]
    async fn probe_signed_spot_detects_can_trade_false_via_mock_http() {
        let (_dir, cfg) = make_venue_config("testnet");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let now_ms = bot_exchange::spot::rest::now_unix_ms();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(buf.get(..n).unwrap());
                let body = if req.starts_with("GET /api/v3/time ") {
                    format!(r#"{{"serverTime":{now_ms}}}"#)
                } else {
                    r#"{"canTrade":false,"updateTime":100,"balances":[]}"#.to_owned()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            }
        });

        let report = run_signed_probe(&SignedProbeOptions {
            venue: Venue::Spot,
            config: Some(cfg),
            rest_url_override: Some(format!("http://{addr}")),
        })
        .await
        .unwrap();
        server.await.unwrap();

        assert!(report.local_sign_verify_ok);
        assert_eq!(report.can_trade, Some(false));
        assert!(!report.ok);
        assert!(
            report
                .hint
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains("cantrade")
        );
    }

    #[tokio::test]
    async fn probe_signed_futures_uses_fapi_v2_account_and_passes_when_can_trade_true() {
        let (_dir, cfg) = make_venue_config("testnet");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let now_ms = bot_exchange::spot::rest::now_unix_ms();
        let server = tokio::spawn(async move {
            let mut seen_v2 = false;
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(buf.get(..n).unwrap());
                let body = if req.starts_with("GET /fapi/v1/time ") {
                    format!(r#"{{"serverTime":{now_ms}}}"#)
                } else {
                    if req.starts_with("GET /fapi/v2/account?") {
                        seen_v2 = true;
                    }
                    r#"{"canTrade":true,"updateTime":100,"assets":[]}"#.to_owned()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            }
            seen_v2
        });

        let report = run_signed_probe(&SignedProbeOptions {
            venue: Venue::UsdsFutures,
            config: Some(cfg),
            rest_url_override: Some(format!("http://{addr}")),
        })
        .await
        .unwrap();
        let seen_v2 = server.await.unwrap();

        assert!(seen_v2, "Futures probe must call GET /fapi/v2/account");
        assert!(report.ok, "{report:?}");
        assert_eq!(report.can_trade, Some(true));
    }

    #[tokio::test]
    async fn probe_signed_maps_minus_1022_to_signature_hint() {
        let (_dir, cfg) = make_venue_config("testnet");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let now_ms = bot_exchange::spot::rest::now_unix_ms();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(buf.get(..n).unwrap());
                let (status, body) = if req.starts_with("GET /api/v3/time ") {
                    ("200 OK", format!(r#"{{"serverTime":{now_ms}}}"#))
                } else {
                    (
                        "400 Bad Request",
                        r#"{"code":-1022,"msg":"Signature for this request is not valid."}"#
                            .to_owned(),
                    )
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            }
        });

        let report = run_signed_probe(&SignedProbeOptions {
            venue: Venue::Spot,
            config: Some(cfg),
            rest_url_override: Some(format!("http://{addr}")),
        })
        .await
        .unwrap();
        server.await.unwrap();

        assert!(!report.ok);
        assert!(
            report
                .hint
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains("ed25519")
        );
    }

    #[tokio::test]
    async fn dry_run_order_offline_quantizes_and_verifies_signature_without_network() {
        use super::{DryRunOrderOptions, run_dry_run_order};
        use rust_decimal::Decimal;

        for (venue, expected_exit) in [
            (Venue::Spot, "orderList.place.oco"),
            (Venue::UsdsFutures, "algoOrder.place"),
        ] {
            let report = run_dry_run_order(&DryRunOrderOptions {
                venue,
                symbol: "BTCUSDT".to_owned(),
                price: Some(Decimal::new(50_000_123, 3)),
                qty: Some(Decimal::new(12_345, 6)),
                offline: true,
                config: None,
                rest_url_override: None,
            })
            .await
            .unwrap();
            assert!(!report.transmitted);
            assert!(report.signature_verified_locally);
            assert_eq!(report.ws_exit_method, expected_exit);
            assert!(
                !report
                    .rest_entry_query_preview
                    .contains("BEGIN PRIVATE KEY")
            );
            assert!(report.notional_usd >= Decimal::from(10));
        }
    }

    #[tokio::test]
    async fn dry_run_order_rejects_notional_below_min() {
        use super::{DryRunOrderOptions, run_dry_run_order};
        use rust_decimal::Decimal;

        let err = run_dry_run_order(&DryRunOrderOptions {
            venue: Venue::Spot,
            symbol: "BTCUSDT".to_owned(),
            price: Some(Decimal::from(100)),
            qty: Some(Decimal::new(1, 5)),
            offline: true,
            config: None,
            rest_url_override: None,
        })
        .await
        .unwrap_err();
        assert!(
            format!("{err:#}").to_ascii_lowercase().contains("notional"),
            "{err:#}"
        );
    }

    #[tokio::test]
    async fn ws_logon_probe_spot_succeeds_and_closes_cleanly_against_mock_ws() {
        use super::{WsLogonProbeOptions, run_ws_logon_probe};
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let (_dir, cfg) = make_venue_config("testnet");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(Message::Text(text))) = ws.next().await {
                let req: serde_json::Value = serde_json::from_str(&text).unwrap();
                let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
                let method = req
                    .get("method")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let result = if method == "userDataStream.subscribe" {
                    serde_json::json!({"subscriptionId": 1})
                } else {
                    serde_json::json!({"authorizedSince": 1_700_000_000_000_i64})
                };
                let resp = serde_json::json!({"id": id, "status": 200, "result": result});
                let _ = ws.send(Message::Text(resp.to_string().into())).await;
            }
        });

        let report = run_ws_logon_probe(&WsLogonProbeOptions {
            venue: Venue::Spot,
            config: Some(cfg),
            ws_url_override: Some(format!("ws://{addr}")),
        })
        .await
        .unwrap();
        server.abort();

        assert!(report.authenticated);
        assert!(report.closed_cleanly);
        assert_eq!(report.auth_mode, "session.logon");
    }

    #[tokio::test]
    async fn ws_logon_probe_futures_uses_per_request_v2_account_balance_against_mock_ws() {
        use super::{WsLogonProbeOptions, run_ws_logon_probe};
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let (_dir, cfg) = make_venue_config("testnet");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut saw_balance = false;
            if let Some(Ok(Message::Text(text))) = ws.next().await {
                let req: serde_json::Value = serde_json::from_str(&text).unwrap();
                let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
                let method = req
                    .get("method")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let has_sig = req
                    .get("params")
                    .and_then(|p| p.get("signature"))
                    .and_then(serde_json::Value::as_str)
                    .is_some();
                saw_balance = method == "v2/account.balance" && has_sig;
                let resp = serde_json::json!({"id": id, "status": 200, "result": []});
                let _ = ws.send(Message::Text(resp.to_string().into())).await;
            }
            saw_balance
        });

        let report = run_ws_logon_probe(&WsLogonProbeOptions {
            venue: Venue::UsdsFutures,
            config: Some(cfg),
            ws_url_override: Some(format!("ws://{addr}")),
        })
        .await
        .unwrap();
        let saw_balance = server.await.unwrap();

        assert!(saw_balance);
        assert!(report.authenticated);
        assert!(report.closed_cleanly);
        assert_eq!(report.auth_mode, "per_request_ed25519");
        assert!(report.note.contains("session.logon"));
    }
}
