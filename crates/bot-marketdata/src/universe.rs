//! Universe filter (§6.2) and `UniverseProvider` trait with 15-minute refresh support.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use bot_config::UniverseConfig;
use bot_types::{Symbol, Venue};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::error::MarketDataError;

/// Known stablecoin base/quote tickers excluded from stable-vs-stable trading pairs (§6.2).
const STABLECOINS: &[&str] = &[
    "USDT", "USDC", "FDUSD", "TUSD", "BUSD", "DAI", "USDP", "USDE", "EUR", "AEUR", "EURI",
];

/// Leveraged token base-asset suffixes excluded from the trading universe (§6.2).
const LEVERAGED_SUFFIXES: &[&str] = &["UP", "DOWN", "BULL", "BEAR"];

/// Candidate symbol metadata evaluated by [`UniverseFilter`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniverseCandidate {
    /// Venue.
    pub venue: Venue,
    /// Full symbol (e.g. `"BTCUSDT"`).
    pub symbol: Symbol,
    /// Base asset ticker (e.g. `"BTC"`).
    pub base_asset: String,
    /// Quote asset ticker (e.g. `"USDT"`).
    pub quote_asset: String,
    /// Whether exchange status is `TRADING` (not `BREAK` or `CANCEL_ONLY`).
    pub status_trading: bool,
    /// 24-hour quote volume in USD.
    pub quote_volume_24h_usd: Decimal,
    /// Median bid-ask spread in basis points.
    pub median_spread_bps: Decimal,
    /// Days elapsed since exchange listing.
    pub listing_age_days: u32,
}

/// Filtered trading universe snapshot for a venue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniverseSnapshot {
    /// Venue.
    pub venue: Venue,
    /// UTC epoch ms when this universe snapshot was produced.
    pub refreshed_at_ms: i64,
    /// Symbols passing core strategy thresholds (S1/S2).
    pub core_symbols: Vec<Symbol>,
    /// Symbols passing scanner thresholds (S3 gainers).
    pub scanner_symbols: Vec<Symbol>,
}

/// Pure deterministic universe filter implementing §6.2 rules.
#[derive(Debug, Clone)]
pub struct UniverseFilter {
    config: UniverseConfig,
    allowed_quotes: HashSet<String>,
    excluded_symbols: HashSet<String>,
}

impl UniverseFilter {
    /// Build a [`UniverseFilter`] from [`UniverseConfig`] and allowed quote assets (default `["USDT"]`).
    #[must_use]
    pub fn new(config: UniverseConfig, allowed_quotes: &[&str]) -> Self {
        let quotes = if allowed_quotes.is_empty() {
            HashSet::from(["USDT".to_owned()])
        } else {
            allowed_quotes
                .iter()
                .map(|q| q.to_ascii_uppercase())
                .collect()
        };
        let excluded = config
            .exclude_symbols
            .iter()
            .map(|s| s.to_ascii_uppercase())
            .collect();
        Self {
            config,
            allowed_quotes: quotes,
            excluded_symbols: excluded,
        }
    }

    /// Configured refresh interval (default 15 minutes per §6.2).
    #[must_use]
    pub fn refresh_interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.config.refresh_minutes.max(1)) * 60)
    }

    /// Evaluate a batch of [`UniverseCandidate`]s and produce a deterministic [`UniverseSnapshot`].
    #[must_use]
    pub fn evaluate(
        &self,
        venue: Venue,
        refreshed_at_ms: i64,
        candidates: &[UniverseCandidate],
    ) -> UniverseSnapshot {
        let mut core_symbols = Vec::new();
        let mut scanner_symbols = Vec::new();

        for c in candidates {
            if c.venue != venue || !self.passes_structural_exclusions(c) {
                continue;
            }
            if c.quote_volume_24h_usd >= self.config.min_quote_volume_24h_usd
                && c.median_spread_bps <= self.config.max_spread_bps
                && c.listing_age_days >= self.config.min_listing_age_days
            {
                core_symbols.push(c.symbol.clone());
            }
            if c.quote_volume_24h_usd >= self.config.scanner_min_quote_volume_24h_usd
                && c.median_spread_bps <= self.config.scanner_max_spread_bps
                && c.listing_age_days >= self.config.scanner_min_listing_age_days
            {
                scanner_symbols.push(c.symbol.clone());
            }
        }

        core_symbols.sort();
        core_symbols.dedup();
        scanner_symbols.sort();
        scanner_symbols.dedup();

        UniverseSnapshot {
            venue,
            refreshed_at_ms,
            core_symbols,
            scanner_symbols,
        }
    }

    fn passes_structural_exclusions(&self, c: &UniverseCandidate) -> bool {
        if !c.status_trading {
            return false;
        }
        let sym_upper = c.symbol.as_str().to_ascii_uppercase();
        if self.excluded_symbols.contains(&sym_upper) {
            return false;
        }
        let quote_upper = c.quote_asset.to_ascii_uppercase();
        if !self.allowed_quotes.contains(&quote_upper) {
            return false;
        }
        let base_upper = c.base_asset.to_ascii_uppercase();
        if is_stablecoin(&base_upper) && is_stablecoin(&quote_upper) {
            return false;
        }
        if is_leveraged_token(&base_upper) {
            return false;
        }
        true
    }
}

fn is_stablecoin(asset: &str) -> bool {
    STABLECOINS.contains(&asset)
}

fn is_leveraged_token(base_asset: &str) -> bool {
    // Only treat as leveraged token if base has a prefix before UP/DOWN/BULL/BEAR
    // (e.g. BTCUP, ETHDOWN, BNBULL), not bare tickers like "UP".
    LEVERAGED_SUFFIXES
        .iter()
        .any(|suf| base_asset.len() > suf.len() && base_asset.ends_with(suf))
}

/// Asynchronous provider supplying refreshed [`UniverseSnapshot`]s (§6.2).
#[async_trait]
pub trait UniverseProvider: Send + Sync {
    /// Fetch fresh candidate metadata and compute an updated [`UniverseSnapshot`].
    async fn refresh(&self) -> Result<UniverseSnapshot, MarketDataError>;

    /// Return the most recently computed snapshot, if any.
    fn latest(&self) -> Option<UniverseSnapshot>;
}

/// In-memory [`UniverseProvider`] backed by a [`UniverseFilter`] and candidate list (useful for tests/paper).
#[derive(Debug)]
pub struct FilteredUniverseProvider {
    venue: Venue,
    filter: UniverseFilter,
    candidates: RwLock<Vec<UniverseCandidate>>,
    latest: RwLock<Option<UniverseSnapshot>>,
}

impl FilteredUniverseProvider {
    /// Create a provider seeded with initial candidates.
    #[must_use]
    pub fn new(
        venue: Venue,
        filter: UniverseFilter,
        initial_candidates: Vec<UniverseCandidate>,
    ) -> Self {
        Self {
            venue,
            filter,
            candidates: RwLock::new(initial_candidates),
            latest: RwLock::new(None),
        }
    }

    /// Replace the candidate pool before the next refresh.
    pub fn set_candidates(&self, candidates: Vec<UniverseCandidate>) {
        if let Ok(mut guard) = self.candidates.write() {
            *guard = candidates;
        }
    }
}

#[async_trait]
impl UniverseProvider for FilteredUniverseProvider {
    async fn refresh(&self) -> Result<UniverseSnapshot, MarketDataError> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_millis()).ok())
            .unwrap_or(0);
        let list = self
            .candidates
            .read()
            .map_err(|_| MarketDataError::SyncFailed("universe candidates lock poisoned".into()))?
            .clone();
        let snap = self.filter.evaluate(self.venue, now_ms, &list);
        if let Ok(mut guard) = self.latest.write() {
            *guard = Some(snap.clone());
        }
        Ok(snap)
    }

    fn latest(&self) -> Option<UniverseSnapshot> {
        self.latest.read().ok().and_then(|g| g.clone())
    }
}

/// Spawn a periodic universe refresh task (default 15 minutes) publishing snapshots via `watch`.
pub fn spawn_universe_refresher(
    provider: Arc<dyn UniverseProvider>,
    interval: Duration,
) -> (
    watch::Receiver<Option<UniverseSnapshot>>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, rx) = watch::channel(provider.latest());
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            if let Ok(snap) = provider.refresh().await
                && tx.send(Some(snap)).is_err()
            {
                break;
            }
        }
    });
    (rx, handle)
}

#[cfg(test)]
mod tests {
    use bot_config::UniverseConfig;
    use bot_types::{Symbol, Venue};
    use rust_decimal::Decimal;

    use super::{UniverseCandidate, UniverseFilter};

    fn cand(
        sym: &str,
        base: &str,
        quote: &str,
        trading: bool,
        vol_m: u64,
        spread_bps: i64,
        age_days: u32,
    ) -> UniverseCandidate {
        UniverseCandidate {
            venue: Venue::Spot,
            symbol: Symbol(sym.into()),
            base_asset: base.into(),
            quote_asset: quote.into(),
            status_trading: trading,
            quote_volume_24h_usd: Decimal::from(vol_m * 1_000_000),
            median_spread_bps: Decimal::from(spread_bps),
            listing_age_days: age_days,
        }
    }

    #[test]
    fn enforces_liquidity_spread_age_and_exclusions() {
        let cfg = UniverseConfig {
            exclude_symbols: vec!["MONITORUSDT".into()],
            ..UniverseConfig::default()
        };
        let filter = UniverseFilter::new(cfg, &["USDT"]);

        let pool = vec![
            // Passes both core & scanner
            cand("BTCUSDT", "BTC", "USDT", true, 500, 1, 1000),
            // Passes scanner only (vol 20M, spread 10 bps, age 30d)
            cand("ALTUSDT", "ALT", "USDT", true, 20, 10, 30),
            // Excluded: leveraged token
            cand("BTCUPUSDT", "BTCUP", "USDT", true, 200, 2, 200),
            // Excluded: stablecoin vs stablecoin
            cand("USDCUSDT", "USDC", "USDT", true, 900, 1, 500),
            // Excluded: static monitoring/seed list
            cand("MONITORUSDT", "MONITOR", "USDT", true, 100, 2, 200),
            // Excluded: BREAK status
            cand("BREAKUSDT", "BREAK", "USDT", false, 100, 2, 200),
            // Excluded: quote not allowed
            cand("ETHBTC", "ETH", "BTC", true, 500, 1, 500),
        ];

        let snap = filter.evaluate(Venue::Spot, 1_700_000_000_000, &pool);
        assert_eq!(snap.core_symbols, vec![Symbol("BTCUSDT".into())]);
        assert_eq!(
            snap.scanner_symbols,
            vec![Symbol("ALTUSDT".into()), Symbol("BTCUSDT".into())]
        );
    }
}
