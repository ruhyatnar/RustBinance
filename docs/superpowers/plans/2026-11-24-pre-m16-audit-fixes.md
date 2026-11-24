# Pre-M16 Audit Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close all Must Fix (#1–#3) and Should Fix (#4–#6) audit findings before M16 (`VenueManager`), wiring `botd` IPC to `EngineActor`, hardening UDS peer auth and config/mode guards, implementing WS-first + REST-fallback transport in `FuturesAdapter`, and cleaning up duplicate docs.

**Architecture:** Six self-contained tasks across `bot-ipc`, `botd`, `bot-exchange`, and `docs/`. Part 1 hardens the daemon boundary (`bot-ipc` + `botd`), Part 2 adds `FuturesRestClient` (`/fapi/v1`) and `FuturesWsSession` (`/ws-fapi/v1`) to `FuturesAdapter` mirroring `SpotAdapter`, and Part 3 removes duplicate `binance-bot-agent-kit/` and updates `docs/STATUS.md` + `CHANGELOG.md`.

**Tech Stack:** Rust 1.89+ (edition 2024), `tokio`, `reqwest`, `tokio-tungstenite`, `ed25519-dalek`, `rust_decimal`, `serde_json`, `thiserror`, `tracing`.

**Spec:** `docs/superpowers/specs/2026-11-24-pre-m16-audit-fixes-design.md`

## Global Constraints
- `#![forbid(unsafe_code)]` across all Rust crates; zero `unwrap()`, `expect()`, `panic!()`, or unchecked indexing in non-test code (`clippy -D warnings`).
- All prices, quantities, balances, and PnL use `rust_decimal::Decimal`.
- Secrets (`Ed25519Credentials`, bearer tokens) flow only via `Secret<T>` and never appear in logs or errors.
- No live network calls in default `cargo test`; all HTTP/WS transport tests use local mock servers (`tokio::net::TcpListener`).
- Never cancel protective OCO/Algo orders on `deactivate()` or `SIGTERM`.

## Review Focus
- `/proc/<pid>/status` missing or unreadable (e.g. short-lived peer process): `peer_supplementary_groups(pid)` must return an empty `Vec` and fall back cleanly to UID/primary GID check without panicking.
- `EngineIpcBackend::close_position` / `set_param` / `disable_strategy` when `EngineHandle` does not yet expose that mutation: must return `IpcError::BadRequest` or `IpcError::Internal` clearly, never silently succeed like the old stub.
- `conf.d` directory missing beside `bot.toml`: `botd` must start normally with `conf_d_dir: None`, and only pass `Some(conf_d)` when the directory exists.
- `FuturesWsSession` disconnects or returns transport error mid-request: `FuturesAdapter` must fall back to `FuturesRestClient` and never lose rate-limit tracking.
- `FuturesAdapter::activate()` when exchange reports Hedge Mode, Multi-Assets ON, Cross margin, leverage > cap, or non-zero position: must return `GatewayError::InvalidRequest` without mutating exchange state.

---

### Task 1: UDS Supplementary Groups in `PeerAuthPolicy` (`#3`)

**Files:**
- Modify: `crates/bot-ipc/src/auth.rs:242-300`
- Modify: `crates/bot-ipc/src/uds.rs:71-89`
- Test: `crates/bot-ipc/src/auth.rs` (unit tests)

**Interfaces:**
- Produces:
  - `pub fn PeerAuthPolicy::is_allowed_with_groups(&self, user_id: u32, group_id: u32, supplementary_gids: &[u32]) -> bool`
  - `pub fn parse_proc_status_groups(status_contents: &str) -> Vec<u32>`
  - `pub fn peer_supplementary_groups(pid: Option<i32>) -> Vec<u32>`

- [ ] **Step 1: Write failing unit tests in `crates/bot-ipc/src/auth.rs`**
  - Test `parse_proc_status_groups("Uid:\t1000 1000\nGid:\t1000 1000\nGroups:\t10 998 1000 \n")` returns `vec![10, 998, 1000]`.
  - Test `PeerAuthPolicy::new([999], [998]).is_allowed_with_groups(1000, 1000, &[10, 998])` returns `true`, while `is_allowed_with_groups(1000, 1000, &[10])` returns `false`.
  - Test `peer_supplementary_groups(None)` returns empty `Vec`.
- [ ] **Step 2: Run test to verify it fails**
  - Run: `cargo test -p bot-ipc auth::tests`
  - Expected: FAIL (functions not defined).
- [ ] **Step 3: Implement `is_allowed_with_groups`, `parse_proc_status_groups`, `peer_supplementary_groups` in `crates/bot-ipc/src/auth.rs` and wire in `crates/bot-ipc/src/uds.rs`**
  - Read `/proc/{pid}/status` when `pid` is `Some(p)` with `p > 0`, parse `Groups:` whitespace-separated `u32`s; call `peer_auth.is_allowed_with_groups(uid, gid, &supp_gids)` in `handle_uds_connection`.
- [ ] **Step 4: Run tests to verify pass**
  - Run: `cargo test -p bot-ipc`
  - Expected: PASS.
- [ ] **Step 5: Commit**
  - `git add crates/bot-ipc/src/auth.rs crates/bot-ipc/src/uds.rs && git commit -m "fix(bot-ipc): check peer supplementary groups in UDS SO_PEERCRED auth"`

---

### Task 2: Wire `EngineIpcBackend` & `TelemetryHub` in `botd` (`#1`)

**Files:**
- Create: `crates/botd/src/ipc_backend.rs`
- Modify: `crates/botd/src/main.rs:1-190`
- Test: `crates/botd/tests/ipc_engine_wiring.rs`

**Interfaces:**
- Consumes: `bot_engine::EngineHandle` (`apply_mode_trigger`, `subscribe_telemetry`), `bot_ipc::IpcBackend`, `bot_ipc::TelemetryHub`
- Produces:
  - `pub struct EngineIpcBackend` implementing `bot_ipc::IpcBackend`
  - `pub fn spawn_telemetry_bridge(rx: watch::Receiver<TelemetrySnapshot>, hub: Arc<TelemetryHub>) -> JoinHandle<()>`

- [ ] **Step 1: Write failing integration test in `crates/botd/tests/ipc_engine_wiring.rs`**
  - Boot `EngineActor` with `SqliteStore::open_in_memory()` + `PaperBroker`, construct `EngineIpcBackend::new(cfg, engine_handle.clone())`, invoke `backend.pause().await` and `backend.panic_flatten().await`, assert `engine_handle.subscribe_telemetry().borrow().mode` transitions to `EngineMode::Paused` and `EngineMode::Killed`.
- [ ] **Step 2: Run test to verify it fails**
  - Run: `cargo test -p botd --test ipc_engine_wiring`
  - Expected: FAIL (`ipc_backend` not found).
- [ ] **Step 3: Implement `EngineIpcBackend` and `spawn_telemetry_bridge` in `crates/botd/src/ipc_backend.rs` and wire in `crates/botd/src/main.rs`**
  - Map `pause` → `ModeTrigger::ManualPause`, `resume` → `ModeTrigger::ManualResume`, `panic_flatten` → `ModeTrigger::Panic`, `reset_kill` → `ModeTrigger::ResetKill`, `snapshot` → build `SnapshotPayload` from `TelemetrySnapshot`.
  - Replace `InMemoryIpcBackend::new(cfg.clone())` in `crates/botd/src/main.rs` with `EngineIpcBackend::new(cfg.clone(), engine_handle.clone())` and spawn `spawn_telemetry_bridge`.
- [ ] **Step 4: Run tests to verify pass**
  - Run: `cargo test -p botd`
  - Expected: PASS (including existing `chaos_spot`).
- [ ] **Step 5: Commit**
  - `git add crates/botd/src/ipc_backend.rs crates/botd/src/main.rs crates/botd/tests/ipc_engine_wiring.rs && git commit -m "fix(botd): wire IPC backend and TelemetryHub directly to EngineActor"`

---

### Task 3: Load `conf.d/` and Reject Non-Paper Mode Until M16 in `botd` (`#2`)

**Files:**
- Modify: `crates/botd/src/main.rs:49-115`
- Test: `crates/botd/tests/ipc_engine_wiring.rs` (add config/mode guard tests)

**Interfaces:**
- Produces:
  - `fn resolve_conf_d_dir(config_path: &Path) -> Option<PathBuf>`
  - `fn enforce_supported_runtime_mode(cfg: &BotConfig) -> Result<()>`

- [ ] **Step 1: Write failing unit/integration tests**
  - Test `resolve_conf_d_dir` returns `Some(dir/conf.d)` when `conf.d` exists beside `bot.toml`, and `None` when absent.
  - Test `enforce_supported_runtime_mode` accepts `TradingMode::Paper` and rejects `TradingMode::Testnet` and `TradingMode::Live` with an explicit error mentioning `VenueManager` / M16.
- [ ] **Step 2: Run test to verify it fails**
  - Run: `cargo test -p botd`
  - Expected: FAIL.
- [ ] **Step 3: Implement `resolve_conf_d_dir` and `enforce_supported_runtime_mode` in `crates/botd/src/main.rs`**
  - Pass `conf_d_dir: resolve_conf_d_dir(config_path)` into `ConfigSourcePaths` and call `enforce_supported_runtime_mode(&cfg)?` right after `enforce_live_confirmation_guard(&cfg)?`.
- [ ] **Step 4: Run tests to verify pass**
  - Run: `cargo test -p botd`
  - Expected: PASS.
- [ ] **Step 5: Commit**
  - `git add crates/botd/src/main.rs crates/botd/tests/ipc_engine_wiring.rs && git commit -m "fix(botd): load sibling conf.d directory and reject non-paper mode until M16"`

---

### Task 4: Implement `FuturesRestClient` (`/fapi/v1` Metadata, Pre-Activation Check, & Fallback) (`#4a`)

**Files:**
- Create: `crates/bot-exchange/src/futures/rest.rs`
- Modify: `crates/bot-exchange/src/futures/mod.rs`
- Test: `crates/bot-exchange/tests/futures_transport.rs`

**Interfaces:**
- Consumes: `crate::auth::Ed25519Credentials`, `crate::rate_limit::RateLimitTracker`, `crate::futures::wire::*`
- Produces:
  - `pub struct FuturesRestClient`
  - `pub async fn fetch_account_setup(&self) -> Result<FuturesAccountSetup, GatewayError>` (queries `/fapi/v1/positionSide/dual`, `/fapi/v1/multiAssetsMargin`, `/fapi/v2/positionRisk`)
  - `pub async fn exchange_info(&self) -> Result<ExchangeInfoSnapshot, GatewayError>`
  - `pub async fn place_order(&self, req: &PlaceOrderRequest) -> Result<OrderAck, GatewayError>`
  - `pub async fn cancel_order(&self, req: &CancelOrderRequest) -> Result<CancelOrderAck, GatewayError>`
  - `pub async fn query_order(&self, req: &QueryOrderRequest) -> Result<OrderSnapshot, GatewayError>`
  - `pub async fn open_orders(&self, symbol: Option<&Symbol>) -> Result<Vec<OrderSnapshot>, GatewayError>`
  - `pub async fn account_snapshot(&self) -> Result<AccountSnapshot, GatewayError>`
  - `pub async fn place_algo_order(&self, req: &PlaceAlgoOrderRequest) -> Result<AlgoOrderAck, GatewayError>`
  - `pub async fn cancel_algo_order(&self, req: &CancelAlgoOrderRequest) -> Result<CancelAlgoOrderAck, GatewayError>`

- [ ] **Step 1: Write failing mock HTTP tests in `crates/bot-exchange/tests/futures_transport.rs`**
  - Spin up local `TcpListener` mock HTTP server serving `/fapi/v1/positionSide/dual`, `/fapi/v1/multiAssetsMargin`, `/fapi/v2/positionRisk`, `/fapi/v1/order`, `/fapi/v1/algoOrder`.
  - Assert `fetch_account_setup()` parses real setup and rejects Hedge Mode / Cross margin; assert `place_order` and `place_algo_order` send signed requests and update `RateLimitTracker` from `X-MBX-USED-WEIGHT-1M`.
- [ ] **Step 2: Run test to verify it fails**
  - Run: `cargo test -p bot-exchange --test futures_transport`
  - Expected: FAIL (`FuturesRestClient` not defined).
- [ ] **Step 3: Implement `FuturesRestClient` in `crates/bot-exchange/src/futures/rest.rs` and export in `mod.rs`**
  - Mirror `SpotRestClient` signing and error mapping (`GatewayError::Api`, `GatewayError::Transport`, `RateLimitTracker::update_from_headers`).
- [ ] **Step 4: Run tests to verify pass**
  - Run: `cargo test -p bot-exchange --test futures_transport`
  - Expected: PASS.
- [ ] **Step 5: Commit**
  - `git add crates/bot-exchange/src/futures/rest.rs crates/bot-exchange/src/futures/mod.rs crates/bot-exchange/tests/futures_transport.rs && git commit -m "feat(bot-exchange): add FuturesRestClient for /fapi metadata and fallback"`

---

### Task 5: Implement `FuturesWsSession` (`/ws-fapi/v1` Primary) & Wire `FuturesAdapter` (`#4b`)

**Files:**
- Create: `crates/bot-exchange/src/futures/session.rs`
- Modify: `crates/bot-exchange/src/futures/adapter.rs`
- Modify: `crates/bot-exchange/src/futures/mod.rs`
- Test: `crates/bot-exchange/tests/futures_transport.rs`

**Interfaces:**
- Consumes: `FuturesRestClient`, `Ed25519Credentials`, `RateLimitTracker`
- Produces:
  - `pub struct FuturesWsSession` with `connect(url, creds, rate_limits)` and `request(method, params) -> Result<serde_json::Value, GatewayError>` (per-request Ed25519 signing, no `/session.logon`)
  - `FuturesAdapter::with_transport(config, creds)` (WS primary + REST fallback; `activate()` fetches real `FuturesAccountSetup` via `FuturesRestClient::fetch_account_setup()` and connects `FuturesWsSession`)
  - Preserves `FuturesAdapter::new(config, account_setup)` for offline/unit tests.

- [ ] **Step 1: Write failing mock WS + REST fallback tests in `crates/bot-exchange/tests/futures_transport.rs`**
  - Local `tokio-tungstenite` mock WS server verifying `order.place` and `algoOrder.place` arrive over WS with `apiKey`, `timestamp`, `signature`.
  - Test fallback: when WS server is closed/unreachable, `FuturesAdapter::place_order` and `place_algo_order` automatically fall back to `FuturesRestClient` and succeed.
  - Test `activate()` against mock REST server with Hedge Mode returns `GatewayError::InvalidRequest`.
- [ ] **Step 2: Run test to verify it fails**
  - Run: `cargo test -p bot-exchange --test futures_transport`
  - Expected: FAIL.
- [ ] **Step 3: Implement `FuturesWsSession` in `crates/bot-exchange/src/futures/session.rs` and wire WS-primary + REST-fallback in `crates/bot-exchange/src/futures/adapter.rs`**
- [ ] **Step 4: Run all `bot-exchange` tests**
  - Run: `cargo test -p bot-exchange`
  - Expected: PASS.
- [ ] **Step 5: Commit**
  - `git add crates/bot-exchange/src/futures/ crates/bot-exchange/tests/futures_transport.rs && git commit -m "feat(bot-exchange): wire FuturesWsSession primary and REST fallback in FuturesAdapter"`

---

### Task 6: Remove Duplicate `binance-bot-agent-kit/`, Update `docs/STATUS.md` & `CHANGELOG.md`, and Verify (`#5, #6`)

**Files:**
- Delete: `binance-bot-agent-kit/` (after confirming no unique files missing from `docs/`)
- Modify: `docs/STATUS.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Verify no unique files in `binance-bot-agent-kit/` and remove it**
  - Run: `diff -rq binance-bot-agent-kit/docs docs | head -n 40` (confirm `docs/` is superset), then remove `binance-bot-agent-kit/`.
- [ ] **Step 2: Update `docs/STATUS.md` ("Catatan sesi terakhir" for M14, M15, and Pre-M16 Audit Fixes) and add entry to `CHANGELOG.md`**
- [ ] **Step 3: Run full verification suite**
  - Run: `./scripts/verify.sh`
  - Expected: exit 0 (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`).
- [ ] **Step 4: Commit**
  - `git add -A && git commit -m "chore(docs): remove duplicate binance-bot-agent-kit and update STATUS/CHANGELOG"`
