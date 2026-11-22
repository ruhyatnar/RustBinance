# STATUS

**Milestone aktif:** M01 (selesai — menunggu persetujuan pemilik untuk lanjut ke M02)
**Mode build:** paper (tidak ada live)
**Terakhir diperbarui:** 2026-11-22

## Progres
- [x] **M00** — Scaffold, CI, Secret<T>, ADR  → `docs/milestones/M00-*.md`
- [x] **M01** — bot-types & bot-config  → `docs/milestones/M01-*.md`
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
- **2026-11-22 (M01):** Implementasikan seluruh tipe domain venue-agnostik `bot-types` (§3.4–§3.5) + `InstrumentRules` pembulatan/validasi `Decimal` presisi murni (`floor_to_step`, `round_to_tick`, `floor_to_tick`, `ceil_to_tick`, validasi `min_qty`/`max_qty`/`min_notional`/`TRADING`) dengan property test (`proptest`) & verifikasi filter notional Spot vs Futures di `docs/verified-facts.md`; implementasikan `bot-config` (§9 + §7.2 + §7.8) lengkap dengan `#[serde(deny_unknown_fields)]`, layering `default → bot.toml → conf.d/*.toml → BOT__*` via `figment`, validasi rentang & invarian keselamatan ketat tanpa clamp diam-diam, `is_tightening` (destructuring menyeluruh), serta `packaging/bot.toml.example`.
- **Tes lulus:** `./scripts/verify.sh` hijau penuh (`rustfmt`, `clippy -D warnings`, `cargo test --workspace` 28/28 passed, `cargo deny check` ok, secret scan ok).
- **Belum teruji:** konektivitas jaringan/testnet Binance, `VenueManager` switching runtime, persistence SQLite, maupun UI (di luar cakupan M01).
