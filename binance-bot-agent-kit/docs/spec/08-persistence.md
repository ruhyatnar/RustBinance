## 8. Persistensi & Pemulihan (`bot-store`)

### 8.1. Prinsip
SQLite, mode WAL, `synchronous=FULL` untuk tabel `journal`; satu writer (engine). Event-sourcing ringan: **journal append-only** + **snapshot** state tiap 60 detik & saat perubahan penting. Pemulihan = snapshot terakhir + replay journal sesudahnya + rekonsiliasi eksternal.

### 8.2. Skema (migrasi bernomor, `migrations/0001_init.sql`)

```sql
PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;

CREATE TABLE journal (
  seq        INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_ms      INTEGER NOT NULL,
  kind       TEXT NOT NULL,               -- OrderIntended, OrderAck, Fill, StopMoved, ModeChanged, ...
  venue TEXT, position_id TEXT, client_id TEXT,
  payload    TEXT NOT NULL                -- JSON (Decimal sebagai string)
);
CREATE INDEX journal_pos ON journal(position_id);

CREATE TABLE orders (
  venue TEXT NOT NULL DEFAULT 'spot', client_id TEXT NOT NULL, exchange_id INTEGER, symbol TEXT NOT NULL, side TEXT NOT NULL,
  kind TEXT NOT NULL, purpose TEXT NOT NULL, price TEXT, qty TEXT NOT NULL, filled_qty TEXT NOT NULL,
  avg_price TEXT, status TEXT NOT NULL, position_id TEXT, created_ms INTEGER, updated_ms INTEGER,
  PRIMARY KEY (venue, client_id)
);
CREATE TABLE fills (
  venue TEXT NOT NULL DEFAULT 'spot', trade_id INTEGER NOT NULL, symbol TEXT NOT NULL, client_id TEXT NOT NULL, price TEXT NOT NULL,
  qty TEXT NOT NULL, fee TEXT NOT NULL, fee_asset TEXT NOT NULL, is_maker INTEGER, ts_ms INTEGER,
  PRIMARY KEY (venue, symbol, trade_id)
);
CREATE TABLE positions (
  id TEXT PRIMARY KEY, venue TEXT NOT NULL DEFAULT 'spot', leverage INTEGER, liq_price TEXT, symbol TEXT NOT NULL, strategy TEXT NOT NULL, direction TEXT NOT NULL,
  state TEXT NOT NULL, entry_price TEXT, qty_initial TEXT, qty_open TEXT, r_distance TEXT,
  stop_price TEXT, tp1_price TEXT, tp2_price TEXT, best_price TEXT, tp1_done INTEGER, be_done INTEGER,
  fees_quote TEXT, realized_pnl_quote TEXT, opened_ms INTEGER, closed_ms INTEGER, close_reason TEXT
);
CREATE TABLE risk_state (k TEXT PRIMARY KEY, v TEXT NOT NULL);   -- day_start_equity, peak_equity, streaks, mode, ...
CREATE TABLE venue_state (venue TEXT PRIMARY KEY, state TEXT NOT NULL, since_ms INTEGER, last_switch_ms INTEGER, switches_today INTEGER);
CREATE TABLE snapshots (id INTEGER PRIMARY KEY AUTOINCREMENT, ts_ms INTEGER, last_seq INTEGER, state TEXT NOT NULL);
CREATE TABLE signals_shadow (                                      -- sinyal & hasil hipotetis (strategi shadow)
  id INTEGER PRIMARY KEY AUTOINCREMENT, ts_ms INTEGER, strategy TEXT, symbol TEXT, payload TEXT, outcome TEXT
);
```
Retensi: `journal` selamanya (kompresi/arsip bulanan opsional); backup harian `sqlite3 .backup` ke `/var/lib/binance-bot/backup/` (retensi 14 hari).

### 8.3. Prosedur Start/Recovery (**MUST**, sebelum `Ready`) — dijalankan **per venue** yang `Activating`/`Draining`
1. Muat snapshot terakhir + replay journal → `EngineState` lokal.
2. Sinkron jam (`time`), muat `exchangeInfo` (filter, rate limit).
3. Ambil dari exchange: saldo, open orders (+ order list OCO), (Futures: posisi + algo orders).
4. Cocokkan: 
   * order lokal aktif yang tidak ada di exchange → query by `clientOrderId` → perbarui status (mungkin terisi saat bot mati → proses fill yang terlewat dengan mengambil riwayat trade `myTrades` sejak `last_seq`).
   * order/proteksi di exchange yang tidak ada di lokal → **orphan** → `orphan_policy`.
   * saldo base > 0 tanpa posisi (di atas dust) → posisi yatim → `orphan_policy`.
5. Pastikan **setiap posisi terbuka punya proteksi** di exchange; bila tidak → pasang (atau flatten bila gagal).
6. Subscribe user data & market streams; setelah stabil ≥ 3 s → `Ready`.
Uji wajib: `kill -9` saat `OpenProtected`, saat `EntryPending`, dan saat `ExitPending` (lihat 13.4).
