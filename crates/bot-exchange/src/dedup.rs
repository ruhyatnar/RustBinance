//! Bounded event deduplicator for make-before-break connection overlap (§4.2).

use std::collections::{HashSet, VecDeque};

/// Deduplication key for execution reports during WS connection rotation overlap.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExecDedupKey {
    /// Trade/fill execution keyed strictly by `(order_id, trade_id)` (§4.2).
    Fill {
        /// Exchange order ID.
        order_id: i64,
        /// Exchange trade ID (`>= 0`).
        trade_id: i64,
    },
    /// Non-trade execution report (NEW, CANCELED, EXPIRED, REJECTED, etc.).
    Lifecycle {
        /// Exchange order ID.
        order_id: i64,
        /// Event timestamp (ms).
        event_time_ms: i64,
        /// Execution type tag (e.g. `"NEW"`, `"CANCELED"`).
        exec_type: String,
    },
}

/// Bounded FIFO deduplicator preventing duplicate events across overlapping connections.
#[derive(Debug)]
pub struct EventDeduplicator {
    capacity: usize,
    seen: HashSet<ExecDedupKey>,
    order: VecDeque<ExecDedupKey>,
}

impl EventDeduplicator {
    /// Create a deduplicator holding at most `capacity` recent keys (minimum 1).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            capacity: cap,
            seen: HashSet::with_capacity(cap),
            order: VecDeque::with_capacity(cap),
        }
    }

    /// Record a key. Returns `true` if this event is **new**, `false` if it is a duplicate.
    pub fn insert(&mut self, key: ExecDedupKey) -> bool {
        if self.seen.contains(&key) {
            return false;
        }
        if self.order.len() >= self.capacity
            && let Some(oldest) = self.order.pop_front()
        {
            self.seen.remove(&oldest);
        }
        self.seen.insert(key.clone());
        self.order.push_back(key);
        true
    }

    /// Helper for `executionReport` events: uses `(order_id, trade_id)` when `trade_id >= 0`,
    /// otherwise `(order_id, event_time_ms, exec_type)`.
    pub fn check_execution_report(
        &mut self,
        order_id: i64,
        trade_id: i64,
        event_time_ms: i64,
        exec_type: &str,
    ) -> bool {
        let key = if trade_id >= 0 {
            ExecDedupKey::Fill { order_id, trade_id }
        } else {
            ExecDedupKey::Lifecycle {
                order_id,
                event_time_ms,
                exec_type: exec_type.to_owned(),
            }
        };
        self.insert(key)
    }
}

#[cfg(test)]
mod tests {
    use super::EventDeduplicator;

    #[test]
    fn deduplicates_fills_by_order_id_and_trade_id() {
        let mut dedup = EventDeduplicator::new(4);
        assert!(dedup.check_execution_report(100, 55, 1_000, "TRADE"));
        // Same (order_id, trade_id) even with different timestamp must be rejected
        assert!(!dedup.check_execution_report(100, 55, 1_001, "TRADE"));
        // Different trade_id on same order is accepted (partial fill 2)
        assert!(dedup.check_execution_report(100, 56, 1_002, "TRADE"));
    }

    #[test]
    fn evicts_oldest_when_capacity_exceeded() {
        let mut dedup = EventDeduplicator::new(2);
        assert!(dedup.check_execution_report(1, 10, 100, "TRADE"));
        assert!(dedup.check_execution_report(2, 20, 200, "TRADE"));
        assert!(dedup.check_execution_report(3, 30, 300, "TRADE"));
        // (1, 10) was evicted
        assert!(dedup.check_execution_report(1, 10, 400, "TRADE"));
        // (3, 30) is still tracked
        assert!(!dedup.check_execution_report(3, 30, 401, "TRADE"));
    }
}
