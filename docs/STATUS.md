# STATUS

**Milestone aktif:** M08 (selesai — menunggu persetujuan pemilik untuk lanjut ke M09)
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
- [x] **M07** — bot-exec + bot-engine + paper broker  → `docs/milestones/M07-*.md`
- [x] **M08** — bot-strategy: S1, S2, shadow S3/S4 (Long)  → `docs/milestones/M08-*.md`
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
- **2026-11-23 (M08):** Implementasikan `bot-strategy` (§6.3–§6.5) sebagai penghasil `StrategyEntryIntent` murni tanpa sizing/order dan catat verifikasi `account.commission` di `docs/verified-facts.md`:
  - `Strategy` trait (`supports(&VenueCaps, Direction)`, `on_bar_close`, `evaluate` dengan `Vec<GateLog>` terstruktur untuk UI Signals), `IndicatorFeed` & `HtfRegimeFeed` (inkremental O(1), hanya bar closed).
  - `S1TrendPullback` (Long: HTF 4h `close > EMA50` & naik 3 bar, 1h `EMA21 > EMA55` & `close > EMA200`, pullback 3 bar, RSI naik, ADX ≥ 20 & `+DI > -DI`, volume ≥ 0.8×SMA20, anti-overextension, filter OBI opsional) dan `S2Breakout` (Long: `close > Donchian(20).upper`, volume ≥ 1.5×SMA20, ADX ≥ 18 naik, RSI 52–75, HTF searah, stop Donchian mid / 2×ATR).
  - Pagar kualitas §6.5 (`evaluate_quality_gates`): cost gate dengan `CommissionRates` aktual dari `account.commission`, spread & kesegaran `bookTicker < 500ms`, gap-free 300 bar, rezim BTC 4h, vol targeting (`clamp(target/realized, 0.5, 1.25)` + cap `<= 0.6` bila ATR% persentil 90d > 85), cooldown 4 bar, dan macro blackout.
  - Kerangka **shadow** (`S3GainersShadow` + `rank_gainers_top_k`, `S4VwapReversionShadow` default disabled): tipe `ShadowSignal` terpisah dari `StrategyEntryIntent` (secara arsitektur tidak bisa membuat order) dan tercatat ke `signals_shadow` SQLite.
  - **Tes lulus:** `./scripts/verify.sh` hijau penuh (`rustfmt`, `clippy -D warnings`, `cargo test --workspace` 92/92 — termasuk golden snapshot `insta`, tes anti look-ahead perturbasi bar masa depan, tes cost gate `< 3×` biaya, dan uji shadow S3/S4 ke `signals_shadow`; `cargo deny check` ok; secret scan ok).
  - **Belum teruji:** Sisi Short (M16), S3 live (pasca gerbang shadow), dan backtest walk-forward (M09); commit M08 belum di-`git push`.
