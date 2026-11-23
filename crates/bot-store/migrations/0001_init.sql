-- Migration 0001: Initial venue-agnostic schema (§8.2)

CREATE TABLE IF NOT EXISTS schema_migrations (
  version    INTEGER PRIMARY KEY,
  applied_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS counters (
  name  TEXT PRIMARY KEY,
  value INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS journal (
  seq         INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_ms       INTEGER NOT NULL,
  kind        TEXT NOT NULL,
  venue       TEXT,
  position_id TEXT,
  client_id   TEXT,
  payload     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS journal_pos ON journal(position_id);

CREATE TABLE IF NOT EXISTS orders (
  venue       TEXT NOT NULL DEFAULT 'spot',
  client_id   TEXT NOT NULL,
  exchange_id INTEGER,
  symbol      TEXT NOT NULL,
  side        TEXT NOT NULL,
  kind        TEXT NOT NULL,
  purpose     TEXT NOT NULL,
  price       TEXT,
  qty         TEXT NOT NULL,
  filled_qty  TEXT NOT NULL,
  avg_price   TEXT,
  status      TEXT NOT NULL,
  position_id TEXT,
  created_ms  INTEGER,
  updated_ms  INTEGER,
  PRIMARY KEY (venue, client_id)
);

CREATE TABLE IF NOT EXISTS fills (
  venue     TEXT NOT NULL DEFAULT 'spot',
  trade_id  INTEGER NOT NULL,
  symbol    TEXT NOT NULL,
  client_id TEXT NOT NULL,
  price     TEXT NOT NULL,
  qty       TEXT NOT NULL,
  fee       TEXT NOT NULL,
  fee_asset TEXT NOT NULL,
  is_maker  INTEGER,
  ts_ms     INTEGER,
  PRIMARY KEY (venue, symbol, trade_id)
);

CREATE TABLE IF NOT EXISTS positions (
  id                 TEXT PRIMARY KEY,
  venue              TEXT NOT NULL DEFAULT 'spot',
  leverage           INTEGER,
  liq_price          TEXT,
  symbol             TEXT NOT NULL,
  strategy           TEXT NOT NULL,
  direction          TEXT NOT NULL,
  state              TEXT NOT NULL,
  entry_price        TEXT,
  qty_initial        TEXT,
  qty_open           TEXT,
  r_distance         TEXT,
  stop_price         TEXT,
  tp1_price          TEXT,
  tp2_price          TEXT,
  best_price         TEXT,
  tp1_done           INTEGER,
  be_done            INTEGER,
  fees_quote         TEXT,
  realized_pnl_quote TEXT,
  opened_ms          INTEGER,
  closed_ms          INTEGER,
  close_reason       TEXT
);

CREATE TABLE IF NOT EXISTS risk_state (
  k TEXT PRIMARY KEY,
  v TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS venue_state (
  venue          TEXT PRIMARY KEY,
  state          TEXT NOT NULL,
  since_ms       INTEGER,
  last_switch_ms INTEGER,
  switches_today INTEGER
);

CREATE TABLE IF NOT EXISTS snapshots (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_ms    INTEGER,
  last_seq INTEGER,
  state    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS signals_shadow (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_ms    INTEGER,
  strategy TEXT,
  symbol   TEXT,
  payload  TEXT,
  outcome  TEXT
);
