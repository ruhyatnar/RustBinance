# M18: Exchange & Runtime Reliability Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden `RustBinance`'s exchange transport layer, daemon process lifecycle, and configuration hygiene so M19–M21 probes/soak ops and G2–G5 testnet gates run on a rock-solid foundation.

**Architecture:** All 7 M18 capabilities are implemented natively in Rust across `bot-exchange`, `bot-engine`, `botd`, and `bot-config` with zero new external dependencies (leveraging Rust 1.89 `std::fs::File::try_lock`), zero `unsafe`, and strict `rust_decimal::Decimal` / `thiserror` / `clippy -D warnings` compliance. Every change is strictly additive and does not modify strategy signal or exit logic.

**Tech Stack:** Rust 2024 edition (MSRV 1.89), Tokio, `reqwest`, `rust_decimal`, `thiserror`, `tracing`, `sqlx` (`bot-store`), `tempfile` (dev).

**Spec:** `docs/superpowers/specs/2026-11-26-m18-m21-prelive-ops-design.md` (§5 M18)

## Global Constraints

- `#![forbid(unsafe_code)]`; no `unwrap`, `expect`, `panic`, or unchecked indexing in non-test code (`clippy -D warnings`, pedantic warn).
- All monetary values (`Decimal`), all state keyed by `(Venue, Symbol)`.
- No network tests in default `cargo test --workspace`; all tests in M18 must be 100% offline and deterministic.
- Never cancel exchange-side OCO/Algo protections on maintenance auto-pause; only block new entries via `EngineMode::Paused`, and never downgrade `Halted`/`Killed`.
- Recovery after maintenance clears: **stay in `EngineMode::Paused`** (clear `maintenance_paused` flag only) until operator explicitly invokes `Resume`.
- Secrets (`api_key`, private keys) must never appear in logs, errors, or lock files.
- Final gate: `scripts/verify.sh` must pass completely.

## Review Focus

1. **`0 ms` clock offset treated as unsynced:** If a local clock matches Binance to the millisecond (`offset_ms == 0`), `ClockSyncState::offset_ms()` must return `Some(0)` and `needs_sync()` must be `false` within TTL — pinned in Task 1.
2. **REST rate-limit header burst without `exchangeInfo` seed:** Receiving `X-MBX-ORDER-COUNT-10S: 45` before `apply_rules` runs must not panic or divide by zero, and once seeded, `>= 90%` usage must block new orders — pinned in Task 3.
3. **Idle WS `BalanceCache` falsely expiring to REST:** A healthy `SessionState::Ready` WS session with no balance-changing events for 45 seconds must still report `BalanceSource::Ws` (not expire to REST until `reconcile_interval_ms = 300_000` or disconnect/gap) — pinned in Task 4.
4. **Maintenance auto-pause downgrading `Killed`/`Halted`:** If the engine is already `EngineMode::Killed` or `EngineMode::Halted`, a maintenance signal or maintenance recovery must **never** overwrite `Killed`/`Halted` to `Paused` or `Ready` — pinned in Task 6.
5. **Stale/dropped process lock release:** Dropping `ProcessLock` must release the OS lock immediately so a subsequent daemon restart acquires it cleanly, while two live locks for the same `(mode, path)` must fail the second acquire — pinned in Task 5.

---

## File Structure

- Create: `crates/bot-exchange/src/clock.rs` — `ClockSyncState` (`Option<i64>` offset sentinel + `last_sync_ms`, `needs_sync`, `adjusted_now_ms`).
- Modify: `crates/bot-exchange/src/spot/rest.rs` — attach `ClockSyncState` + `Arc<RateLimitTracker>`, ingest REST rate-limit headers, add `ping()`.
- Modify: `crates/bot-exchange/src/futures/rest.rs` — attach `ClockSyncState`, add `server_time()` & `ping()`, ingest REST rate-limit headers in `check_response`.
- Modify: `crates/bot-exchange/src/error.rs` — extend `ExchangeErrorKind` (`-1022`, `-2014`, `-4046`, `-4059`), add `ExchangeErrorKind::is_permanent()` and `GatewayError::is_retryable()`.
- Modify: `crates/bot-exchange/src/rate_limit.rs` — add `RateBudgetExceeded`, `record_rest_headers()`, and `can_place_order(now_ms, safety_margin_pct)`.
- Create: `crates/bot-exchange/src/balance_cache.rs` — `BalanceSource`, `BalanceProvenance`, `CachedBalances`, `BalanceCache`.
- Modify: `crates/bot-exchange/src/lib.rs` — re-export `clock`, `balance_cache`, and new error/rate-limit types.
- Create: `crates/botd/src/lock.rs` — `ProcessLock` using Rust 1.89 `std::fs::File::try_lock()`.
- Create: `crates/botd/src/health.rs` — `MaintenanceDetector` + `HealthProbeRunner` (periodic REST `/ping` + SQLite `SELECT 1`).
- Modify: `crates/bot-engine/src/actor.rs` — add `EngineCommand::SetMaintenancePaused(bool)` and `maintenance_paused: bool` / `db_ok: bool` on `TelemetrySnapshot`.
- Modify: `crates/bot-store/src/store.rs` — add `SqliteStore::ping(&self) -> Result<(), StoreError>` (`SELECT 1`).
- Modify: `crates/botd/src/main.rs` — acquire `ProcessLock` at startup and wire `HealthProbeRunner`.
- Create: `crates/bot-config/tests/config_drift.rs` — strict CI drift test verifying `packaging/bot.toml.example` covers `BotConfig` with zero unknown/missing keys.

---

### Task 1: Clock-Offset `Option<i64>` Sentinel (`bot-exchange`)

**Files:**
- Create: `crates/bot-exchange/src/clock.rs`
- Modify: `crates/bot-exchange/src/spot/rest.rs`
- Modify: `crates/bot-exchange/src/futures/rest.rs`
- Modify: `crates/bot-exchange/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct ClockSyncState` (`new() -> Self`, `record(&self, sync: ServerTimeSync)`, `offset_ms(&self) -> Option<i64>`, `last_sync_ms(&self) -> Option<i64>`, `needs_sync(&self, now_ms: i64, max_age_ms: i64) -> bool`, `adjusted_now_ms(&self, local_now_ms: i64) -> i64`)
  - `SpotRestClient::clock_state(&self) -> &ClockSyncState`, `SpotRestClient::ping(&self) -> Result<(), GatewayError>`
  - `FuturesRestClient::clock_state(&self) -> &ClockSyncState`, `FuturesRestClient::server_time(&self) -> Result<ServerTimeSync, GatewayError>`, `FuturesRestClient::ping(&self) -> Result<(), GatewayError>`

- [ ] **Step 1: Write failing unit tests in `crates/bot-exchange/src/clock.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::ClockSyncState;
    use crate::spot::ServerTimeSync;

    #[test]
    fn zero_ms_offset_is_distinct_from_unsynced() {
        let clock = ClockSyncState::new();
        assert_eq!(clock.offset_ms(), None);
        assert_eq!(clock.last_sync_ms(), None);
        assert!(clock.needs_sync(1_000, 60_000));

        clock.record(ServerTimeSync {
            server_time_ms: 1_000,
            local_time_ms: 1_000,
            offset_ms: 0,
        });
        assert_eq!(clock.offset_ms(), Some(0));
        assert_eq!(clock.last_sync_ms(), Some(1_000));
        assert!(!clock.needs_sync(30_000, 60_000));
        assert!(clock.needs_sync(61_001, 60_000));
        assert_eq!(clock.adjusted_now_ms(5_000), 5_000);
    }

    #[test]
    fn nonzero_offset_adjusts_local_timestamp() {
        let clock = ClockSyncState::new();
        clock.record(ServerTimeSync {
            server_time_ms: 10_250,
            local_time_ms: 10_000,
            offset_ms: 250,
        });
        assert_eq!(clock.offset_ms(), Some(250));
        assert_eq!(clock.adjusted_now_ms(20_000), 20_250);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bot-exchange clock::tests`
Expected: FAIL (module `clock` not found).

- [ ] **Step 3: Implement `ClockSyncState` in `crates/bot-exchange/src/clock.rs` and wire into `SpotRestClient` & `FuturesRestClient`**

Implement `ClockSyncState` backed by `Arc<Mutex<ClockInner>>` (storing `offset_ms: Option<i64>` and `last_sync_ms: Option<i64>`). Record sync results inside `SpotRestClient::server_time()` and new `FuturesRestClient::server_time()` (`/fapi/v1/time`). Add `ping()` to both (`GET /api/v3/ping` and `GET /fapi/v1/ping`). Re-export `ClockSyncState` from `crates/bot-exchange/src/lib.rs`.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p bot-exchange clock::tests`
Expected: PASS (2/2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/bot-exchange/src/clock.rs crates/bot-exchange/src/spot/rest.rs crates/bot-exchange/src/futures/rest.rs crates/bot-exchange/src/lib.rs
git commit -m "feat(bot-exchange): add Option<i64> ClockSyncState sentinel and REST ping"
```

---

### Task 2: Explicit Permanent-Error Classifier (`bot-exchange`)

**Files:**
- Modify: `crates/bot-exchange/src/error.rs`

**Interfaces:**
- Produces:
  - `ExchangeErrorKind::{InvalidSignature, ApiKeyFormatInvalid, NoNeedToChangeMarginType, NoNeedToChangePositionSide}`
  - `ExchangeErrorKind::is_permanent(&self) -> bool`
  - `GatewayError::is_retryable(&self) -> bool`
  - `GatewayError::is_maintenance(&self) -> bool`

- [ ] **Step 1: Write failing tests in `crates/bot-exchange/src/error.rs`**

```rust
#[test]
fn classifies_permanent_binance_codes_and_retryability() {
    for (code, expected) in [
        (-1022, ExchangeErrorKind::InvalidSignature),
        (-2014, ExchangeErrorKind::ApiKeyFormatInvalid),
        (-4046, ExchangeErrorKind::NoNeedToChangeMarginType),
        (-4059, ExchangeErrorKind::NoNeedToChangePositionSide),
    ] {
        let kind = ExchangeErrorKind::from_code(code);
        assert_eq!(kind, expected);
        assert!(kind.is_permanent());
        let err = GatewayError::from_exchange_error(400, Some(code), "err".into(), None, None);
        assert!(!err.is_retryable());
    }

    let rate_limited = GatewayError::from_exchange_error(429, Some(-1003), "slow".into(), None, None);
    assert!(rate_limited.is_retryable());
    assert!(!rate_limited.is_maintenance());

    let maint = GatewayError::from_exchange_error(503, None, "Service Unavailable: system maintenance".into(), None, None);
    assert!(maint.is_maintenance());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bot-exchange classifies_permanent_binance_codes_and_retryability`
Expected: FAIL (missing variants/methods).

- [ ] **Step 3: Implement new `ExchangeErrorKind` variants, `is_permanent()`, `is_retryable()`, and `is_maintenance()`**

Map `-1022`, `-2014`, `-4046`, `-4059` in `ExchangeErrorKind::from_code`. Mark auth/signature/filter/order-rejected/margin-noop/position-side-noop/use-algo-api/post-only as permanent (`is_permanent() == true`). In `GatewayError::is_retryable()`: `true` only for `RateLimited`, `Http`, `Transport`, `SessionNotReady`, `UnknownNeedsQuery`; `false` for `IpBanned`, `InvalidAuth`, `TimestampOutOfWindow`, `InvalidRequest`, `KeyLoad`, `Unsupported`, `WireFormat`, `SessionClosed`, and `ExchangeRejected` when `kind.is_permanent()`. In `GatewayError::is_maintenance()`: `true` when `UnknownNeedsQuery` reason contains `"503"` or case-insensitive `"maintenance"` / `"system busy"` / `"-1001"`.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p bot-exchange error::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/bot-exchange/src/error.rs
git commit -m "feat(bot-exchange): classify permanent Binance error codes and retryability"
```

---

### Task 3: Client-Side REST Header Ingestion & Order Rate Budget Guard (`bot-exchange`)

**Files:**
- Modify: `crates/bot-exchange/src/rate_limit.rs`
- Modify: `crates/bot-exchange/src/spot/rest.rs`
- Modify: `crates/bot-exchange/src/futures/rest.rs`
- Modify: `crates/bot-exchange/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct RateBudgetExceeded { pub bucket: RateLimitBucketKey, pub count: u64, pub limit: u64 }`
  - `RateLimitTracker::record_rest_header(&self, header_name: &str, value: &str)`
  - `RateLimitTracker::record_rest_headers<'a, I: IntoIterator<Item = (&'a str, &'a str)>>(&self, headers: I)`
  - `RateLimitTracker::can_place_order(&self, now_ms: i64, safety_margin_pct: u8) -> Result<(), RateBudgetExceeded>`

- [ ] **Step 1: Write failing tests in `crates/bot-exchange/src/rate_limit.rs`**

```rust
#[test]
fn rest_headers_update_buckets_and_enforce_order_budget() {
    let tracker = RateLimitTracker::new();
    // Review Focus #2: headers before rules must not panic or block with limit=0.
    tracker.record_rest_headers([
        ("x-mbx-used-weight-1m", "120"),
        ("x-mbx-order-count-10s", "45"),
    ]);
    assert!(tracker.can_place_order(10_000, 90).is_ok());

    tracker.apply_rules(&[RateLimitRule {
        rate_limit_type: RateLimitType::Orders,
        interval: RateLimitInterval::Second,
        interval_num: 10,
        limit: 50,
    }]);
    // 45 / 50 == 90% -> blocked at 90% safety margin.
    let err = tracker.can_place_order(10_000, 90).unwrap_err();
    assert_eq!(err.count, 45);
    assert_eq!(err.limit, 50);

    // Lower count below threshold -> allowed.
    tracker.record_rest_header("X-MBX-ORDER-COUNT-10S", "44");
    assert!(tracker.can_place_order(10_000, 90).is_ok());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bot-exchange rest_headers_update_buckets_and_enforce_order_budget`
Expected: FAIL (missing methods).

- [ ] **Step 3: Implement header parser, `can_place_order`, and hook into Spot & Futures REST response handling**

Parse case-insensitive `x-mbx-used-weight-<N><s|m|d>` and `x-mbx-order-count-<N><s|m|d>` into `RateLimitBucketKey` and update `count` (preserving existing `limit` or defaulting `limit = 0` when unseeded). In `can_place_order(now_ms, safety_margin_pct)`: reject if `is_ip_banned(now_ms)` or `retry_after_delay(now_ms).is_some()`, or for any `RateLimitType::Orders` bucket with `limit > 0` where `count.saturating_mul(100) >= limit.saturating_mul(u64::from(safety_margin_pct))`. Wire `record_rest_headers` into `SpotRestClient` (add `rate_limits: Arc<RateLimitTracker>`) and `FuturesRestClient::check_response`. Re-export `RateBudgetExceeded`.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p bot-exchange rate_limit::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/bot-exchange/src/rate_limit.rs crates/bot-exchange/src/spot/rest.rs crates/bot-exchange/src/futures/rest.rs crates/bot-exchange/src/lib.rs
git commit -m "feat(bot-exchange): ingest REST rate-limit headers and enforce client order budget"
```

---

### Task 4: WS-First `BalanceCache` + Provenance (`bot-exchange`)

**Files:**
- Create: `crates/bot-exchange/src/balance_cache.rs`
- Modify: `crates/bot-exchange/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub enum BalanceSource { Ws, Rest }`
  - `pub struct BalanceProvenance { pub source: BalanceSource, pub age_ms: u64 }`
  - `pub struct CachedAccountSnapshot { pub snapshot: AccountSnapshot, pub provenance: BalanceProvenance }`
  - `pub struct BalanceCache` (`new(venue: Venue, reconcile_interval_ms: u64) -> Self`, `seed_from_rest(&self, snapshot: AccountSnapshot, now_ms: i64)`, `apply_ws_delta(&self, balances: &[AccountBalance], now_ms: i64)`, `mark_ws_disconnected(&self)`, `get_if_fresh(&self, ws_ready: bool, now_ms: i64) -> Option<CachedAccountSnapshot>`)

- [ ] **Step 1: Write failing tests in `crates/bot-exchange/src/balance_cache.rs`**

```rust
#[cfg(test)]
mod tests {
    use bot_types::Venue;
    use rust_decimal_macros::dec;
    use super::{BalanceCache, BalanceSource};
    use crate::gateway::{AccountBalance, AccountSnapshot};

    #[test]
    fn ws_cache_stays_authoritative_during_idle_ready_session_until_reconcile_interval() {
        let cache = BalanceCache::new(Venue::Spot, 300_000);
        assert!(cache.get_if_fresh(true, 1_000).is_none()); // unseeded

        cache.seed_from_rest(
            AccountSnapshot {
                venue: Venue::Spot,
                balances: vec![AccountBalance {
                    asset: "USDT".into(),
                    free: dec!(1000),
                    locked: dec!(0),
                }],
            },
            1_000,
        );
        // Review Focus #3: idle 45s while WS Ready stays authoritative (source=Ws).
        let snap = cache.get_if_fresh(true, 46_000).expect("fresh");
        assert_eq!(snap.provenance.source, BalanceSource::Ws);
        assert_eq!(snap.provenance.age_ms, 45_000);

        // WS event updates balance in place.
        cache.apply_ws_delta(
            &[AccountBalance {
                asset: "USDT".into(),
                free: dec!(950),
                locked: dec!(50),
            }],
            50_000,
        );
        let updated = cache.get_if_fresh(true, 52_000).expect("fresh");
        assert_eq!(updated.snapshot.balances[0].free, dec!(950));
        assert_eq!(updated.provenance.age_ms, 2_000);

        // Disconnect or exceeding 300s reconcile interval forces REST fallback.
        assert!(cache.get_if_fresh(false, 52_000).is_none());
        assert!(cache.get_if_fresh(true, 355_000).is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bot-exchange balance_cache::tests`
Expected: FAIL (module not found).

- [ ] **Step 3: Implement `BalanceCache` in `crates/bot-exchange/src/balance_cache.rs` and export from `lib.rs`**

Store `BTreeMap<String, AccountBalance>`, `last_rest_seed_ms: Option<i64>`, `last_update_ms: Option<i64>`, and `needs_reseed: bool` inside `Mutex<Inner>`. When `ws_ready && !needs_reseed` and `now_ms - last_rest_seed_ms <= reconcile_interval_ms`, return `Some(CachedAccountSnapshot)` with `BalanceSource::Ws` and `age_ms = (now_ms - last_update_ms).max(0)`. On `mark_ws_disconnected()`, set `needs_reseed = true`.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p bot-exchange balance_cache::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/bot-exchange/src/balance_cache.rs crates/bot-exchange/src/lib.rs
git commit -m "feat(bot-exchange): add WS-first BalanceCache with provenance tracking"
```

---

### Task 5: Single-Instance Lock File (`botd`)

**Files:**
- Create: `crates/botd/src/lock.rs`
- Modify: `crates/botd/src/main.rs`

**Interfaces:**
- Produces:
  - `pub struct ProcessLock` (`acquire(path: &Path) -> Result<Self, ProcessLockError>`, `path(&self) -> &Path`)
  - `pub enum ProcessLockError { AlreadyLocked(PathBuf), Io { path: PathBuf, source: std::io::Error } }`

- [ ] **Step 1: Write failing tests in `crates/botd/src/lock.rs`**

```rust
#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use super::{ProcessLock, ProcessLockError};

    #[test]
    fn exclusive_lock_blocks_second_instance_and_releases_on_drop() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("binance-botd.live.lock");

        let first = ProcessLock::acquire(&lock_path).expect("first lock");
        let second = ProcessLock::acquire(&lock_path);
        assert!(matches!(second, Err(ProcessLockError::AlreadyLocked(_))));

        // Review Focus #5: dropping releases lock immediately.
        drop(first);
        let third = ProcessLock::acquire(&lock_path).expect("re-acquire after drop");
        assert_eq!(third.path(), lock_path.as_path());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p binance-botd lock::tests`
Expected: FAIL (module `lock` not found).

- [ ] **Step 3: Implement `ProcessLock` using Rust 1.89 `File::try_lock()` and wire into `botd/src/main.rs`**

Create parent directories if missing, open with `OpenOptions::new().create(true).read(true).write(true)`, call `file.try_lock()` (mapping `TryLockError::WouldBlock` to `ProcessLockError::AlreadyLocked`), and write current PID to the file. In `botd/src/main.rs`, acquire the lock immediately after `enforce_live_confirmation_guard(&cfg)` (before opening SQLite or exchange adapters) at `<store.sqlite_path parent>/binance-botd.<mode>.lock` and hold `_process_lock` until daemon exit.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p binance-botd lock::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/botd/src/lock.rs crates/botd/src/main.rs
git commit -m "feat(botd): add single-instance OS process lock on startup"
```

---

### Task 6: Exchange Maintenance Auto-Pause & Active DB Probe (`bot-store`, `bot-engine`, `botd`)

**Files:**
- Modify: `crates/bot-store/src/store.rs`
- Modify: `crates/bot-engine/src/actor.rs`
- Create: `crates/botd/src/health.rs`
- Modify: `crates/botd/src/main.rs`

**Interfaces:**
- Produces:
  - `SqliteStore::ping(&self) -> Result<(), StoreError>`
  - `EngineCommand::SetMaintenancePaused { active: bool, reason: String }`
  - `EngineCommand::ReportDbHealth { ok: bool }`
  - `TelemetrySnapshot::{maintenance_paused: bool, db_ok: bool}`
  - `HealthProbeOutcome { exchange_maintenance: bool, db_ok: bool }`

- [ ] **Step 1: Write failing tests in `crates/bot-engine/tests/` (or `actor.rs` tests) and `crates/bot-store`**

Test 1 (`bot-store`): `sqlite_store_ping_succeeds_on_open_db` calls `store.ping().await` and asserts `Ok(())`.
Test 2 (`bot-engine`): `maintenance_pause_transitions_ready_to_paused_and_stays_paused_on_clear` verifies:
- `Ready` + `SetMaintenancePaused { active: true }` → `mode == EngineMode::Paused`, `maintenance_paused == true`.
- `SetMaintenancePaused { active: false }` → `mode == EngineMode::Paused` (stays Paused per Opsi 1!), `maintenance_paused == false`.
- **Review Focus #4:** `Killed` or `Halted` + `SetMaintenancePaused { active: true/false }` → `mode` remains `Killed`/`Halted` unchanged.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p bot-store sqlite_store_ping && cargo test -p bot-engine maintenance_pause`
Expected: FAIL (methods/variants not defined).

- [ ] **Step 3: Implement `SqliteStore::ping`, `EngineActor` maintenance handler, and `botd/src/health.rs` periodic probe**

In `SqliteStore::ping()`, execute `SELECT 1` on the pool. In `EngineActor`, handle `SetMaintenancePaused`: if `active`, set `maintenance_paused = true` and if `mode == EngineMode::Ready`, transition to `EngineMode::Paused` via existing mode machine (never touching open OCO/Algo protections, never downgrading `Halted`/`Killed`). If `!active`, set `maintenance_paused = false` while leaving `EngineMode::Paused` untouched until explicit `Resume`. In `botd/src/health.rs`, spawn a periodic probe task (default 15s, cancelled on shutdown token) calling `store.ping()` and exchange REST `ping()` (checking `GatewayError::is_maintenance()`) and forwarding state transitions via `EngineHandle`.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p bot-store sqlite_store_ping && cargo test -p bot-engine maintenance_pause && cargo test -p binance-botd health::tests`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/bot-store/src/store.rs crates/bot-engine/src/actor.rs crates/botd/src/health.rs crates/botd/src/main.rs
git commit -m "feat(bot-engine,botd): add exchange maintenance auto-pause and active SQLite probe"
```

---

### Task 7: Config Drift Check di CI (`bot-config`) & Final M18 Verification

**Files:**
- Create: `crates/bot-config/tests/config_drift.rs`
- Modify: `packaging/bot.toml.example` (only if drift test reveals missing/outdated keys)
- Modify: `docs/STATUS.md`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Produces: CI-enforced integration test guaranteeing `packaging/bot.toml.example` parses strictly into `BotConfig` and passes `validate()`.

- [ ] **Step 1: Write `crates/bot-config/tests/config_drift.rs`**

```rust
use std::path::Path;
use bot_config::{BotConfig, ConfigSourcePaths, load_from_paths};
use figment::{Figment, providers::{Format, Toml}};

#[test]
fn example_toml_loads_cleanly_and_has_no_unknown_or_invalid_keys() {
    let example_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packaging/bot.toml.example");
    assert!(example_path.is_file(), "packaging/bot.toml.example must exist");

    // Ensure strict deserialization (no typos/drift) and full safety validation.
    let parsed: BotConfig = Figment::new()
        .merge(Toml::file(&example_path))
        .extract()
        .expect("strict BotConfig parse from packaging/bot.toml.example");
    let loaded = load_from_paths(&ConfigSourcePaths {
        base_file: Some(example_path),
        conf_d_dir: None,
    })
    .expect("example config passes load + validate");
    assert_eq!(parsed.general.mode, loaded.general.mode);
}
```

- [ ] **Step 2: Run config drift test**

Run: `cargo test -p bot-config --test config_drift`
Expected: PASS (or if `packaging/bot.toml.example` has any drift, fix `packaging/bot.toml.example` so it passes).

- [ ] **Step 3: Run full verification gate (`scripts/verify.sh`)**

Run: `./scripts/verify.sh`
Expected: ALL GREEN (`cargo fmt`, `clippy -D warnings`, `cargo test --workspace`, `cargo deny`, `web-ui` Vitest + `vite build`, secret scan).

- [ ] **Step 4: Update `docs/STATUS.md`, `CHANGELOG.md`, and run `graphify update .`**

Record M18 completion in `docs/STATUS.md` and `CHANGELOG.md`, and update knowledge graph via `graphify update .`.

- [ ] **Step 5: Commit**

```bash
git add crates/bot-config/tests/config_drift.rs packaging/bot.toml.example docs/STATUS.md CHANGELOG.md
git commit -m "feat(bot-config): add CI config drift test and finalize M18 hardening"
```
