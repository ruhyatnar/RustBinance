#![forbid(unsafe_code)]
//! Single-writer trading engine actor, channel backpressure, crash recovery,
//! and graceful shutdown (`bot-engine`, M07 g–h).

pub mod actor;

pub use actor::{
    COMMAND_CHANNEL_CAPACITY, EngineActor, EngineCommand, EngineError, EngineHandle,
    EngineHandleError, EntrySubmissionResult, MarketTick, TelemetrySnapshot,
    USER_EVENT_CHANNEL_CAPACITY,
};
