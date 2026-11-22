## 7. Manajemen Risiko (`bot-risk`)

### 7.1. Model Berlapis (Defense in Depth)

| Lapis | Mekanisme | Berlaku walau bot mati? |
|---|---|---|
| L0 Akun/Exchange | API key tanpa izin withdraw, whitelist IP, izin minimum; proteksi OCO/algo **di sisi exchange** | **Ya** |
| L1 Pre-trade | Gatekeeper menolak/mengecilkan order (7.2) | Tidak |
| L2 Portofolio | Batas heat, eksposur, korelasi | Tidak |
| L3 Circuit breaker | Mode `ReduceOnly/Halted/Killed` (7.4) | Tidak |
| L4 Operasional | Rekonsiliasi, deteksi data basi, drift jam, rate-limit guard, dead-man | Sebagian |
| L5 Manusia | Tombol Panic (hold-to-confirm), notifikasi, review mingguan | — |

### 7.2. Pre-trade Checks (urutan — berhenti di penolakan pertama; semua alasan dicatat)

1. `EngineMode == Ready` (bukan Paused/ReduceOnly/Halted/Killed).
2. Simbol `TRADING`, tidak di-quarantine, lolos universe & cooldown.
3. BTC Crash Guard tidak aktif; jam jendela larangan (bila ada).
4. Data segar (market < 1500 ms, user stream sehat, jam drift < 800 ms).
5. Batas jumlah posisi, **portfolio heat**, eksposur total, eksposur per-simbol, batas per-strategi, batas **posisi searah/berkorelasi**.
6. Jarak stop valid: `0.3% ≤ |entry−stop|/entry ≤ 8%` (konfig).
7. Sizing (7.3) → pembulatan filter (`LOT_SIZE`, `PRICE_FILTER`, `NOTIONAL`/`MIN_NOTIONAL`, `PERCENT_PRICE_BY_SIDE` `[VERIFY]`) → bila di bawah minimum: **tolak**, jangan dinaikkan.
8. Saldo quote tersedia (≥ notional + buffer fee 0.2%).
9. Rate-limit guard: `REQUEST_WEIGHT` < 60% dan `ORDERS` 10 s < 50% batas.

**Default awal (konservatif — jangan dinaikkan sebelum lulus gerbang Bagian 13):**

| Parameter | Default | Rentang yang boleh | Catatan |
|---|---|---|---|
| `risk_per_trade_pct` | **0.5** | 0.1–1.0 | v2: 1.0 |
| `max_open_positions` | 4 | 1–8 | |
| `portfolio_heat_max_pct` (Σ risiko terbuka dalam R×risk) | **2.0** | 0.5–4 | Menggantikan hitungan posisi mentah |
| `max_notional_per_position_pct` | 20 | 5–35 | % equity |
| `max_total_exposure_pct` | 60 | 20–80 | % equity (Spot) |
| `max_same_direction_correlated` | 3 | 1–5 | Per arah, **lintas venue**: batas jumlah posisi searah/berkorelasi (Spot long-only ⇒ altcoin long; Futures ⇒ long & short dihitung terpisah) |
| `daily_loss_halt_pct` | **−2.0** | −0.5…−4 | Equity vs 00:00 UTC, realized+unrealized |
| `weekly_loss_halt_pct` | −5.0 | | Senin 00:00 UTC |
| `drawdown_kill_pct` (dari equity puncak) | **−10.0** | −5…−20 | Reset **manual** |
| `loss_streak_cooldown` | 3 rugi → 2 jam; 5 rugi → sampai 00:00 UTC | | |
| `max_entry_slippage_bps` | 10 | 2–25 | |
| `max_orders_per_day` | 200 | | Guard terhadap bug loop |

### 7.3. Position Sizing `[FIXED-v3]`

```text
risk_cap      = equity × risk_per_trade_pct/100 × size_mult(6.5) × scanner_mult
qty_raw       = risk_cap / |entry − stop|                       # Spot long
qty_notional  = min(qty_raw × entry,  equity × max_notional_pct/100,  free_quote × 0.98)
qty           = floor_to_step(qty_notional / entry, step_size)
heat_after    = heat_now + (qty × |entry − stop|) / equity
REJECT jika heat_after > portfolio_heat_max atau qty < min_qty atau qty×entry < min_notional
```
Contoh: equity 1.000 USDT, risk 0.5% = 5 USDT, entry 100, stop 98 → qty_raw = 2.5 → notional 250 USDT (25% > batas 20%) → dipotong ke 200 USDT → qty 2.0 (risiko nyata 4 USDT = 0.4%).

* Mode `fixed_risk_pct` (default) dan `fixed_notional` (untuk testnet). 
* **Kelly**: *hanya* sebagai pengurang. Aktif bila ≥100 trade live/paper untuk strategi tersebut; `f = W − (1−W)/R` dihitung dari **batas bawah** interval kepercayaan (mis. bootstrap persentil 25); `risk = min(risk_per_trade, 0.25×f×100)`; bila `f ≤ 0` → strategi di-**disable otomatis** (alarm), bukan sekadar skip.
* Stop yang dipakai sizing = stop **sebenarnya** (termasuk buffer); jangan hitung dari jarak ATR murni bila stop swing lebih jauh.

### 7.4. Mode Engine & Circuit Breaker

```text
Starting → Syncing → Ready ⇄ Paused
Ready → ReduceOnly   (data basi >30 s, reconnect gagal >30 s, drift jam, ketidakcocokan rekonsiliasi kecil)
Ready/ReduceOnly → Halted   (daily/weekly loss tercapai, buffer user-data penuh, anomali state, 3× error order beruntun)
Any → Killed         (drawdown_kill, perintah Panic, kehilangan proteksi tak dapat dipulihkan)
```
| Mode | Entry baru | Kelola posisi/proteksi | Keluar mode |
|---|---|---|---|
| Ready | ya | ya | — |
| Paused | tidak | ya | otomatis saat sinkron/reconnect beres (≥ 3 detik stabil) |
| ReduceOnly | tidak | ya (hanya mengurangi) | otomatis bila kondisi pulih ≥ 60 detik |
| Halted | tidak | ya; proteksi tetap di exchange | otomatis 00:00 UTC (loss harian) **atau** manual (lainnya) |
| Killed | tidak | **flatten** semua (opsional: biarkan proteksi exchange bila `kill_flatten=false`) | **manual saja** (`botctl reset-kill --confirm`) |

Profit lock: v2 menghentikan entry pada +3%/hari — ini memotong ekor kanan strategi trend. v3: `profit_lock_mode = "reduce"` (default): setelah +2% hari ini, `risk_per_trade × 0.5`; opsi `"stop"` tersedia untuk meniru v2.

### 7.5. BTC Crash Guard (dengan Histeresis) `[FIXED-v3]`

* **Trigger**: `BTCUSDT` turun ≥ 1.5% dalam 15 menit **atau** ≥ 3% dalam 60 menit (dari kline 1m/ticker).
* **Aksi**: blok entry baru; geser stop posisi yang untung ke `max(stop, entry)`; kirim alarm.
* **Rilis**: ≥ 30 menit sejak trigger **dan** `ATR(1m,14)/harga` kembali < 1.5× median 24 jam **dan** harga BTC pulih ≥ 50% dari penurunan *atau* 60 menit tanpa low baru. Cooldown minimal 15 menit antar-trigger (cegah flapping).

### 7.6. Kontrol Operasional

* **Rekonsiliasi berkala** tiap 60 s (REST/WS API): saldo, open orders, posisi vs journal. Selisih kecil (dust < min_notional) → catat; selisih material → `ReduceOnly` + alarm; order/proteksi yatim → batalkan/adopsi sesuai kebijakan `orphan_policy` (`adopt_with_stop` | `flatten`).
* **Dead-man**: heartbeat daemon ke file `/run/binance-bot/heartbeat`; watchdog systemd `WatchdogSec=30` + `sd_notify`. Proteksi exchange tetap menjaga posisi walau daemon mati.
* **Dust & fee asset**: pantau saldo BNB bila fee dibayar BNB (peringatan < ambang).
* **Aset quote**: pantau depeg stablecoin quote (USDT) — bila `USDTUSDC` menyimpang > 0.5% → `ReduceOnly`.
* **Pencatatan & kepatuhan**: ekspor trade log CSV (tanggal, simbol, qty, harga, fee) untuk kebutuhan pelaporan pajak/kepatuhan lokal; pemilik bertanggung jawab atas aturan setempat.

### 7.7. Kebersihan Kunci API
1. Buat key **Ed25519** **terpisah per venue** (keypair & API key Spot ≠ Futures); unggah hanya public key. **Withdrawal MUST off; Universal Transfer MUST off.** Spot: Reading + Spot Trading. Futures: Reading + Futures. Jangan menyalakan izin yang venue itu tidak butuhkan.
2. Whitelist IP publik mesin bot.
3. Private key: `0600`, milik user layanan; tidak di dalam repo; masuk `.gitignore` & pre-commit secret scan.
4. Rotasi key tiap 90 hari (runbook).

### 7.8. Risiko Khusus Futures & Anggaran Lintas-Venue `[NEW-v3.1]`

**Default `[risk.futures]`** (override atas `[risk]`; semua **tighten-only** saat runtime):

| Parameter | Default | Rentang | Catatan |
|---|---|---|---|
| `risk_per_trade_pct` | **0.35** | 0.1–0.75 | Lebih kecil dari Spot (leverage, gap, funding) |
| `short_size_multiplier` | 0.75 | 0.25–1.0 | Risiko *squeeze* |
| `max_leverage` (exchange) | **3** | 1–5 | Hard cap; micro-live mulai 1–2× |
| `max_effective_leverage` (Σ notional / equity venue) | 2.0 | 0.5–3 | Dicek di gatekeeper |
| `margin_mode` | isolated | isolated | `cross` **ditolak** |
| `liq_buffer_mult` | 3.0 | 2–6 | `|entry−liq| ≥ mult × |entry−stop|` |
| `liq_emergency_mult` | 1.5 | 1–2 | Posisi terbuka dengan jarak likuidasi < 1.5× jarak stop → kurangi/tutup |
| `funding_adverse_block_pct` | 0.03 | 0.01–0.1 | Per periode 8 jam |
| `max_funding_cost_r` | 0.15 | | Perkiraan biaya funding maks (dalam R) per trade |
| `mark_last_divergence_pct` | 0.5 | 0.2–1 | Pause entry simbol |
| `max_notional_per_position_pct` | 20 | | Dari equity venue |
| `max_open_positions` | 3 | 1–6 | |

**Sizing Futures**
```text
risk_cap  = equity_venue × risk% × size_mult × (short ? short_size_multiplier : 1)
qty_raw   = risk_cap / |entry − stop|
notional  = min(qty_raw × entry, equity_venue × max_notional_pct, equity_venue × max_effective_leverage − notional_terbuka)
margin    = notional / leverage_exchange   → harus ≤ 0.8 × free_margin
TOLAK bila: |entry − liq_est| < liq_buffer_mult × |entry − stop|  atau notional < min_notional  atau funding gate gagal
```

**Circuit breaker tambahan Futures**
* `MARGIN_CALL` atau jarak likuidasi < `liq_emergency_mult` → `ReduceOnly` pada venue + kurangi posisi paling berisiko (market reduce-only) + alarm.
* Kuantil ADL tinggi `[VERIFY]` → alarm & larang menambah ukuran.
* Funding ekstrem (> 0.1%/8 jam melawan kita) pada posisi terbuka → pertimbangkan exit (`funding_exit_enabled`, default off, hanya alarm).
* Gangguan/maintenance Futures → venue `Paused`; proteksi algo tetap di exchange.

**Anggaran lintas-venue (global)**
* Equity gabungan (USD) = `equity_spot + equity_futures(margin balance)`; `daily/weekly/drawdown` limit (7.2) dihitung dari ini → satu venue yang rugi besar menghentikan **keduanya**.
* `portfolio_heat_max_pct` berlaku pada **jumlah** risiko terbuka kedua venue (heat = Σ qty×|entry−stop| / equity gabungan).
* `capital_allocation_usd` per venue membatasi modal yang boleh dipakai; sisa tidak disentuh.
* Peralihan venue **tidak** mereset counter risiko (hari/minggu/drawdown/streak).
