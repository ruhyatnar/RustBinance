//! USDⓈ-M Futures adapter, WS API session (`/ws-fapi/v1`), REST fallback (`/fapi/v1`),
//! and wire codecs (§4.5).

pub mod adapter;
pub mod rest;
pub mod session;
pub mod wire;

pub use adapter::{FuturesAccountSetup, FuturesAdapter, FuturesAdapterConfig, FuturesMarginType};
pub use rest::FuturesRestClient;
pub use session::FuturesWsSession;
pub use wire::{
    FuturesUserEvent, build_cancel_algo_order_params, build_place_algo_order_params,
    parse_futures_exchange_info, parse_futures_user_event,
};
