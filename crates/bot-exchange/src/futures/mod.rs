//! USDⓈ-M Futures adapter, wire parsers, and safe activation validation (§4.5).

pub mod adapter;
pub mod wire;

pub use adapter::{FuturesAccountSetup, FuturesAdapter, FuturesAdapterConfig, FuturesMarginType};
pub use wire::{
    FuturesUserEvent, build_cancel_algo_order_params, build_place_algo_order_params,
    parse_futures_exchange_info, parse_futures_user_event,
};
