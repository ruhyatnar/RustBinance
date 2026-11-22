## 14. Struktur Repo & Roadmap Implementasi untuk Agen

### 14.1. Struktur Repo (Cargo workspace)

```text
binance-trading-bot/
├── Cargo.toml  Cargo.lock  rust-toolchain.toml (stable, ≥1.86)  deny.toml  clippy.toml  rustfmt.toml
├── AGENTS.md                       (salin Bagian 0 + perintah verifikasi)
├── CHANGELOG.md  docs/{decisions/,verified-facts.md,runbook.md}
├── crates/
│   ├── bot-types/        (domain: Decimal newtypes, enum, event, InstrumentRules)
│   ├── bot-config/       (TOML layering, validasi, tighten-only)
│   ├── bot-exchange/     (trait ExchangeGateway; binance/common: ws_api.rs ws_stream.rs rest.rs auth_ed25519.rs session.rs ratelimit.rs; binance/spot/; binance/futures/ [margin_setup, algo_orders, funding]; mock/)
│   ├── bot-marketdata/   (kline buffer, order book sync, universe, OBI)
│   ├── bot-indicators/   (EMA RSI ATR ADX Donchian VWAP — inkremental, tanpa I/O)
│   ├── bot-strategy/     (trait Strategy; trend_pullback.rs breakout.rs gainers.rs vwap_reversion.rs; shadow.rs)
│   ├── bot-risk/         (gatekeeper, sizing, heat, circuit_breaker, btc_guard)
│   ├── bot-exec/         (order FSM, router, protection (dual-OCO), position lifecycle, reconciler, paper_broker)
│   ├── bot-store/        (SQLite, migrations/, journal, snapshot, backup)
│   ├── bot-ipc/          (uds.rs, http_ws.rs, auth.rs, protocol.rs [+ JSON schema], conflate.rs)
│   ├── bot-venue/        (VenueManager, VenueAdapter, state machine & switching, anggaran lintas-venue)
│   ├── bot-engine/       (engine actor, mode FSM, wiring)
│   ├── bot-backtest/     (replay clock, sim broker, reports, walk-forward)
│   ├── botd/             (bin: binance-botd, sd_notify, signal handling)
│   └── botctl/           (bin: binance-botctl: keygen, status, panic, config, replay)
├── desktop-ui/           (Qt6/QML, lihat 11.4)
├── web-ui/               (SvelteKit, lihat 11.5)
├── packaging/            (binance-botd.service, bot.toml.example, .desktop, [opsional] PKGBUILD)
├── scripts/              (deps-arch.sh, build.sh, install-system.sh)
├── protocol/             (ipc-v1.schema.json — sumber kebenaran untuk Rust, QML, TS)
└── tests/                (integration, chaos, fixtures/)
```

Dependensi inti (pin versi eksak via `Cargo.lock`; ambil rilis stabil terbaru saat `cargo add`, **jangan** menyalin nomor versi dari dokumen ini): `tokio`, `tokio-tungstenite` (rustls), `futures-util`, `serde`/`serde_json`, `rust_decimal` (+`serde-str`), `time`, `uuid` (v7), `rusqlite` (bundled), `thiserror`, `anyhow` (hanya bin), `tracing` + `tracing-subscriber` (json, env-filter), `axum` (ws) + `tower-http`, `figment`, `clap`, `ed25519-dalek` + `pkcs8` + `zeroize`, `sd-notify`, `binance-sdk` (fitur `spot` dan `derivatives_trading_usds_futures`), dev: `proptest`, `insta`, `wiremock`/mock WS.

### 14.2. Milestone (urut; satu milestone = satu PR besar dengan test)

| ID | Deliverable | Acceptance (dapat diuji) |
|---|---|---|
| M0 | Scaffold workspace, CI (fmt/clippy/test/deny), `AGENTS.md`, ADR-0001, logging JSON, `Secret<T>` | CI hijau; `cargo test` lulus; secret tidak muncul di `Debug` |
| M1 | `bot-types`, `bot-config` (+validasi, tighten-only) | Test rentang nilai; config contoh 9 valid; nilai buruk → gagal start |
| M2 | `bot-exchange`: REST (`time`, `exchangeInfo`), auth Ed25519, WS API session (logon, korelasi id, ping/pong, rotasi 23 j, reconnect, subscribe user data) + **mock server** | Tes mock lulus; testnet: logon + `order.place`/`cancel` sukses; event fill diterima |
| M3 | `bot-marketdata`: stream kline/bookTicker/depth, **book sync**, universe filter | Fixture rekaman gap → resync; uji kontrak testnet |
| M4 | `bot-indicators` | Cocok dengan nilai referensi (toleransi 1e-6); O(1) per update |
| M5 | `bot-store`: skema, journal, snapshot, recovery | `kill -9` test: state pulih identik |
| M6 | `bot-risk`: gatekeeper, sizing, heat, circuit breakers, BTC guard | Property test lulus; tabel mode 7.4 terpenuhi |
| M7 | `bot-exec` + `bot-engine` + **paper broker**: FSM order, entry pasif, dual-OCO (simulasi), lifecycle, reconciler | Skenario end-to-end di paper: entry→TP1→BE→trail→exit; semua posisi selalu terproteksi |
| M8 | `bot-strategy`: S1, S2 + shadow S3/S4 + logging alasan gate | Golden test sinyal; no-look-ahead test |
| M9 | `bot-backtest` + laporan walk-forward | Hasil reproducible; laporan memuat biaya, DD, R-distribusi, sensitivitas parameter |
| M10 | `bot-ipc` + `botctl` | Auth negatif lulus; snapshot/delta/conflation; nonce panic |
| M11 | `web-ui` | Cockpit hidup di testnet; read-only vs full token; hold-to-confirm |
| M12 | `desktop-ui` (Qt6/Plasma 6) | Build CMake bersih di CachyOS; tray + notifikasi; reconnect IPC otomatis |
| M13 | Packaging + installer + systemd + chaos suite (13.4) | Semua skenario 13.4 lulus di testnet; `systemd-analyze security binance-botd` skor baik |
| M14 | **Operasional (bukan kode)**: G2→G5 untuk **Spot** | Dokumentasi hasil per gerbang di `docs/gates/spot/` |
| M15 | **FuturesAdapter**: setup akun (one-way/isolated/leverage/multi-assets), WS API + REST, user data (`ORDER_TRADE_UPDATE`/`ACCOUNT_UPDATE`/`ALGO_UPDATE`/`MARGIN_CALL`), proteksi algo make-before-break, funding & likuidasi, paper broker Futures | Tes mock + testnet Futures: entry→SL/TP algo→BE→trail→exit (Long & Short); funding gate & liq-buffer menolak kasus uji |
| M16 | **VenueManager & peralihan Spot ⇄ Futures** (3.5) + risiko lintas-venue (7.8) + sisi Short (6.4.1) + `switch_venue` di IPC + Venue Switcher UI (11.6) | Semua baris *Venue switching* di 13.4 lulus; counter risiko tidak ter-reset saat switch; rollback aktivasi teruji |
| M17 | **Operasional**: G1–G5 untuk **Futures-Long** lalu **Futures-Short**; opsional: mode `dual`, S3 live | Hasil di `docs/gates/futures/`; `dual` hanya via ADR terpisah setelah G4 kedua venue |

**Catatan urutan**: M0–M14 membangun & memvalidasi Spot, tetapi seluruh kode inti **MUST** venue-agnostic (kunci `(Venue,Symbol)`, kolom `venue`, `VenueAdapter`, event membawa `venue`) agar M15–M16 hanya menambah adapter & manajer — bukan refaktor besar. M15 dapat dikerjakan paralel dengan M14 (validasi Spot berjalan lama).

### 14.3. Prompt Awal yang Disarankan untuk Agen
> "Baca `binance-rust-bot-blueprint-v3.md` seluruhnya. Mulai dari M0 dan rancang semua tipe/DB/IPC **venue-agnostik sejak awal** (Spot & Futures, lihat 3.5). Patuhi Bagian 0. Untuk setiap item bertanda `[VERIFY]`, baca dokumentasi resmi Binance dan catat hasil di `docs/verified-facts.md` sebelum menulis kode terkait. Jangan melompati milestone. Setelah selesai satu milestone, jalankan perintah verifikasi Bagian 0 dan ringkas hasilnya."
