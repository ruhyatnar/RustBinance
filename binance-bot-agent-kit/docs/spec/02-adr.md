## 2. Keputusan Arsitektur (ADR Ringkas)

| ID | Keputusan | Alasan | Alternatif |
|---|---|---|---|
| A1 | **Dua venue, satu engine**: Spot dan USDⓈ-M Futures diimplementasikan sebagai `VenueAdapter` yang dikelola `VenueManager`; pengguna dapat **berganti mode Spot ⇄ Futures saat runtime** (3.5). Urutan *build* tetap Spot dulu (M2–M14), Futures menyusul (M15–M16), tetapi seluruh kode inti **MUST** venue-agnostic sejak M0 (kunci `(Venue, Symbol)`) | Permintaan pemilik; isolasi kegagalan per venue; Spot lebih sederhana untuk validasi awal | Dua binary terpisah (ditolak: duplikasi & UI ganda); venue dikunci di config (ditolak: tak bisa berpindah) |
| A1b | Tiap venue **lulus gerbang validasi sendiri** (Bagian 13.3) sebelum live; kelulusan Spot tidak diwarisi Futures, dan Futures-short dinilai terpisah dari Futures-long | Profil risiko berbeda (leverage, likuidasi, funding, short squeeze) | — |
| A2 | Daemon headless Rust (`binance-botd`) = satu-satunya pemilik state trading. UI (Qt & Web) = klien tipis | UI crash tidak boleh mempengaruhi trading | UI in-process (ditolak) |
| A3 | Core **single-writer engine actor** (satu task memiliki `EngineState`), I/O di task terpisah via channel bounded | Tanpa lock pada state; deterministik; mudah di-replay | `Arc<Mutex<State>>` (ditolak) |
| A4 | Auth **Ed25519** + `session.logon` pada WS API | Tanpa signing per-request; syarat user data stream baru | HMAC (deprecated oleh Binance) |
| A5 | WebSocket API = jalur order/akun; WebSocket Streams = data pasar; REST = rekonsiliasi & fallback | Sesuai permintaan | — |
| A6 | SDK resmi `binance-sdk` (crate Rust resmi, fitur `spot`, `derivatives_trading_usds_futures`) **dibungkus** trait `ExchangeGateway`; boleh diganti klien WS tipis sendiri bila semantik reconnect SDK tidak cukup | Isolasi dari perubahan API; testability (mock) | Panggil SDK langsung (ditolak). Pin versi **eksak** setelah `cargo add`; fitur penandatanganan private key mungkin butuh backend TLS OpenSSL `[VERIFY]` |
| A7 | Persistence: **SQLite (WAL)** via `rusqlite` (bundled) — event journal append-only + snapshot | Tahan crash, bisa diaudit, bisa di-replay | `sled` (ditolak), Postgres (berlebihan) |
| A8 | Desktop: **Qt6/QML + Kirigami (KF6)**, kode C++ tipis (`QLocalSocket` → model QML). Opsional: `cxx-qt` bila ingin glue Rust | Native Plasma 6, tray, notifikasi | Electron/Tauri (di luar permintaan) |
| A9 | Web: **SvelteKit (adapter-static) + Svelte 5 + Lightweight Charts**, dilayani Axum dari daemon | Satu artefak deploy | SPA terpisah |
| A10 | Target OS utama: **CachyOS (Arch)**; sekunder: Debian/Ubuntu/Fedora lewat script terpisah | Sesuai lingkungan pemilik | — |
| A11 | Waktu: sinkron NTP (chrony **atau** systemd-timesyncd); bot mengukur offset server dan **pause** bila > 800 ms | Error `-1021` | — |
| A12 | Telemetry & log: `tracing` JSON ke journald; metrics Prometheus opsional (`/metrics` lokal) | Observability 24/7 | — |
