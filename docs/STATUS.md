# STATUS

**Milestone aktif:** M00 (selesai — menunggu persetujuan pemilik untuk lanjut ke M01)
**Mode build:** paper (tidak ada live)
**Terakhir diperbarui:** 2026-11-22

## Progres
- [x] **M00** — Scaffold, CI, Secret<T>, ADR  → `docs/milestones/M00-*.md`
- [ ] **M01** — bot-types & bot-config  → `docs/milestones/M01-*.md`
- [ ] **M02** — bot-exchange Spot: REST + WS API session  → `docs/milestones/M02-*.md`
- [ ] **M03** — bot-marketdata: stream, kline, order book sync, universe  → `docs/milestones/M03-*.md`
- [ ] **M04** — bot-indicators  → `docs/milestones/M04-*.md`
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
- **2026-11-22 (M00):** Promosi metadata kit ke root repo (ADR-0002), `.gitignore`, CI `.github/workflows/ci.yml`, workspace Cargo 15 crate dengan `#![forbid(unsafe_code)]`, clippy pedantic + deny unwrap/expect/panic/indexing di non-test, `deny.toml` (`cargo-deny` v2), `bot-types::Secret<T>` (`zeroize` + redacted `Debug`), skeleton JSON tracing `binance-botd`, serta pencatatan 5 fakta terverifikasi dari `binance-docs` ke `docs/verified-facts.md` + sinkronisasi 3 koreksi kecil spec.
- **Tes lulus:** `./scripts/verify.sh` hijau penuh (`rustfmt`, `clippy -D warnings`, `cargo test --workspace` 4/4 passed, `cargo deny check` ok, secret scan ok).
- **Belum teruji:** belum ada logika bisnis, koneksi jaringan/testnet, maupun UI (di luar cakupan M00).
