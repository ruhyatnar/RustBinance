# Desain M12 — `desktop-ui` (Qt6/QML, Plasma 6)

**Tanggal:** 2026-11-23  
**Status:** Disetujui pemilik (telah melalui audit *builder check*)  
**Milestone:** M12 (`docs/milestones/M12-desktop-ui.md`)  
**ADR Terkait:** `docs/decisions/0004-desktop-ui-qt6-optional-kf6.md`

---

## 1. Tujuan & Mengapa
Membangun klien desktop native (`binance-bot-ui`) untuk CachyOS / Plasma 6 sebagai klien tipis (*thin client*) yang terhubung ke daemon `binance-botd` melalui Unix Domain Socket (`bot.sock`, protokol IPC v1). UI ini memberikan pemantauan 24/7, system tray & notifikasi sistem, serta kontrol darurat (*Hold-to-Confirm* 1,5 detik + *confirm nonce* 10 detik untuk `panic_flatten` / `reset_kill`) tanpa menaruh logika trading maupun angka hardcode di sisi UI.

## 2. Goals, Non-Goals, & Anti-Goals
- **Goals:**
  - Klien C++20 + Qt6/QML (`binance-bot-ui`) via UDS (`QLocalSocket`, JSON-lines, auto-reconnect backoff eksponensial, deteksi gap `seq` → `request_snapshot`).
  - 9 layar sesuai `docs/spec/11-ui.md` §11.2: `Cockpit`, `Positions`, `Signals`, `Market`, `Risk`, `Performance`, `Logs`, `Settings` (tighten-only), `System`.
  - **View-Only Lock Default (keputusan pemilik):** Meskipun UDS `SO_PEERCRED` memiliki `AuthScope::Full`, UI mulai dalam mode terkunci (`controlsUnlocked = false`) sehingga seluruh aksi mutasi (`Pause/Resume`, `Close`, `Settings`, `PANIC`, `Reset Kill`) tersembunyi/terkunci sampai operator menekan tombol eksplisit `Unlock Controls` di header (selaras dengan `web-ui` demi mencegah salah klik pada monitor 24/7).
  - **Hold-to-Confirm 1 Gerakan (keputusan pemilik):** Menahan `HoldToConfirmButton` selama 1,5 detik penuh secara otomatis menjalankan `request_confirm` → mendapatkan `confirm_nonce` (TTL 10 detik) → mengirim `panic_flatten` (atau `reset_kill`) dalam satu alur asinkron berkorelasi `cmd_id` (`QUuid::WithoutBraces`), tanpa pintasan keyboard 1 tombol.
  - **System Tray & Close-to-Tray (keputusan pemilik):** Memakai `KStatusNotifierItem` + `KNotification` saat KF6 tersedia di Plasma 6, atau `QSystemTrayIcon` sebagai fallback. Ikon tray berwarna per mode (`PAPER` biru, `TESTNET` kuning, `LIVE` merah-oranye) dengan menu `Open Cockpit`, `Pause / Resume` (hanya aktif saat terkoneksi), dan `Quit UI` (**tanpa** `PANIC` di menu tray). Menutup jendela utama `[X]` saat tray tersedia menyembunyikan jendela ke tray (`setQuitOnLastWindowClosed(false)`), tanpa mematikan daemon.
  - Tema Breeze terang/gelap otomatis (`SystemPalette` + `qqc2-desktop-style` di Plasma 6); aksesibilitas buta-warna (ikon `▲`/`▼`/`•` + label teks, bukan warna semata).
  - **Keamanan Socket Discovery:** Urutan penemuan implisit hanya menyentuh direktori aman: `--socket <path>` → `BINANCE_BOT_SOCKET` → `/run/binance-bot/bot.sock` → `$XDG_RUNTIME_DIR/binance-bot/bot.sock`. Direktori bersama seperti `/tmp/...` **tidak pernah** dipakai secara implisit (hanya bila diberikan eksplisit lewat `--socket` / `BINANCE_BOT_SOCKET`). Selain itu, `IpcClient` memvalidasi `SO_PEERCRED` socket peer di Linux (UID peer == `geteuid()` atau `0`, atau GID peer == grup `binancebot` bila ada).
- **Non-Goals:**
  - Venue Switcher Spot ⇄ Futures (dijadwalkan di M16).
  - Instalasi sistem penuh / `systemd` / `.desktop` packaging (dijadwalkan di M13).
  - Logika trading, kalkulasi indikator (seperti EMA di UI), atau angka hardcode di QML.
- **Anti-Goals:**
  - Ketergantungan QML pada `import org.kde.kirigami` atau `QtGraphs` yang memicu ketidakcocokan antara Qt 6.4 (Ubuntu 24.04 CI) dan Qt 6.12 (CachyOS).
  - Memakai `QGuiApplication` tanpa `Qt6::Widgets` (yang menyebabkan abort fatal saat membuat `QMenu` / `QSystemTrayIcon` / `KStatusNotifierItem`).

## 3. Batasan (Constraints) & Kepatuhan Aturan Keras
- **Aturan Keras #1–#8:** Default `mode=paper`; tanpa `unsafe` di Rust; tanpa `unwrap/expect/panic/indexing` di kode non-test Rust; semua kunci & pesan IPC membawa `venue`; parameter risiko hanya boleh diperketat (*tighten-only*).
- **Aturan Keras #4 (Presisi Desimal di UI):** Semua harga, qty, saldo, PnL, R-multiple, dan limit risiko disimpan dan ditampilkan murni sebagai `QString` (diformat dengan operasi string tanpa konversi `double`). Konversi ke `double` **hanya** diizinkan di dalam perhitungan koordinat piksel `CandlesChart.qml` (`Canvas`) dan rasio lebar bar `RiskGauge.qml` (identik dengan batas di M11 `web-ui`).
- **Kompatibilitas Baseline Qt 6.4 → Qt 6.12:**
  - `CMakeLists.txt` wajib mencari `Qt6 REQUIRED COMPONENTS Core Gui Widgets Qml Quick QuickControls2 Network Test`.
  - Menggunakan `qt_add_qml_module(... URI BinanceBot VERSION 1.0 RESOURCE_PREFIX /qt/qml ...)` dan memuat via `engine.load(QUrl(QStringLiteral("qrc:/qt/qml/BinanceBot/qml/Main.qml")))` (kompatibel di Qt 6.4 maupun Qt 6.12 tanpa bergantung pada `loadFromModule` yang baru ada di Qt 6.5).
  - `qml/Theme.qml` ditandai dengan `set_source_files_properties(qml/Theme.qml PROPERTIES QT_QML_SINGLETON_TYPE TRUE)` sebelum `qt_add_qml_module`.
  - Menyertakan `resources/binance-bot-ui.notifyrc` di QRC (`:/knotifications6/binance-bot-ui.notifyrc`) dan menonaktifkan tray/notifikasi secara *silent no-op* saat `QGuiApplication::platformName() == "offscreen"` atau sesi D-Bus/tray tidak tersedia agar tes headless bebas warning 100%.

## 4. Pendekatan yang Dipertimbangkan
1. **Pendekatan 1 — Pure QML Controls + Backend C++20 Adaptif KF6 + Custom Canvas Chart (Dipilih):**
   - Seluruh QML satu basis kode murni (`QtQuick`, `QtQuick.Controls`, `QtQuick.Layouts`, `Canvas`), otomatis mewarisi tema Breeze terang/gelap di Plasma 6 lewat `qqc2-desktop-style` + `SystemPalette`.
   - Integrasi KF6 (`KF6StatusNotifierItem`, `KF6Notifications`, `KF6I18n`) diisolasi di C++ (`#ifdef HAVE_KF6`), otomatis aktif di CachyOS dan turun ke `QSystemTrayIcon` di Ubuntu CI.
2. **Pendekatan 2 — Dua Varian QML (`qml-kf6/` & `qml-qt6/`):** Ditolak (duplikasi 9 layar, rawan tidak sinkron di M16).
3. **Pendekatan 3 — `cxx-qt` Rust-QML bridge:** Ditolak (butuh `unsafe` FFI dan rapuh terhadap pembaruan versi Qt di rolling-release).

## 5. Kontrak Wire IPC v1 (Perluasan Sinkron Lintas Target)
Untuk menjaga satu kabel UDS tanpa HTTP/bearer token sekaligus mematuhi **Aturan Keras #6 (`venue` wajib ada)** dan validasi ketat `protocol/ipc-v1.schema.json`:
1. **`ClientMessage` baru (didukung baik di UDS `uds.rs` maupun WS `http.rs`):**
   - `get_klines`: `{ "req_id": "<UuidString>", "venue": "spot"|"futures", "symbol": "BTCUSDT", "tf": "1h", "limit": 120 }`
   - `get_trades`: `{ "req_id": "<UuidString>", "venue": "spot"|"futures"|null, "from_ms": int|null, "to_ms": int|null }`
2. **`ServerMessage` baru:**
   - `klines`: `{ "req_id": "<UuidString>", "venue": "spot"|"futures", "symbol": "BTCUSDT", "tf": "1h", "klines": [KlineBar, ...] }` dengan `KlineBar { open_time_ms, close_time_ms, open, high, low, close, volume }` (semua harga/volume `DecimalString`).
   - `trades`: `{ "req_id": "<UuidString>", "venue": "spot"|"futures"|null, "trades": [TradeRecord, ...] }` dengan `TradeRecord { id, venue, symbol, strategy, side, entry, exit, qty, pnl_quote, profit_r, closed_at_ms }`.
3. **Sinkronisasi skema & tes kontrak:**
   - Memperbarui `protocol/ipc-v1.schema.json`, menambah fixture `protocol/fixtures/v1/{klines,trades}.json`, memperbarui `IpcBackend` trait di `crates/bot-ipc/src/command.rs` (beserta endpoint HTTP `/api/v1/klines` & `/api/v1/trades` agar tetap kompatibel dengan `web-ui`), memperbarui `crates/bot-ipc/tests/contract_suite.rs`, dan menyelaraskan `web-ui/src/lib/api/types.ts` (`VALID_ENVELOPE_TYPES` + tipe baru).

## 6. Rincian Komponen `desktop-ui` & Skrip
- **C++20 (`desktop-ui/src/`):**
  - `DecimalFormat.{h,cpp}`: pemformat string desimal presisi tanpa `double` (`formatDecimal`, `decimalSign`, `directionGlyph` `▲`/`▼`/`•`).
  - `IpcClient.{h,cpp}`: `QLocalSocket`, auto-discovery socket aman + cek `SO_PEERCRED`, parser JSON-lines `WireEnvelope`, deteksi gap `seq` (`seq != last_seq + 1` → kirim `request_snapshot`), backoff eksponensial (250 ms s/d 5.000 ms), korelasi `cmd_id` & `req_id`.
  - `StateStore.{h,cpp}`: memegang state reaktif `Q_PROPERTY` dari `hello`, `snapshot`, `delta`, `heartbeat`, `market`, `event` (maks 250), `klines`, `trades`, `controlsUnlocked`, serta deduplikasi ID alert agar reconnect tidak memicu spam notifikasi.
  - `PositionsModel.{h,cpp}`, `OrdersModel.{h,cpp}`, `SignalsModel.{h,cpp}`: `QAbstractListModel` dengan role lengkap termasuk `venue`.
  - `CommandService.{h,cpp}`: `Q_INVOKABLE` untuk `pause()`, `resume()`, `closePosition(id)`, `setParam(key, value)`, `disableStrategy(id)`, `ackAlert(id)`, `executeConfirmedAction(action)` (`request_confirm` → `panic_flatten` / `reset_kill`), `refreshKlines(venue, symbol, tf, limit)`, `refreshTrades()`.
  - `Notifier.{h,cpp}`: abstraksi tray + notifikasi (`#ifdef HAVE_KF6` memakai `KStatusNotifierItem` + `KNotification`, fallback `QSystemTrayIcon`), ikon dinamis dibuat secara terprogram per `BuildMode`/`EngineMode` (bebas aset eksternal hilang), memicu notifikasi pada transisi ke `Halted`/`Killed`, alert baru belum di-ack, dan event `level >= warn` / `Fill`.
- **QML (`desktop-ui/qml/`):**
  - `Theme.qml` (Singleton adaptif `SystemPalette` Breeze terang/gelap), `Main.qml` (`ApplicationWindow` 1280×820 + sidebar navigasi 9 layar + banner umpan balik perintah).
  - `components/`: `ModeBanner.qml` (termasuk tombol `Unlock Controls` & `HoldToConfirmButton` PANIC), `PanelCard.qml`, `RiskGauge.qml`, `HoldToConfirmButton.qml`, `DepthLadder.qml` (tabel top-of-book `MarketTickSummary`), `EventList.qml`, `CandlesChart.qml` (`Canvas` candlestick + garis `entry`/`stop`/`tp1`/`tp2`).
  - `pages/`: `CockpitPage.qml`, `PositionsPage.qml`, `SignalsPage.qml`, `MarketPage.qml`, `RiskPage.qml`, `PerformancePage.qml`, `LogsPage.qml`, `SettingsPage.qml` (diisi nilai nyata dari `StateStore`, tanpa angka hardcode), `SystemPage.qml` (termasuk pengubah path socket runtime).
- **Pengujian & Build:**
  - Contoh Rust `crates/bot-ipc/examples/mock_uds_daemon.rs` untuk tes integrasi nyata UDS.
  - `desktop-ui/tests/`:
    1. `tst_decimal_and_store`: uji `DecimalFormat`, `StateStore` (`snapshot` + `delta` + deduplikasi alert), dan model list.
    2. `tst_ipc_uds_integration`: menjalankan `mock_uds_daemon` Rust sesungguhnya (atau `QLocalServer` untuk injeksi gap `seq`), menguji `hello`+`snapshot`, `get_klines`/`get_trades`, alur nonce `panic_flatten`, deteksi gap `seq` → `request_snapshot`, serta **restart daemon → auto-reconnect**.
    3. `tst_qml_smoke`: memuat `Main.qml` dengan `QT_QPA_PLATFORM=offscreen` + menyuntikkan snapshot penuh dan gagal bila ada satu pun warning QML atau komponen gagal ter-render.
  - Memperbarui `scripts/deps-arch.sh` (termasuk `qqc2-desktop-style`), `scripts/verify.sh` (menambahkan `ctest --test-dir desktop-ui/build --output-on-failure`), dan `.github/workflows/ci.yml` (instal paket Qt6 Ubuntu + `verify.sh`), ditambah uji build KF6 penuh via container Arch Linux.
