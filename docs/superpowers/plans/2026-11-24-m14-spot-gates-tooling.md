# M14 Spot Gates Tooling, G1 Runner, & ADR-0005 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver M14's engineering & operational tooling: ADR-0005 (separating engineering progress in paper/testnet from live sign-off), a thin `bot-backtest` CLI (`g1` walk-forward runner and `gate-report` SQLite journal analyzer), Spot gate templates/runbooks (`docs/gates/spot/G1.md`–`G5.md`), and updated `STATUS.md`/`CHANGELOG.md`.

**Architecture:** Add a pure, testable `gate_report` module in `crates/bot-backtest/src/gate_report.rs` that formats G1 walk-forward results and analyzes `SqliteStore` journal events per `(Venue, Symbol)` into Markdown gate reports (G2/G3/G4) without modifying any strategy/risk parameters. Expose both via a thin CLI binary `crates/bot-backtest/src/bin/bot-backtest.rs` using `std::env::args` (zero new crate dependencies). Document ADR-0005 and Spot gate runbooks/reports under `docs/decisions/` and `docs/gates/spot/`.

**Tech Stack:** Rust (workspace), `bot-backtest`, `bot-store` (`SqliteStore`, `JournalEvent`), `bot-types` (`Venue`, `Symbol`, `Decimal`), `rust_decimal`.

**Spec:** `docs/superpowers/specs/2026-11-24-m14-spot-gates-tooling-design.md`

## Global Constraints
- `rust_decimal::Decimal` for all price/qty/PnL/slippage/metric calculations; no `f64` outside indicator calculators.
- `#![forbid(unsafe_code)]`; no `unwrap`/`expect`/`panic`/indexing in non-test code; typed errors (`thiserror`), `anyhow` only in bin if needed (or typed error + `ExitCode`).
- Default mode remains `paper`; `mode=live` remains locked behind `live_confirmation_file`; no §13.3 gate thresholds loosened.
- All DB queries and reports carry `venue` (`Venue::Spot`).
- Zero network calls in default `cargo test`; `scripts/verify.sh` must pass green.

## Review Focus
1. Empty or missing SQLite journal DB passed to `gate-report` — must return a clean structured error (never panic) or a clearly marked "0 trades / INCOMPLETE" report for an empty initialized DB.
2. Mixed-venue SQLite journal (both `Spot` and `Futures` events present) — `gate-report --venue spot` must strictly filter to `Venue::Spot` and never leak `Futures` fills or incidents.
3. G1 threshold boundary checks — if any §13.3 criterion fails (e.g. trades < 300, PF < 1.3, expectancy < +0.15R, MaxDD > 15%, neighbor pass < 70%, or top-3-removed PF <= 1.0), the generated Markdown MUST mark the verdict `FAIL` (never silently pass).
4. Invalid CLI arguments or unknown gate name (`--gate G9`) — must exit non-zero with a clear usage message, never panic.
5. `docs/gates/spot/G2.md`–`G5.md` must clearly state `STATUS: PENDING OPERATIONAL RUN (NOT APPROVED FOR LIVE)` so no reader mistakes engineering completion for live sign-off.

---

### Task 1: ADR-0005 — Parallel Engineering (Paper/Testnet) & Operational Live Sign-off

**Files:**
- Create: `docs/decisions/0005-parallel-engineering-and-operational-gates.md`

**Interfaces:**
- Produces: Architectural decision record referenced by `docs/STATUS.md` and `docs/gates/spot/`.

- [ ] **Step 1: Write `docs/decisions/0005-parallel-engineering-and-operational-gates.md`**

Include: Context (M14/M17 require 12+ weeks of real-world wall-clock time), Decision (separate Engineering Done in `paper`/`testnet` from Operational Live Sign-off; allow M15–M17 engineering to proceed after M14 tooling is complete; keep §13.3 thresholds and `live_confirmation_file` guard unchanged), Consequences (faster full-codebase completion with zero compromise on live fund safety).

- [ ] **Step 2: Verify formatting and links**

Run: `ls -la docs/decisions/0005-parallel-engineering-and-operational-gates.md`
Expected: File exists and is readable.

- [ ] **Step 3: Commit**

```bash
git add docs/decisions/0005-parallel-engineering-and-operational-gates.md
git commit -m "docs(adr): add ADR-0005 separating engineering progress from live gate sign-off"
```

---

### Task 2: `gate_report` Module in `bot-backtest` (G1 & SQLite G2–G4 Evaluators)

**Files:**
- Create: `crates/bot-backtest/src/gate_report.rs`
- Modify: `crates/bot-backtest/src/lib.rs`
- Test: `crates/bot-backtest/tests/gate_report_suite.rs`

**Interfaces:**
- Consumes: `bot_backtest::{BacktestMetrics, ParameterSensitivityReport, WalkForwardSummary}`, `bot_store::{SqliteStore, JournalEvent}`, `bot_types::Venue`, `rust_decimal::Decimal`.
- Produces:
  - `pub enum SpotGate { G1, G2, G3, G4 }`
  - `pub struct G1GateVerdict { pub passed: bool, pub checks: Vec<GateCheckItem> }`
  - `pub struct OperationalGateMetrics { pub venue: Venue, pub gate: SpotGate, pub total_fills: usize, pub realized_pnl_quote: Decimal, pub reconcile_incidents: usize, pub orphan_incidents: usize, pub halt_events: usize, pub passed: bool, pub checks: Vec<GateCheckItem> }`
  - `pub fn evaluate_g1_gate(metrics: &BacktestMetrics, sensitivity: &ParameterSensitivityReport, span_days: u64) -> G1GateVerdict`
  - `pub fn render_g1_markdown(venue: Venue, summary: &WalkForwardSummary, verdict: &G1GateVerdict, data_source: &str) -> String`
  - `pub fn evaluate_sqlite_gate(store: &SqliteStore, venue: Venue, gate: SpotGate) -> Result<OperationalGateMetrics, GateReportError>`
  - `pub fn render_operational_gate_markdown(metrics: &OperationalGateMetrics) -> String`

- [ ] **Step 1: Write failing tests in `crates/bot-backtest/tests/gate_report_suite.rs`**

Cover:
1. `g1_gate_passes_only_when_all_section_13_3_criteria_met` (verifies trades ≥ 300, span ≥ 1095 days, PF ≥ 1.3, expectancy ≥ 0.15, MaxDD ≤ 15%, sensitivity ≥ 70%, top-3-removed PF > 1.0).
2. `g1_gate_fails_when_any_single_threshold_misses` (Review Focus #3).
3. `sqlite_gate_report_filters_strictly_by_venue` (Review Focus #2 — inserts both Spot and Futures events into `SqliteStore::open_in_memory()`, verifies Spot report excludes Futures).
4. `sqlite_gate_report_handles_empty_db_cleanly` (Review Focus #1 — returns `total_fills == 0`, `passed == false`, no panic).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bot-backtest --test gate_report_suite`
Expected: FAIL (module `gate_report` does not exist).

- [ ] **Step 3: Implement `crates/bot-backtest/src/gate_report.rs` and export from `crates/bot-backtest/src/lib.rs`**

Implement the structs, typed `GateReportError`, §13.3 threshold evaluator, SQLite journal reader (filtering by `venue`), and Markdown renderers using `Decimal` arithmetic only.

- [ ] **Step 4: Run tests and clippy**

Run: `cargo test -p bot-backtest && cargo clippy -p bot-backtest --all-targets -- -D warnings`
Expected: PASS, zero warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/bot-backtest/src/gate_report.rs crates/bot-backtest/src/lib.rs crates/bot-backtest/tests/gate_report_suite.rs
git commit -m "feat(bot-backtest): add G1 and SQLite operational gate report evaluators"
```

---

### Task 3: Thin `bot-backtest` CLI Binary (`g1` & `gate-report`)

**Files:**
- Create: `crates/bot-backtest/src/bin/bot-backtest.rs`
- Test: `crates/bot-backtest/tests/cli_smoke.rs`

**Interfaces:**
- Consumes: `bot_backtest::gate_report::{SpotGate, evaluate_g1_gate, render_g1_markdown, evaluate_sqlite_gate, render_operational_gate_markdown}`, `bot_backtest::{parse_binance_kline_csv, WalkForwardRunner, WalkForwardConfig}`, `bot_store::SqliteStore`.
- Produces: Binary `bot-backtest` with subcommands:
  - `bot-backtest g1 --input <csv_path> --symbol <SYMBOL> --out <md_path>`
  - `bot-backtest gate-report --db <sqlite_path> --venue spot --gate <G2|G3|G4> --out <md_path>`

- [ ] **Step 1: Write failing CLI integration tests in `crates/bot-backtest/tests/cli_smoke.rs`**

Cover:
1. `cli_rejects_unknown_subcommand_and_invalid_gate` (Review Focus #4 — exits non-zero, prints usage).
2. `cli_gate_report_generates_markdown_from_temp_sqlite_db` (creates temp SQLite DB, runs `bot-backtest gate-report --db ... --venue spot --gate G2 --out ...`, asserts exit 0 and Markdown contains `Spot` and `G2`).
3. `cli_g1_generates_markdown_from_temp_kline_csv` (writes small synthetic Binance kline CSV to temp dir, runs `bot-backtest g1 --input ... --symbol BTCUSDT --out ...`, asserts exit 0 and Markdown contains `FAIL` because synthetic sample < 3 years / < 300 trades).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p bot-backtest --test cli_smoke`
Expected: FAIL (binary `bot-backtest` not found).

- [ ] **Step 3: Implement `crates/bot-backtest/src/bin/bot-backtest.rs`**

Parse arguments with `std::env::args` (no new dependencies), dispatch `g1` and `gate-report`, write Markdown output atomically, return `ExitCode::SUCCESS` / `ExitCode::FAILURE`.

- [ ] **Step 4: Run tests and clippy**

Run: `cargo test -p bot-backtest && cargo clippy -p bot-backtest --all-targets -- -D warnings`
Expected: PASS, zero warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/bot-backtest/src/bin/bot-backtest.rs crates/bot-backtest/tests/cli_smoke.rs
git commit -m "feat(bot-backtest): add CLI binary for G1 backtest and SQLite gate reports"
```

---

### Task 4: Spot Gate Documents (`G1.md`–`G5.md`), Testnet Runbook, `STATUS.md`, & `CHANGELOG.md`

**Files:**
- Create: `docs/gates/spot/G1.md`
- Create: `docs/gates/spot/G2.md`
- Create: `docs/gates/spot/G3.md`
- Create: `docs/gates/spot/G4.md`
- Create: `docs/gates/spot/G5.md`
- Modify: `docs/STATUS.md`
- Modify: `CHANGELOG.md`

**Interfaces:**
- Produces: Complete Spot gate documentation under `docs/gates/spot/` with explicit `STATUS: PENDING OPERATIONAL RUN (NOT APPROVED FOR LIVE)` markers (Review Focus #5) and exact `bot-backtest` / `binance-botd` testnet commands.

- [ ] **Step 1: Create `docs/gates/spot/G1.md`–`G5.md`**

- `G1.md`: Document G1 §13.3 criteria, exact command to run on 3+ year `data.binance.vision` CSV (`cargo run --release -p bot-backtest --bin bot-backtest -- g1 ...`), and owner sign-off section (`STATUS: PENDING FULL 3-YEAR DATASET RUN`).
- `G2.md`: Testnet ≥ 2 weeks runbook (CachyOS systemd setup, `mode = "paper"` / testnet endpoint config, `iptables` chaos check, `bot-backtest gate-report --gate G2` command, `STATUS: PENDING OPERATIONAL RUN (NOT APPROVED FOR LIVE)`).
- `G3.md`, `G4.md`, `G5.md`: Paper, Micro-live (0.1% risk + manual `live_confirmation_file`), and Scale templates referencing §13.3 and `bot-backtest gate-report`.

- [ ] **Step 2: Update `docs/STATUS.md` and `CHANGELOG.md`**

- Mark M14 as engineering/tooling complete per ADR-0005 (operational G2–G5 pending real-time testnet/paper runs; ready for owner approval to proceed to M15).
- Add M14 entry to `CHANGELOG.md`.

- [ ] **Step 3: Run full verification suite**

Run: `./scripts/verify.sh`
Expected: `OK: semua pemeriksaan lulus` (note: `desktop-ui` Qt6 check if Qt6 is absent in devbox, all Rust + web-ui + secret scan must be green).

- [ ] **Step 4: Commit**

```bash
git add docs/gates/spot/ docs/STATUS.md CHANGELOG.md
git commit -m "docs(m14): add Spot G1-G5 gate templates, testnet runbook, and update status"
```
