# STATUS

**Milestone aktif:** M16 (M15 `FuturesAdapter` selesai; menunggu persetujuan pemilik untuk lanjut ke M16)
**Mode build:** paper / testnet (tidak ada live)
**Terakhir diperbarui:** 2026-11-24

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
- [ ] **M16** — VenueManager & peralihan Spot ⇄ Futures, Short  → `docs/milestones/M16-*.md`
- [ ] **M17** — OPERASIONAL: gerbang Futures; opsional dual  → `docs/milestones/M17-*.md`

## Pertanyaan terbuka (untuk pemilik)
_(kosong)_

## Catatan sesi terakhir
- **2026-11-24 (Perbaikan Audit Pra-M16):** Menutup seluruh temuan Must Fix (#1–#3) & Should Fix (#4–#6):
  - `crates/bot-ipc`: `PeerAuthPolicy::is_allowed_with_groups` memeriksa *supplementary groups* klien via `/proc/<pid>/status` (nol `unsafe`).
  - `crates/botd`: `EngineIpcBackend` + `spawn_telemetry_bridge` menghubungkan `CommandDispatcher` dan `TelemetryHub` langsung ke `EngineHandle` & `watch::Receiver<TelemetrySnapshot>`; `botd` memuat sibling `conf.d/` dan menolak mode non-`paper` hingga M16 (`VenueManager`) terpasang.
  - `crates/bot-exchange`: `FuturesAdapter::with_transport` kini memakai `FuturesWsSession` (`/ws-fapi/v1`, per-request Ed25519 signing) sebagai jalur utama dan `FuturesRestClient` (`/fapi/v1`) untuk `exchangeInfo`, validasi akun nyata saat `activate()`, serta fallback otomatis saat WS terputus (teruji via mock WS/HTTP di `tests/futures_transport.rs`).
  - Hygiene: menghapus duplikat direktori `binance-bot-agent-kit/` dan memperbarui `docs/STATUS.md` + `CHANGELOG.md`.
- **2026-11-24 (M15):** Implementasi `FuturesAdapter` (`bot-exchange`), Futures markPrice@1s + `pu` depth sync (`bot-marketdata`), Futures funding/liquidation/mark-last risk guard (`bot-risk`), dan `AlgoProtectionManager` (make-before-break) + `PaperFuturesBroker` (`bot-exec`).
- **2026-11-24 (M14):** Tooling gerbang Spot G1–G5 & laporan markdown (`ADR-0005`); eksekusi operasional testnet G2–G5 pending sebelum live.
- **2026-11-24 (M13):** Implementasikan packaging CachyOS/Arch (Opsi A tanpa `PKGBUILD`), daemon `binance-botd` penuh, hardening `systemd`, dan chaos suite Spot §13.4 (`13/13` tes lulus).
