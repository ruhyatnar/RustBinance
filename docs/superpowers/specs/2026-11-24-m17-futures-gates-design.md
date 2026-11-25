# M17 — Futures Operational Gates Tooling & Runbooks Design

- **Date:** 2026-11-24
- **Milestone:** M17 (`docs/milestones/M17-OPERASIONAL-futures-gates.md`)
- **Status:** Draft (awaiting owner approval)
- **Reference:** `docs/spec/13-validation.md` §13.3–§13.4, `docs/spec/07-risk.md` §7.8, `ADR-0005`

## Intent & Why
Milestone M17 requires separate G1–G5 validation gates for **Futures-Long** and **Futures-Short** (no cross-inheritance), testnet verification of venue switching (§13.4), and an optional `dual` mode strictly gated behind a separate ADR after both Spot G4 and Futures G4 pass. Following `ADR-0005`, we deliver complete engineering/tooling and operational runbooks while keeping `mode=live` hard-locked until real multi-week operational evidence is signed off by the owner.

## Goals, Non-Goals, Anti-Goals
- **Goals:**
  1. Extend `crates/bot-backtest` (`gate_report.rs` + `bot-backtest` CLI) with direction-aware operational gate evaluation (`GateScope`: `Spot`, `FuturesLong`, `FuturesShort`) so SQLite journal metrics strictly separate Futures-Long and Futures-Short trades and check Futures-specific safety invariants (`liquidation_incidents == 0`, `unprotected_algo_incidents == 0`).
  2. Create structured operational gate documentation and runbooks under `docs/gates/futures/`:
     - `long/G1.md` .. `G5.md`
     - `short/G1.md` .. `G5.md`
     - `chaos.md` (Futures §13.4 chaos matrix)
     - `venue-switch-testnet.md` (Spot ⇄ Futures testnet switch runbook)
  3. Create `docs/decisions/0006-dual-venue-mode-gate.md` (`Status: Draft / Pending G4 Spot & G4 Futures`) documenting the exact prerequisites before `dual` mode may ever be enabled.
- **Non-Goals:**
  - Running multi-week live/testnet sessions inside CI or enabling `mode=live`.
  - Loosening any §13.3 threshold, leverage cap (1–2× micro-live), or 0.1% micro-live risk cap.
- **Anti-Goals:**
  - Marking any Futures gate `APPROVED FOR LIVE` without owner-signed operational evidence.
  - Mixing Futures-Long and Futures-Short trades in a single gate verdict.

## Approaches Considered
1. **Option A (Selected — Recommended):** Follow M14 + `ADR-0005` pattern: extend `bot-backtest` evaluator/CLI for `(Venue, Direction)` + Futures incident checks, create `docs/gates/futures/` runbooks/templates, and add draft `ADR-0006` for `dual` mode.
   - *Trade-off:* Slightly more code than docs-only, but eliminates manual error when auditing Futures-Long vs Futures-Short SQLite journals.
2. **Option B (Rejected):** Documentation/runbooks only under `docs/gates/futures/` without extending `bot-backtest`.
   - *Trade-off:* Faster, but leaves G2–G4 Futures-Long vs Futures-Short journal evaluation manual and error-prone.

## Technical Sketch
- **`crates/bot-backtest/src/gate_report.rs`:**
  - Add `GateScope` enum (`Spot`, `FuturesLong`, `FuturesShort`) with `parse()` (`spot`, `futures-long`, `futures-short`), `venue()`, `direction() -> Option<Direction>`, and `as_str()`.
  - Extend `OperationalGateMetrics` with `scope: GateScope`, `liquidation_incidents: usize`, and `unprotected_algo_incidents: usize` (keeping backwards-compatible `evaluate_operational_gate` for Spot and adding `evaluate_operational_gate_for_scope`).
  - For Futures scopes (`FuturesLong` / `FuturesShort`), `evaluate_operational_gate_verdict` requires `liquidation_incidents == 0` and `unprotected_algo_incidents == 0` in addition to duration, closed trades, and `unexplained_reconcile_incidents == 0`.
  - Update `bot-backtest gate-report` CLI to accept `--scope spot|futures-long|futures-short` (defaulting to `--venue` for backward compatibility).
- **Tests:** Unit + SQLite journal integration tests verifying strict separation of `FuturesLong` vs `FuturesShort` positions and failure when liquidation/unprotected incidents > 0.
- **Docs:** `docs/gates/futures/{long,short}/G{1..5}.md`, `docs/gates/futures/chaos.md`, `docs/gates/futures/venue-switch-testnet.md`, and `docs/decisions/0006-dual-venue-mode-gate.md`.
