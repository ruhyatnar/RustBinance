# Desain Pengembangan Lanjutan M18–M21: Hardening Pra-Live, Diagnostik, Soak Ops & Observabilitas

**Tanggal:** 2026-11-26  
**Status:** Approved (2026-11-26)  
**Pendekatan Terpilih:** Opsi A — Jalur Pra-Live Cepat (`M18` → `M19` + `M20` + `M21` paralel; `M22`–`M23` ditunda ke backlog pasca-G2/G3)  
**Referensi Bahan:** Audit & adaptasi native Rust dari pola operasional teruji di `ultimate_bot` dan `ApexRust`

---

## 1. Latar Belakang & Tujuan (*Intent & Why*)

Seluruh milestone engineering inti `RustBinance` (`M00–M17`) telah selesai. Tahap berikutnya sebelum mempertimbangkan akun live adalah **uji operasional testnet & soak 24/7 (gerbang G2–G5 untuk Spot dan Futures Long/Short)** sebagaimana tercatat di `docs/STATUS.md`.

Saat ini, beberapa kebutuhan operasional lapangan yang krusial untuk menjalankan G2–G5 secara aman dan mudah diaudit masih belum tersedia secara native di `RustBinance`:
1. Belum ada satu perintah diagnostik pra-live (`doctor`, `key fingerprint/inspect`, `probe --signed`, `probe --dry-run-order`) yang memverifikasi kunci Ed25519, clock drift, konektivitas, dan filter `exchangeInfo` tanpa mengirim order nyata.
2. Belum ada pengawas soak 24 jam+ (`soak-watch`) beserta notifikasi eksternal (Webhook Discord/HTTP) untuk alert crash/stall, fill, circuit breaker trip, dan ringkasan periodik.
3. Belum ada verifikasi kelayakan `minNotional` per pair (`NotionalFeasibility`), pengelompokan *partial exit* (*leg-to-trade attribution*), dan *cross-check* PnL realisasi Futures terhadap `userTrades`.
4. Beberapa celah hardening runtime masih bisa diperkuat: lock proses tunggal (`botd`), auto-pause saat Binance maintenance, sentinel `Option<i64>` untuk clock offset `0 ms`, klasifikasi error permanen Futures, pelacakan header rate-limit REST, cache saldo WS-first, tiket WS sekali pakai + guard bind non-loopback, serta akurasi heartbeat & tampilan *mirrored bracket ladder* Short di UI.

Seluruh bahan yang sebelumnya berbasis Python (`ultimate_bot`) atau Rust/TS (`ApexRust`) diadaptasi **100% native Rust** ke dalam crate workspace `RustBinance` (serta komponen SvelteKit/QML untuk UI) tanpa menambah runtime Python di target host CachyOS.

---

## 2. Goals, Non-Goals, dan Anti-Goals

### Goals
- Menyediakan fondasi exchange & daemon yang lebih tahan banting terhadap kegagalan jaringan, rate-limit, maintenance, dan eksekusi proses ganda (**M18**).
- Menyediakan perangkat CLI pra-live non-destruktif di `binance-botctl` yang memakai `bot-exchange` dan `bot-config` yang sama persis dengan daemon (**M19**).
- Menyediakan perangkat soak operasional G2–G5, notifikasi webhook aman, validasi kelayakan `minNotional`, serta akurasi statistik trade & PnL Futures (**M20**).
- Memperkuat keamanan akses IPC/Web dan menyelaraskan indikator liveness + visualisasi posisi Long/Short di Web UI & Desktop UI (**M21**).
- Menjaga **nol perubahan pada logika entry/exit strategi** yang sudah lulus `M08–M17` agar hasil backtest G1 tetap valid.

### Non-Goals
- Mengubah strategi S1/S2/shadow S3–S4, menambah *session windows*, atau mengganti screener universe (ditunda ke `M22`/`M23`).
- Menambahkan fitur live tuning `.env`/TOML dari Web UI atau simulator di browser.
- Mengaktifkan mode live secara otomatis — default tetap `mode = paper` dan live tetap mensyaratkan `live_confirmation_file`.

### Anti-Goals *(Kondisi Kegagalan)*
- Probe pra-live (`doctor`, `probe --signed`, `probe --dry-run-order`) **tidak boleh pernah** mengirim order nyata ke exchange.
- Private key, API key, Bearer token, atau URL webhook yang mengandung secret **tidak boleh pernah** muncul di log, stdout CLI, telemetry, atau URL query string.
- Penambahan fitur observabilitas/notifikasi **tidak boleh** memblokir *hot path* `EngineActor` (semua HTTP webhook / soak report berjalan asinkron & *best-effort* dengan bounded channel).
- Tidak boleh melonggarkan parameter risiko saat runtime atau membatalkan proteksi OCO/Algo saat maintenance auto-pause.

---

## 3. Batasan Keras (*Constraints*)

Mengacu pada `AGENTS.md`:
1. **Keselamatan dana > kebenaran > kesederhanaan > performa.**
2. Semua harga/qty/saldo/PnL memakai `rust_decimal::Decimal`.
3. Tanpa `unsafe` (kecuali binding Qt yang sudah ada ADR-nya); tanpa `unwrap`/`expect`/`panic`/indexing di kode non-test; tanpa `sleep`/busy-loop untuk menunggu fill.
4. Semua kunci state = `(Venue, Symbol)`; setiap event/telemetry/tabel membawa `venue`.
5. Tes jaringan ke Binance hanya ke **testnet** di balik feature `testnet-it` + env var; `cargo test --workspace` default wajib 100% offline/deterministik.
6. Seluruh perubahan tetap hijau di `scripts/verify.sh` (`cargo fmt`, `clippy -D warnings`, `cargo test --workspace`, `cargo deny`, `web-ui` Vitest + `vite build`, secret scan).
7. Eksekusi milestone mengikuti alur `AGENTS.md`: **satu milestone aktif per sesi**, diakhiri update `docs/STATUS.md`, `CHANGELOG.md`, dan verifikasi penuh.

---

## 4. Pendekatan yang Dipertimbangkan (*Approaches*)

### Opsi A — Jalur Pra-Live Cepat *(Dipilih)*
- **Urutan:** `M18` (fondasi) → `M19` + `M20` + `M21` (bisa paralel atau berurutan per sesi) → langsung masuk uji operasional testnet **G2–G5**. `M22` (guardrail eksekusi/universe) dan `M23` (ekstensi strategi) ditunda sampai ada data nyata dari G2/G3.
- **Trade-off:** Paling cepat membekali operator untuk fase G2–G5 tanpa mengganggu perilaku strategi yang sudah tervalidasi di G1. Risiko regresi logika trading hampir nol.

### Opsi B — Lengkap Sebelum Soak
- **Urutan:** `M18` → `M19–M21` → `M22` (per-symbol loss re-entry cooldown, pending-order runaway ATR guard, watchlist dwell/hysteresis, kline sequence-gap guard) baru masuk soak G2–G5.
- **Trade-off:** Proteksi eksekusi/universe lebih ketat sejak hari pertama soak, tetapi waktu pengerjaan lebih lama dan mewajibkan re-validasi backtest G1 setelah `M22`.

**Alasan Pemilihan Opsi A:** Fokus saat ini adalah menutup celah operasional dan observabilitas agar gerbang G2–G5 dapat segera dijalankan dengan bukti terukur. Guardrail strategi tambahan (`M22`) lebih tepat dievaluasi setelah melihat perilaku nyata di soak testnet.

---

## 5. Arsitektur & Spesifikasi per Milestone

```text
                ┌──► M19: CLI Diagnostik & Pra-Live Probes (botctl)
                │
M18: Exchange & ├──► M20: Soak Ops, Webhook & Akurasi PnL (store/exec/risk/backtest/botd)
Runtime Hardening
                └──► M21: Keamanan IPC & Observabilitas UI (bot-ipc/web-ui/desktop-ui)
```

### Kontrak Bersama (`bot-ipc` v1 — Additive & Backward-Compatible)
Agar `M19`, `M20`, dan `M21` dapat berjalan independen tanpa bentrok skema, semua penambahan field pada `SnapshotPayload`, `HeartbeatPayload`, dan `PositionView` di `crates/bot-ipc/src/protocol.rs` wajib menggunakan `#[serde(default, skip_serializing_if = "Option::is_none")]` (tetap `PROTOCOL_VERSION = 1`):
- `HeartbeatPayload`: tambah `loop_cadence_ms: Option<u64>`, `last_decision_tick_ms: Option<i64>`, `maintenance_paused: bool`, `db_ok: bool`.
- `SnapshotPayload`: tambah `notional_feasibility: Option<Vec<PairFeasibility>>` dan `balance_provenance: Option<BalanceProvenance>` (`source: "ws" | "rest"`, `age_ms: u64`).
- `PositionView`: tambah `direction: Option<Direction>` (`"Long" | "Short"`, dipetakan dari posisi asli engine di `botd/src/ipc_backend.rs`) agar komponen *mirrored bracket ladder* tidak bergantung pada inferensi `stop` vs `entry` yang rusak saat `be_done == true`.
- `protocol/ipc-v1.schema.json`, `web-ui/src/lib/api/types.ts`, dan `desktop-ui/src/Models.cpp` diperbarui serempak dengan tes kontrak `protocol-contract.test.ts`.

---

### M18 — Exchange & Runtime Reliability Hardening
**Crate terdampak:** `bot-exchange`, `bot-engine`, `botd`, `bot-config`

1. **Clock-Offset `Option<i64>` Sentinel (`bot-exchange`)**
   - Ubah penyimpanan offset jam di Spot & Futures REST/WS client menjadi `Option<i64>` (`None` = belum pernah sync; `Some(0)` = tersinkron presisi `0 ms`) disertai timestamp `last_sync_ms: Option<i64>`.
   - Mencegah pemanggilan `/time` berulang saat offset kebetulan tepat `0 ms`, serta membedakan "belum terukur" vs `+0ms` di diagnostik.
2. **Explicit Permanent-Error Classifier (`bot-exchange/src/error.rs`)**
   - Perluas `ExchangeErrorKind` dengan kode permanen yang belum tercakup: `-1022` (`InvalidSignature`), `-2014` (`ApiKeyFormatInvalid`), `-4046` (`NoNeedToChangeMarginType`), `-4059` (`NoNeedToChangePositionSide`).
   - Tambahkan method `GatewayError::is_retryable(&self) -> bool` agar lapisan retry langsung berhenti pada error permanen tanpa menghabiskan rate-limit.
3. **Client-Side Order Rate Budget Tracker (`bot-exchange/src/rate_limit.rs`)**
   - Perluas `RateLimitTracker` agar juga meng-ingest header HTTP REST (`X-MBX-USED-WEIGHT-1M`, `X-MBX-ORDER-COUNT-10S`, `X-MBX-ORDER-COUNT-1M`, `X-MBX-ORDER-COUNT-1D`) dan menyediakan `can_place_order(&self, now_ms: i64) -> Result<(), RateBudgetExceeded>` dengan margin pengaman (default 90% limit).
4. **WS-First Balance Cache + Provenance (`bot-exchange`)**
   - Tambahkan `BalanceCache` per `Venue` yang menjaga saldo dari event WS user-data stream (`outboundAccountPosition` / `ACCOUNT_UPDATE`). Karena event tersebut hanya dikirim saat saldo berubah (bukan heartbeat), selama sesi WS berada di `SessionState::Ready` dan cache sudah ter-seed, cache WS tetap otoritatif (`source = Ws`, `age_ms` = waktu sejak event WS terakhir atau seed REST terakhir).
   - Fallback ke REST snapshot dilakukan saat WS terputus, cache belum ter-seed, pasca-reconnect/sequence gap, atau saat interval rekonsiliasi pengaman berkala (default `reconcile_interval_ms = 300_000` / 5 menit) terlampaui — lalu *seeding* balik ke cache.
5. **Single-Instance Lock File (`botd`)**
   - Saat startup `binance-botd`, kunci file eksklusif non-blocking (via `std::fs::File::try_lock` tanpa `unsafe`, atau crate `fs2` jika MSRV perlu) di `<runtime_dir>/binance-botd.<mode>.lock` (mode `live` hanya izinkan 1 instance per host; `paper` boleh terpisah berdasarkan config/data dir). Gagal memperoleh lock → exit dengan pesan jelas sebelum menyentuh exchange/DB.
6. **Exchange Maintenance Auto-Pause & Active DB Probe (`bot-engine`, `botd`)**
   - Task health-check periodik melakukan probe REST `/ping` + SQLite `SELECT 1`.
   - Jika exchange mengembalikan indikasi maintenance / system busy (`503`, `-1001` beruntun, atau pesan maintenance), set flag `maintenance_paused = true` dan transisikan engine ke `EngineMode::Paused` (entry baru diblokir; proteksi OCO/Algo posisi terbuka **tetap utuh**; tidak menurunkan `Halted`/`Killed`).
   - **Kebijakan pemulihan (dipilih pemilik — Opsi 1):** saat `/ping` kembali sehat, **tetap di `EngineMode::Paused`** (hanya bersihkan flag `maintenance_paused = false`, catat `JournalEvent`, dan kirim alert webhook) hingga operator memberi perintah `Resume` secara eksplisit.
7. **Config Drift Check di CI (`bot-config`, `scripts/verify.sh`)**
   - Tambahkan unit/integration test di `bot-config` yang mem-parse file contoh konfigurasi TOML di repo dan memverifikasi tidak ada field wajib/default baru di `BotConfig` yang tertinggal atau tidak valid (strict unknown-field + round-trip check).

---

### M19 — CLI Diagnostik, Key Audit & Pra-Live Probes
**Crate terdampak:** `botctl` (menggunakan `bot-exchange`, `bot-config`, `bot-indicators`, `bot-strategy`)

1. **`binance-botctl key fingerprint` & `key inspect`**
   - Membaca public key atau menurunkan public key dari private key Ed25519 secara lokal untuk menampilkan SHA-256 fingerprint, format PEM valid, dan verifikasi izin file Unix (`0600` private key, `0700` direktori). **Tidak pernah** mencetak isi private key.
2. **`binance-botctl doctor [--config <path>] [--socket <path>] [--json]`**
   - Menjalankan matriks pengecekan dalam satu perintah: validasi config, keberadaan & izin key, keberadaan `live_confirmation_file` (jika mode live), konektivitas REST `/ping` + clock offset, status daemon via UDS, dan versi binary. Mengembalikan exit code non-zero jika ada check `FAIL`.
3. **`binance-botctl probe signed [--venue spot|futures] [--config <path>] [--json]`**
   - Menjalankan jalur klien asli `bot-exchange`: load credential Ed25519, self-test sign+verify lokal, cek clock offset (`-1021` guard), dan signed account read (`GET /api/v3/account` untuk Spot atau `GET /fapi/v2/account` untuk Futures — terverifikasi menyertakan `canTrade`, sedangkan `/fapi/v3/account` tidak) dengan *actionable hint* untuk kode error umum (`-2014`, `-2015`, `-1021`, `-1022`, `canTrade=false`). **Nol order dikirim.**
4. **`binance-botctl probe dry-run-order [--venue spot|futures] [--symbol BTCUSDT] [--json]`**
   - Mengambil `exchangeInfo` nyata (atau fixture offline saat `--offline`), menjalankan quantize `LOT_SIZE`/`PRICE_FILTER`/`NOTIONAL` dan menyusun payload + signature order entry & exit untuk jalur REST dan WS-API menggunakan stub transport (tidak mentransmisikan paket order ke jaringan).
5. **`binance-botctl probe ws-logon` & `binance-botctl dry-run signal --symbol <SYM> [--venue spot|futures]`**
   - `ws-logon`: untuk Spot menjalankan `session.logon` WS-API (`/ws-api/v3`) lalu menutup sesi bersih; untuk Futures (yang per `docs/verified-facts.md` tidak memiliki `session.logon` di `/ws-fapi/v1` dan memakai per-request Ed25519 signing) menghubungkan ke `/ws-fapi/v1`, menjalankan probe read-only signed `v2/account.balance`, menutup koneksi bersih, dan melaporkan secara eksplisit bahwa Futures memakai per-request signing.
   - `dry-run signal`: mengambil kline publik terbaru (`GET /api/v3/klines` / `GET /fapi/v1/klines`, atau fixture offline saat `--offline`), menjalankan pipeline `bot-indicators` + `bot-strategy`, dan mencetak breakdown gate per indikator tanpa menyalakan daemon dan tanpa mengirim order.

---

### M20 — Soak Operations, Webhook Alerts & Akurasi Ledger/PnL
**Crate terdampak:** `bot-store`, `bot-exec`, `bot-risk`, `bot-backtest`, `botd`, `botctl`

1. **Notifikasi Webhook Asinkron (`botd` / modul `notify`)**
   - Konfigurasi opsional `[notify]` di `bot-config` (URL webhook lewat env var / file terpisah `Secret<String>`, default mati).
   - Mendukung format Discord webhook & generic JSON HTTP POST; *bounded channel* (drop-oldest saat penuh), batching + cooldown per kategori event agar tidak spam rate-limit Discord.
   - Event yang dikirim: fill entry/exit, circuit breaker trip, maintenance auto-pause, soak watchdog alert, dan ringkasan harian UTC.
2. **Soak Watchdog & Summary Report (`botctl soak-watch` & `bot-backtest soak-report`)**
   - Membaca langsung SQLite `bot-store` (read-only WAL connection) + UDS heartbeat `botd`.
   - Mendeteksi daemon mati, decision loop stall melebihi `stall_threshold_secs`, atau circuit breaker trip; mengirim alert webhook + menulis log JSONL terstruktur.
   - Menghasilkan ringkasan soak akhir periode (durasi, jumlah trade, WR, PF, max DD, insiden likuidasi/unprotected algo) yang langsung kompatibel dengan templat gerbang `docs/gates/`.
3. **Notional Feasibility Check (`bot-risk`)**
   - Fungsi pure `evaluate_notional_feasibility(venue, equity, risk_cfg, venue_filters, live_atr_stop_pct)` menggunakan `Decimal`:
     - Pada saat *cold boot* (sebelum kline/ATR warmup), gunakan batas konservatif `max_stop_pct` dari `RiskConfig` (menghasilkan estimasi notional terkecil / pengecekan kelayakan paling ketat tanpa menambah field config baru); setelah warmup di `SnapshotPayload`, boleh memakai live ATR stop % per simbol dengan fallback ke `max_stop_pct`.
     - Hormati konfigurasi venue terkait (`risk.futures.risk_per_trade_pct` termasuk `short_size_multiplier` untuk sisi Short Futures, serta cap `max_notional_per_position_pct`; untuk `FixedNotional`, bandingkan langsung fixed notional terhadap `min_notional`).
     - Menghasilkan status layak/blokir dan `required_equity` per pair (`min_notional × stop_pct / risk_per_trade_pct`), dicatat sebagai peringatan saat boot `botd` dan diterbitkan di `SnapshotPayload` untuk UI.
4. **Leg-to-Trade Attribution & Expectancy/PF Regime Badge (`bot-store`, `bot-backtest`)**
   - Menyimpan `exit_reason` eksplisit pada order exit (bukan inferensi dari tanda PnL) dan mengelompokkan *partial exit* / *scale-out* dengan exit final ke dalam satu *logical trade* (via `position_id` / `trade_id` di `bot-store`) agar jumlah `closed_trades`, win rate, dan Profit Factor di `gate-report` akurat.
   - Menambahkan klasifikasi rezim Profit Factor di `bot-backtest` (`NetLosing < 1.0`, `SubThreshold 1.0–1.3`, `ViableEdge 1.3–2.0`, `StrongEdge 2.0–3.0`, `OverfitWarning > 3.0`) dengan flag signifikansi `sample_sufficient = trades >= 50`.
5. **Exchange PnL Cross-Check (`bot-exec`, `bot-store`)**
   - Setelah exit posisi Futures terisi, lakukan verifikasi asinkron (non-blocking) terhadap `userTrades` untuk `orderId` terkait dengan membandingkan **net realized PnL** (`sum(realizedPnl) - sum(commission_in_quote)`) terhadap `Position.realized_pnl_quote` lokal (funding dicatat terpisah).
   - Catat event `PnlDriftDetected` di journal & kirim alert webhook apabila selisih memenuhi **kedua** ambang: `abs_diff > $0.02` **AND** `rel_diff > 5%` (mencegah *false positive* dari pembulatan kecil pada trade besar).

---

### M21 — Keamanan IPC/Web & Observabilitas UI
**Crate/App terdampak:** `bot-ipc`, `web-ui`, `desktop-ui`

1. **One-Time WS Ticket, Non-Loopback Bind Safety & Cloudflare Mesh Origins (`bot-ipc`, `bot-config`, `botd`, `web-ui`)**
   - Tambahkan endpoint `POST /api/v1/ws-ticket` (terproteksi Bearer token di header `Authorization`) yang menerbitkan tiket acak sekali pakai berumur pendek (TTL 30 detik, single-use, bounded map). Klien WebSocket menukar tiket tersebut pada first-message auth sehingga Bearer token utama tidak pernah terpapar di URL atau log proxy.
   - Tambahkan `ipc.allowed_origins: Vec<String>` (default kosong = loopback-only) di `IpcConfig` agar akses langsung melalui jaringan **Cloudflare Mesh** (non-loopback) dapat diizinkan secara eksplisit baik pada `OriginPolicy` maupun header CSP `connect-src` (`bot-ipc/src/http.rs` dan `botd/src/main.rs`).
   - Tambahkan guard startup `evaluate_bind_safety(bind_addr, auth_configured, allowed_origins)`:
     - Jika `web_bind` bukan loopback (`127.0.0.1` / `::1`), **wajib** memiliki auth token non-kosong; bila tidak, `botd` menolak start dengan error konfigurasi eksplisit.
     - Jika `web_bind` adalah loopback dan `web_token_file` tidak ada, pertahankan perilaku aman saat ini (warning + HTTP server dinonaktifkan, UDS tetap jalan).
2. **Engine Health Heartbeat & Single Freshness Budget (`bot-ipc`, `web-ui`, `desktop-ui`)**
   - Terbitkan `loop_cadence_ms`, `last_decision_tick_ms`, `maintenance_paused`, dan `db_ok` pada `HeartbeatPayload`.
   - Di `web-ui`, satukan konstanta ambang staleness di satu modul (`web-ui/src/lib/format/freshness.ts`) sehingga `ModeBanner`, halaman `System`, dan `Dashboard` tidak pernah memberi status berbeda — serta tidak melabeli engine sebagai "stale" hanya karena `Paused` atau tidak ada sinyal baru.
3. **Direction-Aware Mirrored Bracket Ladder UI (`web-ui`, `desktop-ui`)**
   - Buat utilitas tunggal `positionMath.ts` (di `web-ui`) dan helper setara di `desktop-ui` untuk menghitung posisi relatif `SL ↔ Mark ↔ TP` yang *direction-aware*: untuk **Long** (`SL < Mark < TP`), untuk **Short** (`TP < Mark < SL` — bar terbalik rapi sehingga sisi profit selalu konsisten dan tidak pernah kolaps ke `—`).
   - Tampilkan pula panel ringkas `Notional Feasibility` & `Balance Provenance` (`WS` vs `REST` + usia) pada halaman `Risk` / `System`.
4. **Browser DOM Smoke Test *(Opsional di akhir M21)***
   - Skrip ringan di `web-ui/` yang dapat memverifikasi render halaman utama terhadap mock/live snapshot API tanpa error konsol (di-skip bersih jika browser tidak tersedia di lingkungan CI headless).

---

## 6. Backlog Ditunda (Pasca-G2/G3)

- **M22 — Guardrail Eksekusi, Risk & Marketdata:** Per-symbol loss re-entry cooldown (`#16`), pending-order runaway ATR guard (`#17`), watchlist anti-thrashing dwell + hysteresis (`#21`), kline sequence-gap guard (`#22`). Wajib diuji ulang terhadap backtest G1 sebelum diaktifkan.
- **M23 — Ekstensi Strategi & Utilitas Sekunder:** UTC liquidity session windows (`#25`), 4-factor Z-score + CTS dynamic screener (`#26`), ATR volatility spike alerts (`#27`), sanitized git config backup (`#29`), standalone exchange position watchdog (`#30a`).

---

## 7. Keputusan yang Diserahkan ke Implementer (*Left to the Builder*)

- Penamaan persis modul internal baru di dalam masing-masing crate (mis. `crates/bot-exchange/src/balance_cache.rs`, `crates/botd/src/lock.rs`, `crates/botd/src/notify.rs`).
- Detail struktur tabel/migrasi SQLite di `crates/bot-store/migrations/` untuk `exit_reason` dan pengelompokan `trade_id` selama tetap *idempotent* dan kompatibel dengan DB yang sudah ada.
- Pemilihan warna/styling detail komponen `BracketLadder` di `web-ui` dan `desktop-ui` selama mengikuti token desain Romer Hybrid (`app.css` / `Theme.qml`) yang sudah berlaku.
- Detail format teks pesan Discord/JSON webhook selama seluruh nilai uang terformat rapi dan tidak pernah memuat secret.
