# STATUS

**Milestone aktif:** M07 (selesai — menunggu persetujuan pemilik untuk lanjut ke M08)
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
- **2026-11-23 (M07):** Implementasikan `bot-exec` (sub-langkah a–f) dan `bot-engine` (sub-langkah g–h) serta catat verifikasi `orderList.place.oco`/`belowTrailingDelta`, `listStatus`, dan kode penolakan `LIMIT_MAKER` `-2010` di `docs/verified-facts.md`:
  - `bot-exec`: `client_id` deterministik (`{bot}r{run}{S|F}-{seq:06}-{purpose}`), `OrderFsm` (§5.3: dedup fill idempoten per `(venue, symbol, trade_id)`, `filled_qty` monoton, resolusi `UnknownNeedsQuery` 3×), `PaperBroker` di balik `ExchangeGateway + VenueAdapter` (fee maker/taker, slippage, `-2010`, `cancelReplace`, Dual-OCO + auto-cancel leg pasangan), `execute_passive_entry` (§5.4 journal-first `OrderIntended`, chasing terbatas, retry `-2010` di tick book berikutnya), `DualOcoManager` (§4.4) + `HardStopWatcher` (§5.5), `evaluate_position_tick` (§5.5 urutan keselamatan dulu + laporan PnL), serta `reconcile_venue` (§8.3 + `orphan_policy`).
  - `bot-engine`: `EngineActor` single-writer (§3.2–§3.3: command `mpsc(64)` → `Busy`, user events `mpsc(4096)`, telemetry `watch`, seluruh entry lewat `Gatekeeper` + journal-first) dan shutdown SIGTERM (Aturan Keras #7: batalkan hanya order entry pending, **proteksi OCO tetap aktif**).
  - **Tes lulus:** `./scripts/verify.sh` hijau penuh (`rustfmt`, `clippy -D warnings`, `cargo test --workspace` 88/88 — termasuk `proptest` invarian FSM & `hard_stop_grace_ms`, E2E paper entry → TP1 → BE → trailing → TP2, stop-loss, time-stop, orphan `Flatten`/`AdoptWithStop`, crash proses anak `abort()` pada `EntryPending`/`OpenProtected`/`ExitPending`, dan uji eksplisit SIGTERM proteksi tetap; `cargo deny check` ok; secret scan ok).
  - **Belum teruji:** Strategi trading (M08), IPC/CLI/UI (M10–M12), Futures (M15–M16), dan commit M07 belum di-`git push`.
