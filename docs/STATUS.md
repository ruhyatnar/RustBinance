# STATUS

**Milestone aktif:** M06 (selesai — menunggu persetujuan pemilik untuk lanjut ke M07)
**Mode build:** paper (tidak ada live)
**Terakhir diperbarui:** 2026-11-23

## Progres
- [x] **M00** — Scaffold, CI, Secret<T>, ADR  → `docs/milestones/M00-*.md`
- [x] **M01** — bot-types & bot-config  → `docs/milestones/M01-*.md`
- [x] **M02** — bot-exchange Spot: REST + WS API session  → `docs/milestones/M02-*.md`
- [x] **M03** — bot-marketdata: stream, kline, order book sync, universe  → `docs/milestones/M03-*.md`
- [x] **M04** — bot-indicators  → `docs/milestones/M04-*.md`
- [x] **M05** — bot-store: SQLite journal/snapshot/recovery  → `docs/milestones/M05-*.md`
- [x] **M06** — bot-risk: gatekeeper, sizing, breakers  → `docs/milestones/M06-*.md`
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
- **2026-11-23 (M06):** Implementasikan `bot-risk` (§7.1–§7.7, §6.5) dengan aritmetika `Decimal` murni:
  - `Gatekeeper` (§7.2): pipeline pre-trade 9 tahap berurutan (`EngineMode::Ready`, simbol/universe/cooldown, BTC Crash Guard, kesegaran data & drift jam, batas posisi/heat/eksposur/korelasi lintas-venue, jarak stop `0.3%..=8%`, Kelly reducer + `PERCENT_PRICE_BY_SIDE` + sizing Spot + buffer fee `0.2%`, rate-limit guard `<60%`/`<50%`), update konfigurasi runtime strictly tighten-only via `bot_config::is_tightening`, serta hook `check_operational_health`.
  - `size_spot_position` (§7.3: `FixedRiskPct` & `FixedNotional`, pembulatan ke bawah `floor_to_step`, tolak bila `< min_qty` / `< min_notional` / `> heat_max`, tidak pernah dibulatkan naik) & `ratchet_stop_monotonic`; `evaluate_kelly` (hanya pengurang, aktif ≥100 trade, auto-disable strategi bila `f <= 0`).
  - `EngineModeMachine` (§7.4: seluruh transisi `Starting…Killed`, auto-release `Paused` ≥3 s & `ReduceOnly` ≥60 s, `Halted` harian reset 00:00 UTC, `Killed` hanya reset manual terkonfirmasi).
  - `CombinedEquityCounters` (equity gabungan lintas venue, harian/mingguan/drawdown, profit-lock `reduce` +2% → 0.5× / `stop` +3%, loss-streak cooldown 3 rugi → 2 jam / 5 rugi → 00:00 UTC) dan `BtcCrashGuard` (§7.5 histeresis 30 mnt + ATR normal + pemulihan ≥50% / 60 mnt tanpa low baru + cooldown anti-flapping 15 mnt).
  - Indeks `codegraph` & `graphify` disinkronkan.
- **Tes lulus:** `./scripts/verify.sh` hijau penuh (`rustfmt`, `clippy -D warnings`, `cargo test --workspace` 72/72 — termasuk 2 unit test Kelly + 6 tes `risk_suite`: golden test §7.3 equity 1.000 → qty 2.0, `proptest` risk/heat/step/monotonic stop, tabel transisi §7.4 + latch `Killed`, skenario histeresis & anti-flapping BTC Crash Guard, serta Gatekeeper + tighten-only + equity counters; `cargo deny check` ok; secret scan ok).
- **Belum teruji:** Risiko khusus Futures (§7.8: margin/likuidasi/funding/ADL — cakupan M15–M16); integrasi runtime ke `bot-engine` (cakupan M07); commit M06 baru tersimpan lokal dan belum di-`git push`.
