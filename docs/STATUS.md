# STATUS

**Milestone aktif:** M04 (selesai — menunggu persetujuan pemilik untuk lanjut ke M05)
**Mode build:** paper (tidak ada live)
**Terakhir diperbarui:** 2026-11-22

## Progres
- [x] **M00** — Scaffold, CI, Secret<T>, ADR  → `docs/milestones/M00-*.md`
- [x] **M01** — bot-types & bot-config  → `docs/milestones/M01-*.md`
- [x] **M02** — bot-exchange Spot: REST + WS API session  → `docs/milestones/M02-*.md`
- [x] **M03** — bot-marketdata: stream, kline, order book sync, universe  → `docs/milestones/M03-*.md`
- [x] **M04** — bot-indicators  → `docs/milestones/M04-*.md`
- [ ] **M05** — bot-store: SQLite journal/snapshot/recovery  → `docs/milestones/M05-*.md`
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
- **2026-11-22 (M04):** Implementasikan seluruh indikator teknikal inkremental O(1), bebas I/O, tanpa alokasi per `update` di `bot-indicators` (§6.3):
  - `Ema` (seed SMA, `warmup_bars = 3n`), `Rsi` Wilder (`warmup_bars = n + 1`), `Atr` Wilder (`warmup_bars = n + 1`), `Adx` / `+DI` / `−DI` Wilder (`warmup_bars = 2n`), `Donchian` (bar sebelumnya dengan fixed-capacity circular monotonic deque, `warmup_bars = n + 1`, bebas look-ahead), `SessionVwap` (reset 00:00 UTC), dan `VolumeSma` (ring buffer O(1)) + trait `Indicator`.
  - Definisi matematis Wilder dicatat di `docs/verified-facts.md`; fixture referensi 650 bar disimpan di `crates/bot-indicators/tests/fixtures/reference_indicators.json`.
  - Commit M03 (`89da521`) sudah di-push ke `origin/main`; indeks `codegraph` dan `graphify` disinkronkan.
- **Tes lulus:** `./scripts/verify.sh` hijau (`rustfmt`, `clippy -D warnings`, `cargo test --workspace` — termasuk 4 tes `bot-indicators`: kecocokan fixture referensi toleransi `1e-6`, edge case warm-up & input degeneratif, uji anti look-ahead, dan benchmark O(1) 50.000 bar; secret scan ok).
- **Belum teruji:** `cargo-deny` belum terpasang di VM ini; commit M04 baru tersimpan lokal dan belum di-`git push`.
