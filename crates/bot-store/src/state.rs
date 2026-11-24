//! Deterministic replayable engine state reconstructed from snapshots + journal (§8.1, §8.3).

use std::collections::BTreeMap;

use bot_types::{EngineMode, Order, OrderStatus, Position, PositionState, Symbol, Venue};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::{FillRecord, JournalEnvelope, JournalEvent, VenueStateRecord};

/// Maximum number of recent fills retained inside a [`PersistedEngineState`] snapshot
/// (full historical fills remain permanently in the SQLite `fills` table).
pub const MAX_RETAINED_SNAPSHOT_FILLS: usize = 512;

/// Deterministic persisted engine state that can be snapshotted and replayed identically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedEngineState {
    /// Highest `journal.seq` applied to this state (`0` if empty).
    pub last_seq: i64,
    /// Current global engine mode.
    pub mode: EngineMode,
    /// Active (non-terminal) orders keyed strictly by `(Venue, client_id)`.
    #[serde(with = "orders_map_serde")]
    pub orders: BTreeMap<(Venue, String), Order>,
    /// Bounded recent fills keyed strictly by `(Venue, Symbol, trade_id)` (idempotent).
    #[serde(with = "fills_map_serde")]
    pub fills: BTreeMap<(Venue, Symbol, i64), FillRecord>,
    /// Active (non-closed) positions keyed by UUID.
    pub positions: BTreeMap<Uuid, Position>,
    /// Persistent risk state key-value pairs.
    pub risk_state: BTreeMap<String, String>,
    /// Per-venue lifecycle states.
    pub venue_states: BTreeMap<Venue, VenueStateRecord>,
}

impl Default for PersistedEngineState {
    fn default() -> Self {
        Self {
            last_seq: 0,
            mode: EngineMode::Starting,
            orders: BTreeMap::new(),
            fills: BTreeMap::new(),
            positions: BTreeMap::new(),
            risk_state: BTreeMap::new(),
            venue_states: BTreeMap::new(),
        }
    }
}

impl PersistedEngineState {
    /// Deterministically apply a single journaled event with sequence number `seq`.
    ///
    /// Events with `seq <= self.last_seq` are ignored (idempotent replay).
    /// Terminal orders and closed positions are pruned from active snapshot state
    /// (full history remains in the relational SQLite tables).
    #[allow(clippy::too_many_lines)]
    pub fn apply_event(&mut self, seq: i64, envelope: &JournalEnvelope) {
        if seq <= self.last_seq {
            return;
        }
        match &envelope.event {
            JournalEvent::OrderIntended { order } => {
                self.orders
                    .insert((order.venue, order.client_id.clone()), order.clone());
            }
            JournalEvent::OrderStatusUpdated {
                venue,
                client_id,
                exchange_id,
                status,
                filled_qty,
                avg_fill_price,
                updated_at,
            } => {
                if matches!(
                    status,
                    OrderStatus::Filled
                        | OrderStatus::Canceled
                        | OrderStatus::Rejected
                        | OrderStatus::Expired
                ) {
                    self.orders.remove(&(*venue, client_id.clone()));
                } else if let Some(order) = self.orders.get_mut(&(*venue, client_id.clone())) {
                    if exchange_id.is_some() {
                        order.exchange_id = *exchange_id;
                    }
                    order.status = *status;
                    // §5.3 invariant: filled_qty is monotonic non-decreasing.
                    if filled_qty.as_decimal() > order.filled_qty.as_decimal() {
                        order.filled_qty = *filled_qty;
                    }
                    if avg_fill_price.is_some() {
                        order.avg_fill_price = *avg_fill_price;
                    }
                    order.updated_at = *updated_at;
                }
            }
            JournalEvent::FillRecorded { fill } => {
                let key = (fill.venue, fill.symbol.clone(), fill.trade_id);
                self.fills.entry(key).or_insert_with(|| fill.clone());
                while self.fills.len() > MAX_RETAINED_SNAPSHOT_FILLS {
                    let oldest_key = self
                        .fills
                        .iter()
                        .min_by_key(|(k, v)| (v.ts_ms, k.2))
                        .map(|(k, _)| k.clone());
                    if let Some(k) = oldest_key {
                        self.fills.remove(&k);
                    } else {
                        break;
                    }
                }
            }
            JournalEvent::PositionOpened { position } => {
                self.positions.insert(position.id, position.clone());
            }
            JournalEvent::PositionStopMoved {
                position_id,
                new_stop_price,
                breakeven_done,
                ..
            } => {
                if let Some(pos) = self.positions.get_mut(position_id) {
                    pos.stop_price = *new_stop_price;
                    pos.breakeven_done = pos.breakeven_done || *breakeven_done;
                }
            }
            JournalEvent::PositionBestPriceUpdated {
                position_id,
                best_price,
                ..
            } => {
                if let Some(pos) = self.positions.get_mut(position_id) {
                    pos.best_price = *best_price;
                }
            }
            JournalEvent::PositionTp1Done {
                position_id,
                qty_open,
                realized_pnl_quote,
                fees_paid_quote,
                ..
            } => {
                if let Some(pos) = self.positions.get_mut(position_id) {
                    pos.tp1_done = true;
                    pos.qty_open = *qty_open;
                    pos.realized_pnl_quote = *realized_pnl_quote;
                    pos.fees_paid_quote = *fees_paid_quote;
                }
            }
            JournalEvent::PositionStateChanged {
                position_id,
                state,
                qty_open,
                realized_pnl_quote,
                fees_paid_quote,
                ..
            } => {
                if *state == PositionState::Closed {
                    self.positions.remove(position_id);
                } else if let Some(pos) = self.positions.get_mut(position_id) {
                    pos.state = *state;
                    pos.qty_open = *qty_open;
                    pos.realized_pnl_quote = *realized_pnl_quote;
                    pos.fees_paid_quote = *fees_paid_quote;
                }
            }
            JournalEvent::EngineModeChanged { mode, .. } => {
                self.mode = *mode;
            }
            JournalEvent::VenueStateChanged { record } => {
                self.venue_states.insert(record.venue, record.clone());
            }
            JournalEvent::VenueSwitched { .. } | JournalEvent::Command { .. } => {
                // Audit-only record; venue state mutations are emitted as VenueStateChanged.
            }
            JournalEvent::RiskStateSet { key, value } => {
                self.risk_state.insert(key.clone(), value.clone());
            }
        }
        self.last_seq = seq;
    }
}

/// Custom serde for `BTreeMap<(Venue, String), Order>` as a JSON array of entries
/// (JSON objects only allow string keys).
mod orders_map_serde {
    use std::collections::BTreeMap;

    use bot_types::{Order, Venue};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(
        map: &BTreeMap<(Venue, String), Order>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let items: Vec<&Order> = map.values().collect();
        items.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<(Venue, String), Order>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let items = Vec::<Order>::deserialize(deserializer)?;
        let mut map = BTreeMap::new();
        for o in items {
            map.insert((o.venue, o.client_id.clone()), o);
        }
        Ok(map)
    }
}

/// Custom serde for `BTreeMap<(Venue, Symbol, i64), FillRecord>` as a JSON array of entries.
mod fills_map_serde {
    use std::collections::BTreeMap;

    use bot_types::{Symbol, Venue};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::events::FillRecord;

    type FillsMap = BTreeMap<(Venue, Symbol, i64), FillRecord>;

    pub fn serialize<S>(map: &FillsMap, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let items: Vec<&FillRecord> = map.values().collect();
        items.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<FillsMap, D::Error>
    where
        D: Deserializer<'de>,
    {
        let items = Vec::<FillRecord>::deserialize(deserializer)?;
        let mut map = BTreeMap::new();
        for f in items {
            map.insert((f.venue, f.symbol.clone(), f.trade_id), f);
        }
        Ok(map)
    }
}
