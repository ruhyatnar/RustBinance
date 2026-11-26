# M19: CLI Diagnostik, Key Audit & Pra-Live Probes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Equip `binance-botctl` with non-destructive pre-live diagnostic commands (`key fingerprint`/`inspect`, `doctor`, `probe signed`, `probe dry-run-order`, `probe ws-logon`, and `dry-run signal`) sharing the exact `bot-exchange`, `bot-config`, `bot-indicators`, and `bot-strategy` code paths used by `binance-botd`.

**Architecture:** Extend `bot-exchange` minimally with read-only signed Spot account probe (`GET /api/v3/account`), Futures account tradeability inspection (`GET /fapi/v2/account`), and public kline REST fetchers (`GET /api/v3/klines`, `GET /fapi/v1/klines`). Implement all M19 CLI subcommands as focused, offline-testable modules in `crates/botctl/src/` with structured JSON & human-readable output, actionable Binance error hints, and zero order transmission.

**Tech Stack:** Rust 2024 edition (MSRV 1.89), Tokio, `clap`, `ed25519-dalek`, `sha2` (via `ed25519-dalek` / workspace or `sha2`), `reqwest` (via `bot-exchange`), `rust_decimal`, `serde_json`, `tempfile` (dev), local mock HTTP/WS servers for 100% offline tests.

**Spec:** `docs/superpowers/specs/2026-11-26-m18-m21-prelive-ops-design.md` (§5 M19) and `docs/verified-facts.md` (M19 entry)

## Global Constraints

- `#![forbid(unsafe_code)]`; no `unwrap`, `expect`, `panic`, or unchecked indexing in non-test code (`clippy -D warnings`, pedantic warn).
- **Zero secret leakage:** private key PEM contents, raw 32-byte seeds, and full API keys must **never** appear in stdout, stderr, JSON output, error messages, or logs. All private key material must stay inside `Secret<T>` / `Zeroizing<T>`.
- **Zero order transmission:** `probe signed`, `probe dry-run-order`, `probe ws-logon`, and `dry-run signal` must never send an order to Binance (live or testnet).
- All monetary/price/qty calculations use `rust_decimal::Decimal` (`f64` only inside `bot-indicators` / `bot-strategy`).
- Default `cargo test --workspace` must be 100% offline (no external Binance network calls; use mock HTTP/WS servers and offline fixtures).
- `probe signed --venue futures` uses `GET /fapi/v2/account` (verified to include `canTrade`; `/fapi/v3/account` omits `canTrade`).
- `probe ws-logon --venue futures` connects to `/ws-fapi/v1` and executes read-only signed `v2/account.balance` (Futures WS API has no `session.logon`), explicitly reporting per-request signing mode.
- Final gate: `scripts/verify.sh` must pass completely.

## Review Focus

1. **Private key leakage in `key inspect` / `key fingerprint` error or JSON output:** Passing a private key PEM (or malformed PEM containing secret-like lines) must never echo raw PEM content in stdout, JSON, or error messages — pinned in Task 1.
2. **Loose Unix file permissions silently accepted:** A private key with mode `0644` or parent directory with mode `0755` must produce a `FAIL` status in both `key inspect` and `doctor` (causing non-zero exit code), while `0600` key in `0700` dir passes — pinned in Task 1 & Task 3.
3. **`doctor` exit code & partial failure reporting:** When one check fails (e.g. missing `live_confirmation_file` in `mode=live` or unreachable daemon UDS) while others pass, `doctor` must still run remaining independent checks, print the full matrix (text or `--json`), and return a non-zero exit status — pinned in Task 3.
4. **Actionable error hints & `canTrade=false` detection in `probe signed`:** Binance error codes `-2014`, `-2015`, `-1021`, `-1022` and a `200 OK` response with `canTrade: false` must each map to a distinct, actionable remediation hint and fail the probe — pinned in Task 4.
5. **Filter quantization failure in `probe dry-run-order`:** When a synthetic order violates `LOT_SIZE`, `PRICE_FILTER`, or `NOTIONAL` (e.g. notional below `min_notional`), `probe dry-run-order` must surface the exact `RulesError` rather than panicking or silently emitting an invalid payload, and must never open a network socket when `--offline` is set — pinned in Task 5.

---

## File Structure

- Modify: `crates/bot-exchange/src/spot/rest.rs` — add `SpotAccountProbeInfo`, `SpotRestClient::account_probe(&self, creds: &Ed25519Credentials, recv_window_ms: u64)` (`GET /api/v3/account`), and `SpotRestClient::klines(&self, symbol: &str, interval: &str, limit: u16)` (`GET /api/v3/klines`).
- Modify: `crates/bot-exchange/src/futures/rest.rs` — add `FuturesAccountProbeInfo`, `FuturesRestClient::account_probe(&self, recv_window_ms: u64)` (`GET /fapi/v2/account` extracting `canTrade`, `totalWalletBalance`, `availableBalance`), and `FuturesRestClient::klines(&self, symbol: &str, interval: &str, limit: u16)` (`GET /fapi/v1/klines`).
- Modify: `crates/bot-exchange/src/lib.rs` — re-export `SpotAccountProbeInfo` and `FuturesAccountProbeInfo` (plus `FuturesRestClient`, `FuturesWsSession`).
- Modify: `crates/botctl/Cargo.toml` — add dependencies on `bot-exchange`, `bot-indicators`, `bot-strategy`, `rust_decimal`, `sha2` (reuse workspace/lock versions).
- Create: `crates/botctl/src/key_audit.rs` — `KeyInspection`, `inspect_key_file(path: &Path)`, `fingerprint_key_file(path: &Path)` (supports both SPKI public PEM and PKCS#8 private PEM, SHA-256 fingerprint, Unix permission checks `0600`/`0700`).
- Create: `crates/botctl/src/doctor.rs` — `CheckStatus` (`Pass | Warn | Fail`), `DoctorCheck`, `DoctorReport`, `run_doctor(opts: &DoctorOptions)` running config validation, key audit, `live_confirmation_file` check, REST `/ping` + clock offset, daemon UDS status, and binary version.
- Create: `crates/botctl/src/probes.rs` — `actionable_hint_for_error(&GatewayError)`, `SignedProbeReport`, `run_signed_probe(...)`, `DryRunOrderReport`, `run_dry_run_order(...)`, `WsLogonProbeReport`, `run_ws_logon_probe(...)`.
- Create: `crates/botctl/src/dry_run_signal.rs` — `SignalGateBreakdown`, `DryRunSignalReport`, `run_dry_run_signal(...)` feeding `IndicatorFeed` + `HtfRegimeFeed` + `S1TrendPullback`/`S2Breakout` and printing gate breakdown without starting daemon or sending orders.
- Modify: `crates/botctl/src/cli.rs` — add `Key { Fingerprint, Inspect }`, `Doctor`, `Probe { Signed, DryRunOrder, WsLogon }`, and `DryRun { Signal }` subcommands.
- Modify: `crates/botctl/src/lib.rs` — wire all new subcommands and ensure `doctor` / failed probes return `Err` (non-zero exit) while still allowing structured output.
- Create: `crates/botctl/tests/m19_cli_probes.rs` — end-to-end offline integration tests with local mock HTTP & WS servers and fixtures.

---

### Task 1: Key Audit (`binance-botctl key fingerprint` & `key inspect`)

**Files:**
- Modify: `crates/botctl/Cargo.toml`
- Create: `crates/botctl/src/key_audit.rs`
- Modify: `crates/botctl/src/cli.rs`
- Modify: `crates/botctl/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub enum KeyFileKind { PublicSpki, PrivatePkcs8 }`
  - `pub struct KeyInspection { pub path: PathBuf, pub kind: KeyFileKind, pub pem_valid: bool, pub sha256_fingerprint: String, pub file_mode_octal: String, pub file_mode_ok: bool, pub dir_mode_octal: String, pub dir_mode_ok: bool, pub issues: Vec<String> }`
  - `pub fn inspect_key_file(path: &Path) -> Result<KeyInspection>`
  - `pub fn fingerprint_key_file(path: &Path) -> Result<String>` (format: `SHA256:<lowercase-hex>` over raw 32-byte Ed25519 public key)
  - CLI: `binance-botctl key fingerprint --file <path> [--json]` and `binance-botctl key inspect --file <path> [--json]`

- [ ] **Step 1: Write failing unit tests in `crates/botctl/src/key_audit.rs`**

Test cases to pin:
- `fingerprint_matches_between_private_and_public_pem`: generate a keypair via `generate_ed25519_keypair` in a `0700` tempdir; assert `fingerprint_key_file(priv)` == `fingerprint_key_file(pub)` and starts with `"SHA256:"` (71 chars total).
- `inspect_accepts_0600_private_key_in_0700_dir`: assert `file_mode_ok == true`, `dir_mode_ok == true`, `issues.is_empty()`.
- `inspect_flags_loose_private_key_and_dir_permissions`: chmod private key to `0644` and dir to `0755`; assert `file_mode_ok == false`, `dir_mode_ok == false`, `issues.len() >= 2`.
- `inspect_and_error_never_leak_private_pem_content`: serialize `KeyInspection` to JSON and also inspect a malformed PEM containing `"SECRET_SENTINEL_DO_NOT_LEAK"`; assert neither JSON output nor `Err` string contains `"BEGIN PRIVATE KEY"` or `"SECRET_SENTINEL_DO_NOT_LEAK"`.

- [ ] **Step 2: Run tests to verify failure**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p botctl key_audit`
Expected: FAIL (module not yet implemented).

- [ ] **Step 3: Implement `key_audit.rs` and wire `Command::Key` in `cli.rs` & `lib.rs`**

Add `sha2` to `crates/botctl/Cargo.toml` (check `Cargo.lock` for exact version already in workspace). Read key file into `Zeroizing<String>`, try `VerifyingKey::from_public_key_pem`, fallback to `SigningKey::from_pkcs8_pem` (deriving `verifying_key()` without exposing private bytes), hash the 32-byte public key with `Sha256`, and check Unix `std::os::unix::fs::PermissionsExt::mode() & 0o777` (`0o600` for private key, `0o700` for parent dir when inspecting a private key). Wire `Command::Key { subcommand: KeySubcommand::{Fingerprint, Inspect} }` in `cli.rs` and `lib.rs`.

- [ ] **Step 4: Run tests and clippy**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p botctl && PATH=$HOME/.cargo/bin:$PATH cargo clippy -p botctl --all-targets -- -D warnings`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/botctl/Cargo.toml crates/botctl/src/key_audit.rs crates/botctl/src/cli.rs crates/botctl/src/lib.rs Cargo.lock
git commit -m "feat(botctl): add key fingerprint and key inspect audit commands"
```

---

### Task 2: Read-Only Account Probe & Public Kline REST Helpers (`bot-exchange`)

**Files:**
- Modify: `crates/bot-exchange/src/spot/rest.rs`
- Modify: `crates/bot-exchange/src/futures/rest.rs`
- Modify: `crates/bot-exchange/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct SpotAccountProbeInfo { pub can_trade: bool, pub can_withdraw: bool, pub can_deposit: bool, pub account_type: String, pub permissions: Vec<String>, pub non_zero_balances: usize }`
  - `pub struct FuturesAccountProbeInfo { pub can_trade: bool, pub can_deposit: bool, pub can_withdraw: bool, pub fee_tier: i64, pub total_wallet_balance: Decimal, pub available_balance: Decimal, pub open_positions_count: usize }`
  - `SpotRestClient::account_probe(&self, creds: &Ed25519Credentials, recv_window_ms: u64) -> Result<SpotAccountProbeInfo, GatewayError>` (`GET /api/v3/account`, signed with `X-MBX-APIKEY` + Ed25519 `signature`)
  - `SpotRestClient::klines(&self, symbol: &str, interval: &str, limit: u16) -> Result<Vec<Kline>, GatewayError>` (`GET /api/v3/klines`)
  - `FuturesRestClient::account_probe(&self, recv_window_ms: u64) -> Result<FuturesAccountProbeInfo, GatewayError>` (`GET /fapi/v2/account`)
  - `FuturesRestClient::klines(&self, symbol: &str, interval: &str, limit: u16) -> Result<Vec<Kline>, GatewayError>` (`GET /fapi/v1/klines`)

- [ ] **Step 1: Write failing unit tests in `spot/rest.rs` and `futures/rest.rs`**

Test cases to pin:
- `parse_spot_account_probe_extracts_can_trade_and_non_zero_balances`: test both `canTrade: true` and `canTrade: false`, counting only balances where `free > 0 || locked > 0`.
- `parse_futures_account_probe_extracts_can_trade_and_balances`: parse `/fapi/v2/account` fixture with `canTrade: false`, `totalWalletBalance`, `availableBalance`, and `positions[]` (counting non-zero `positionAmt`).
- `parse_rest_klines_parses_12_column_array`: parse Binance 12-element kline JSON array into `Vec<bot_types::Kline>` with exact `Decimal` OHLCV and `is_closed = true` for historical bars.
- Mock-HTTP test verifying `SpotRestClient::account_probe` attaches `X-MBX-APIKEY`, `timestamp`, `recvWindow`, and valid Ed25519 `signature`.

- [ ] **Step 2: Run tests to verify failure**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p bot-exchange account_probe`
Expected: FAIL (methods not defined).

- [ ] **Step 3: Implement account probe & kline REST methods in `spot/rest.rs`, `futures/rest.rs`, and re-export in `lib.rs`**

Reuse `ClockSyncState::adjusted_now_ms`, `Ed25519Credentials::sign_params`, `canonical_payload`, and existing `check_rest_response` / `check_response` (which also ingests REST rate-limit headers and classifies `-2014`/`-2015`/`-1021`/`-1022` into `GatewayError`).

- [ ] **Step 4: Run tests and clippy**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p bot-exchange && PATH=$HOME/.cargo/bin:$PATH cargo clippy -p bot-exchange --all-targets -- -D warnings`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/bot-exchange/src/spot/rest.rs crates/bot-exchange/src/futures/rest.rs crates/bot-exchange/src/lib.rs
git commit -m "feat(bot-exchange): add read-only account_probe and public klines REST helpers"
```

---

### Task 3: Diagnostic Matrix (`binance-botctl doctor`)

**Files:**
- Modify: `crates/botctl/Cargo.toml`
- Create: `crates/botctl/src/doctor.rs`
- Modify: `crates/botctl/src/cli.rs`
- Modify: `crates/botctl/src/lib.rs`

**Interfaces:**
- Consumes: `key_audit::inspect_key_file`, `SpotRestClient::{ping, server_time}`, `FuturesRestClient::{ping, server_time}`, `UdsClient::connect`, `bot_config::load_from_paths`
- Produces:
  - `pub enum CheckStatus { Pass, Warn, Fail }`
  - `pub struct DoctorCheck { pub name: String, pub status: CheckStatus, pub detail: String, pub hint: Option<String> }`
  - `pub struct DoctorReport { pub binary_version: String, pub overall_ok: bool, pub checks: Vec<DoctorCheck> }`
  - `pub struct DoctorOptions { pub config: Option<PathBuf>, pub socket: PathBuf, pub spot_rest_url: Option<String>, pub futures_rest_url: Option<String>, pub offline: bool }`
  - `pub async fn run_doctor(opts: &DoctorOptions) -> DoctorReport`
  - CLI: `binance-botctl doctor [--config <path>] [--socket <path>] [--offline] [--json]`

- [ ] **Step 1: Write failing unit/integration tests in `crates/botctl/src/doctor.rs`**

Test cases to pin:
- `doctor_passes_with_valid_paper_config_and_0600_keys_offline`: valid `mode=paper` TOML + `0600` key in `0700` dir + `--offline` → `config` & `keys` checks `Pass`; daemon check `Warn` when socket absent in offline/stopped state (or `Fail` when explicitly required — pin exact rule: missing daemon socket is `Warn` when `mode=paper` and `Fail` when `mode=live`, or `Fail` if socket exists but unreadable).
- `doctor_fails_when_live_mode_missing_confirmation_file_and_runs_other_checks`: `mode=live` TOML with missing `live_confirmation_file` + loose `0644` key → `overall_ok == false`, both `live_confirmation` and `key_permissions` appear as `Fail` in `report.checks` (no short-circuit).
- `doctor_checks_rest_ping_and_clock_offset_against_mock_server`: point `spot_rest_url` to local mock HTTP server returning `/api/v3/ping` 200 and `/api/v3/time` with `offset_ms > max_clock_drift_ms` → `clock_offset` check is `Fail` with NTP hint.

- [ ] **Step 2: Run tests to verify failure**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p botctl doctor`
Expected: FAIL.

- [ ] **Step 3: Implement `doctor.rs` and wire `Command::Doctor` in `cli.rs` & `lib.rs`**

Checks executed in order (never short-circuiting independent checks):
1. `binary_version` (`env!("CARGO_PKG_VERSION")`)
2. `config` (`load_from_paths` / `load_config`)
3. `key_files` (for each enabled venue: API key file exists & non-empty + `inspect_key_file` on `private_key_path` checking `0600`/`0700`)
4. `live_confirmation` (if `mode == Live`: verify non-empty `live_confirmation_file` for each enabled venue; in `Paper`/`Testnet`: `Pass` with note `"not required in paper/testnet"`)
5. `rest_connectivity` & `clock_offset` (unless `--offline`: `/ping` + `/time` against testnet/prod URL per `mode`, failing if `offset_ms.abs() > max_clock_drift_ms`)
6. `daemon_uds` (attempt `UdsClient::connect(&opts.socket)` with short timeout; report engine mode & active venue if running)

When `!report.overall_ok`, `run_cli` prints the report (human or `--json`) and exits non-zero (return a dedicated error or exit-code wrapper in `main.rs` so JSON output is still cleanly printed to stdout without corrupting JSON!).

- [ ] **Step 4: Run tests and clippy**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p botctl && PATH=$HOME/.cargo/bin:$PATH cargo clippy -p botctl --all-targets -- -D warnings`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/botctl/Cargo.toml crates/botctl/src/doctor.rs crates/botctl/src/cli.rs crates/botctl/src/lib.rs crates/botctl/src/main.rs
git commit -m "feat(botctl): add binance-botctl doctor pre-live diagnostic matrix"
```

---

### Task 4: Signed Account Probe (`binance-botctl probe signed`) & Actionable Hints

**Files:**
- Create: `crates/botctl/src/probes.rs`
- Modify: `crates/botctl/src/cli.rs`
- Modify: `crates/botctl/src/lib.rs`

**Interfaces:**
- Consumes: `Ed25519Credentials::from_pkcs8_pem_file`, `SpotRestClient::{server_time, account_probe}`, `FuturesRestClient::{server_time, account_probe}`
- Produces:
  - `pub fn actionable_hint_for_error(err: &GatewayError) -> Option<&'static str>` (covers `-2014`, `-2015`, `-1021`, `-1022`)
  - `pub struct SignedProbeReport { pub venue: Venue, pub endpoint: String, pub local_sign_verify_ok: bool, pub clock_offset_ms: i64, pub clock_within_limit: bool, pub can_trade: Option<bool>, pub ok: bool, pub error: Option<String>, pub hint: Option<String> }`
  - `pub async fn run_signed_probe(...) -> Result<SignedProbeReport>`
  - CLI: `binance-botctl probe signed [--venue spot|futures] [--config <path>] [--json]`

- [ ] **Step 1: Write failing tests in `crates/botctl/src/probes.rs`**

Test cases to pin:
- `actionable_hints_cover_2014_2015_1021_1022`: assert each `ExchangeErrorKind` (`ApiKeyFormatInvalid`, `InvalidApiKeyOrPermission`, `TimestampOutOfWindow`, `InvalidSignature`) returns a non-empty actionable hint mentioning the root cause.
- `probe_signed_spot_detects_can_trade_false_via_mock_http`: mock `/api/v3/time` + `/api/v3/account` returning `canTrade: false` → `local_sign_verify_ok == true`, `can_trade == Some(false)`, `ok == false`, `hint` mentions enabling Spot trading permission.
- `probe_signed_futures_uses_fapi_v2_account_and_passes_when_can_trade_true`: mock `/fapi/v1/time` + `/fapi/v2/account` returning `canTrade: true` → `ok == true`, `can_trade == Some(true)`.
- `probe_signed_maps_minus_1022_to_signature_hint`: mock returns HTTP 400 `{"code":-1022,"msg":"Signature for this request is not valid."}` → `ok == false`, `hint` mentions Ed25519 public key registration on Binance.

- [ ] **Step 2: Run tests to verify failure**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p botctl probes`
Expected: FAIL.

- [ ] **Step 3: Implement `run_signed_probe` and `actionable_hint_for_error` in `probes.rs` and wire `Command::Probe { Signed }` in `cli.rs` & `lib.rs`**

Perform local Ed25519 sign+verify self-test using `Ed25519Credentials::sign_params` + `VerifyingKey::verify_strict` before making any network call. Sync server time first, verify `offset_ms.abs() <= max_clock_drift_ms`, then invoke `account_probe`. Never print API key or private key.

- [ ] **Step 4: Run tests and clippy**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p botctl && PATH=$HOME/.cargo/bin:$PATH cargo clippy -p botctl --all-targets -- -D warnings`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/botctl/src/probes.rs crates/botctl/src/cli.rs crates/botctl/src/lib.rs
git commit -m "feat(botctl): add binance-botctl probe signed with actionable hints"
```

---

### Task 5: Offline-Capable Order Dry-Run (`probe dry-run-order`) & WS-API Auth Probe (`probe ws-logon`)

**Files:**
- Modify: `crates/botctl/src/probes.rs`
- Modify: `crates/botctl/src/cli.rs`
- Modify: `crates/botctl/src/lib.rs`

**Interfaces:**
- Consumes: `InstrumentRules` (`round_price_down`, `round_qty_down`, `validate_order`), `Ed25519Credentials`, `SpotWsSession`, `FuturesWsSession`, `bot_exchange::spot::wire`, `bot_exchange::futures::rest::build_place_order_params`
- Produces:
  - `pub struct DryRunOrderReport { pub venue: Venue, pub symbol: String, pub quantized_price: Decimal, pub quantized_qty: Decimal, pub notional_usd: Decimal, pub rest_entry_query_preview: String, pub ws_entry_method: String, pub ws_exit_method: String, pub signature_verified_locally: bool, pub transmitted: bool }` (`transmitted` is always `false`)
  - `pub async fn run_dry_run_order(...) -> Result<DryRunOrderReport>`
  - `pub struct WsLogonProbeReport { pub venue: Venue, pub endpoint: String, pub auth_mode: String, pub authenticated: bool, pub closed_cleanly: bool, pub note: String }`
  - `pub async fn run_ws_logon_probe(...) -> Result<WsLogonProbeReport>`
  - CLI: `binance-botctl probe dry-run-order [--venue spot|futures] [--symbol BTCUSDT] [--offline] [--config <path>] [--json]` and `binance-botctl probe ws-logon [--venue spot|futures] [--config <path>] [--json]`

- [ ] **Step 1: Write failing tests in `crates/botctl/src/probes.rs`**

Test cases to pin:
- `dry_run_order_offline_quantizes_and_verifies_signature_without_network`: `--offline` for Spot and Futures `BTCUSDT` → `transmitted == false`, `signature_verified_locally == true`, `rest_entry_query_preview` contains redacted/no secret key, `ws_exit_method` is `"orderList.place.oco"` (Spot) or `"algoOrder.place"` (Futures).
- `dry_run_order_rejects_notional_below_min`: synthetic price/qty below `min_notional` → returns `Err` mentioning `NotionalBelowMin` (no panic).
- `ws_logon_probe_spot_succeeds_and_closes_cleanly_against_mock_ws`: local `tokio-tungstenite` mock WS server accepting `session.logon` and `userDataStream.subscribe` (or clean close) → `authenticated == true`, `closed_cleanly == true`, `auth_mode == "session.logon"`.
- `ws_logon_probe_futures_uses_per_request_v2_account_balance_against_mock_ws`: local mock WS server expecting `method == "v2/account.balance"` with `apiKey` + `signature` → `authenticated == true`, `closed_cleanly == true`, `auth_mode == "per_request_ed25519"`, `note` explains Futures WS API has no `session.logon`.

- [ ] **Step 2: Run tests to verify failure**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p botctl dry_run_order ws_logon`
Expected: FAIL.

- [ ] **Step 3: Implement `run_dry_run_order` and `run_ws_logon_probe` in `probes.rs` and wire CLI**

In `run_dry_run_order`, when `--offline` is set, use built-in deterministic `InstrumentRules` fixture for the requested symbol (and ephemeral in-memory Ed25519 key if no key file is configured) so it runs without any network socket. Never transmit any order over the network.

- [ ] **Step 4: Run tests and clippy**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p botctl && PATH=$HOME/.cargo/bin:$PATH cargo clippy -p botctl --all-targets -- -D warnings`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/botctl/src/probes.rs crates/botctl/src/cli.rs crates/botctl/src/lib.rs
git commit -m "feat(botctl): add probe dry-run-order and probe ws-logon commands"
```

---

### Task 6: Standalone Signal Dry-Run (`binance-botctl dry-run signal`) & Integration Suite

**Files:**
- Create: `crates/botctl/src/dry_run_signal.rs`
- Modify: `crates/botctl/src/cli.rs`
- Modify: `crates/botctl/src/lib.rs`
- Create: `crates/botctl/tests/m19_cli_probes.rs`

**Interfaces:**
- Consumes: `SpotRestClient::klines`, `FuturesRestClient::klines`, `IndicatorFeed`, `HtfRegimeFeed`, `S1TrendPullback`, `S2Breakout`, `Strategy::evaluate`
- Produces:
  - `pub struct StrategyGateBreakdown { pub strategy: String, pub decision: String, pub gates: Vec<GateLog> }`
  - `pub struct DryRunSignalReport { pub venue: Venue, pub symbol: String, pub bars_loaded: usize, pub latest_close: Decimal, pub strategies: Vec<StrategyGateBreakdown>, pub order_sent: bool }` (`order_sent` is always `false`)
  - `pub async fn run_dry_run_signal(...) -> Result<DryRunSignalReport>`
  - CLI: `binance-botctl dry-run signal --symbol <SYM> [--venue spot|futures] [--offline] [--config <path>] [--json]`

- [ ] **Step 1: Write failing tests in `crates/botctl/tests/m19_cli_probes.rs`**

Test cases to pin:
- `dry_run_signal_offline_evaluates_s1_and_s2_with_gate_breakdown`: deterministic synthetic 650-bar 5m + 1h series → `bars_loaded >= 600`, `strategies` contains both `S1` and `S2` with non-empty `GateLog` breakdown, `order_sent == false`.
- `dry_run_signal_fetches_public_klines_from_mock_rest`: mock `/api/v3/klines` (Spot) and `/fapi/v1/klines` (Futures) → returns valid `DryRunSignalReport` JSON.
- `cli_end_to_end_json_output_is_valid_json_for_all_m19_commands`: run `run_cli` with `--json` for `key fingerprint`, `key inspect`, `doctor --offline`, `probe dry-run-order --offline`, and `dry-run signal --offline`, asserting valid JSON and zero secret leakage.

- [ ] **Step 2: Run tests to verify failure**

Run: `PATH=$HOME/.cargo/bin:$PATH cargo test -p botctl --test m19_cli_probes`
Expected: FAIL.

- [ ] **Step 3: Implement `dry_run_signal.rs`, wire `Command::DryRun { Signal }` in `cli.rs` & `lib.rs`, and update `docs/STATUS.md` + `CHANGELOG.md`**

Feed fetched (or offline fixture) 5m and 1h klines into `IndicatorFeed` and `HtfRegimeFeed`, evaluate `S1TrendPullback` and `S2Breakout` with `StrategyCtx`, and serialize the gate-by-gate `GateLog` breakdown.

- [ ] **Step 4: Run full verification gate**

Run: `PATH=$HOME/.cargo/bin:$PATH ./scripts/verify.sh`
Expected: `OK: semua pemeriksaan lulus` (exit 0).

- [ ] **Step 5: Commit & update graphify**

```bash
git add crates/botctl/ docs/STATUS.md CHANGELOG.md
git commit -m "feat(botctl): add dry-run signal gate breakdown and M19 integration suite"
~/.local/bin/graphify update .
```
