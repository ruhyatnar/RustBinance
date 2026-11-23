#![forbid(unsafe_code)]
//! Execution, Order Management System (OMS), Paper Broker, Dual-OCO protection,
//! position lifecycle, and reconciliation (`bot-exec`, M07).

pub mod client_id;
pub mod entry;
pub mod fsm;
pub mod lifecycle;
pub mod paper;
pub mod protection;
pub mod reconcile;

pub use client_id::{
    ClientIdError, MAX_CLIENT_ORDER_ID_LEN, ParsedClientOrderId, format_client_order_id,
    parse_client_order_id, parse_purpose_tag, purpose_tag,
};
pub use entry::{
    EntryExecError, MIN_VIABLE_FILL_RATIO, PassiveEntryOutcome, PassiveEntryPlan, TopOfBook,
    execute_passive_entry,
};
pub use fsm::{
    FsmApplyOutcome, MAX_UNKNOWN_QUERY_ATTEMPTS, OrderFsm, convert_fee_to_quote, is_terminal,
};
pub use lifecycle::{
    BEST_PRICE_JOURNAL_ATR_MULT, EXHAUSTION_MIN_PROFIT_R, ExitPnlReport, LifecycleAction,
    PositionTickInput, TIME_STOP_STAGNATION_R, compute_exit_pnl_report, evaluate_position_tick,
};
pub use paper::{PaperBroker, PaperBrokerConfig};
pub use protection::{
    DEFAULT_STOP_LIMIT_OFFSET_BPS, DualOcoManager, HardStopWatcher, PositionProtection,
    ProtectionError,
};
pub use reconcile::{ReconcileConfig, ReconcileError, ReconcileReport, reconcile_venue};
