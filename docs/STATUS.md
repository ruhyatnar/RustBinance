# STATUS

**Milestone aktif:** M13 (selesai — menunggu persetujuan pemilik untuk lanjut ke M14)
**Mode build:** paper (tidak ada live)
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
- [ ] **M14** — OPERASIONAL: gerbang Spot G2→G5  → `docs/milestones/M14-*.md`
- [ ] **M15** — FuturesAdapter  → `docs/milestones/M15-*.md`
- [ ] **M16** — VenueManager & peralihan Spot ⇄ Futures, Short  → `docs/milestones/M16-*.md`
- [ ] **M17** — OPERASIONAL: gerbang Futures; opsional dual  → `docs/milestones/M17-*.md`

## Pertanyaan terbuka (untuk pemilik)
_(kosong)_

## Catatan sesi terakhir
- **2026-11-24 (M13):** Implementasikan packaging CachyOS/Arch (Opsi A tanpa `PKGBUILD`), daemon `binance-botd` penuh, hardening `systemd`, dan chaos suite Spot §13.4:
  - `packaging/binance-botd.service` (§12.4 + hardening tambahan: `systemd-analyze security --offline=true` → **`1.3 OK :-)`**, dicatat di `docs/verified-facts.md`), `packaging/binance-trading-bot.desktop`, `scripts/build.sh` (non-root), `scripts/install-system.sh` (root, izin ketat §12.1, keypair Ed25519 terpisah per venue, **tidak** auto-enable), dan `README.md`.
  - `crates/botd`: implementasikan `sd_notify` + watchdog (`READY=1`, `WATCHDOG=1`, `STOPPING=1`) tanpa `unsafe`, CLI `--config`, guard Aturan Keras #2 (`mode=live` ditolak tanpa `live_confirmation_file`), bootstrap `SqliteStore` + `EngineActor` + `PaperBroker` + `bot-ipc` UDS server, serta handler `SIGTERM`/`SIGINT` yang hanya membatalkan order entry pending (proteksi OCO dipertahankan).
  - `crates/botd/tests/chaos_spot.rs` & `docs/gates/spot/chaos.md`: **13/13** tes lulus (seluruh 12 skenario Spot §13.4 + E2E `binance-botd` `NOTIFY_SOCKET` & `SIGTERM`).
  - **Belum teruji:** Eksekusi `sudo ./scripts/install-system.sh` & uji `iptables` drop paket nyata pada host CachyOS fisik (dijalankan bersama gerbang G2 Testnet di M14); commit belum di-`git push`.
