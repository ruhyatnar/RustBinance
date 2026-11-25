#![forbid(unsafe_code)]
//! Secure local IPC gateway: UDS (`SO_PEERCRED`), loopback HTTP/WS, protocol v1,
//! confirmation nonces, conflation, and audited command dispatch (§10).

pub mod auth;
pub mod command;
pub mod conflate;
pub mod error;
pub mod http;
pub mod nonce;
pub mod protocol;
pub mod uds;

pub use auth::{AuthScope, OriginPolicy, PeerAuthPolicy, RateLimiter, TokenAuth};
pub use command::{CommandDispatcher, InMemoryIpcBackend, IpcBackend};
pub use conflate::{TelemetryHub, conflate};
pub use error::IpcError;
pub use http::{HttpServerConfig, build_router, serve_http};
pub use nonce::{ConfirmNonceStore, DEFAULT_CONFIRM_TTL, IssuedNonce};
pub use protocol::{
    ClientMessage, CommandKind, CommandPayload, CommandResultPayload, ConfirmAction,
    GetKlinesPayload, GetTradesPayload, HttpErrorResponse, KlineBar, KlinesPayload,
    PROTOCOL_VERSION, ServerMessage, SnapshotPayload, TradeRecord, TradesPayload, WireEnvelope,
};
pub use uds::{UdsServerConfig, serve_uds};
