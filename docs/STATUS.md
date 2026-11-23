# STATUS

**Milestone aktif:** M03 (selesai — menunggu persetujuan pemilik untuk lanjut ke M04)
**Mode build:** paper (tidak ada live)
**Terakhir diperbarui:** 2026-11-22

## Progres
- [x] **M00** — Scaffold, CI, Secret<T>, ADR  → `docs/milestones/M00-*.md`
- [x] **M01** — bot-types & bot-config  → `docs/milestones/M01-*.md`
- [x] **M02** — bot-exchange Spot: REST + WS API session  → `docs/milestones/M02-*.md`
- [x] **M03** — bot-marketdata: stream, kline, order book sync, universe  → `docs/milestones/M03-*.md`
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
- **2026-11-22 (M03):** Implementasikan `bot-marketdata` (§3.3, §4.3, §6.2):
  - Klien Market Streams (`StreamPlan` membatasi ≤1024 stream/koneksi, combined URL `/stream?streams=...`, parser `kline`, `bookTicker`, `depthUpdate`, `!miniTicker@arr`, `serverShutdown`; `spawn_market_stream` dengan watchdog basi 1500 ms, balasan `Pong` menyalin payload `Ping`, backoff eksponensial + full jitter).
  - Kebijakan backpressure §3.3: `closed_klines_rx` via `mpsc` bounded (4096, tidak pernah drop); `bookTicker`, `BookTelemetry`, dan `!miniTicker@arr` via `watch` conflation.
  - `KlineBuffer` per `(Venue, Symbol, Timeframe)`: hanya `x=true` yang memicu event, deteksi gap + backfill via `KlineBackfillClient` (`SpotMarketRestClient` `GET /api/v3/klines`), dan status `is_ready()` setelah warm-up terpenuhi tanpa gap.
  - `LocalOrderBook` dengan prosedur sinkronisasi resmi Binance Spot (buffer → snapshot `GET /api/v3/depth` → buang `u <= lastUpdateId` → cek kontinuitas → `BookStatus::Invalid` saat gap + resync), kalkulasi `spread_bps` (`Decimal`) dan `OBI` top-N + EMA (`f64`, selalu `None` saat `Invalid`).
  - `UniverseFilter` (§6.2: kuotasi USDT, status `TRADING`, volume/spread/usia untuk core & scanner, eksklusi leveraged token, stable-vs-stable, dan `exclude_symbols`) + trait `UniverseProvider` & `spawn_universe_refresher` (15 menit).
  - Verifikasi prosedur order book Spot & batas Market Streams dicatat di `docs/verified-facts.md`; indeks `codegraph` dan `graphify` disinkronkan.
- **Tes lulus:** `./scripts/verify.sh` hijau (`rustfmt`, `clippy -D warnings`, `cargo test --workspace` — termasuk 6 unit test + 4 mock integration test `bot-marketdata`: fixture gap → `Invalid` → resync konsisten, kline non-closed tidak memicu sinyal + backfill gap, tes beban 20 simbol × 3 TF = 300 closed klines tanpa drop, watchdog 1500 ms reconnect + ping/pong; secret scan ok); kompilasi & clippy `--features testnet-it` juga hijau.
- **Belum teruji:** Tes kontrak live Spot Testnet (`cargo test -p bot-marketdata --features testnet-it`) belum dijalankan; `cargo-deny` belum terpasang di VM ini; perubahan M03 baru di-commit lokal dan belum di-`git push`.
