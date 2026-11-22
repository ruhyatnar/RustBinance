//! Complete configuration schema matching Blueprint v3.1 (§9, §7.2, §7.8).

use bot_types::{TradingMode, Venue};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::decimal_serde;

/// Root bot configuration loaded from defaults, TOML files, and `BOT__*` environment overrides.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BotConfig {
    /// General runtime settings.
    pub general: GeneralConfig,
    /// Safety switches.
    pub safety: SafetyConfig,
    /// Binance connectivity timings.
    pub binance: BinanceConfig,
    /// Tradable symbol universe filters.
    pub universe: UniverseConfig,
    /// Strategy configurations.
    pub strategy: StrategyConfig,
    /// Exit and trailing-stop parameters.
    pub exit: ExitConfig,
    /// Global & Spot risk limits plus `[risk.futures]` overrides.
    pub risk: RiskConfig,
    /// Venue activation, switching, and per-venue settings.
    pub venues: VenuesConfig,
    /// Local IPC and web server settings.
    pub ipc: IpcConfig,
    /// Persistence paths.
    pub storage: StorageConfig,
}

/// `[general]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GeneralConfig {
    /// Execution mode (`paper` | `testnet` | `live`); always defaults to `paper`.
    pub mode: TradingMode,
    /// Permitted quote assets (e.g. `["USDT"]`).
    pub quote_assets: Vec<String>,
    /// Reporting display timezone only; all risk calculations run in UTC.
    pub timezone_reporting: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            mode: TradingMode::Paper,
            quote_assets: vec!["USDT".to_owned()],
            timezone_reporting: "Asia/Jakarta".to_owned(),
        }
    }
}

/// `[safety]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SafetyConfig {
    /// Whether entering `Killed` flattens all open positions.
    pub kill_flatten: bool,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self { kill_flatten: true }
    }
}

/// `[binance]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BinanceConfig {
    /// Request `recvWindow` in milliseconds.
    pub recv_window_ms: u64,
    /// Enable REST fallback when WebSocket API is unavailable.
    pub rest_fallback: bool,
    /// Maximum tolerated local-to-server clock drift in milliseconds.
    pub max_clock_drift_ms: u64,
    /// Market data staleness threshold in milliseconds.
    pub stale_market_ms: u64,
}

impl Default for BinanceConfig {
    fn default() -> Self {
        Self {
            recv_window_ms: 5000,
            rest_fallback: true,
            max_clock_drift_ms: 800,
            stale_market_ms: 1500,
        }
    }
}

/// `[universe]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct UniverseConfig {
    /// Minimum 24h quote volume in USD for core strategies.
    #[serde(with = "decimal_serde")]
    pub min_quote_volume_24h_usd: Decimal,
    /// Minimum 24h quote volume in USD for scanner strategies.
    #[serde(with = "decimal_serde")]
    pub scanner_min_quote_volume_24h_usd: Decimal,
    /// Minimum listing age in days for core strategies.
    pub min_listing_age_days: u32,
    /// Minimum listing age in days for scanner strategies.
    pub scanner_min_listing_age_days: u32,
    /// Maximum bid-ask spread in bps for core strategies.
    #[serde(with = "decimal_serde")]
    pub max_spread_bps: Decimal,
    /// Maximum bid-ask spread in bps for scanner strategies.
    #[serde(with = "decimal_serde")]
    pub scanner_max_spread_bps: Decimal,
    /// Universe refresh cadence in minutes.
    pub refresh_minutes: u32,
    /// Static symbol exclusion list (e.g. Binance monitoring/seed tokens).
    pub exclude_symbols: Vec<String>,
}

impl Default for UniverseConfig {
    fn default() -> Self {
        Self {
            min_quote_volume_24h_usd: Decimal::from(50_000_000_u64),
            scanner_min_quote_volume_24h_usd: Decimal::from(10_000_000_u64),
            min_listing_age_days: 90,
            scanner_min_listing_age_days: 14,
            max_spread_bps: Decimal::new(50, 1),
            scanner_max_spread_bps: Decimal::new(150, 1),
            refresh_minutes: 15,
            exclude_symbols: Vec::new(),
        }
    }
}

/// `[strategy]` configuration container.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct StrategyConfig {
    /// S1 Trend Pullback strategy parameters.
    pub trend_pullback: TrendPullbackConfig,
    /// S2 Donchian Breakout strategy parameters.
    pub breakout: BreakoutConfig,
    /// S3 Gainers Momentum strategy parameters.
    pub gainers_momentum: GainersMomentumConfig,
    /// S4 VWAP Mean Reversion strategy parameters.
    pub vwap_reversion: VwapReversionConfig,
}

/// `[strategy.trend_pullback]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TrendPullbackConfig {
    /// Whether this strategy is enabled.
    pub enabled: bool,
    /// Signal timeframe (e.g. `"1h"`).
    pub signal_tf: String,
    /// Higher timeframe trend filter (e.g. `"4h"`).
    pub htf: String,
    /// Minimum RSI threshold for pullback entry.
    #[serde(with = "decimal_serde")]
    pub rsi_min: Decimal,
    /// Maximum RSI threshold for pullback entry.
    #[serde(with = "decimal_serde")]
    pub rsi_max: Decimal,
    /// Minimum ADX trend strength.
    #[serde(with = "decimal_serde")]
    pub adx_min: Decimal,
    /// TP1 reward multiple in R.
    #[serde(with = "decimal_serde")]
    pub tp1_r: Decimal,
    /// TP2 reward multiple in R.
    #[serde(with = "decimal_serde")]
    pub tp2_r: Decimal,
    /// Fraction of position closed at TP1.
    #[serde(with = "decimal_serde")]
    pub tp1_close_ratio: Decimal,
    /// Minimum expected reward-to-cost multiple.
    #[serde(with = "decimal_serde")]
    pub cost_multiple: Decimal,
}

impl Default for TrendPullbackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            signal_tf: "1h".to_owned(),
            htf: "4h".to_owned(),
            rsi_min: Decimal::from(40_u32),
            rsi_max: Decimal::from(62_u32),
            adx_min: Decimal::from(20_u32),
            tp1_r: Decimal::new(15, 1),
            tp2_r: Decimal::new(30, 1),
            tp1_close_ratio: Decimal::new(5, 1),
            cost_multiple: Decimal::new(30, 1),
        }
    }
}

/// `[strategy.breakout]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct BreakoutConfig {
    /// Whether this strategy is enabled.
    pub enabled: bool,
    /// Donchian channel lookback period in bars.
    pub donchian: u32,
    /// Minimum volume multiple versus moving average.
    #[serde(with = "decimal_serde")]
    pub min_volume_mult: Decimal,
}

impl Default for BreakoutConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            donchian: 20,
            min_volume_mult: Decimal::new(15, 1),
        }
    }
}

/// `[strategy.gainers_momentum]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GainersMomentumConfig {
    /// Whether this strategy is enabled.
    pub enabled: bool,
    /// If true, signals are logged without placing orders.
    pub shadow_only: bool,
    /// Minimum 24h percentage gain filter.
    #[serde(with = "decimal_serde")]
    pub min_gain_24h_pct: Decimal,
    /// Maximum 24h percentage gain filter.
    #[serde(with = "decimal_serde")]
    pub max_gain_24h_pct: Decimal,
    /// Position size multiplier relative to base risk.
    #[serde(with = "decimal_serde")]
    pub size_multiplier: Decimal,
    /// Maximum concurrent open positions from this strategy.
    pub max_concurrent: u32,
    /// Maximum hold duration in hours before time-stop.
    pub time_stop_hours: u32,
}

impl Default for GainersMomentumConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            shadow_only: true,
            min_gain_24h_pct: Decimal::from(5_u32),
            max_gain_24h_pct: Decimal::from(25_u32),
            size_multiplier: Decimal::new(5, 1),
            max_concurrent: 2,
            time_stop_hours: 24,
        }
    }
}

/// `[strategy.vwap_reversion]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VwapReversionConfig {
    /// Whether this strategy is enabled.
    pub enabled: bool,
    /// If true, signals are logged without placing orders.
    pub shadow_only: bool,
}

impl Default for VwapReversionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            shadow_only: true,
        }
    }
}

/// `[exit]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ExitConfig {
    /// R multiple at which stop moves to breakeven.
    #[serde(with = "decimal_serde")]
    pub be_trigger_r: Decimal,
    /// Fee buffer added to breakeven stop in bps.
    #[serde(with = "decimal_serde")]
    pub be_fee_buffer_bps: Decimal,
    /// R multiple at which trailing stop activates.
    #[serde(with = "decimal_serde")]
    pub trail_activate_r: Decimal,
    /// Trailing stop distance as ATR multiple.
    #[serde(with = "decimal_serde")]
    pub trail_atr_mult: Decimal,
    /// Minimum trailing stop ratchet step in ATR multiples.
    #[serde(with = "decimal_serde")]
    pub trail_min_step_atr: Decimal,
    /// RSI threshold indicating exhaustion for tighter trailing.
    #[serde(with = "decimal_serde")]
    pub exhaustion_rsi: Decimal,
    /// Trailing multiplier tightening factor on exhaustion.
    #[serde(with = "decimal_serde")]
    pub exhaustion_trail_factor: Decimal,
    /// Maximum bars held before time-stop.
    pub max_bars_in_trade: u32,
    /// Grace period in milliseconds after entry fill before emergency hard stop.
    pub hard_stop_grace_ms: u64,
}

impl Default for ExitConfig {
    fn default() -> Self {
        Self {
            be_trigger_r: Decimal::new(10, 1),
            be_fee_buffer_bps: Decimal::from(20_u32),
            trail_activate_r: Decimal::new(15, 1),
            trail_atr_mult: Decimal::new(30, 1),
            trail_min_step_atr: Decimal::new(25, 2),
            exhaustion_rsi: Decimal::from(78_u32),
            exhaustion_trail_factor: Decimal::new(7, 1),
            max_bars_in_trade: 48,
            hard_stop_grace_ms: 1500,
        }
    }
}

/// Position sizing mode (§7.3).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizingMode {
    /// Risk a fixed percentage of equity per trade (default).
    #[default]
    FixedRiskPct,
    /// Fixed quote notional per trade (intended for testnet).
    FixedNotional,
}

/// Daily profit-lock behavior (§7.4). Ordering reflects strictness: `Off < Reduce < Stop`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfitLockMode {
    /// No daily profit lock.
    Off,
    /// Halve per-trade risk after reaching daily profit threshold (default).
    #[default]
    Reduce,
    /// Stop new entries for the rest of the UTC day.
    Stop,
}

/// Policy for handling orphaned exchange positions discovered on reconciliation (§7.6).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrphanPolicy {
    /// Adopt orphaned position and immediately attach stop protection.
    #[default]
    AdoptWithStop,
    /// Flatten orphaned position immediately.
    Flatten,
}

/// `[risk]` configuration section (§7.2, §9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RiskConfig {
    /// Position sizing mode.
    pub sizing_mode: SizingMode,
    /// Risk per trade as percentage of equity (`0.1..=1.0`).
    #[serde(with = "decimal_serde")]
    pub risk_per_trade_pct: Decimal,
    /// Maximum simultaneous open positions (`1..=8`).
    pub max_open_positions: u32,
    /// Maximum combined portfolio heat percentage (`0.5..=4.0`).
    #[serde(with = "decimal_serde")]
    pub portfolio_heat_max_pct: Decimal,
    /// Maximum notional per position as percentage of equity (`5..=35`).
    #[serde(with = "decimal_serde")]
    pub max_notional_per_position_pct: Decimal,
    /// Maximum total exposure as percentage of equity (`20..=80`).
    #[serde(with = "decimal_serde")]
    pub max_total_exposure_pct: Decimal,
    /// Maximum correlated positions in the same direction across venues (`1..=5`).
    pub max_same_direction_correlated: u32,
    /// Daily realized+unrealized loss halt percentage (`-4.0..=-0.5`).
    #[serde(with = "decimal_serde")]
    pub daily_loss_halt_pct: Decimal,
    /// Weekly realized+unrealized loss halt percentage (`-15.0..=-1.0`, `<= daily_loss_halt_pct`).
    #[serde(with = "decimal_serde")]
    pub weekly_loss_halt_pct: Decimal,
    /// Peak-to-trough drawdown kill-switch percentage (`-20.0..=-5.0`, `<= weekly_loss_halt_pct`).
    #[serde(with = "decimal_serde")]
    pub drawdown_kill_pct: Decimal,
    /// Daily profit-lock mode (`reduce` | `stop` | `off`).
    pub profit_lock_mode: ProfitLockMode,
    /// Maximum tolerated entry slippage in bps (`2..=25`).
    #[serde(with = "decimal_serde")]
    pub max_entry_slippage_bps: Decimal,
    /// Daily order count circuit breaker (`1..=1000`).
    pub max_orders_per_day: u32,
    /// Policy for orphaned exchange positions.
    pub orphan_policy: OrphanPolicy,
    /// BTC 15-minute drop percentage triggering BTC Crash Guard.
    #[serde(with = "decimal_serde")]
    pub btc_crash_15m_pct: Decimal,
    /// BTC 60-minute drop percentage triggering BTC Crash Guard.
    #[serde(with = "decimal_serde")]
    pub btc_crash_60m_pct: Decimal,
    /// Futures-specific risk overrides (`[risk.futures]`).
    pub futures: FuturesRiskConfig,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            sizing_mode: SizingMode::FixedRiskPct,
            risk_per_trade_pct: Decimal::new(5, 1),
            max_open_positions: 4,
            portfolio_heat_max_pct: Decimal::new(20, 1),
            max_notional_per_position_pct: Decimal::from(20_u32),
            max_total_exposure_pct: Decimal::from(60_u32),
            max_same_direction_correlated: 3,
            daily_loss_halt_pct: Decimal::new(-20, 1),
            weekly_loss_halt_pct: Decimal::new(-50, 1),
            drawdown_kill_pct: Decimal::new(-100, 1),
            profit_lock_mode: ProfitLockMode::Reduce,
            max_entry_slippage_bps: Decimal::from(10_u32),
            max_orders_per_day: 200,
            orphan_policy: OrphanPolicy::AdoptWithStop,
            btc_crash_15m_pct: Decimal::new(15, 1),
            btc_crash_60m_pct: Decimal::new(30, 1),
            futures: FuturesRiskConfig::default(),
        }
    }
}

/// `[risk.futures]` configuration section (§7.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FuturesRiskConfig {
    /// Futures risk per trade percentage (`0.1..=0.75`).
    #[serde(with = "decimal_serde")]
    pub risk_per_trade_pct: Decimal,
    /// Short position sizing multiplier (`0.25..=1.0`).
    #[serde(with = "decimal_serde")]
    pub short_size_multiplier: Decimal,
    /// Maximum effective portfolio leverage (`0.5..=3.0`).
    #[serde(with = "decimal_serde")]
    pub max_effective_leverage: Decimal,
    /// Minimum liquidation distance multiple of stop distance (`2.0..=6.0`).
    #[serde(with = "decimal_serde")]
    pub liq_buffer_mult: Decimal,
    /// Emergency liquidation distance multiple triggering reduction (`1.0..=2.0`, `< liq_buffer_mult`).
    #[serde(with = "decimal_serde")]
    pub liq_emergency_mult: Decimal,
    /// Adverse 8h funding rate percentage blocking new entries (`0.01..=0.1`).
    #[serde(with = "decimal_serde")]
    pub funding_adverse_block_pct: Decimal,
    /// Maximum expected funding cost per trade in R (`0.01..=0.5`).
    #[serde(with = "decimal_serde")]
    pub max_funding_cost_r: Decimal,
    /// Maximum mark-vs-last price divergence percentage before pausing entries (`0.2..=1.0`).
    #[serde(with = "decimal_serde")]
    pub mark_last_divergence_pct: Decimal,
    /// Maximum notional per Futures position as percentage of venue equity (`5..=35`).
    #[serde(with = "decimal_serde")]
    pub max_notional_per_position_pct: Decimal,
    /// Maximum simultaneous open Futures positions (`1..=6`).
    pub max_open_positions: u32,
    /// BTC 15-minute spike percentage blocking Futures short entries.
    #[serde(with = "decimal_serde")]
    pub btc_spike_15m_pct: Decimal,
    /// BTC 60-minute spike percentage blocking Futures short entries.
    #[serde(with = "decimal_serde")]
    pub btc_spike_60m_pct: Decimal,
}

impl Default for FuturesRiskConfig {
    fn default() -> Self {
        Self {
            risk_per_trade_pct: Decimal::new(35, 2),
            short_size_multiplier: Decimal::new(75, 2),
            max_effective_leverage: Decimal::new(20, 1),
            liq_buffer_mult: Decimal::new(30, 1),
            liq_emergency_mult: Decimal::new(15, 1),
            funding_adverse_block_pct: Decimal::new(3, 2),
            max_funding_cost_r: Decimal::new(15, 2),
            mark_last_divergence_pct: Decimal::new(5, 1),
            max_notional_per_position_pct: Decimal::from(20_u32),
            max_open_positions: 3,
            btc_spike_15m_pct: Decimal::new(15, 1),
            btc_spike_60m_pct: Decimal::new(30, 1),
        }
    }
}

/// Default policy for open positions when switching venues (§3.5).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnOpenPositionsPolicy {
    /// Drain old venue while managing existing positions and protections until flat.
    #[default]
    KeepManaging,
    /// Reject switch if any position is still open on the old venue.
    RequireFlat,
    /// Flatten open positions before switching (requires confirmation nonce).
    Flatten,
}

/// Cross-venue policy when the same base symbol is traded on another venue (§3.5).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossVenueSameSymbolPolicy {
    /// Forbid opening opposite direction on the same symbol in the other venue (default).
    #[default]
    BlockOpposite,
    /// Allow independent positions across venues.
    Allow,
}

/// Futures position mode (§7.8, §9).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionMode {
    /// One-way position mode (only permitted mode).
    #[default]
    OneWay,
    /// Hedge mode (rejected by validation).
    Hedge,
}

/// Futures margin mode (§7.8).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarginMode {
    /// Isolated margin per symbol (only permitted mode).
    #[default]
    Isolated,
    /// Cross margin (rejected by validation).
    Cross,
}

/// `[venues]` configuration section (§3.5, §9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VenuesConfig {
    /// Active venue at startup (`spot` | `futures`).
    pub default_active: Venue,
    /// Whether runtime venue switching is allowed.
    pub allow_switch: bool,
    /// Minimum cooldown between venue switches in seconds.
    pub switch_cooldown_s: u64,
    /// Maximum venue switches per UTC day.
    pub max_switches_per_day: u32,
    /// Default open-position handling policy during venue switches.
    pub on_open_positions_default: OnOpenPositionsPolicy,
    /// Cross-venue same-symbol direction policy.
    pub cross_venue_same_symbol: CrossVenueSameSymbolPolicy,
    /// Spot venue configuration (`[venues.spot]`).
    pub spot: SpotVenueConfig,
    /// Futures venue configuration (`[venues.futures]`).
    pub futures: FuturesVenueConfig,
}

impl Default for VenuesConfig {
    fn default() -> Self {
        Self {
            default_active: Venue::Spot,
            allow_switch: true,
            switch_cooldown_s: 60,
            max_switches_per_day: 3,
            on_open_positions_default: OnOpenPositionsPolicy::KeepManaging,
            cross_venue_same_symbol: CrossVenueSameSymbolPolicy::BlockOpposite,
            spot: SpotVenueConfig::default(),
            futures: FuturesVenueConfig::default(),
        }
    }
}

/// `[venues.spot]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SpotVenueConfig {
    /// Whether Spot venue is enabled.
    pub enabled: bool,
    /// Path to Spot API key file.
    pub api_key_file: String,
    /// Path to Spot Ed25519 private key PEM file.
    pub private_key_path: String,
    /// Capital cap in USD (`0` = entire Spot quote balance).
    #[serde(with = "decimal_serde")]
    pub capital_allocation_usd: Decimal,
    /// Path to Spot live-mode confirmation token file (required for `mode = "live"`).
    pub live_confirmation_file: String,
}

impl Default for SpotVenueConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key_file: "/etc/binance-bot/keys/api_key_spot.txt".to_owned(),
            private_key_path: "/etc/binance-bot/keys/binance_spot_ed25519.pem".to_owned(),
            capital_allocation_usd: Decimal::ZERO,
            live_confirmation_file: String::new(),
        }
    }
}

/// `[venues.futures]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FuturesVenueConfig {
    /// Whether Futures venue is enabled.
    pub enabled: bool,
    /// Path to Futures API key file.
    pub api_key_file: String,
    /// Path to Futures Ed25519 private key PEM file.
    pub private_key_path: String,
    /// Capital cap in USD (`0` = entire Futures quote balance).
    #[serde(with = "decimal_serde")]
    pub capital_allocation_usd: Decimal,
    /// Path to Futures live-mode confirmation token file (required for `mode = "live"`).
    pub live_confirmation_file: String,
    /// Whether short entries are allowed on Futures.
    pub allow_short: bool,
    /// Exchange position mode (`one_way` only).
    pub position_mode: PositionMode,
    /// Exchange margin mode (`isolated` only; `cross` is rejected).
    pub margin_mode: MarginMode,
    /// Exchange leverage hard cap (`1..=5`).
    pub max_leverage: u32,
    /// Minimum 24h quote volume in USD for Futures symbols.
    #[serde(with = "decimal_serde")]
    pub min_quote_volume_24h_usd: Decimal,
    /// Minimum listing age in days for Futures symbols.
    pub min_listing_age_days: u32,
}

impl Default for FuturesVenueConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key_file: "/etc/binance-bot/keys/api_key_futures.txt".to_owned(),
            private_key_path: "/etc/binance-bot/keys/binance_futures_ed25519.pem".to_owned(),
            capital_allocation_usd: Decimal::ZERO,
            live_confirmation_file: String::new(),
            allow_short: true,
            position_mode: PositionMode::OneWay,
            margin_mode: MarginMode::Isolated,
            max_leverage: 3,
            min_quote_volume_24h_usd: Decimal::from(100_000_000_u64),
            min_listing_age_days: 30,
        }
    }
}

/// `[ipc]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct IpcConfig {
    /// Unix domain socket path for local desktop UI and CLI.
    pub uds_path: String,
    /// Local HTTP/WS bind address (loopback).
    pub web_bind: String,
    /// Path to full-control bearer token file.
    pub web_token_file: String,
    /// Optional path to read-only bearer token file.
    pub web_readonly_token_file: String,
    /// Conflated UI telemetry publish rate in Hz.
    pub telemetry_hz: u32,
}

impl Default for IpcConfig {
    fn default() -> Self {
        Self {
            uds_path: "/run/binance-bot/bot.sock".to_owned(),
            web_bind: "127.0.0.1:3001".to_owned(),
            web_token_file: "/etc/binance-bot/keys/web_token".to_owned(),
            web_readonly_token_file: String::new(),
            telemetry_hz: 8,
        }
    }
}

/// `[storage]` configuration section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct StorageConfig {
    /// Path to SQLite WAL state database.
    pub db_path: String,
    /// Directory for periodic SQLite backups.
    pub backup_dir: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            db_path: "/var/lib/binance-bot/state.sqlite".to_owned(),
            backup_dir: "/var/lib/binance-bot/backup".to_owned(),
        }
    }
}
