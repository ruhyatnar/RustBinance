//! §13.3 Gate evaluators (G1 walk-forward backtest + G2–G4 SQLite operational journal analyzer)
//! and Markdown report renderers (M14).

use std::fmt::Write as _;

use bot_store::{JournalEvent, SqliteStore, StoreError};
use bot_types::{EngineMode, PositionState, Venue};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::metrics::{BacktestMetrics, ParameterSensitivityReport};
use crate::walk_forward::WalkForwardSummary;

/// Spot validation gate identifier (§13.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpotGate {
    /// G1: Walk-forward backtest (>= 3 years, >= 300 trades, net 2x slippage).
    G1,
    /// G2: Testnet (>= 2 weeks, chaos tests passed, 0 unexplained reconcile incidents).
    G2,
    /// G3: Paper on live data (>= 6 weeks & >= 100 trades).
    G3,
    /// G4: Micro-live at 0.1% risk (>= 4 weeks & >= 50 trades).
    G4,
}

impl SpotGate {
    /// Parse gate string (`G1`, `G2`, `G3`, `G4`, case-insensitive).
    #[must_use]
    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_uppercase().as_str() {
            "G1" => Some(Self::G1),
            "G2" => Some(Self::G2),
            "G3" => Some(Self::G3),
            "G4" => Some(Self::G4),
            _ => None,
        }
    }

    /// Human-readable gate code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::G1 => "G1",
            Self::G2 => "G2",
            Self::G3 => "G3",
            Self::G4 => "G4",
        }
    }

    /// Minimum required operational duration in days (§13.3).
    #[must_use]
    pub const fn min_duration_days(self) -> u64 {
        match self {
            Self::G1 => 1095, // >= 3 years
            Self::G2 => 14,   // >= 2 weeks
            Self::G3 => 42,   // >= 6 weeks
            Self::G4 => 28,   // >= 4 weeks
        }
    }

    /// Minimum required closed trades (§13.3).
    #[must_use]
    pub const fn min_trades(self) -> usize {
        match self {
            Self::G1 => 300,
            Self::G2 => 1,
            Self::G3 => 100,
            Self::G4 => 50,
        }
    }
}

/// Errors produced while evaluating or rendering gate reports.
#[derive(Debug, Error)]
pub enum GateReportError {
    /// SQLite store error while reading journal events.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

/// Single pass/fail criterion row in a §13.3 gate evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateCheckItem {
    /// Criterion name.
    pub name: String,
    /// Required threshold description.
    pub threshold: String,
    /// Observed value description.
    pub observed: String,
    /// Whether this criterion passed.
    pub passed: bool,
}

/// Evaluation verdict for G1 Backtest (§13.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct G1GateVerdict {
    /// True only if every §13.3 G1 criterion passed.
    pub passed: bool,
    /// Individual criterion checks.
    pub checks: Vec<GateCheckItem>,
}

/// Operational metrics aggregated from `SqliteStore` journal for a specific `(Venue, Gate)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationalGateMetrics {
    /// Target venue strictly filtered from the journal.
    pub venue: Venue,
    /// Gate evaluated (`G2`, `G3`, or `G4`).
    pub gate: SpotGate,
    /// Total trade fills recorded for `venue`.
    pub total_fills: usize,
    /// Total closed positions recorded for `venue`.
    pub closed_positions: usize,
    /// Sum of realized `PnL` in quote currency across closed positions for `venue`.
    #[serde(with = "rust_decimal::serde::str")]
    pub realized_pnl_quote: Decimal,
    /// Sum of commissions paid across fills for `venue`.
    #[serde(with = "rust_decimal::serde::str")]
    pub total_fees_quote: Decimal,
    /// Observed duration in whole days between first and last venue event.
    pub observed_days: u64,
    /// Number of `Halted` / `Killed` engine mode transitions recorded.
    pub halt_events: usize,
    /// Number of position close reasons mentioning `orphan` or `reconcil` for `venue`.
    pub reconcile_incidents: usize,
    /// True only if all automated §13.3 criteria passed.
    pub passed: bool,
    /// Individual criterion checks.
    pub checks: Vec<GateCheckItem>,
}

/// Evaluate all §13.3 G1 Backtest criteria without relaxing any threshold.
#[must_use]
pub fn evaluate_g1_gate(
    metrics: &BacktestMetrics,
    sensitivity: &ParameterSensitivityReport,
    span_days: u64,
) -> G1GateVerdict {
    let min_pf = Decimal::new(13, 1); // 1.3
    let min_exp = Decimal::new(15, 2); // 0.15
    let max_dd = Decimal::new(15, 0); // 15.0
    let min_sens = Decimal::new(70, 0); // 70.0
    let min_top3_pf = Decimal::ONE; // > 1.0

    let checks = vec![
        GateCheckItem {
            name: "Dataset span".into(),
            threshold: ">= 1095 days (>= 3 years)".into(),
            observed: format!("{span_days} days"),
            passed: span_days >= 1095,
        },
        GateCheckItem {
            name: "Aggregate trades".into(),
            threshold: ">= 300".into(),
            observed: metrics.total_trades.to_string(),
            passed: metrics.total_trades >= 300,
        },
        GateCheckItem {
            name: "Profit Factor (net 2x slippage)".into(),
            threshold: ">= 1.3".into(),
            observed: metrics.profit_factor.to_string(),
            passed: metrics.profit_factor >= min_pf,
        },
        GateCheckItem {
            name: "Expectancy (R)".into(),
            threshold: ">= +0.15R".into(),
            observed: format!("{}R", metrics.expectancy_r),
            passed: metrics.expectancy_r >= min_exp,
        },
        GateCheckItem {
            name: "Max Drawdown".into(),
            threshold: "<= 15%".into(),
            observed: format!("{}%", metrics.max_drawdown_pct),
            passed: metrics.max_drawdown_pct <= max_dd,
        },
        GateCheckItem {
            name: "Parameter neighbors (±20% PF > 1.1)".into(),
            threshold: ">= 70%".into(),
            observed: format!("{}%", sensitivity.passing_ratio_pct),
            passed: sensitivity.passing_ratio_pct >= min_sens,
        },
        GateCheckItem {
            name: "PF without top-3 trades".into(),
            threshold: "> 1.0".into(),
            observed: metrics.profit_factor_without_top3.to_string(),
            passed: metrics.profit_factor_without_top3 > min_top3_pf,
        },
    ];

    let passed = checks.iter().all(|c| c.passed);
    G1GateVerdict { passed, checks }
}

/// Render a Markdown report for G1 Backtest (`docs/gates/spot/G1.md`).
#[must_use]
pub fn render_g1_markdown(
    venue: Venue,
    summary: &WalkForwardSummary,
    verdict: &G1GateVerdict,
    data_source: &str,
) -> String {
    let mut out = String::new();
    let status = if verdict.passed { "PASS" } else { "FAIL" };
    let _ = writeln!(out, "# Gate G1 — Walk-Forward Backtest ({venue:?})\n");
    let _ = writeln!(out, "- **Status:** `{status}`");
    let _ = writeln!(out, "- **Data source:** `{data_source}`");
    let _ = writeln!(out, "- **Walk-forward windows:** {}", summary.windows.len());
    let _ = writeln!(
        out,
        "- **Total parameter trials:** {}",
        summary.total_parameter_trials
    );
    let _ = writeln!(
        out,
        "- **Median OOS Profit Factor:** {}",
        summary.median_test_profit_factor
    );
    let _ = writeln!(
        out,
        "- **Median OOS Expectancy (R):** {}",
        summary.median_test_expectancy_r
    );
    let _ = writeln!(
        out,
        "- **Worst OOS Max Drawdown (%):** {}\n",
        summary.worst_test_max_drawdown_pct
    );

    let _ = writeln!(out, "## §13.3 Criteria Check\n");
    let _ = writeln!(out, "| Criterion | Threshold | Observed | Result |");
    let _ = writeln!(out, "|---|---|---|---|");
    for c in &verdict.checks {
        let mark = if c.passed { "PASS" } else { "FAIL" };
        let _ = writeln!(
            out,
            "| {} | {} | {} | **{}** |",
            c.name, c.threshold, c.observed, mark
        );
    }
    out
}

/// Analyze a `SqliteStore` journal strictly for `venue` and evaluate automated §13.3 gate criteria.
///
/// # Errors
/// Returns [`GateReportError::Store`] if reading from SQLite fails.
pub fn evaluate_sqlite_gate(
    store: &SqliteStore,
    venue: Venue,
    gate: SpotGate,
) -> Result<OperationalGateMetrics, GateReportError> {
    let rows = store.read_journal_after(0)?;

    let mut total_fills: usize = 0;
    let mut closed_positions: usize = 0;
    let mut realized_pnl_quote = Decimal::ZERO;
    let mut total_fees_quote = Decimal::ZERO;
    let mut halt_events: usize = 0;
    let mut reconcile_incidents: usize = 0;
    let mut min_ts_ms: Option<i64> = None;
    let mut max_ts_ms: Option<i64> = None;

    for (_seq, ts_ms, env) in rows {
        let event_venue = env.venue();
        let belongs_to_venue = event_venue == Some(venue);

        if belongs_to_venue {
            min_ts_ms = Some(min_ts_ms.map_or(ts_ms, |m| m.min(ts_ms)));
            max_ts_ms = Some(max_ts_ms.map_or(ts_ms, |m| m.max(ts_ms)));
        }

        match &env.event {
            JournalEvent::FillRecorded { fill } if fill.venue == venue => {
                total_fills += 1;
                total_fees_quote += fill.fee;
            }
            JournalEvent::PositionStateChanged {
                venue: pos_venue,
                state,
                realized_pnl_quote: pnl,
                close_reason,
                ..
            } if *pos_venue == venue => {
                if *state == PositionState::Closed {
                    closed_positions += 1;
                    realized_pnl_quote += *pnl;
                }
                if let Some(reason) = close_reason {
                    let lower = reason.to_ascii_lowercase();
                    if lower.contains("orphan") || lower.contains("reconcil") {
                        reconcile_incidents += 1;
                    }
                }
            }
            JournalEvent::EngineModeChanged { mode, .. } => {
                if matches!(mode, EngineMode::Halted | EngineMode::Killed) {
                    halt_events += 1;
                }
            }
            _ => {}
        }
    }

    let observed_days = match (min_ts_ms, max_ts_ms) {
        (Some(start), Some(end)) if end > start => {
            u64::try_from((end - start) / 86_400_000).unwrap_or(0)
        }
        _ => 0,
    };

    let min_days = gate.min_duration_days();
    let min_trades = gate.min_trades();

    let checks = vec![
        GateCheckItem {
            name: "Operational duration".into(),
            threshold: format!(">= {min_days} days"),
            observed: format!("{observed_days} days"),
            passed: observed_days >= min_days,
        },
        GateCheckItem {
            name: "Fills / trades recorded".into(),
            threshold: format!(">= {min_trades}"),
            observed: format!("{total_fills} fills ({closed_positions} closed positions)"),
            passed: total_fills >= min_trades,
        },
        GateCheckItem {
            name: "Unexplained reconcile / orphan incidents".into(),
            threshold: "== 0".into(),
            observed: reconcile_incidents.to_string(),
            passed: reconcile_incidents == 0,
        },
    ];

    let passed = checks.iter().all(|c| c.passed);

    Ok(OperationalGateMetrics {
        venue,
        gate,
        total_fills,
        closed_positions,
        realized_pnl_quote,
        total_fees_quote,
        observed_days,
        halt_events,
        reconcile_incidents,
        passed,
        checks,
    })
}

/// Render a Markdown report for an operational gate (`G2`, `G3`, or `G4`).
#[must_use]
pub fn render_operational_gate_markdown(metrics: &OperationalGateMetrics) -> String {
    let mut out = String::new();
    let status = if metrics.passed {
        "AUTOMATED CHECKS PASS (PENDING OWNER SIGN-OFF)"
    } else {
        "INCOMPLETE / FAIL (NOT APPROVED FOR LIVE)"
    };
    let _ = writeln!(
        out,
        "# Gate {} — Operational Report ({:?})\n",
        metrics.gate.as_str(),
        metrics.venue
    );
    let _ = writeln!(out, "- **Status:** `{status}`");
    let _ = writeln!(out, "- **Venue:** `{:?}`", metrics.venue);
    let _ = writeln!(out, "- **Observed duration:** {} days", metrics.observed_days);
    let _ = writeln!(out, "- **Total fills:** {}", metrics.total_fills);
    let _ = writeln!(out, "- **Closed positions:** {}", metrics.closed_positions);
    let _ = writeln!(
        out,
        "- **Realized PnL (quote):** {}",
        metrics.realized_pnl_quote
    );
    let _ = writeln!(
        out,
        "- **Total fees paid (quote):** {}",
        metrics.total_fees_quote
    );
    let _ = writeln!(out, "- **Engine Halt/Kill events:** {}", metrics.halt_events);
    let _ = writeln!(
        out,
        "- **Reconcile / orphan incidents:** {}\n",
        metrics.reconcile_incidents
    );

    let _ = writeln!(out, "## §13.3 Automated Criteria Check\n");
    let _ = writeln!(out, "| Criterion | Threshold | Observed | Result |");
    let _ = writeln!(out, "|---|---|---|---|");
    for c in &metrics.checks {
        let mark = if c.passed { "PASS" } else { "FAIL" };
        let _ = writeln!(
            out,
            "| {} | {} | {} | **{}** |",
            c.name, c.threshold, c.observed, mark
        );
    }
    out
}
