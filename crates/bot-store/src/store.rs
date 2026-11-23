//! Single-writer SQLite WAL store with numbered migrations, append-only journal,
//! snapshots, crash recovery, persistent counters, and online `.backup` (§8).

use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bot_types::{
    Direction, Order, OrderKind, OrderPurpose, OrderStatus, Position, PositionState, Price, Qty,
    Side, StrategyId, Symbol, Venue, VenueState,
};
use rusqlite::backup::Backup;
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StoreError;
use crate::events::{EVENT_SCHEMA_VERSION, FillRecord, JournalEnvelope, VenueStateRecord};
use crate::state::PersistedEngineState;

const MIGRATION_0001: &str = include_str!("../migrations/0001_init.sql");

/// Counter key for daemon run ID (§5.2 `run`).
pub const COUNTER_RUN_ID: &str = "run_id";
/// Counter key for monotonic `clientOrderId` sequence (§5.2 `seq`).
pub const COUNTER_ORDER_SEQ: &str = "client_order_seq";

/// Single-writer SQLite WAL persistence store for the trading engine (§8.1).
pub struct SqliteStore {
    conn: Connection,
}

impl std::fmt::Debug for SqliteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteStore").finish_non_exhaustive()
    }
}

impl SqliteStore {
    /// Open (or create) a SQLite store at `path`, configure WAL + `synchronous=FULL` +
    /// `foreign_keys=ON`, and run numbered migrations idempotently.
    ///
    /// # Errors
    /// Returns [`StoreError`] (including [`StoreError::IoError`] / [`StoreError::DiskFull`]).
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::init_connection(conn, true)
    }

    /// Open an in-memory SQLite store (useful for fast deterministic unit/property tests).
    ///
    /// # Errors
    /// Returns [`StoreError`] if schema initialization fails.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::init_connection(conn, false)
    }

    fn init_connection(conn: Connection, enable_wal: bool) -> Result<Self, StoreError> {
        conn.busy_timeout(Duration::from_secs(5))?;
        if enable_wal {
            conn.pragma_update(None, "journal_mode", "WAL")?;
        }
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let mut store = Self { conn };
        store.run_migrations()?;
        Ok(store)
    }

    /// Run numbered forward migrations idempotently.
    fn run_migrations(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(MIGRATION_0001)?;
        let already_applied: Option<i64> = tx
            .query_row(
                "SELECT version FROM schema_migrations WHERE version = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if already_applied.is_none() {
            tx.execute(
                "INSERT INTO schema_migrations (version, applied_ms) VALUES (?1, ?2)",
                params![1_i64, now_unix_ms()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Verify SQLite database integrity (`PRAGMA integrity_check`).
    ///
    /// # Errors
    /// Returns [`StoreError::Corruption`] if SQLite reports anything other than `"ok"`.
    pub fn integrity_check(&self) -> Result<(), StoreError> {
        let res: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if res.eq_ignore_ascii_case("ok") {
            Ok(())
        } else {
            Err(StoreError::Corruption(res))
        }
    }

    /// Append a versioned event to the append-only `journal` table and return its assigned `seq`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on serialization or write failure (`DiskFull`/`IoError` → `Halted`).
    pub fn append_event(
        &mut self,
        ts_ms: i64,
        envelope: &JournalEnvelope,
    ) -> Result<i64, StoreError> {
        if envelope.schema_version > EVENT_SCHEMA_VERSION {
            return Err(StoreError::UnsupportedEventVersion {
                version: envelope.schema_version,
                max_supported: EVENT_SCHEMA_VERSION,
            });
        }
        let payload = serde_json::to_string(envelope)?;
        let venue_str = envelope.venue().map(|v| v.to_string());
        let pos_id_str = envelope.position_id().map(|u| u.to_string());
        let client_id = envelope.client_id();

        self.conn.execute(
            "INSERT INTO journal (ts_ms, kind, venue, position_id, client_id, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ts_ms,
                envelope.kind(),
                venue_str,
                pos_id_str,
                client_id,
                payload
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Read all journal entries strictly after `after_seq` in ascending `seq` order.
    ///
    /// # Errors
    /// Returns [`StoreError`] on read error, malformed JSON, or unsupported schema version.
    pub fn read_journal_after(
        &self,
        after_seq: i64,
    ) -> Result<Vec<(i64, i64, JournalEnvelope)>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, ts_ms, payload FROM journal WHERE seq > ?1 ORDER BY seq ASC")?;
        let rows = stmt.query_map(params![after_seq], |row| {
            let seq: i64 = row.get(0)?;
            let ts_ms: i64 = row.get(1)?;
            let payload: String = row.get(2)?;
            Ok((seq, ts_ms, payload))
        })?;

        let mut out = Vec::new();
        for row in rows {
            let (seq, ts_ms, payload) = row?;
            let env: JournalEnvelope = serde_json::from_str(&payload)?;
            if env.schema_version > EVENT_SCHEMA_VERSION {
                return Err(StoreError::UnsupportedEventVersion {
                    version: env.schema_version,
                    max_supported: EVENT_SCHEMA_VERSION,
                });
            }
            out.push((seq, ts_ms, env));
        }
        Ok(out)
    }

    /// Upsert an [`Order`] row keyed by `(venue, client_id)`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite failure.
    pub fn upsert_order(&mut self, order: &Order) -> Result<(), StoreError> {
        let created_ms = offset_dt_to_ms(order.created_at);
        let updated_ms = offset_dt_to_ms(order.updated_at);
        self.conn.execute(
            "INSERT INTO orders (
                venue, client_id, exchange_id, symbol, side, kind, purpose,
                price, qty, filled_qty, avg_price, status, position_id, created_ms, updated_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(venue, client_id) DO UPDATE SET
                exchange_id = COALESCE(excluded.exchange_id, orders.exchange_id),
                price       = excluded.price,
                qty         = excluded.qty,
                filled_qty  = excluded.filled_qty,
                avg_price   = COALESCE(excluded.avg_price, orders.avg_price),
                status      = excluded.status,
                position_id = COALESCE(excluded.position_id, orders.position_id),
                updated_ms  = excluded.updated_ms",
            params![
                venue_to_str(order.venue),
                &order.client_id,
                order.exchange_id,
                order.symbol.as_str(),
                serde_enum_str(&order.side)?,
                serde_enum_str(&order.kind)?,
                serde_enum_str(&order.purpose)?,
                order.price.map(|p| p.as_decimal().to_string()),
                order.qty.as_decimal().to_string(),
                order.filled_qty.as_decimal().to_string(),
                order.avg_fill_price.map(|p| p.as_decimal().to_string()),
                serde_enum_str(&order.status)?,
                order.position_id.map(|u| u.to_string()),
                created_ms,
                updated_ms,
            ],
        )?;
        Ok(())
    }

    /// Fetch a single [`Order`] by `(venue, client_id)`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite or parsing failure.
    pub fn get_order(&self, venue: Venue, client_id: &str) -> Result<Option<Order>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT venue, client_id, exchange_id, symbol, side, kind, purpose,
                        price, qty, filled_qty, avg_price, status, position_id, created_ms, updated_ms
                 FROM orders WHERE venue = ?1 AND client_id = ?2",
                params![venue_to_str(venue), client_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, Option<String>>(7)?,
                        r.get::<_, String>(8)?,
                        r.get::<_, String>(9)?,
                        r.get::<_, Option<String>>(10)?,
                        r.get::<_, String>(11)?,
                        r.get::<_, Option<String>>(12)?,
                        r.get::<_, i64>(13)?,
                        r.get::<_, i64>(14)?,
                    ))
                },
            )
            .optional()?;

        let Some((
            v_str,
            cid,
            ex_id,
            sym,
            side_s,
            kind_s,
            purpose_s,
            price_s,
            qty_s,
            filled_s,
            avg_s,
            status_s,
            pos_s,
            created_ms,
            updated_ms,
        )) = row
        else {
            return Ok(None);
        };

        Ok(Some(Order {
            venue: str_to_venue(&v_str)?,
            client_id: cid,
            exchange_id: ex_id,
            symbol: Symbol(sym),
            side: parse_enum_str::<Side>(&side_s)?,
            kind: parse_enum_str::<OrderKind>(&kind_s)?,
            price: price_s.as_deref().map(parse_price).transpose()?,
            qty: parse_qty(&qty_s)?,
            filled_qty: parse_qty(&filled_s)?,
            avg_fill_price: avg_s.as_deref().map(parse_price).transpose()?,
            status: parse_enum_str::<OrderStatus>(&status_s)?,
            position_id: pos_s.as_deref().map(parse_uuid).transpose()?,
            purpose: parse_enum_str::<OrderPurpose>(&purpose_s)?,
            created_at: ms_to_offset_dt(created_ms)?,
            updated_at: ms_to_offset_dt(updated_ms)?,
        }))
    }

    /// Idempotently upsert a trade fill keyed by `(venue, symbol, trade_id)`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite failure.
    pub fn upsert_fill(&mut self, fill: &FillRecord) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO fills (
                venue, trade_id, symbol, client_id, price, qty, fee, fee_asset, is_maker, ts_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(venue, symbol, trade_id) DO NOTHING",
            params![
                venue_to_str(fill.venue),
                fill.trade_id,
                fill.symbol.as_str(),
                &fill.client_id,
                fill.price.as_decimal().to_string(),
                fill.qty.as_decimal().to_string(),
                fill.fee.to_string(),
                &fill.fee_asset,
                i64::from(fill.is_maker),
                fill.ts_ms,
            ],
        )?;
        Ok(())
    }

    /// List all fills for `(venue, symbol)` ordered by `ts_ms, trade_id`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite or parsing failure.
    pub fn list_fills(&self, venue: Venue, symbol: &Symbol) -> Result<Vec<FillRecord>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT venue, trade_id, symbol, client_id, price, qty, fee, fee_asset, is_maker, ts_ms
             FROM fills WHERE venue = ?1 AND symbol = ?2 ORDER BY ts_ms ASC, trade_id ASC",
        )?;
        let rows = stmt.query_map(params![venue_to_str(venue), symbol.as_str()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, i64>(8)?,
                r.get::<_, i64>(9)?,
            ))
        })?;

        let mut out = Vec::new();
        for r in rows {
            let (v_s, tid, sym, cid, p_s, q_s, f_s, fa, maker, ts) = r?;
            out.push(FillRecord {
                venue: str_to_venue(&v_s)?,
                trade_id: tid,
                symbol: Symbol(sym),
                client_id: cid,
                price: parse_price(&p_s)?,
                qty: parse_qty(&q_s)?,
                fee: parse_dec(&f_s)?,
                fee_asset: fa,
                is_maker: maker != 0,
                ts_ms: ts,
            });
        }
        Ok(out)
    }

    /// Upsert a [`Position`] row keyed by `id`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite failure.
    pub fn upsert_position(
        &mut self,
        pos: &Position,
        closed_ms: Option<i64>,
        close_reason: Option<&str>,
    ) -> Result<(), StoreError> {
        let opened_ms = offset_dt_to_ms(pos.opened_at);
        self.conn.execute(
            "INSERT INTO positions (
                id, venue, leverage, liq_price, symbol, strategy, direction, state,
                entry_price, qty_initial, qty_open, r_distance, stop_price, tp1_price, tp2_price,
                best_price, tp1_done, be_done, fees_quote, realized_pnl_quote,
                opened_ms, closed_ms, close_reason
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                ?16, ?17, ?18, ?19, ?20,
                ?21, ?22, ?23
             )
             ON CONFLICT(id) DO UPDATE SET
                leverage           = excluded.leverage,
                liq_price          = excluded.liq_price,
                state              = excluded.state,
                qty_open           = excluded.qty_open,
                stop_price         = excluded.stop_price,
                tp1_price          = excluded.tp1_price,
                tp2_price          = excluded.tp2_price,
                best_price         = excluded.best_price,
                tp1_done           = excluded.tp1_done,
                be_done            = excluded.be_done,
                fees_quote         = excluded.fees_quote,
                realized_pnl_quote = excluded.realized_pnl_quote,
                closed_ms          = COALESCE(excluded.closed_ms, positions.closed_ms),
                close_reason       = COALESCE(excluded.close_reason, positions.close_reason)",
            params![
                pos.id.to_string(),
                venue_to_str(pos.venue),
                pos.leverage.map(i64::from),
                pos.liq_price.map(|p| p.as_decimal().to_string()),
                pos.symbol.as_str(),
                serde_enum_str(&pos.strategy)?,
                serde_enum_str(&pos.direction)?,
                serde_enum_str(&pos.state)?,
                pos.entry_price.as_decimal().to_string(),
                pos.qty_initial.as_decimal().to_string(),
                pos.qty_open.as_decimal().to_string(),
                pos.r_distance.to_string(),
                pos.stop_price.as_decimal().to_string(),
                pos.tp1_price.as_decimal().to_string(),
                pos.tp2_price.map(|p| p.as_decimal().to_string()),
                pos.best_price.as_decimal().to_string(),
                i64::from(pos.tp1_done),
                i64::from(pos.breakeven_done),
                pos.fees_paid_quote.to_string(),
                pos.realized_pnl_quote.to_string(),
                opened_ms,
                closed_ms,
                close_reason,
            ],
        )?;
        Ok(())
    }

    /// Fetch a [`Position`] by UUID.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite or parsing failure.
    pub fn get_position(&self, id: Uuid) -> Result<Option<Position>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT id, venue, leverage, liq_price, symbol, strategy, direction, state,
                        entry_price, qty_initial, qty_open, r_distance, stop_price, tp1_price,
                        tp2_price, best_price, tp1_done, be_done, fees_quote, realized_pnl_quote,
                        opened_ms
                 FROM positions WHERE id = ?1",
                params![id.to_string()],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, String>(8)?,
                        r.get::<_, String>(9)?,
                        r.get::<_, String>(10)?,
                        r.get::<_, String>(11)?,
                        r.get::<_, String>(12)?,
                        r.get::<_, String>(13)?,
                        r.get::<_, Option<String>>(14)?,
                        r.get::<_, String>(15)?,
                        r.get::<_, i64>(16)?,
                        r.get::<_, i64>(17)?,
                        r.get::<_, String>(18)?,
                        r.get::<_, String>(19)?,
                        r.get::<_, i64>(20)?,
                    ))
                },
            )
            .optional()?;

        let Some((
            id_s,
            v_s,
            lev,
            liq_s,
            sym,
            strat_s,
            dir_s,
            state_s,
            entry_s,
            qty_init_s,
            qty_open_s,
            r_s,
            stop_s,
            tp1_s,
            tp2_s,
            best_s,
            tp1_done,
            be_done,
            fees_s,
            pnl_s,
            opened_ms,
        )) = row
        else {
            return Ok(None);
        };

        Ok(Some(Position {
            id: parse_uuid(&id_s)?,
            symbol: Symbol(sym),
            venue: str_to_venue(&v_s)?,
            direction: parse_enum_str::<Direction>(&dir_s)?,
            strategy: parse_enum_str::<StrategyId>(&strat_s)?,
            entry_price: parse_price(&entry_s)?,
            qty_initial: parse_qty(&qty_init_s)?,
            qty_open: parse_qty(&qty_open_s)?,
            r_distance: parse_dec(&r_s)?,
            leverage: lev.map(|v| u32::try_from(v).unwrap_or(1)),
            liq_price: liq_s.as_deref().map(parse_price).transpose()?,
            stop_price: parse_price(&stop_s)?,
            tp1_price: parse_price(&tp1_s)?,
            tp2_price: tp2_s.as_deref().map(parse_price).transpose()?,
            best_price: parse_price(&best_s)?,
            tp1_done: tp1_done != 0,
            breakeven_done: be_done != 0,
            fees_paid_quote: parse_dec(&fees_s)?,
            realized_pnl_quote: parse_dec(&pnl_s)?,
            state: parse_enum_str::<PositionState>(&state_s)?,
            opened_at: ms_to_offset_dt(opened_ms)?,
        }))
    }

    /// Read a value from `risk_state`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite failure.
    pub fn risk_state_get(&self, key: &str) -> Result<Option<String>, StoreError> {
        let val = self
            .conn
            .query_row(
                "SELECT v FROM risk_state WHERE k = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(val)
    }

    /// Write or update a key/value entry in `risk_state`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite failure.
    pub fn risk_state_set(&mut self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO risk_state (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            params![key, value],
        )?;
        Ok(())
    }

    /// Read a per-venue row from `venue_state`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite or parsing failure.
    pub fn venue_state_get(&self, venue: Venue) -> Result<Option<VenueStateRecord>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT venue, state, since_ms, last_switch_ms, switches_today
                 FROM venue_state WHERE venue = ?1",
                params![venue_to_str(venue)],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;

        let Some((v_s, st_s, since_ms, last_sw, sw_today)) = row else {
            return Ok(None);
        };
        Ok(Some(VenueStateRecord {
            venue: str_to_venue(&v_s)?,
            state: parse_enum_str::<VenueState>(&st_s)?,
            since_ms,
            last_switch_ms: last_sw,
            switches_today: u32::try_from(sw_today).unwrap_or(0),
        }))
    }

    /// Upsert a per-venue row in `venue_state`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite failure.
    pub fn venue_state_set(&mut self, rec: &VenueStateRecord) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO venue_state (venue, state, since_ms, last_switch_ms, switches_today)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(venue) DO UPDATE SET
                state          = excluded.state,
                since_ms       = excluded.since_ms,
                last_switch_ms = excluded.last_switch_ms,
                switches_today = excluded.switches_today",
            params![
                venue_to_str(rec.venue),
                serde_enum_str(&rec.state)?,
                rec.since_ms,
                rec.last_switch_ms,
                i64::from(rec.switches_today),
            ],
        )?;
        Ok(())
    }

    /// Persist a full [`PersistedEngineState`] snapshot at `state.last_seq`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on serialization or SQLite failure.
    pub fn snapshot_write(
        &mut self,
        ts_ms: i64,
        state: &PersistedEngineState,
    ) -> Result<i64, StoreError> {
        let json = serde_json::to_string(state)?;
        self.conn.execute(
            "INSERT INTO snapshots (ts_ms, last_seq, state) VALUES (?1, ?2, ?3)",
            params![ts_ms, state.last_seq, json],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Read the most recent [`PersistedEngineState`] snapshot, if any.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite or deserialization failure.
    pub fn snapshot_read_latest(&self) -> Result<Option<PersistedEngineState>, StoreError> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT state FROM snapshots ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match raw {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    /// Recover [`PersistedEngineState`] deterministically: load latest snapshot (or empty default)
    /// and replay all `journal` events with `seq > snapshot.last_seq` (§8.1, §8.3).
    ///
    /// # Errors
    /// Returns [`StoreError`] on read, deserialization, or schema version failure.
    pub fn recover_state(&self) -> Result<PersistedEngineState, StoreError> {
        let mut state = self.snapshot_read_latest()?.unwrap_or_default();
        let events = self.read_journal_after(state.last_seq)?;
        for (seq, _ts_ms, env) in events {
            state.apply_event(seq, &env);
        }
        Ok(state)
    }

    /// Atomically increment and persist the daemon `run_id` counter (§5.2).
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite failure.
    pub fn next_run_id(&mut self) -> Result<u64, StoreError> {
        self.increment_counter(COUNTER_RUN_ID)
    }

    /// Atomically increment and persist the `clientOrderId` sequence counter (§5.2).
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite failure.
    pub fn next_order_seq(&mut self) -> Result<u64, StoreError> {
        self.increment_counter(COUNTER_ORDER_SEQ)
    }

    /// Read the current value of a named persistent counter (`0` if never incremented).
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite failure.
    pub fn current_counter(&self, name: &str) -> Result<u64, StoreError> {
        let val: Option<i64> = self
            .conn
            .query_row(
                "SELECT value FROM counters WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()?;
        Ok(val.and_then(|v| u64::try_from(v).ok()).unwrap_or(0))
    }

    fn increment_counter(&mut self, name: &str) -> Result<u64, StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO counters (name, value) VALUES (?1, 1)
             ON CONFLICT(name) DO UPDATE SET value = counters.value + 1",
            params![name],
        )?;
        let val: i64 = tx.query_row(
            "SELECT value FROM counters WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?;
        tx.commit()?;
        u64::try_from(val).map_err(|_| StoreError::Corruption(format!("negative counter {name}")))
    }

    /// Perform an online consistent SQLite `.backup` to `dest_path` (§8.2).
    ///
    /// # Errors
    /// Returns [`StoreError`] if opening the destination or stepping the backup fails.
    pub fn backup_to(&self, dest_path: &Path) -> Result<(), StoreError> {
        if let Some(parent) = dest_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut dst = Connection::open(dest_path)?;
        let backup = Backup::new(&self.conn, &mut dst)?;
        backup.run_to_completion(100, Duration::from_millis(10), None)?;
        Ok(())
    }

    /// Insert a shadow strategy signal record into `signals_shadow` (§6.4, M08).
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite failure.
    pub fn insert_shadow_signal(
        &mut self,
        ts_ms: i64,
        strategy: &str,
        symbol: &Symbol,
        payload_json: &str,
        outcome: &str,
    ) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO signals_shadow (ts_ms, strategy, symbol, payload, outcome)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts_ms, strategy, symbol.as_str(), payload_json, outcome],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// List shadow signal rows `(id, ts_ms, strategy, symbol, payload, outcome)` ordered by `id`.
    ///
    /// # Errors
    /// Returns [`StoreError`] on SQLite failure.
    #[allow(clippy::type_complexity)]
    pub fn list_shadow_signals(
        &self,
    ) -> Result<Vec<(i64, i64, String, String, String, String)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts_ms, strategy, symbol, payload, outcome
             FROM signals_shadow ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Expose raw connection transaction helper exclusively for crash/fault tests.
    #[doc(hidden)]
    pub fn execute_raw_batch(&self, sql: &str) -> Result<(), StoreError> {
        self.conn.execute_batch(sql)?;
        Ok(())
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn offset_dt_to_ms(dt: OffsetDateTime) -> i64 {
    i64::try_from(dt.unix_timestamp_nanos() / 1_000_000).unwrap_or(0)
}

fn ms_to_offset_dt(ms: i64) -> Result<OffsetDateTime, StoreError> {
    let nanos = i128::from(ms) * 1_000_000;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .map_err(|e| StoreError::Serialization(e.to_string()))
}

const fn venue_to_str(v: Venue) -> &'static str {
    match v {
        Venue::Spot => "spot",
        Venue::UsdsFutures => "futures",
    }
}

fn str_to_venue(s: &str) -> Result<Venue, StoreError> {
    match s {
        "spot" => Ok(Venue::Spot),
        "futures" | "usds_futures" => Ok(Venue::UsdsFutures),
        other => Err(StoreError::Serialization(format!(
            "unknown venue `{other}`"
        ))),
    }
}

fn serde_enum_str<T: serde::Serialize>(val: &T) -> Result<String, StoreError> {
    let v = serde_json::to_value(val)?;
    v.as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| StoreError::Serialization("enum did not serialize as string".into()))
}

fn parse_enum_str<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, StoreError> {
    let v = serde_json::Value::String(s.to_owned());
    Ok(serde_json::from_value(v)?)
}

fn parse_dec(s: &str) -> Result<Decimal, StoreError> {
    Decimal::from_str(s).map_err(|_| StoreError::Serialization(format!("invalid decimal `{s}`")))
}

fn parse_price(s: &str) -> Result<Price, StoreError> {
    Ok(Price(parse_dec(s)?))
}

fn parse_qty(s: &str) -> Result<Qty, StoreError> {
    Ok(Qty(parse_dec(s)?))
}

fn parse_uuid(s: &str) -> Result<Uuid, StoreError> {
    Uuid::parse_str(s).map_err(|e| StoreError::Serialization(e.to_string()))
}
