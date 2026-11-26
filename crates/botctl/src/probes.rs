//! Pre-live exchange probes (`binance-botctl probe signed|dry-run-order|ws-logon`, M19).
//!
//! Strictly read-only / non-destructive: zero orders are ever transmitted to Binance,
//! and secret credentials never appear in output or error messages.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use bot_config::{BotConfig, ConfigSourcePaths, load_config, load_from_paths};
use bot_exchange::auth::canonical_payload;
use bot_exchange::{
    Ed25519Credentials, ExchangeErrorKind, FuturesRestClient, GatewayError, RateLimitTracker,
    SpotRestClient,
};
use bot_types::{TradingMode, Venue};
use ed25519_dalek::{Signature, VerifyingKey};
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
        load_from_paths(&ConfigSourcePaths {
            base_file: Some(path.to_path_buf()),
            conf_d_dir: None,
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
            let ok = clock_within_limit && snap.can_trade;
            Ok(SignedProbeReport {
                venue: opts.venue,
                endpoint,
                local_sign_verify_ok: true,
                clock_offset_ms,
                clock_within_limit,
                can_trade: Some(snap.can_trade),
                non_zero_balances: snap.balances.len(),
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
}
