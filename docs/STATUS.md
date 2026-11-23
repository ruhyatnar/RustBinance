# STATUS

**Milestone aktif:** M10 (selesai — menunggu persetujuan pemilik untuk lanjut ke M11)
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
- **2026-11-23 (M10):** Implementasikan `bot-ipc`, `protocol/ipc-v1.schema.json` + fixture UI, audit `JournalEvent::Command`, serta CLI `binance-botctl` (§10, §7.4, §12.3, §12.5):
  - `protocol/ipc-v1.schema.json` & `protocol/fixtures/v1/*.json`: skema JSON Draft 2020-12 sebagai sumber kebenaran tunggal untuk Rust, TypeScript (Web UI), dan QML (Desktop UI), mencakup `hello`, `snapshot`, `delta`, `market`, `event`, `heartbeat`, `command_result`, `command`, `auth`, `request_snapshot`, dan respons error HTTP seragam (`{ "error": { "code", "message" } }`); semua desimal bertipe string dan seluruh posisi/order/market membawa `venue`.
  - `bot-ipc`: server UDS (`SO_PEERCRED` via `PeerAuthPolicy`, izin `0660`, NDJSON) dan Axum HTTP/WS loopback (`127.0.0.1`) dengan auth wajib di semua endpoint (`TokenAuth` full vs read-only via `Secret<String>` & perbandingan constant-time `subtle`), penolakan eksplisit token di query string (`?token=`/`?access_token=`), cek `Origin` ketat (CORS default tutup), header `Content-Security-Policy`, `RateLimiter` per-caller, `ConfirmNonceStore` 10 detik sekali pakai untuk `panic_flatten`/`reset_kill`, `CommandDispatcher` idempoten (`cmd_id`) dengan `set_param` tighten-only (`bot_config::is_tightening`), stub `switch_venue` → `not_supported` sampai M16, audit penuh ke SQLite `journal` (`kind = "Command"`), serta `conflate` (5–10 Hz) + `TelemetryHub` non-blocking yang memutus klien lambat tanpa memblok engine.
  - `botctl` (`binance-botctl`): subperintah `keygen` (Ed25519 PKCS#8 `0600` & SPKI `0644` sejak dibuat via `OpenOptions::create_new`, zeroize memori kunci, menolak menimpa kunci lama), `config validate`, `status [--json]`, `pause`, `resume`, `panic-flatten --confirm`, dan `reset-kill --confirm`.
  - **Tes lulus:** `./scripts/verify.sh` hijau penuh (`rustfmt`, `clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` **108/108** lulus — termasuk 2 tes kontrak skema & fixture UI, 5 tes keamanan/auth/Origin/query-token/rate-limit/nonce/audit/UDS `0660`+`SO_PEERCRED`/conflation/klien lambat, serta 3 tes integrasi `botctl`; `cargo deny check` ok; secret scan ok).
  - **Belum teruji:** Wiring daemon penuh `binance-botd` dengan engine + IPC (M13), Web UI (M11), Desktop UI (M12), dan logika `switch_venue` sungguhan (M16); commit M10 belum di-`git push`.
