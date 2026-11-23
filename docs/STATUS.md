# STATUS

**Milestone aktif:** M09 (selesai — menunggu persetujuan pemilik untuk lanjut ke M10)
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
- [x] **M09** — bot-backtest: replay & walk-forward  → `docs/milestones/M09-*.md`
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
- **2026-11-23 (M09):** Implementasikan `bot-backtest` (§13.2–§13.3, §6.1) memakai kode `EngineActor`, `PaperBroker`, `Gatekeeper`, dan `Strategy` produksi yang sama (tanpa fork), serta catat verifikasi sumber & skema 12 kolom CSV `data.binance.vision` / `/api/v3/klines` di `docs/verified-facts.md`:
  - `data`: URL builder arsip resmi `data.binance.vision` (`build_data_vision_kline_url` & `.CHECKSUM` untuk Spot & USDⓈ-M Futures, bulanan & harian), parser/writer CSV 12 kolom (`parse_binance_kline_csv` / `write_binance_kline_csv` dengan deteksi header opsional & normalisasi otomatis timestamp milidetik pra-2025 vs mikrodetik 2025+), serta validator kontinuitas `validate_kline_series_gaps`.
  - `PaperBroker` & `runner`: dukungan mode replay §13.2 (`allow_same_bar_entry_fill = false` untuk eksekusi bar berikutnya, `strict_limit_maker_penetration = true` agar `LIMIT_MAKER` hanya terisi bila harga benar-benar menembus level, `CostModelConfig` default `1 tick + 0.5×spread` dan uji stres `2×`), serta `BacktestRunner` (`run`, `run_with_trace`, `run_with_trace_and_trials`).
  - `walk_forward`: `WalkForwardConfig` (`default_12m_3m_3m`: train 12 bln / test 3 bln / geser 3 bln), `WalkForwardRunner::run_s1` yang menyimpan hasil per jendela dan menghitung total percobaan parameter (`total_parameter_trials`) untuk koreksi *multiple testing*.
  - `metrics`: `BacktestReport` & `BacktestMetrics` lengkap (`net_pnl_usd`, `profit_factor`, `profit_factor_without_top3` tanpa 3 trade terbaik, `expectancy_r`, `max_drawdown_pct`, `sharpe_ratio`, `sortino_ratio`, `deflated_sharpe`, `turnover_multiple`, `exposure_pct`, `r_distribution` min/p25/median/p75/max, dan `evaluate_parameter_sensitivity` ±20%).
  - **Tes lulus:** `./scripts/verify.sh` hijau penuh (`rustfmt`, `clippy -D warnings`, `cargo test --workspace` 98/98 — termasuk uji reproducibility identik, uji anti look-ahead perturbasi bar masa depan pada seluruh pipeline replay, uji `LIMIT_MAKER` strict penetration & next-bar fill, laporan contoh S1/S2, dan walk-forward; `cargo deny check` ok; secret scan ok).
  - **Belum teruji:** Evaluasi gerbang operasional penuh 3 tahun data nyata G1–G5 (cakupan M14), IPC & CLI `botctl` (M10); commit M09 belum di-`git push`.
