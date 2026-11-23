# STATUS

**Milestone aktif:** M05 (selesai — menunggu persetujuan pemilik untuk lanjut ke M06)
**Mode build:** paper (tidak ada live)
**Terakhir diperbarui:** 2026-11-22

## Progres
- [x] **M00** — Scaffold, CI, Secret<T>, ADR  → `docs/milestones/M00-*.md`
- [x] **M01** — bot-types & bot-config  → `docs/milestones/M01-*.md`
- [x] **M02** — bot-exchange Spot: REST + WS API session  → `docs/milestones/M02-*.md`
- [x] **M03** — bot-marketdata: stream, kline, order book sync, universe  → `docs/milestones/M03-*.md`
- [x] **M04** — bot-indicators  → `docs/milestones/M04-*.md`
- [x] **M05** — bot-store: SQLite journal/snapshot/recovery  → `docs/milestones/M05-*.md`
- [ ] **M06** — bot-risk: gatekeeper, sizing, breakers  → `docs/milestones/M06-*.md`
- [ ] **M07** — bot-exec + bot-engine + paper broker  → `docs/milestones/M07-*.md`
- [ ] **M08** — bot-strategy: S1, S2, shadow S3/S4 (Long)  → `docs/milestones/M08-*.md`
- [ ] **M09** — bot-backtest: replay & walk-forward  → `docs/milestones/M09-*.md`
- [ ] **M10** — bot-ipc + botctl  → `docs/milestones/M10-*.md`
- [ ] **M11** — web-ui  → `docs/milestones/M11-*.md`
- [ ] **M12** — desktop-ui (Qt6/Plasma 6)  → `docs/milestones/M12-*.md`
- [ ] **M13** — Packaging, installer, systemd, chaos suite  → `docs/milestones/M13-*.md`
- [ ] **M14** — OPERASIONAL: gerbang Spot G2→G5  → `docs/milestones/M14-*.md`
- [ ] **M15** — FuturesAdapter  → `docs/milestones/M15-*.md`
- [ ] **M16** — VenueManager & peralihan Spot ⇄ Futures, Short  → `docs/milestones/M16-*.md`
- [ ] **M17** — OPERASIONAL: gerbang Futures; opsional dual  → `docs/milestones/M17-*.md`

## Pertanyaan terbuka (untuk pemilik)
_(kosong)_

## Catatan sesi terakhir
- **2026-11-22 (M05):** Implementasikan `bot-store` (§8.1–§8.2, §5.1–§5.2) dengan SQLite WAL + `synchronous=FULL` + `foreign_keys=ON` (`rusqlite` 0.43.0 bundled SQLite 3.54+ menambal `RUSTSEC-2026-0459` / `CVE-2026-119375`; workspace MSRV dinaikkan ke 1.89):
  - Migrasi bernomor idempoten `migrations/0001_init.sql` (mencakup kolom `venue`, tabel `journal`, `orders`, `fills`, `positions`, `risk_state`, `venue_state`, `snapshots`, `signals_shadow`, `counters`, `schema_migrations`).
  - `JournalEnvelope` berversi (`EVENT_SCHEMA_VERSION = 1`, JSON dengan `Decimal` sebagai string), `PersistedEngineState` dengan replay deterministik (`recover_state()` = snapshot terakhir + journal setelah `last_seq`), API lengkap (`append_event`, `upsert_order/fill/position`, `snapshot_write/read_latest`, `risk_state_get/set`, `venue_state_get/set`, counter persisten `next_run_id`/`next_order_seq`, dan online `.backup` via `backup_to`), serta `StoreError` bertipe (`DiskFull`/`IoError`/`Corruption` → `is_fatal_storage()`).
  - Commit M04 (`526a467`) sudah di-push ke `origin/main`; `cargo-deny` dan `cargo-nextest` terpasang; indeks `codegraph` dan `graphify` disinkronkan.
- **Tes lulus:** `./scripts/verify.sh` hijau penuh (`rustfmt`, `clippy -D warnings`, `cargo test --workspace` 64/64 — termasuk 4 tes `bot-store`: migrasi idempoten/CRUD/counter/backup, `proptest` replay snapshot+journal acak == state asli, uji crash proses anak `std::process::abort()` di tengah transaksi + `PRAGMA integrity_check`, dan uji error I/O & versi skema; `cargo deny check` advisories/bans/licenses/sources OK; secret scan OK).
- **Belum teruji:** Rekonsiliasi dengan exchange pasca-recovery (cakupan M07); simulasi disk penuh nyata pada filesystem terbatas (`SQLITE_FULL` dipetakan langsung ke `StoreError::DiskFull`); commit M05 baru tersimpan lokal dan belum di-`git push`.
