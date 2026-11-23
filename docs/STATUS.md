# STATUS

**Milestone aktif:** M11 (selesai — menunggu persetujuan pemilik untuk lanjut ke M12)
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
- [x] **M10** — bot-ipc + botctl  → `docs/milestones/M10-*.md`
- [x] **M11** — web-ui  → `docs/milestones/M11-*.md`
- [ ] **M12** — desktop-ui (Qt6/Plasma 6)  → `docs/milestones/M12-*.md`
- [ ] **M13** — Packaging, installer, systemd, chaos suite  → `docs/milestones/M13-*.md`
- [ ] **M14** — OPERASIONAL: gerbang Spot G2→G5  → `docs/milestones/M14-*.md`
- [ ] **M15** — FuturesAdapter  → `docs/milestones/M15-*.md`
- [ ] **M16** — VenueManager & peralihan Spot ⇄ Futures, Short  → `docs/milestones/M16-*.md`
- [ ] **M17** — OPERASIONAL: gerbang Futures; opsional dual  → `docs/milestones/M17-*.md`

## Pertanyaan terbuka (untuk pemilik)
_(kosong)_

## Catatan sesi terakhir
- **2026-11-23 (M11):** Implementasikan `web-ui` (SvelteKit 2 `adapter-static` SPA + Svelte 5 runes + Tailwind CSS v4 + Lightweight Charts) serta 4 perbaikan kompatibilitas `bot-ipc`/CI (§11.1–§11.3, §11.5, §10):
  - `bot-ipc`: tambahkan `"auth_scope": "readonly" | "full"` pada `GET /api/v1/health` dan selaraskan `InMemoryIpcBackend::build_mode()` dengan `config.general.mode` (+ tes regresi).
  - `web-ui/src/lib/api/`: mirror TypeScript penuh `protocol/ipc-v1.schema.json` (`types.ts` + tes kontrak terhadap seluruh 9 fixture `protocol/fixtures/v1/*.json`), `IpcHttpClient` (header `Authorization: Bearer`, normalisasi `HttpErrorResponse` & `CommandResultPayload` ke `ApiError`), serta runes store `stream.svelte.ts` (token hanya memori/`sessionStorage`, nol `localStorage`, *first-message WS auth*, reconnect backoff eksponensial, deteksi gap `seq` → `request_snapshot`, mode read-only default + unlock eksplisit untuk token penuh).
  - `web-ui/src/lib/format/decimal.ts`: formatter desimal murni berbasis string (`formatDecimalString`, `decimalSign`, `decimalDirectionIcon` ▲/▼/•) tanpa `parseFloat` untuk nilai presisi.
  - Komponen & 9 layar: `AuthGate`, `ModeBanner` (`PAPER` biru / `TESTNET` kuning / `LIVE` merah-oranye bergaris + venue + status engine), `HoldToConfirm` (tahan 1,5 detik + nonce 10 detik, blokir pintasan 1 tombol), `RiskGauge`, `PositionTable`, `SignalsFeed`, `EventLog`, `DepthLadder`, `Candles` (Lightweight Charts), serta 9 rute (`Cockpit`, `Positions`, `Signals`, `Market`, `Risk`, `Performance`, `Logs`, `Settings` tighten-only, `System`).
  - **Tes lulus:** `./scripts/verify.sh` hijau penuh (`rustfmt`, `clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` **109/109** lulus, `cargo deny check` ok, `web-ui`: `npm ci && npm run check` 0 error/0 warning, `npm test` **23/23** Vitest lulus, `npm run build` static SPA selesai, secret scan ok).
  - **Belum teruji:** Uji manual browser interaktif terhadap daemon paper/testnet hidup (`binance-botd` wiring di M13), Desktop UI Qt6/QML (M12), dan Venue Switcher (M16); commit M11 belum di-`git push`.
