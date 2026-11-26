//! WS-first account balance cache with provenance tracking.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use bot_types::Venue;
use serde::{Deserialize, Serialize};

use crate::gateway::{AccountBalance, AccountSnapshot};

/// Origin of a served [`AccountSnapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BalanceSource {
    /// Served from the WS-maintained cache seeded by REST and kept current via WS events.
    Ws,
    /// Served directly from a fresh REST fallback query.
    Rest,
}

/// Freshness and origin metadata attached to a cached account snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceProvenance {
    /// Source of the snapshot.
    pub source: BalanceSource,
    /// Age in milliseconds since the most recent seed or WS balance update.
    pub age_ms: u64,
}

/// Account snapshot paired with its [`BalanceProvenance`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedAccountSnapshot {
    /// Account snapshot payload.
    pub snapshot: AccountSnapshot,
    /// Origin and age metadata.
    pub provenance: BalanceProvenance,
}

#[derive(Debug, Default)]
struct Inner {
    balances: BTreeMap<String, AccountBalance>,
    can_trade: bool,
    snapshot_update_time_ms: i64,
    last_rest_seed_ms: Option<i64>,
    last_update_ms: Option<i64>,
    needs_reseed: bool,
}

/// Thread-safe, WS-first account balance cache with REST-seed reconciliation bounds.
///
/// Once seeded via [`BalanceCache::seed_from_rest`], the cache remains authoritative
/// across idle WS sessions until either `reconcile_interval_ms` elapses or
/// [`BalanceCache::mark_ws_disconnected`] is called.
#[derive(Debug)]
pub struct BalanceCache {
    venue: Venue,
    reconcile_interval_ms: u64,
    inner: Mutex<Inner>,
}

impl BalanceCache {
    /// Create an unseeded [`BalanceCache`] for `venue` with the given REST reconcile interval.
    #[must_use]
    pub fn new(venue: Venue, reconcile_interval_ms: u64) -> Self {
        Self {
            venue,
            reconcile_interval_ms,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Seed (or re-seed) the cache from an authoritative REST [`AccountSnapshot`].
    ///
    /// Replaces all stored balances, clears any pending reseed flag, and resets
    /// both the reconcile timer and update age to `now_ms`.
    pub fn seed_from_rest(&self, snapshot: AccountSnapshot, now_ms: i64) {
        let mut inner = self.lock_inner();
        inner.balances.clear();
        for balance in snapshot.balances {
            inner.balances.insert(balance.asset.clone(), balance);
        }
        inner.can_trade = snapshot.can_trade;
        inner.snapshot_update_time_ms = snapshot.update_time_ms;
        inner.last_rest_seed_ms = Some(now_ms);
        inner.last_update_ms = Some(now_ms);
        inner.needs_reseed = false;
    }

    /// Apply incremental balance updates received on the WS user data stream.
    pub fn apply_ws_delta(&self, balances: &[AccountBalance], now_ms: i64) {
        let mut inner = self.lock_inner();
        for balance in balances {
            inner
                .balances
                .insert(balance.asset.clone(), balance.clone());
        }
        inner.snapshot_update_time_ms = now_ms;
        inner.last_update_ms = Some(now_ms);
    }

    /// Mark the WS session as disconnected, forcing REST re-seeding before serving cached state.
    pub fn mark_ws_disconnected(&self) {
        self.lock_inner().needs_reseed = true;
    }

    /// Return a cached [`CachedAccountSnapshot`] with [`BalanceSource::Ws`] if the WS
    /// session is ready, no reseed is pending, and the reconcile interval has not elapsed.
    #[must_use]
    pub fn get_if_fresh(&self, ws_ready: bool, now_ms: i64) -> Option<CachedAccountSnapshot> {
        if !ws_ready {
            return None;
        }
        let inner = self.lock_inner();
        if inner.needs_reseed {
            return None;
        }
        let seed_ms = inner.last_rest_seed_ms?;
        let update_ms = inner.last_update_ms?;

        let elapsed_since_seed_ms = elapsed_ms(now_ms, seed_ms);
        if elapsed_since_seed_ms > self.reconcile_interval_ms {
            return None;
        }

        let age_ms = elapsed_ms(now_ms, update_ms);
        let balances = inner.balances.values().cloned().collect();
        Some(CachedAccountSnapshot {
            snapshot: AccountSnapshot {
                venue: self.venue,
                can_trade: inner.can_trade,
                update_time_ms: inner.snapshot_update_time_ms,
                balances,
            },
            provenance: BalanceProvenance {
                source: BalanceSource::Ws,
                age_ms,
            },
        })
    }

    fn lock_inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn elapsed_ms(now_ms: i64, then_ms: i64) -> u64 {
    u64::try_from(now_ms.saturating_sub(then_ms).max(0)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use bot_types::Venue;
    use rust_decimal::Decimal;

    use super::{BalanceCache, BalanceSource};
    use crate::gateway::{AccountBalance, AccountSnapshot};

    #[test]
    fn ws_cache_stays_authoritative_during_idle_ready_session_until_reconcile_interval() {
        let cache = BalanceCache::new(Venue::Spot, 300_000);
        assert!(cache.get_if_fresh(true, 1_000).is_none()); // unseeded

        cache.seed_from_rest(
            AccountSnapshot {
                venue: Venue::Spot,
                can_trade: true,
                update_time_ms: 1_000,
                balances: vec![AccountBalance {
                    asset: "USDT".into(),
                    free: Decimal::from(1000),
                    locked: Decimal::ZERO,
                }],
            },
            1_000,
        );
        // Review Focus #3: idle 45s while WS Ready stays authoritative (source=Ws).
        let snap = cache.get_if_fresh(true, 46_000).expect("fresh");
        assert_eq!(snap.provenance.source, BalanceSource::Ws);
        assert_eq!(snap.provenance.age_ms, 45_000);
        assert!(snap.snapshot.can_trade);

        // WS event updates balance in place.
        cache.apply_ws_delta(
            &[AccountBalance {
                asset: "USDT".into(),
                free: Decimal::from(950),
                locked: Decimal::from(50),
            }],
            50_000,
        );
        let updated = cache.get_if_fresh(true, 52_000).expect("fresh");
        assert_eq!(updated.snapshot.balances[0].free, Decimal::from(950));
        assert_eq!(updated.snapshot.balances[0].locked, Decimal::from(50));
        assert_eq!(updated.provenance.age_ms, 2_000);
        assert_eq!(updated.snapshot.update_time_ms, 50_000);

        // Disconnect or exceeding 300s reconcile interval forces REST fallback.
        assert!(cache.get_if_fresh(false, 52_000).is_none());
        assert!(cache.get_if_fresh(true, 355_000).is_none());
    }

    #[test]
    fn mark_ws_disconnected_forces_none_until_reseeded_from_rest() {
        let cache = BalanceCache::new(Venue::UsdsFutures, 300_000);
        cache.seed_from_rest(
            AccountSnapshot {
                venue: Venue::UsdsFutures,
                can_trade: true,
                update_time_ms: 1_000,
                balances: vec![AccountBalance {
                    asset: "USDT".into(),
                    free: Decimal::from(500),
                    locked: Decimal::ZERO,
                }],
            },
            1_000,
        );
        assert!(cache.get_if_fresh(true, 2_000).is_some());

        // WS disconnect invalidates freshness even if ws_ready later flips true before REST reseed.
        cache.mark_ws_disconnected();
        assert!(cache.get_if_fresh(true, 2_500).is_none());

        // WS deltas alone while needing reseed must not clear needs_reseed.
        cache.apply_ws_delta(
            &[AccountBalance {
                asset: "USDT".into(),
                free: Decimal::from(600),
                locked: Decimal::ZERO,
            }],
            3_000,
        );
        assert!(cache.get_if_fresh(true, 3_500).is_none());

        // Re-seeding from REST restores freshness and clears stale assets.
        cache.seed_from_rest(
            AccountSnapshot {
                venue: Venue::UsdsFutures,
                can_trade: true,
                update_time_ms: 4_000,
                balances: vec![AccountBalance {
                    asset: "USDC".into(),
                    free: Decimal::from(750),
                    locked: Decimal::from(25),
                }],
            },
            4_000,
        );
        let reseeded = cache.get_if_fresh(true, 5_000).expect("reseeded fresh");
        assert_eq!(reseeded.provenance.source, BalanceSource::Ws);
        assert_eq!(reseeded.provenance.age_ms, 1_000);
        assert_eq!(reseeded.snapshot.balances.len(), 1);
        assert_eq!(reseeded.snapshot.balances[0].asset, "USDC");
        assert_eq!(reseeded.snapshot.balances[0].free, Decimal::from(750));
    }

    #[test]
    fn boundary_and_multi_asset_updates() {
        let cache = BalanceCache::new(Venue::Spot, 10_000);
        cache.seed_from_rest(
            AccountSnapshot {
                venue: Venue::Spot,
                can_trade: false,
                update_time_ms: 900,
                balances: vec![AccountBalance {
                    asset: "BTC".into(),
                    free: Decimal::new(15, 2),
                    locked: Decimal::ZERO,
                }],
            },
            1_000,
        );

        // Exact boundary (now_ms - last_rest_seed_ms == reconcile_interval_ms) is fresh.
        let at_boundary = cache.get_if_fresh(true, 11_000).expect("boundary fresh");
        assert_eq!(at_boundary.provenance.age_ms, 10_000);
        assert!(!at_boundary.snapshot.can_trade);

        // One ms past boundary forces REST fallback.
        assert!(cache.get_if_fresh(true, 11_001).is_none());

        // Adding another asset via WS delta preserves existing assets in deterministic order.
        cache.apply_ws_delta(
            &[AccountBalance {
                asset: "ETH".into(),
                free: Decimal::from(2),
                locked: Decimal::ZERO,
            }],
            5_000,
        );
        let multi = cache.get_if_fresh(true, 6_000).expect("multi fresh");
        assert_eq!(multi.snapshot.balances.len(), 2);
        assert_eq!(multi.snapshot.balances[0].asset, "BTC");
        assert_eq!(multi.snapshot.balances[1].asset, "ETH");
        assert_eq!(multi.provenance.age_ms, 1_000);
    }
}
