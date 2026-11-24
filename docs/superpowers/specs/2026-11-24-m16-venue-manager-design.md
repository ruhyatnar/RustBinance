# Desain M16 — `VenueManager`, Peralihan Spot ⇄ Futures, Risiko Lintas-Venue, Sisi Short & UI Switcher

**Tanggal:** 2026-11-24  
**Status:** Menunggu persetujuan pemilik  
**Milestone:** M16 (`docs/milestones/M16-venue-manager.md`)

---

## 1. Tujuan & Alasan
Menghadirkan peralihan runtime **Spot ⇄ USDⓈ-M Futures** yang aman, atomik, dan teraudit tanpa mengorbankan keselamatan dana:
- Proteksi posisi (OCO/algo reduce-only) **tidak pernah dibatalkan** saat peralihan.
- Counter risiko (harian, mingguan, drawdown, loss streak) bersifat **global** dan tidak ter-reset oleh peralihan venue.
- Mendukung sisi **Short** (cermin Long) khusus Futures dengan pagar *short squeeze* (BTC Spike Guard & `short_size_multiplier`).
- Operator dapat memantau dan mengalihkan venue secara aman melalui Web UI, Desktop UI (Qt6/QML), dan `botctl`.

## 2. Ruang Lingkup

### Goals
1. `crates/bot-venue`: `VenueManager` penuh dengan state machine `Disabled → Activating → Active → Draining`, prosedur `switch_venue` atomik, cooldown, limit harian, rollback, dan event jurnal `VenueSwitched`.
2. `crates/bot-risk`: Anggaran risiko lintas-venue (equity gabungan USD, `portfolio_heat` gabungan, `cross_venue_same_symbol = block_opposite`, `capital_allocation_usd`, override `[risk.futures]` tighten-only) dan **BTC Spike Guard** untuk Short.
3. `crates/bot-strategy`: Sisi Short S1 & S2 (§6.4.1), `supports(caps, dir)` per venue/arah, S3 tetap Long-only, S4 shadow dua arah.
4. `crates/bot-engine` + `crates/bot-ipc` + `crates/botctl` + `crates/botd`: Integrasi `VenueManager` ke `EngineActor`, aktivasi perintah `switch_venue` (+ nonce wajib untuk `flatten`), dan `venues` di snapshot telemetri.
5. `web-ui` & `desktop-ui`: `VenueSwitcher`, `VenueSwitchDialog`, lencana venue di setiap baris, kolom khusus Futures, dan banner status `Draining`.
6. Uji chaos §13.4 (*Venue switching*) lulus seluruhnya di CI (paper/mock) dan siap untuk testnet.

### Non-Goals (Di luar cakupan M16)
- Mode `dual` (Spot dan Futures `Active` bersamaan) — ditunda ke M17 opsional.
- Eksekusi live Futures — tetap diblokir sampai gerbang M17 lulus.

### Anti-Goals
- Tidak boleh ada keadaan setengah-jadi (*half-switched*) bila aktivasi target gagal.
- Tidak boleh ada celah menghindari `Halted`/`Killed` dengan cara berpindah venue.
- Tidak boleh ada transfer dana otomatis antar wallet (Spot ⇄ Futures).

## 3. Pendekatan Arsitektur (Approaches)

| Opsi | Ringkasan | Trade-off | Keputusan |
|---|---|---|---|
| **Opsi A** | `VenueManager` hidup di dalam `EngineActor` (`bot-engine` bergantung pada `bot-venue`) | Menjamin atomisitas penuh terhadap state engine (sesuai §3.5), bebas race antara `switch_venue` vs tick/order/rekonsiliasi, deterministik untuk chaos suite §13.4. Menambah 1 dependensi `bot-engine → bot-venue`. | **Dipilih** |
| **Opsi B** | `VenueManager` di luar `EngineActor` (di `botd`) | `bot-engine` tidak bergantung pada `bot-venue`, namun koordinasi pause/cancel/switch terpisah rawan race condition saat `EntryPending`/`Syncing` dan rollback lebih rapuh. | Ditolak |

---

## 4. Desain Per Subsistem

### 4.1. `crates/bot-venue` — `VenueManager` & Prosedur `switch_venue`
- **State per venue** (`VenueSlot`): `venue: Venue`, `state: VenueState` (`Disabled | Activating | Active | Draining`), `enabled: bool`, `credentials_valid: bool`, `live_confirmation_present: bool`.
- **Konfigurasi switch** (`SwitchConfig`):
  - `switch_cooldown_s: u64` (default `60`)
  - `max_switches_per_day: u32` (default `3`)
  - `default_policy: SwitchOpenPositionPolicy` (`KeepManaging`)
- **Prasyarat `switch_venue(target, policy, confirm_nonce, ctx)`** (tolak dengan `SwitchError` yang jelas bila salah satu gagal):
  1. `EngineMode ∈ {Ready, Paused}` (tolak saat `Halted`, `Killed`, atau `Syncing`/rekonsiliasi).
  2. `target != active_venue`.
  3. `target.enabled == true`, kredensial valid, dan bila `TradingMode::Live` maka `live_confirmation_file` venue target wajib ada.
  4. Cooldown terpenuhi (`now - last_switch_at >= switch_cooldown_s`) dan jumlah switch hari ini `< max_switches_per_day`.
  5. Bila `policy == Flatten`: wajib `confirm_nonce` valid (diverifikasi di `bot-ipc` sebelum diteruskan ke engine).
  6. Bila `policy == RequireFlat`: tolak jika masih ada posisi terbuka di venue lama.
- **Eksekusi atomik di dalam `EngineActor`**:
  1. Hentikan entry baru di venue lama; batalkan semua order entry yang belum terisi (`EntryPending`); **pertahankan** seluruh order proteksi (OCO / algo reduce-only).
  2. Terapkan `policy`:
     - `KeepManaging`: venue lama → `Draining` (lifecycle posisi, proteksi, rekonsiliasi tetap berjalan; otomatis transisi ke `Disabled` begitu posisi venue lama menjadi flat).
     - `RequireFlat`: sudah dipastikan 0 posisi terbuka → venue lama → `Disabled`.
     - `Flatten`: tutup semua posisi venue lama (market/IOC reduce-only) → venue lama → `Disabled`.
  3. Venue target → `Activating` → `adapter.activate()` → rekonsiliasi per-venue → `Active`.
  4. **Rollback**: bila langkah 3 gagal → target kembali `Disabled`; venue lama dikembalikan ke `Active` (jika belum `Disabled`); engine masuk `Paused`; kirim alarm; tidak ada keadaan setengah-jadi.
  5. Catat event jurnal `VenueSwitched { from, to, policy, actor, ts_ms }` + perbarui `VenueStateRecord` di `bot-store` + broadcast event ke UI.

### 4.2. `crates/bot-risk` — Risiko Lintas-Venue & BTC Spike Guard
- **Equity & Heat Gabungan** (`CrossVenueBudget`):
  - `combined_equity_usd = equity_spot + equity_futures_margin_balance`.
  - Daily/weekly/drawdown breaker (§7.2) dievaluasi terhadap `combined_equity_usd` dan counter **tidak pernah di-reset** oleh `switch_venue`.
  - `portfolio_heat = Σ(qty × |entry − stop|) / combined_equity_usd` mencakup posisi terbuka di **kedua** venue (termasuk venue yang sedang `Draining`).
- **Alokasi Modal**: `effective_venue_equity = min(equity_venue, capital_allocation_usd)` bila `capital_allocation_usd > 0`.
- **Cross-venue same symbol**: `cross_venue_same_symbol = BlockOpposite` (default) menolak pembukaan posisi arah berlawanan pada simbol dasar yang sama jika venue lain masih memegang posisi terbuka.
- **Override `[risk.futures]`**: parameter khusus Futures (§7.8) hanya boleh diperketat (*tighten-only*) saat runtime.
- **BTC Spike Guard (untuk Short)**: blok entry Short baru bila BTC naik `≥ 1.5%` dalam 15 menit atau `≥ 3.0%` dalam 60 menit, dengan histeresis pemulihan setara BTC Crash Guard (§7.5).

### 4.3. `crates/bot-strategy` — Sisi Short (S1/S2) & `supports()`
- `supports(caps, dir)`:
  - `Direction::Short` hanya dikembalikan `true` bila `caps.can_short && config.allow_short`.
  - S3 (gainers) tetap `Long`-only (`supports(_, Short) == false`).
  - S4 (shadow) mendukung kedua arah bila `caps.can_short`.
- **S1 Short** (§6.4.1): cermin Long — HTF 4h `close < EMA50` & EMA50 turun; 1h `EMA21 < EMA55` & `close < EMA200`; pullback `high ≥ EMA21 − 0.1×ATR` dalam 3 bar dan close `< EMA21`; `RSI ∈ [38, 60]` & turun; `ADX ≥ 20`, `−DI > +DI`; `|close − EMA21| ≤ 1.5×ATR`; stop `max(high_5bar, entry + 1.5×ATR) + 0.1×ATR`.
- **S2 Short** (§6.4.1): `close < Donchian(20).lower`, volume `≥ 1.5×SMA20`, `RSI ∈ [25, 48]`, HTF turun.
- **Manajemen posisi Short**: `profit_R = (entry − price) / r_distance`, `best_price` = low-water mark, stop ratchet hanya **turun**, BE = `entry − fee_buffer`, trailing = `best_price + k×ATR`, TP di bawah entry.

### 4.4. `crates/bot-ipc`, `crates/botctl`, `crates/botd`
- Hapus stub `not_supported until M16` di `bot-ipc/src/command.rs` dan `botctl/src/client.rs`.
- `Command::SwitchVenue { target, on_open_positions, confirm_nonce }`:
  - Jika `on_open_positions == Flatten`, wajib validasi `confirm_nonce` untuk `ConfirmAction::SwitchVenue`.
  - Teruskan ke `EngineIpcBackend::switch_venue(...)` → `EngineHandle::switch_venue(...)`.
- `TelemetrySnapshot.venues` mencerminkan state aktual (`Disabled | Activating | Active | Draining`) dari `VenueManager`.

### 4.5. `web-ui` & `desktop-ui`
- **Web UI (SvelteKit):**
  - `VenueSwitcher.svelte` di header: segmen `[ SPOT | FUTURES ]`, lencana status (`Active ✓` / `Draining ⏳` / `Disabled –`), info `ISOLATED · maks 3×` untuk Futures, tersembunyi untuk token read-only.
  - `VenueSwitchDialog.svelte`: target venue, jumlah posisi terbuka & PnL, radio kebijakan (`Kelola sampai selesai` / `Wajib flat dulu` / `Tutup semua sekarang` + hold-to-confirm 1,5 detik + nonce), alasan penolakan bila disabled.
  - Banner `Draining`, lencana venue di setiap baris posisi/order, filter `Semua | Spot | Futures`, dan kolom khusus Futures (Lev, Liq Price, Jarak Liq %, Funding).
- **Desktop UI (Qt6/QML):**
  - `VenueSwitcher.qml`, `VenueSwitchDialog.qml`, properti `StateStore.venues`, `CommandService.switchVenue(target, policy, confirmNonce)`, dan indikator venue (S/F) pada status/tray.

---

## 5. Rencana Verifikasi & Acceptance
1. **Unit & Property Tests**:
   - State machine `VenueManager` (semua transisi valid/invalid, cooldown, limit harian, rollback).
   - `CrossVenueBudget` (heat gabungan, `block_opposite`, `capital_allocation_usd`, non-reset counter saat switch) + BTC Spike Guard.
   - S1/S2 Short (golden/mirror test terhadap sinyal Long) + `supports()` matrix.
2. **Chaos Suite §13.4 (baris Venue switching)**:
   - Switch Spot→Futures saat `EntryPending`: entry dibatalkan, proteksi tetap, tidak ada order ganda.
   - Switch dengan posisi terbuka (`keep_managing`): venue lama `Draining` tetap dikelola, venue baru `Active` hanya setelah rekonsiliasi sukses.
   - Aktivasi target gagal (key salah / WS down / margin multi-asset): rollback ke venue lama, engine `Paused`, alarm, tanpa keadaan setengah.
   - Flapping (4× dalam 1 jam) & switch saat `Halted`/`Killed`: ditolak oleh cooldown/limit harian/halt global.
3. **Isolasi simbol sama**: `(Spot, BTCUSDT)` dan `(UsdsFutures, BTCUSDT)` tidak tabrakan di kunci state, `clientOrderId`, maupun SQLite.
4. **Full CI**: `./scripts/verify.sh` hijau seluruhnya.
