#![forbid(unsafe_code)]
//! Layered configuration loading, validation, and tighten-only checks.

mod decimal_serde;
pub mod error;
pub mod loader;
pub mod schema;
pub mod tighten;
mod validate;

pub use error::{ConfigError, LoosenError, LoosenViolation, RangeViolation};
pub use loader::{
    ConfigSourcePaths, load_config, load_from_paths, load_from_paths_with_env_pairs,
    load_from_toml_str,
};
pub use schema::{
    BinanceConfig, BotConfig, BreakoutConfig, CrossVenueSameSymbolPolicy, ExitConfig,
    FuturesRiskConfig, FuturesVenueConfig, GainersMomentumConfig, GeneralConfig, IpcConfig,
    MarginMode, OnOpenPositionsPolicy, OrphanPolicy, PositionMode, ProfitLockMode, RiskConfig,
    SafetyConfig, SizingMode, SpotVenueConfig, StorageConfig, StrategyConfig, TrendPullbackConfig,
    UniverseConfig, VenuesConfig, VwapReversionConfig,
};
pub use tighten::is_tightening;
