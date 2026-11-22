## 13. Validasi, Pengujian, dan Gerbang Rilis

### 13.1. Pipeline Bertahap (tidak boleh dilompati)

```text
Unit/Property → Replay/Backtest (walk-forward) → Testnet (fault injection) → Paper (data live, fill simulasi)
  → Micro-live (risiko 0.1%) → Scale bertahap
```

### 13.2. Backtester / Replay
* Memakai **engine yang sama** dengan produksi (clock & broker disimulasikan) agar logika tidak bercabang. Input: data historis kline (mis. dump publik `data.binance.vision` `[VERIFY]`) + opsional aggTrade/bookTicker untuk fill realistis.
* Model biaya: fee maker/taker sesuai tier akun, slippage (model default: 1 tick + 0.5×spread; stres uji 2×), tidak ada fill pada bar yang sama dengan sinyal (eksekusi bar berikutnya), LIMIT_MAKER terisi hanya bila harga *menembus* level (bukan menyentuh).
* **Tanpa look-ahead**: indikator HTF hanya dari bar closed; uji otomatis "perturb future bars tidak mengubah sinyal masa lalu".
* Optimasi parameter: **walk-forward** (train 12 bln / test 3 bln, geser 3 bln) + simpan hasil per jendela; laporkan sebaran, bukan satu angka terbaik. Hitung jumlah percobaan parameter dan koreksi *multiple testing* (mis. Deflated Sharpe) di laporan.

### 13.3. Gerbang Kelulusan (ambang awal yang disarankan; ubah via ADR dengan alasan)

| Gerbang | Kriteria |
|---|---|
| G1 Backtest | ≥ 3 tahun termasuk fase bear; ≥ 300 trade agregat; **net** biaya+slippage 2×: Profit Factor ≥ 1.3, expectancy ≥ +0.15R, MaxDD ≤ 15%; tetangga parameter (±20%) ≥ 70% masih PF > 1.1; hapus 3 trade terbaik → PF masih > 1.0 |
| G2 Testnet | ≥ 2 minggu; semua uji di 13.4 lulus; 0 insiden state/rekonsiliasi tak terjelaskan |
| G3 Paper | ≥ 6 minggu **dan** ≥ 100 trade (+shadow); metrik dalam toleransi terhadap backtest; slippage model tidak lebih optimis dari realisasi data book |
| G4 Micro-live | risiko 0.1%/trade, ≥ 4 minggu & ≥ 50 trade; slippage realisasi ≤ 1.5× model; fee aktual = model; 0 orphan/unprotected |
| G5 Scale | naikkan risiko bertahap 0.1 → 0.25 → 0.5% tiap ≥ 4 minggu bila G4 terpenuhi; turunkan bila DD > 5% |

**Aturan per-venue**: G1–G5 berlaku **terpisah untuk Spot, Futures-Long, dan Futures-Short**; kelulusan satu venue/arah **tidak** diwarisi yang lain. Micro-live Futures mulai dari leverage exchange 1–2× dan risiko 0.1%. Fitur peralihan venue lulus uji 13.4 (baris *Venue switching*) di testnet sebelum dipakai di live.

### 13.4. Fault Injection / Chaos (otomatis di CI bila memungkinkan, manual di testnet)

| Skenario | Perilaku yang HARUS terjadi |
|---|---|
| Putus WS market (`iptables -A OUTPUT -p tcp --dport 9443 -j DROP`; Spot stream juga bisa 443) | Deteksi basi ≤ 1500 ms → `Paused`; reconnect + resync book; tidak ada entry saat basi |
| Putus WS API saat order in-flight | Status `UnknownNeedsQuery` → query by `clientOrderId`; tidak ada order ganda |
| Rotasi 24 jam / `serverShutdown` | Tanpa kehilangan event; dedup fill |
| `kill -9` saat `EntryPending`, `OpenProtected`, `ExitPending` | Recovery 8.3 → state benar; proteksi tetap ada |
| Fill ganda / event tidak berurutan | Idempoten (kunci `tradeId`) |
| 429 lalu 418 (mock) | Back-off sesuai `retryAfter`; tidak spam |
| Drift jam > 800 ms | `Paused` + alarm; `-1021` tidak memicu loop retry |
| Gap pada depth stream | Book `Invalid` → resync → OBI tidak dipakai selama invalid |
| Proteksi OCO dibatalkan manual | Terdeteksi → pasang ulang ≤ 2 s atau flatten |
| Saldo/order berubah manual di app Binance | Rekonsiliasi mendeteksi → orphan policy |
| Disk penuh / SQLite error | `Halted` (tidak trading tanpa journal) |
| Flash crash BTC (replay data) | Crash Guard aktif, histeresis benar |
| *Venue switching*: switch Spot→Futures saat `EntryPending` | Order entry venue lama dibatalkan; proteksi tetap; tidak ada order ganda |
| *Venue switching*: switch dengan posisi terbuka (`keep_managing`) | Venue lama `Draining` & tetap dikelola; venue baru `Active` hanya setelah rekonsiliasi sukses |
| *Venue switching*: aktivasi target gagal (key salah / WS down / margin multi-asset) | Rollback ke venue lama, alarm, tanpa keadaan setengah |
| *Venue switching*: flapping (4× dalam 1 jam) / saat `Halted` | Ditolak oleh cooldown, limit harian, dan aturan halt global |
| Simbol sama (`BTCUSDT`) aktif di dua venue | Tidak tabrakan (kunci `(Venue,Symbol)`, `clientOrderId` berbeda, DB berkolom venue) |
| Futures: mark ↔ last divergen > 0.5% | Entry simbol dipause |
| Futures: `MARGIN_CALL` / jarak likuidasi < 1.5× stop | `ReduceOnly` venue + kurangi posisi |
| Futures: funding ekstrem melawan posisi | Entry diblok; cost gate memperhitungkan; alarm |
| Futures: algo SL dibatalkan manual | Terdeteksi via `ALGO_UPDATE`/rekonsiliasi → dipasang ulang ≤ 2 s |
| Futures: ganti SL (make-before-break) | Tidak pernah ada interval tanpa SL aktif |

### 13.5. Matriks Tes per Crate
`bot-exchange`: mock WS server (korelasi id, rotasi, reconnect), tes kontrak terhadap testnet. `bot-marketdata`: book sync dengan fixture rekaman (termasuk gap). `bot-strategy`: indikator vs nilai referensi; sinyal deterministik pada dataset tetap (golden files); property: tidak look-ahead. `bot-risk`: property-based (`proptest`) — risiko aktual ≤ cap, heat tidak pernah melebihi batas, stop monoton. `bot-exec`: simulasi urutan event acak untuk status order konsisten. `bot-store`: replay == state sebelum crash. `bot-ipc`: auth negatif (tanpa token, Origin salah, nonce kedaluwarsa), conflation. UI: smoke test render + uji kontrak JSON (schema versi `v:1`).
