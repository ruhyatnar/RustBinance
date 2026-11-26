# STATUS

**Milestone aktif:** Selesai (M00–M18 engineering/tooling tuntas; operasional G2–G5 Spot & Futures pending sebelum live per `ADR-0005`)
**Mode build:** paper / testnet (tidak ada live)
**Terakhir diperbarui:** 2026-11-26

## Progres
- [x] **M00** — Scaffold, CI, Secret<T>, ADR  → `docs/milestones/M00-*.md`
- [x] **M01** — bot-types & bot-config  → `docs/milestones/M01-*.md`
- [x] **M02** — bot-exchange Spot: REST + WS API session  → `docs/milestones/M02-*.md`
- [x] **M03** — bot-marketdata: stream, kline, order book sync, universe  → `docs/milestones/M03-*.md`
- [x] **M04** — bot-indicators  → `docs/milestones/M04-*.md`
- [x] **M05** — bot-store: SQLite journal/snapshot/recovery  → `docs/milestones/M05-*.md`
- [x] **M06** — bot-risk: gatekeeper, sizing, breakers  → `docs/milestones/M06-*.md`
- [x] **M07** — bot-exec + bot-engine + paper broker  → `docs/milestones/M07-*.md`
- [x] **M08** — bot-strategy: S1, S2, shadow S3/S4 (Long)  → `docs/milestones/M08-*.md`
- [x] **M09** — bot-backtest: replay & walk-forward  → `docs/milestones/M09-*.md`
- [x] **M10** — bot-ipc + botctl  → `docs/milestones/M10-*.md`
- [x] **M11** — web-ui  → `docs/milestones/M11-*.md`
- [x] **M12** — desktop-ui (Qt6/Plasma 6)  → `docs/milestones/M12-*.md`
- [x] **M13** — Packaging, installer, systemd, chaos suite  → `docs/milestones/M13-*.md`
- [x] **M14** — OPERASIONAL: gerbang Spot G2→G5 (engineering/tooling selesai via `ADR-0005`; operasional G2–G5 pending sebelum live) → `docs/milestones/M14-*.md`
- [x] **M15** — FuturesAdapter  → `docs/milestones/M15-*.md`
- [x] **M16** — VenueManager & peralihan Spot ⇄ Futures, Short  → `docs/milestones/M16-*.md`
- [x] **M17** — OPERASIONAL: gerbang Futures; opsional dual (engineering/tooling & runbook selesai via `ADR-0005`; operasional G2–G5 Futures-Long/Short & `ADR-0006` mode `dual` pending sebelum live) → `docs/milestones/M17-*.md`
- [x] **M18** — Exchange & Runtime Reliability Hardening (engineering tuntas; M19–M21 & operasional G2–G5 Spot/Futures pending sebelum live) → `docs/superpowers/specs/2026-11-26-m18-m21-prelive-ops-design.md`

## Pertanyaan terbuka (untuk pemilik)
_(kosong)_

## Catatan sesi terakhir
- **2026-11-26 (M18 — Exchange & Runtime Reliability Hardening):** selesaikan seluruh 7 tugas M18 (`docs/superpowers/plans/2026-11-26-m18-exchange-runtime-hardening.md`):
  - `crates/bot-exchange`: `ClockSyncState` (`Option<i64>` membedakan belum sinkron vs offset `0 ms`) + `ping()` di Spot/Futures REST, klasifikasi error permanen Binance (`-1022`, `-2014`, `-4046`, `-4059`) + `is_retryable()`/`is_maintenance()`, ingesti header rate-limit REST (`X-MBX-USED-WEIGHT-*`/`X-MBX-ORDER-COUNT-*`) + `RateBudgetExceeded`/`can_place_order()`, dan `BalanceCache` WS-first dengan pelacakan `BalanceProvenance`.
  - `crates/botd`, `crates/bot-engine`, `crates/bot-store`: `ProcessLock` single-instance via `File::try_lock()` (`binance-botd.<mode>.lock`), `SqliteStore::ping()` (`SELECT 1`), serta `HealthProbeRunner` + `EngineCommand::{SetMaintenancePaused, ReportDbHealth}` (auto-pause ke `Paused` saat maintenance tanpa membatalkan proteksi OCO/Algo, tidak pernah menurunkan `Halted`/`Killed`, tetap `Paused` pasca-pemulihan hingga `Resume` eksplisit operator).
  - `crates/bot-config`: tambah uji drift CI `tests/config_drift.rs` memverifikasi `packaging/bot.toml.example` terurai ketat ke `BotConfig` dan lulus `load_from_paths` (`./scripts/verify.sh` hijau penuh). Catatan operasional: gerbang G2–G5 Spot & Futures tetap pending sebelum live.
- **2026-11-25 (Redesign Web UI — Romer Hybrid):** merombak menyeluruh `web-ui/` berbasis referensi `stitch_romer_saas_dashboard.zip` (spec `docs/superpowers/specs/2026-11-25-web-ui-romer-redesign-design.md`, plan `docs/superpowers/plans/2026-11-25-web-ui-romer-redesign.md`): token Tailwind v4 `@theme` + font offline `@fontsource/manrope` & `@fontsource/inter`, 4 primitif UI (`Panel`, `KpiCard`, `StatusPill`, `PageHeader`), sidebar kiri terkelompok + drawer responsif (`lg:hidden`), `VenueSwitchDialog` dengan `HoldToConfirm` 1,5 detik untuk `flatten`, tema obsidian Lightweight Charts, filter venue `All | Spot | Futures` (menjaga event global `venue == null`), dan restyle seluruh 9 rute + `AuthGate`. `scripts/verify.sh` hijau penuh.
- **2026-11-25 (Perbaikan Audit Lanjutan):** perbaiki temuan race TTL drain in-flight exec reports & mark unknown needs query saat `cancel_replace` network error di `crates/bot-exec/src/entry.rs`; perbaiki log terstruktur shutdown `botd/src/main.rs`; tambah regression test `execute_passive_entry_drains_in_flight_fill_on_timeout`; pasang Rust toolchain dan jalankan `scripts/verify.sh` penuh (semua pemeriksaan lulus).
- **2026-11-25 (Audit repo ponytail):** perbaiki 5 temuan di `bot-exec` (`entry.rs`, `paper.rs`) & `botd` (guard `live_confirmation_file` non-kosong); `scripts/verify.sh` hijau. Belum teruji: tes kegagalan store pada `entry.rs`; jalur fill susulan setelah cancel TTL (andalkan rekonsiliasi). Terbuka: format isi konfirmasi live yang lebih ketat (butuh ADR).
- **2026-11-24 (Audit Menyeluruh Pasca-M17 #1–#16 & Installer Terpadu `install.sh`):**
  - Menutup seluruh 16 temuan audit (Must Fix #1–#9, Should Fix #10–#14, Nice to Have #15–#16) di `bot-engine`, `bot-ipc`, `botd`, `bot-backtest`, `botctl`, `web-ui`, `desktop-ui`, `scripts/verify.sh`, dan `scripts/install-system.sh`.
  - Menambahkan installer interaktif terpadu `./install.sh` dengan deteksi otomatis distro Linux (`pacman`/`apt`/`dnf`/`zypper`/Container), jaminan Rust ≥ 1.89, dan `14/14` tes packaging lulus.
  - `./scripts/verify.sh` lulus penuh (`cargo fmt`, `clippy -D warnings`, `cargo test --workspace`, `web-ui` `27/27` Vitest + `vite build`, secret scan).
- **2026-11-24 (M17 — Tooling & Runbook Gerbang Futures + Draft ADR-0006 `dual`):**
  - `crates/bot-backtest`: tambah `GateScope` (`Spot`, `FuturesLong`, `FuturesShort`), `evaluate_sqlite_gate_for_scope` (pemisahan ketat Long vs Short + pengecekan `liquidation_incidents == 0` & `unprotected_algo_incidents == 0`), flag `--scope` di CLI `bot-backtest gate-report`, dan tes integrasi SQLite di `gate_report_suite.rs`.
  - `docs/gates/futures/`: tambah runbook & templat gerbang `long/G1.md`–`G5.md`, `short/G1.md`–`G5.md`, `chaos.md` (§13.4 Futures), dan `venue-switch-testnet.md` (uji peralihan Spot ⇄ Futures di testnet) — seluruhnya bertanda `PENDING OPERATIONAL RUN (NOT APPROVED FOR LIVE)`.
  - `docs/decisions/0006-dual-venue-mode-gate.md`: draft ADR prasyarat aktivasi mode `dual` (wajib lulus G4 Spot & G4 Futures + konfirmasi pemilik).
- **2026-11-24 (M16 — `VenueManager`, Spot ⇄ Futures switching, Cross-Venue Risk, Short & UI Switcher):**
  - `crates/bot-venue`: `VenueManager` state machine (`Disabled → Activating → Active → Draining`), prasyarat mode/cooldown 60s/limit 3× per hari, rollback atomik, dan transisi `Draining → Disabled`.
  - `crates/bot-risk`: `CrossVenueBudget` (equity gabungan USD, `portfolio_heat` gabungan, `cross_venue_same_symbol = block_opposite`, `capital_allocation_usd`) + `BtcSpikeGuard` untuk Short (+1.5%/15m atau +3%/60m dengan histeresis).
  - `crates/bot-strategy`: S1 & S2 Short mirror (§6.4.1), `max_high_3`/`max_high_5` di `IndicatorFeed`, dan matriks `supports(caps, dir)` (`allow_short`).
  - `crates/bot-store` & `crates/bot-engine`: `JournalEvent::VenueSwitched`, integrasi atomik `VenueManager` ke `EngineActor` (`EngineCommand::SwitchVenue`, `TelemetrySnapshot.{spot_state, futures_state}`), dan uji chaos §13.4 (`venue_switching_chaos.rs`).
  - `crates/bot-ipc` & `crates/botd`: aktifkan `switch_venue` (wajib `confirm_nonce` untuk `flatten`) dan `venues` live di snapshot telemetri.
  - `web-ui` & `desktop-ui`: tambahkan `VenueSwitcher.svelte` / `VenueSwitchDialog.svelte` (+ Vitest) dan `VenueSwitcher.qml` / `VenueSwitchDialog.qml`.
- **2026-11-24 (Perbaikan Audit Pra-M16):** Menutup seluruh temuan Must Fix (#1–#3) & Should Fix (#4–#6):
  - `crates/bot-ipc`: `PeerAuthPolicy::is_allowed_with_groups` memeriksa *supplementary groups* klien via `/proc/<pid>/status` (nol `unsafe`).
  - `crates/botd`: `EngineIpcBackend` + `spawn_telemetry_bridge` menghubungkan `CommandDispatcher` dan `TelemetryHub` langsung ke `EngineHandle` & `watch::Receiver<TelemetrySnapshot>`; `botd` memuat sibling `conf.d/` dan menolak mode non-`paper` hingga M16 (`VenueManager`) terpasang.
  - `crates/bot-exchange`: `FuturesAdapter::with_transport` kini memakai `FuturesWsSession` (`/ws-fapi/v1`, per-request Ed25519 signing) sebagai jalur utama dan `FuturesRestClient` (`/fapi/v1`) untuk `exchangeInfo`, validasi akun nyata saat `activate()`, serta fallback otomatis saat WS terputus (teruji via mock WS/HTTP di `tests/futures_transport.rs`).
  - Hygiene: menghapus duplikat direktori `binance-bot-agent-kit/` dan memperbarui `docs/STATUS.md` + `CHANGELOG.md`.
- **2026-11-24 (M15):** Implementasi `FuturesAdapter` (`bot-exchange`), Futures markPrice@1s + `pu` depth sync (`bot-marketdata`), Futures funding/liquidation/mark-last risk guard (`bot-risk`), dan `AlgoProtectionManager` (make-before-break) + `PaperFuturesBroker` (`bot-exec`).
- **2026-11-24 (M14):** Tooling gerbang Spot G1–G5 & laporan markdown (`ADR-0005`); eksekusi operasional testnet G2–G5 pending sebelum live.
- **2026-11-24 (M13):** Implementasikan packaging CachyOS/Arch (Opsi A tanpa `PKGBUILD`), daemon `binance-botd` penuh, hardening `systemd`, dan chaos suite Spot §13.4 (`13/13` tes lulus).
