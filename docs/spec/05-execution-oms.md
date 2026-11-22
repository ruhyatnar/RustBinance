## 5. Eksekusi & Order Management (`bot-exec`)

### 5.1. Prinsip
1. **Journal-first**: setiap niat (intent) → catat event `OrderIntended` ke SQLite **sebelum** kirim ke exchange.
2. **Idempoten**: `clientOrderId` deterministik; retry memakai id yang sama.
3. **Source of truth**: exchange (via user data stream + rekonsiliasi), bukan memori lokal.
4. Event-driven: tidak ada polling-sleep. Timeout via `tokio::time::timeout` / `sleep_until` di dalam `select!`.

### 5.2. Format `clientOrderId`
`{bot}{run}{venue}-{seq}-{purpose}` dengan `venue ∈ {S,F}` mis. `b01r7S-000123-ENT` / `b01r7F-000124-SL` (≤36 karakter, regex `^[A-Za-z0-9_-]{1,36}$`). `run` = id jalan daemon (naik tiap start), `seq` = counter persisten di SQLite. Parsing id mengembalikan `position_id` via tabel `orders`.

### 5.3. Mesin Status Order

```text
PendingNew --ack--> New --partial--> PartiallyFilled --fill--> Filled
PendingNew --timeout/5xx--> UnknownNeedsQuery --query ketemu--> (status dari exchange)
                                           \--query tidak ada setelah 3 percobaan--> Rejected(Lost)
New|PartiallyFilled --cancel ack--> Canceled      New --expire--> Expired
* --reject--> Rejected
```
Aturan: event fill **MUST** diproses idempoten (kunci `tradeId`); `filled_qty` hanya naik; fee dicatat dari event (`commission`, `commissionAsset`) — bila fee dibayar dengan BNB/aset lain, konversi ke quote untuk PnL.

### 5.4. Algoritma Entry Pasif dengan Chasing Terbatas `[FIXED-v3]`

```text
ENTRY(intent):  # intent berisi symbol, qty (sudah lolos Risk), limit_price_max, ttl_ms, max_replaces
  price = best_bid (Long)                       # dari bookTicker lokal (fresh < 500 ms)
  place LIMIT_MAKER(price, qty, cid)
  loop select!:
     fill event        -> akumulasi; bila Filled -> on_entry_filled(); break
     bookTicker update -> bila best_bid naik & |price - best_bid| ≥ 1 tick & replaces < max_replaces
                          & best_bid ≤ limit_price_max -> cancelReplace ke best_bid
     reject(-2010)     -> tunggu update book berikutnya (bukan sleep) lalu ulangi
     timeout ttl_ms    -> cancel; bila partial -> pakai qty terisi saja
  KEBIJAKAN: jangan mengejar melewati limit_price_max (slippage cap). Trade terlewat = OK.
```
* Exit stop-loss **tidak pernah** pasif: selalu market/IOC (atau OCO stop yang sudah di exchange).
* Exit TP boleh pasif (LIMIT_MAKER) — sudah bagian OCO.
* Partial fill entry < `min_viable_fill` (mis. < 30% qty) → batalkan sisa & tutup sisa posisi bila notional < `min_notional` tidak bisa dijual → catat sebagai *dust*.

### 5.5. Siklus Hidup Posisi (dievaluasi setiap tick harga & tiap kline close)

Urutan evaluasi `[FIXED-v3]` (keselamatan dulu):
1. **Cek proteksi**: apakah OCO/algo masih ada di exchange? Bila tidak (mis. dibatalkan manual) → `OpenUnprotected` → pasang ulang proteksi segera; bila gagal 2× → `EmergencyExit` market.
2. **Hard stop watcher**: bila harga menembus `stop_price` dan proteksi exchange belum terisi dalam `hard_stop_grace_ms` (default 1500) → market exit.
3. Update `best_price` (high-water mark) → journal hanya bila berubah ≥ 0.1 ATR.
4. `profit_R = (price − entry)/r_distance` (Long).
5. **Break-even**: bila `profit_R ≥ be_trigger_R` (default 1.0) → SL = entry + `fee_buffer` (= 2×taker_fee_bps×entry + 1 tick).
6. **Trailing monoton** bila `profit_R ≥ trail_activate_R` (default 1.5): `stop = best_price − k×ATR` (k default 3.0; chandelier), ratchet satu arah.
7. **Pengetatan kelelahan momentum**: bila `RSI(14) ≥ 78` (Long) dan `profit_R ≥ 2` → `k = k × 0.7`.
8. **TP1** (default 1.5R, tutup 50%) dan **TP2** (default 3R) — dieksekusi oleh order di exchange; software hanya menyinkronkan state dari event.
9. **Time stop**: posisi tanpa progres (`|profit_R| < 0.3`) setelah `max_bars_in_trade` (default 48 bar 1h) → exit.
10. Semua exit → laporan PnL (gross, fee, net, R-multiple) ke journal & UI.
