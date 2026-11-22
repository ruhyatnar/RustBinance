## 6. Strategi Trading (`bot-strategy`)

### 6.1. Kesimpulan Riset (mengubah desain)

| Temuan | Implikasi desain |
|---|---|
| Momentum/trend di kripto punya dukungan empiris (mis. faktor trend CTREND, JFQA 2025; Liu–Tsyvinski–Wu; Borri), **tetapi** hasil tidak konsisten antar sampel; satu studi menemukan **satu outlier = 37% total return** momentum, dan efek terutama di aset kapitalisasi besar | Universe = aset **likuid** besar; ekspektasi realistis; jangan bergantung pada beberapa trade lucky; kurangi ekor kiri |
| *Momentum crash* terjadi di kripto; **volatility-managed** momentum mengurangi crash | **Vol targeting** pada sizing (6.5) |
| Parameter trend-following kripto **tidak stabil**: Sharpe optimal in-sample pada 1 tahun tidak memprediksi tahun berikutnya; permukaan parameter berlonjak | Sedikit parameter, rentang lebar, **walk-forward**, uji kestabilan lingkungan parameter, bukan mencari satu "angka ajaib" |
| Klaim backtest bagus sering runtuh di walk-forward ketat; koreksi look-ahead/settlement menurunkan Sharpe signifikan | Replay tanpa look-ahead (hanya bar closed), biaya & slippage realistis, paper trading wajib |
| Biaya transaksi Spot (≈0.1%/sisi default VIP0 `[VERIFY via account.commission]`) memakan edge timeframe pendek | **Cost gate** & TF default 1h/4h |

### 6.2. Universe & Penyaringan

* Spot, quote `USDT` (opsi `USDC`/`FDUSD`), `status == TRADING`.
* Core universe: `quoteVolume24h ≥ 50 juta USD` (konfig), spread median ≤ 5 bps, usia listing ≥ 90 hari. Scanner gainers: `≥ 10 juta USD`, spread ≤ 15 bps, usia ≥ 14 hari.
* Eksklusi: token leverage (UP/DOWN/BULL/BEAR), stablecoin vs stablecoin, simbol `CANCEL_ONLY`/break, serta daftar eksklusi statis di `bot.toml` (`exclude_symbols`) untuk simbol bertanda monitoring/seed (tag ini tidak diekspos di `exchangeInfo`).
* Refresh universe tiap 15 menit; simbol keluar universe tidak boleh dibuka baru tetapi posisi yang ada tetap dikelola.

### 6.3. Indikator (spesifikasi tegas — agar tidak ambigu)

| Indikator | Definisi |
|---|---|
| EMA(n) | α=2/(n+1); seed = SMA n bar pertama; **warm-up ≥ 3n bar** (EMA200 butuh ≥ 600 bar) |
| RSI(14) | **Wilder smoothing** (α=1/14), bukan SMA |
| ATR(14) | Wilder smoothing atas True Range |
| ADX(14), +DI, −DI | Wilder (DM+, DM−, TR smoothing 14; ADX = Wilder-smoothed DX) |
| Donchian(n) | highest high / lowest low **n bar sebelumnya** (tidak termasuk bar sinyal) |
| VWAP sesi | Reset 00:00 UTC; Σ(typical×vol)/Σvol |
| Volume SMA(20) | SMA volume 20 bar closed |

Semua indikator **MUST** inkremental (`update(bar)` O(1)), dites terhadap nilai referensi (fixture dari TA-Lib/pandas-ta untuk dataset tetap, toleransi 1e-6). Tidak boleh ada look-ahead: sinyal hanya dihitung pada bar **closed**; bar HTF yang dipakai juga harus closed.

### 6.4. Profil Strategi

**Kerangka**: `trait Strategy { fn on_bar_close(&mut self, ctx:&Ctx) -> Option<EntryIntent>; }` — strategi **tidak** mengukur ukuran posisi dan tidak kirim order; hanya mengusulkan `EntryIntent{symbol, direction, entry_ref, stop, tp1, tp2, atr, strategy, rationale}`. Risk Gatekeeper yang memutuskan.

**S1 — Trend-Pullback** (profil utama, TF sinyal 1h, HTF 4h)
* Rezim HTF (4h, bar closed): `close > EMA50` dan `EMA50` naik (slope > 0 selama 3 bar).
* Tren 1h: `EMA21 > EMA55` dan `close > EMA200`.
* Pullback: dalam 3 bar terakhir `low ≤ EMA21 + 0.1×ATR` dan bar sinyal **close kembali > EMA21** (penolakan), `close ≤ EMA9 + 0.5×ATR` (tidak overextended).
* Momentum: `RSI ∈ [40, 62]` dan `RSI > RSI[1]`; `ADX ≥ 20` dan `+DI > −DI`.
* Volume: `vol ≥ 0.8×SMA20`.
* Anti-overextension: `|close − EMA21| ≤ 1.5×ATR`.
* Filter lembut (opsional, bisa dimatikan): `OBI_smooth ≥ +0.05`.
* Stop awal: `min(low 5 bar terakhir, entry − 1.5×ATR) − 0.1×ATR`. TP1 = 1.5R, TP2 = 3R.

**S2 — Breakout** (TF 1h, HTF 4h)
* `close > Donchian(20).upper`, `vol ≥ 1.5×SMA20`, `ADX ≥ 18` dan naik, `RSI ∈ [52, 75]`, HTF searah, `close − EMA21 ≤ 2.5×ATR`.
* Stop: `Donchian mid` atau `entry − 2×ATR` (yang lebih dekat tetapi ≥ 1×ATR). Trailing: lower Donchian(10) atau chandelier.

**S3 — Gainers Momentum Scanner** (hipotesis eksperimental; **shadow dulu**)
* Tiap 5 menit hitung skor = z(return 24h, di-cap) + z(return 4h) + z(rasio volume 1h/rata-rata 24h) di antara simbol lolos filter likuiditas.
* Kandidat = top K=5. Entry hanya bila: return 24h **antara +5% dan +25%** (di atas itu = risiko blow-off/pump-and-dump → tolak), harga > VWAP dan > EMA21 (15m), pullback ke VWAP/EMA21 dengan `RSI(14,15m) ∈ [45, 65]`, spread ≤ 15 bps, BTC regime OK.
* Ukuran = **0.5×** risiko normal; maks 2 posisi scanner; stop 1.2–1.5×ATR(15m); **time stop 24 jam**; cooldown per simbol 6 jam setelah rugi.

**S4 — VWAP Reversion** — `enabled=false` default; hanya jalan **shadow**. Mean reversion melawan tren kuat berisiko tinggi; aktifkan hanya bila shadow ≥ 200 trade menunjukkan expectancy positif setelah biaya. Syarat: `ADX < max(adx_min+12, 38)`, deviasi VWAP ≥ 1.1×ATR, RSI ekstrem + slope berbalik.

### 6.4.1. Sisi Short & Kapabilitas Venue `[NEW-v3.1]`

* Tiap strategi mengimplementasikan `supports(caps, dir)`. **Spot ⇒ hanya Long.** **Futures ⇒ Long dan Short**, masing-masing dapat di-enable terpisah (`[strategy.*].allow_short`) dan **dinilai gerbang sendiri** (13.3).
* Rumus Short = **cermin** Long: `profit_R = (entry − price)/r_distance`; `best_price` = low-water mark; stop ratchet **turun** saja; BE = `entry − fee_buffer`; trailing = `best_price + k×ATR`; TP di bawah entry.
* **S1 Short**: HTF 4h `close < EMA50` & EMA50 turun; 1h `EMA21 < EMA55` & `close < EMA200`; pullback `high ≥ EMA21 − 0.1×ATR` dalam 3 bar dan close kembali **< EMA21**; `RSI ∈ [38, 60]` & turun; `ADX ≥ 20`, `−DI > +DI`; anti-overextension `|close − EMA21| ≤ 1.5×ATR`; stop `max(high 5 bar, entry + 1.5×ATR) + 0.1×ATR`.
* **S2 Short**: `close < Donchian(20).lower`, volume ≥ 1.5×SMA20, `RSI ∈ [25, 48]`, HTF turun.
* **Regime BTC untuk Short**: BTC 4h `close < EMA50` **dan** **BTC Spike Guard** tidak aktif (BTC naik ≥ 1.5% dalam 15 menit atau ≥ 3% dalam 60 menit → blok entry short baru; histeresis sama seperti 7.5). Risiko *short squeeze*: ukuran Short × `short_size_multiplier` (default 0.75), stop **tidak boleh** lebih jauh dari `max_stop_pct`.
* **S3 (gainers)** tetap **Long-only** (pump-and-dump); **S4** shadow di kedua arah.
* Pada Futures, pagar kualitas 6.5 ditambah: funding gate, margin/likuidasi (7.8), divergensi mark↔last.

### 6.5. Pagar Kualitas Sinyal (berlaku semua profil)

1. **Cost gate**: `(tp1 − entry)/entry ≥ cost_multiple × round_trip_cost`, `cost_multiple=3`, `round_trip_cost = fee_entry + fee_exit + 2×expected_slippage` (bps).
2. Spread ≤ `max_spread_bps` (core 5, scanner 15); data segar (bookTicker < 500 ms).
3. Tidak ada gap bar pada 300 bar terakhir; warm-up terpenuhi.
4. **BTC regime**: entry long altcoin hanya bila `BTC 4h close > EMA50` **dan** Crash Guard tidak aktif.
5. **Vol targeting** pada ukuran: `size_mult = clamp(target_vol / realized_vol_30d, 0.5, 1.25)`; bila ATR%-percentile (90 hari) > 85 → `size_mult ≤ 0.6`.
6. Satu posisi per simbol; cooldown 4 bar setelah exit pada simbol yang sama.
7. Tidak ada entry baru pada menit −5…+2 sekitar jadwal event makro besar *(opsional; kalender statis di konfigurasi)*.
