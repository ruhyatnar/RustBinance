# M16 — `VenueManager`, Spot ⇄ Futures Switching, Cross-Venue Risk, Short & UI Switcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement safe runtime Spot ⇄ USDⓈ-M Futures switching via `VenueManager` inside `EngineActor`, cross-venue risk budgeting + BTC Spike Guard, Short side for S1/S2, IPC/`botctl` activation, Web & Desktop Venue Switcher UI, and the §13.4 Venue switching chaos suite.

**Architecture:** `VenueManager` lives inside `EngineActor` (`bot-engine` depends on `bot-venue`) so `switch_venue` executes atomically within the actor loop without races against `EntryPending`, `Syncing`, or reconciliation. Risk counters remain global in `bot-risk` across switches, and position protections (OCO / algo reduce-only) are never cancelled by switching.

**Tech Stack:** Rust (`tokio`, `rust_decimal`, `thiserror`, `proptest`), SQLite (`bot-store`), SvelteKit + Vitest (`web-ui`), Qt6/QML (`desktop-ui`).

**Spec:** [2026-11-24-m16-venue-manager-design.md](file:///home/antigravity/ruhyatnar/RustBinance/docs/superpowers/specs/2026-11-24-m16-venue-manager-design.md)

## Global Constraints
- Safety > correctness > simplicity > performance; no `unsafe` in Rust crates; no `unwrap`/`expect`/`panic`/indexing in non-test code.
- Prices/qty/balances/PnL use `rust_decimal::Decimal`; `f64` only inside indicator calculators.
- All state keys use `(Venue, Symbol)` (`InstrumentKey`); all DB rows/events/IPC carry `venue`.
- Protection orders (OCO / algo reduce-only) **must never** be cancelled by `switch_venue` or shutdown; only unfilled entry orders are cancelled.
- Risk counters (daily/weekly/drawdown/loss streak) **must never** be reset by `switch_venue`; switching is rejected in `Halted`/`Killed`.
- `mode=paper` remains default; `dual` mode and live Futures are out of scope for M16.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny`, and `web-ui` tests/build must all pass via `./scripts/verify.sh`.

## Review Focus
- Switching to the already-active venue must be rejected cleanly without consuming daily switch quota or triggering cooldown.
- A `Draining` venue that reaches 0 open positions must automatically transition to `Disabled` so it does not linger in `Draining`.
- `Flatten` policy without a valid `confirm_nonce` must be rejected at IPC before reaching `EngineActor`.
- Same symbol open on Spot (Long) while switching to Futures must block opening Futures Short (`cross_venue_same_symbol = BlockOpposite`), while allowing Futures Long if within combined heat.
- Activation rollback when the old venue had open positions (`KeepManaging`) must restore the old venue to `Active` and put the engine in `Paused` with an alarm, never leaving both venues `Disabled` or both `Active`.

---

### Task 1: `crates/bot-venue` — `VenueManager`, State Machine & Switch Guardrails

**Files:**
- Modify: `crates/bot-venue/Cargo.toml`
- Create: `crates/bot-venue/src/error.rs`
- Create: `crates/bot-venue/src/manager.rs`
- Modify: `crates/bot-venue/src/lib.rs`
- Test: `crates/bot-venue/tests/venue_manager.rs`

**Interfaces:**
- Consumes: `bot_types::{Venue, VenueState, EngineMode, TradingMode}`, `bot_ipc::SwitchOpenPositionPolicy` (or defines `SwitchPolicy` in `bot-venue` and maps in `bot-ipc`/`botd`).
- Produces:
  - `pub enum SwitchPolicy { KeepManaging, RequireFlat, Flatten }`
  - `pub struct SwitchConfig { pub switch_cooldown_s: u64, pub max_switches_per_day: u32, pub default_policy: SwitchPolicy }`
  - `pub struct VenueSlot { pub venue: Venue, pub state: VenueState, pub enabled: bool, pub credentials_valid: bool, pub live_confirmation_present: bool }`
  - `pub struct SwitchPrecheckContext { pub engine_mode: EngineMode, pub trading_mode: TradingMode, pub syncing: bool, pub old_open_positions: usize, pub now_ms: i64, pub utc_day: u32 }`
  - `pub enum SwitchError { InvalidEngineMode(EngineMode), SyncingInProgress, AlreadyActive(Venue), TargetDisabled(Venue), InvalidCredentials(Venue), MissingLiveConfirmation(Venue), CooldownActive { remaining_s: u64 }, DailyLimitReached { limit: u32 }, OpenPositionsRequireFlat { count: usize }, ActivationFailed { venue: Venue, reason: String } }`
  - `pub struct VenueManager` with `new(active: Venue, config: SwitchConfig)`, `state(venue) -> VenueState`, `active_venue() -> Venue`, `slots()`, `precheck_switch(target, policy, &ctx) -> Result<Venue, SwitchError>`, `begin_switch(target, policy, now_ms, utc_day) -> Result<Venue, SwitchError>`, `complete_switch(target, policy, now_ms, utc_day)`, `rollback_switch(old_venue, target)`, `mark_draining_flat(venue) -> bool`.

- [ ] **Step 1: Write failing unit & property tests in `crates/bot-venue/tests/venue_manager.rs`**
  - Test valid `Spot → UsdsFutures` (`KeepManaging` → Spot `Draining`, Futures `Active`; `mark_draining_flat(Spot)` → Spot `Disabled`).
  - Test `RequireFlat` rejected when `old_open_positions > 0`.
  - Test rejection when `EngineMode::Halted`, `EngineMode::Killed`, `syncing == true`, `target == active_venue`, cooldown active (`< 60s`), daily limit reached (`>= 3`), missing live confirmation in `TradingMode::Live`.
  - Test `rollback_switch` restores old venue to `Active` and target to `Disabled`, without leaving half-switched state.
- [ ] **Step 2: Run `cargo test -p bot-venue`**
  - Expected: FAIL (modules/types not defined).
- [ ] **Step 3: Implement `error.rs`, `manager.rs`, and re-export in `lib.rs`**
  - Implement all structs, state transitions, UTC-day switch counter rollover, cooldown check, and rollback.
- [ ] **Step 4: Run `cargo test -p bot-venue`**
  - Expected: PASS.
- [ ] **Step 5: Commit**
  - `git commit -m "feat(bot-venue): implement VenueManager state machine and switch guardrails"`

---

### Task 2: `crates/bot-risk` — Cross-Venue Risk Budget & BTC Spike Guard

**Files:**
- Create: `crates/bot-risk/src/cross_venue.rs`
- Modify: `crates/bot-risk/src/btc_guard.rs`
- Modify: `crates/bot-risk/src/lib.rs`
- Test: `crates/bot-risk/tests/cross_venue_risk.rs`

**Interfaces:**
- Consumes: `bot_types::{Venue, Direction, Symbol}`, `rust_decimal::Decimal`, `crate::btc_guard::BtcCrashGuard` pattern.
- Produces:
  - `pub enum CrossVenueSameSymbolPolicy { BlockOpposite, Allow }`
  - `pub struct CrossVenuePosition { pub venue: Venue, pub symbol: Symbol, pub direction: Direction, pub risk_usd: Decimal }`
  - `pub struct CrossVenueBudget` with:
    - `combined_equity_usd(spot_equity: Decimal, futures_equity: Decimal) -> Decimal`
    - `effective_venue_equity(venue_equity: Decimal, capital_allocation_usd: Decimal) -> Decimal`
    - `combined_portfolio_heat_pct(positions: &[CrossVenuePosition], combined_equity_usd: Decimal) -> Decimal`
    - `check_same_symbol(policy: CrossVenueSameSymbolPolicy, existing: &[CrossVenuePosition], candidate_venue: Venue, candidate_symbol: &Symbol, candidate_dir: Direction) -> Result<(), String>`
  - `pub struct BtcSpikeGuard` in `btc_guard.rs`: blocks Short entries when BTC rises `≥ 1.5%` in 15m or `≥ 3.0%` in 60m, with hysteresis recovery matching `BtcCrashGuard`.

- [ ] **Step 1: Write failing tests in `crates/bot-risk/tests/cross_venue_risk.rs`**
  - Test `combined_equity_usd = spot + futures` and `effective_venue_equity = min(venue_eq, alloc)` when `alloc > 0`.
  - Test `combined_portfolio_heat_pct` sums risk across Spot (Draining) + Futures (Active).
  - Test `BlockOpposite` rejects Futures Short when Spot Long is open on `BTCUSDT`, allows Futures Long.
  - Test `BtcSpikeGuard` triggers at `+1.5%` / 15m and `+3.0%` / 60m, stays active until hysteresis recovery.
- [ ] **Step 2: Run `cargo test -p bot-risk --test cross_venue_risk`**
  - Expected: FAIL.
- [ ] **Step 3: Implement `cross_venue.rs` and `BtcSpikeGuard` in `btc_guard.rs`**
  - Pure `Decimal` arithmetic; no `unwrap`/`expect`.
- [ ] **Step 4: Run `cargo test -p bot-risk`**
  - Expected: PASS.
- [ ] **Step 5: Commit**
  - `git commit -m "feat(bot-risk): add cross-venue risk budget and BTC Spike Guard for Short"`

---

### Task 3: `crates/bot-strategy` — S1/S2 Short Side & Venue Capability Matrix

**Files:**
- Modify: `crates/bot-strategy/src/s1_trend_pullback.rs`
- Modify: `crates/bot-strategy/src/s2_breakout.rs`
- Modify: `crates/bot-strategy/src/shadow.rs`
- Test: `crates/bot-strategy/tests/short_strategies.rs`

**Interfaces:**
- Consumes: `bot_types::{Direction, VenueCaps}`, `crate::types::{Strategy, StrategyContext, Signal}`.
- Produces:
  - `S1TrendPullback::supports(caps, dir)`: `true` for `Long`, `caps.can_short && self.allow_short` for `Short`.
  - `S2Breakout::supports(caps, dir)`: `true` for `Long`, `caps.can_short && self.allow_short` for `Short`.
  - `S1TrendPullback::evaluate(&ctx)` & `S2Breakout::evaluate(&ctx)` emit `Direction::Short` signals per §6.4.1 mirror formulas when `supports(caps, Short)` is true and Short conditions + BTC bearish regime hold.
  - `S3` remains Long-only (`supports(_, Short) == false`); `S4` shadow supports both directions when `caps.can_short`.

- [ ] **Step 1: Write failing tests in `crates/bot-strategy/tests/short_strategies.rs`**
  - Test `supports()` matrix across Spot (`can_short = false`) vs Futures (`can_short = true`) with `allow_short = false/true` for S1, S2, S3, S4.
  - Test S1 Short mirror signal generation (4h `close < EMA50` falling, 1h `EMA21 < EMA55` & `close < EMA200`, pullback `high >= EMA21 - 0.1*ATR` & `close < EMA21`, `RSI in [38, 60]` falling, `ADX >= 20`, `-DI > +DI`, stop `max(high_5bar, entry + 1.5*ATR) + 0.1*ATR`, TP below entry).
  - Test S2 Short mirror signal generation (`close < Donchian(20).lower`, volume `>= 1.5*SMA20`, `RSI in [25, 48]`, HTF bearish).
- [ ] **Step 2: Run `cargo test -p bot-strategy --test short_strategies`**
  - Expected: FAIL.
- [ ] **Step 3: Implement Short support in `s1_trend_pullback.rs`, `s2_breakout.rs`, and `shadow.rs`**
- [ ] **Step 4: Run `cargo test -p bot-strategy`**
  - Expected: PASS.
- [ ] **Step 5: Commit**
  - `git commit -m "feat(bot-strategy): add S1/S2 Short mirror signals and venue supports() matrix"`

---

### Task 4: `crates/bot-store` & `crates/bot-engine` — Atomic `switch_venue` & §13.4 Chaos Suite

**Files:**
- Modify: `crates/bot-store/src/events.rs` (add `JournalEvent::VenueSwitched`)
- Modify: `crates/bot-store/src/state.rs` (apply `VenueSwitched` in replay)
- Modify: `crates/bot-engine/Cargo.toml` (add `bot-venue`)
- Modify: `crates/bot-engine/src/actor.rs` (embed `VenueManager`, add `EngineCommand::SwitchVenue`, expose `EngineHandle::switch_venue`, include `VenueStates` in `TelemetrySnapshot`)
- Test: `crates/bot-engine/tests/venue_switching_chaos.rs`

**Interfaces:**
- Consumes: `bot_venue::{VenueManager, SwitchPolicy, SwitchError, SwitchPrecheckContext}`, `bot_store::JournalEvent::VenueSwitched`.
- Produces:
  - `EngineHandle::switch_venue(&self, target: Venue, policy: SwitchPolicy, actor: String) -> Result<VenueSwitchReport, String>`
  - `TelemetrySnapshot.spot_state: VenueState`, `TelemetrySnapshot.futures_state: VenueState`

- [ ] **Step 1: Write failing §13.4 Venue switching chaos tests in `crates/bot-engine/tests/venue_switching_chaos.rs`**
  - Scenario 1: Switch Spot→Futures during `EntryPending` → entry order cancelled, protection order preserved, no duplicate orders.
  - Scenario 2: Switch with open position (`KeepManaging`) → Spot becomes `Draining` and remains managed, Futures becomes `Active` only after reconciliation succeeds.
  - Scenario 3: Target activation fails → rollback to Spot `Active`, Futures `Disabled`, engine `Paused`, alarm emitted, no half-switched state.
  - Scenario 4: Flapping (4× in 1 hour) and switch while `Halted`/`Killed` → rejected by cooldown, daily limit, and global halt; risk counters NOT reset by switch.
  - Scenario 5: Same symbol (`BTCUSDT`) on Spot and Futures does not collide in keys, `clientOrderId`, or SQLite store.
- [ ] **Step 2: Run `cargo test -p bot-engine --test venue_switching_chaos`**
  - Expected: FAIL.
- [ ] **Step 3: Add `JournalEvent::VenueSwitched` in `bot-store` and wire `VenueManager` into `EngineActor`**
- [ ] **Step 4: Run `cargo test -p bot-store -p bot-engine`**
  - Expected: PASS.
- [ ] **Step 5: Commit**
  - `git commit -m "feat(bot-engine): integrate VenueManager atomic switch_venue and 13.4 chaos suite"`

---

### Task 5: `crates/bot-ipc`, `crates/botctl`, `crates/botd` — Enable `switch_venue` End-to-End

**Files:**
- Modify: `crates/bot-ipc/src/command.rs` (replace `not_supported` stub with `IpcBackend::switch_venue`, enforce `confirm_nonce` for `Flatten`)
- Modify: `crates/bot-ipc/tests/security_and_transport_suite.rs` (update `switch_venue` test from stub to live dispatch + nonce check)
- Modify: `crates/botctl/src/client.rs` (enable `switch-venue` command)
- Modify: `crates/botd/src/ipc_backend.rs` (implement `switch_venue` forwarding to `EngineHandle` and populate `venues` from `TelemetrySnapshot`)

**Interfaces:**
- Consumes: `EngineHandle::switch_venue`, `TelemetrySnapshot.{spot_state, futures_state}`.
- Produces: Working `POST /api/v1/cmd/switch_venue`, UDS `switch_venue`, and `botctl switch-venue`.

- [ ] **Step 1: Update `bot-ipc` test in `crates/bot-ipc/tests/security_and_transport_suite.rs` to expect `switch_venue` dispatch (and reject `flatten` without valid nonce)**
- [ ] **Step 2: Run `cargo test -p bot-ipc --test security_and_transport_suite`**
  - Expected: FAIL (still returns `not_supported`).
- [ ] **Step 3: Implement `switch_venue` in `bot-ipc/src/command.rs`, `botctl/src/client.rs`, and `botd/src/ipc_backend.rs`**
- [ ] **Step 4: Run `cargo test -p bot-ipc -p botctl -p botd`**
  - Expected: PASS.
- [ ] **Step 5: Commit**
  - `git commit -m "feat(bot-ipc,botctl,botd): enable switch_venue command and live venues telemetry"`

---

### Task 6: `web-ui` & `desktop-ui` — `VenueSwitcher`, `VenueSwitchDialog`, Badges & Draining Banner

**Files:**
- Create: `web-ui/src/lib/components/VenueSwitcher.svelte`
- Create: `web-ui/src/lib/components/VenueSwitchDialog.svelte`
- Modify: `web-ui/src/lib/components/PositionTable.svelte` (venue badge, Futures columns: Lev, Liq Price, Liq Dist %, Funding, filter `All | Spot | Futures`)
- Modify: `web-ui/src/routes/+layout.svelte` (or cockpit header) to mount `VenueSwitcher` + `Draining` banner
- Test: `web-ui/src/lib/components/venue-switcher.test.ts`
- Create: `desktop-ui/qml/components/VenueSwitcher.qml`
- Create: `desktop-ui/qml/components/VenueSwitchDialog.qml`
- Modify: `desktop-ui/src/StateStore.h` / `StateStore.cpp` & `CommandService.h` / `CommandService.cpp` (if present, expose `venues` and `switchVenue`)

**Interfaces:**
- Consumes: `web-ui/src/lib/api/types.ts` (`VenueState`, `ConfirmAction`, `switch_venue`), `CommandService.switchVenue(target, policy, confirmNonce)`.
- Produces: Interactive Venue Switcher in Web & Desktop UI matching §11.6.

- [ ] **Step 1: Write failing Vitest tests in `web-ui/src/lib/components/venue-switcher.test.ts`**
  - Test `VenueSwitcher` renders `[ SPOT | FUTURES ]`, status badges, `ISOLATED · maks 3×`, hides for read-only token, and shows Draining banner when a venue is `Draining`.
  - Test `VenueSwitchDialog` defaults to `keep_managing`, requires hold-to-confirm + nonce for `flatten`, and displays rejection reason when `Halted`/`Killed`.
- [ ] **Step 2: Run `npm --prefix web-ui test`**
  - Expected: FAIL.
- [ ] **Step 3: Implement Web UI components (`VenueSwitcher.svelte`, `VenueSwitchDialog.svelte`, `PositionTable.svelte` updates) and Desktop QML (`VenueSwitcher.qml`, `VenueSwitchDialog.qml`)**
- [ ] **Step 4: Run `npm --prefix web-ui test && npm --prefix web-ui run build`**
  - Expected: PASS.
- [ ] **Step 5: Commit**
  - `git commit -m "feat(ui): add VenueSwitcher, VenueSwitchDialog, venue badges, and Draining banner"`

---

### Task 7: Full Verification, Docs & Knowledge Graph Sync

**Files:**
- Modify: `docs/STATUS.md` (check off M16, record session notes)
- Modify: `CHANGELOG.md` (add M16 entries)

- [ ] **Step 1: Run `./scripts/verify.sh`**
  - Expected: Exit code 0 (fmt, clippy pedantic `-D warnings`, cargo test, cargo deny, web-ui tests & build, secret scan).
- [ ] **Step 2: Update `docs/STATUS.md` and `CHANGELOG.md`**
- [ ] **Step 3: Run `graphify update .` (per `.agents/rules/graphify.md`)**
- [ ] **Step 4: Commit**
  - `git commit -m "chore(docs): mark M16 complete in STATUS and CHANGELOG"`
