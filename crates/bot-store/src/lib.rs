#![forbid(unsafe_code)]
//! SQLite WAL event journal, snapshots, crash recovery, and persistent counters (§8).

pub mod error;
pub mod events;
pub mod state;
pub mod store;

pub use error::StoreError;
pub use events::{
    EVENT_SCHEMA_VERSION, FillRecord, JournalEnvelope, JournalEvent, VenueStateRecord,
};
pub use state::PersistedEngineState;
pub use store::{COUNTER_ORDER_SEQ, COUNTER_RUN_ID, SqliteStore};
