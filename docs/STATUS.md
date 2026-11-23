# STATUS

**Milestone aktif:** M02 (selesai — menunggu persetujuan pemilik untuk lanjut ke M03)
**Mode build:** paper (tidak ada live)
**Terakhir diperbarui:** 2026-11-22

## Progres
- [x] **M00** — Scaffold, CI, Secret<T>, ADR  → `docs/milestones/M00-*.md`
- [x] **M01** — bot-types & bot-config  → `docs/milestones/M01-*.md`
- [x] **M02** — bot-exchange Spot: REST + WS API session  → `docs/milestones/M02-*.md`
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
- **2026-11-22 (M02):** Implementasikan `bot-exchange` Spot (§4.1–§4.2, §4.4, §3.5, Lampiran A):
  - Trait venue-agnostik `ExchangeGateway` & `VenueAdapter`, DTO/event bertag `venue: Venue`, klasifikasi error Binance (`GatewayError`, `ExchangeErrorKind`, `UnknownNeedsQuery` untuk timeout/5xx, `429`/`418` `retryAfter`).
  - Autentikasi Ed25519 PKCS#8 PEM (`ed25519-dalek` + `Secret<[u8; 32]>`) dengan canonical signing `session.logon` dan zeroization otomatis; ADR-0003 mencatat pilihan klien WS/REST kustom (`tokio-tungstenite` + `reqwest` rustls).
  - REST `SpotRestClient` (`GET /api/v3/time` offset jam & `GET /api/v3/exchangeInfo` → `InstrumentRules` + `rateLimits`).
  - Sesi WS API persisten `SpotWsSession`: state machine (`Disconnected → Connecting → LoggingOn → Subscribing → Ready → Draining`), peta korelasi UUID v7 `id → oneshot`, timeout 10 s, balasan `Pong` dengan salinan payload `Ping`, watchdog stale, `userDataStream.subscribe`, parser event (`executionReport`, `outboundAccountPosition`, `balanceUpdate`, `listStatus`, `eventStreamTerminated`, `serverShutdown`), rotasi ±23 jam make-before-break + dedup `(orderId, tradeId)`, reconnect backoff eksponensial 250 ms → 30 s full jitter + penghormatan `retryAfter`, dan `SpotAdapter::deactivate()` yang tidak membatalkan proteksi.
  - Verifikasi spec WS API Spot dicatat di `docs/verified-facts.md`.
- **Tes lulus:** `./scripts/verify.sh` hijau (`rustfmt`, `clippy -D warnings`, `cargo test --workspace` — termasuk 12 unit test + 6 mock WS/REST integration test `bot-exchange` mencakup korelasi paralel 32 request, rotasi `serverShutdown` make-before-break + dedup fill, reconnect + resubscribe, 429/418 `retryAfter`, `UnknownNeedsQuery` → `query_order`, dan uji log capture nol rahasia; secret scan ok); kompilasi & clippy `--features testnet-it` juga hijau.
- **Belum teruji:** Tes integrasi live terhadap Binance Spot Testnet (`cargo test -p bot-exchange --features testnet-it`) membutuhkan env var kredensial testnet milik pengguna; `cargo-deny` belum terpasang di VM ini (`deny.toml` sudah diperbarui untuk lisensi permisif `ISC`/`BSD-3-Clause`/`CDLA-Permissive-2.0` dari stack `rustls`/`aws-lc-rs`/`webpki-roots`).
