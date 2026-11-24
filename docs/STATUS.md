# STATUS

**Milestone aktif:** M12 (selesai — menunggu persetujuan pemilik untuk lanjut ke M13)
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
- [x] **M12** — desktop-ui (Qt6/Plasma 6)  → `docs/milestones/M12-*.md`
- [ ] **M13** — Packaging, installer, systemd, chaos suite  → `docs/milestones/M13-*.md`
- [ ] **M14** — OPERASIONAL: gerbang Spot G2→G5  → `docs/milestones/M14-*.md`
- [ ] **M15** — FuturesAdapter  → `docs/milestones/M15-*.md`
- [ ] **M16** — VenueManager & peralihan Spot ⇄ Futures, Short  → `docs/milestones/M16-*.md`
- [ ] **M17** — OPERASIONAL: gerbang Futures; opsional dual  → `docs/milestones/M17-*.md`

## Pertanyaan terbuka (untuk pemilik)
_(kosong)_

## Catatan sesi terakhir
- **2026-11-23 (M12):** Implementasikan `desktop-ui` (`binance-bot-ui`, C++20 + Qt6/QML + integrasi opsional KF6 Plasma 6, `ADR-0004`) serta perluasan kontrak UDS/WS IPC v1 (§11.1–§11.4, §10.2, §12.2):
  - `bot-ipc` & `protocol/ipc-v1.schema.json`: tambahkan envelope venue-aware `get_klines` / `klines` (`KlineBar`) dan `get_trades` / `trades` (`TradeRecord`) di UDS & WS (+ fixture `klines.json` & `trades.json`, sinkronisasi `web-ui/src/lib/api/types.ts`, dan contoh daemon `crates/bot-ipc/examples/mock_uds_daemon.rs`).
  - `desktop-ui/src/`: `DecimalFormat` murni berbasis string tanpa `double` untuk teks presisi, `StateStore` + 7 `QAbstractListModel` (`PositionsModel`, `OrdersModel`, `SignalsModel`, `MarketTicksModel`, `EventsModel`, `AlertsModel`, `TradesModel`) dengan deduplikasi notifikasi alert, `IpcClient` (`QLocalSocket` NDJSON, auto-discovery socket aman tanpa `/tmp` implisit + verifikasi `SO_PEERCRED` peer UID/GID, deteksi gap `seq` → `request_snapshot`, backoff eksponensial), `CommandService` (`QUuid::WithoutBraces`, mode *View-Only* terkunci default + `Unlock Controls`, alur 1 gerakan tahan 1,5 detik `request_confirm` → `panic_flatten` / `reset_kill`), serta `Notifier` (`KStatusNotifierItem` + `KNotification` saat `HAVE_KF6=1`, fallback `QSystemTrayIcon`, ikon terprogram per mode, close-to-tray, nol `PANIC` di menu tray).
  - `desktop-ui/qml/` & `scripts/deps-arch.sh`: singleton `Theme.qml` adaptif Breeze terang/gelap (`SystemPalette` + `qqc2-desktop-style`), `ModeBanner`, `HoldToConfirmButton` (blokir pintasan 1 tombol), `RiskGauge`, `DepthLadder`, `EventList`, `CandlesChart` (`Canvas` 2D + garis `entry`/`SL`/`TP1`/`TP2`), serta 9 layar (`Cockpit`, `Positions`, `Signals`, `Market`, `Risk`, `Performance`, `Logs`, `Settings` tighten-only tanpa angka hardcode, `System`).
  - **Tes lulus:** `./scripts/verify.sh` hijau penuh di Ubuntu 24.04 (`Qt 6.4.2`, `HAVE_KF6=0`) **dan** build `-DBOT_REQUIRE_KF6=ON` + CTest hijau penuh di container Arch Linux (`Qt 6.12.0`, `GCC 16.2.1`, `KF6StatusNotifierItem`/`KF6Notifications`/`KF6I18n` aktif, **3/3** suite CTest `tst_decimal_and_store`, `tst_ipc_uds_integration` terhadap daemon Rust nyata + restart reconnect, dan `tst_qml_smoke` 9 layar 0 warning).
  - **Belum teruji:** Uji klik interaktif ikon tray Wayland pada desktop fisik CachyOS Plasma 6, penyatuan daemon penuh `binance-botd` + `systemd` (M13), dan `VenueSwitcher` Spot ⇄ Futures (M16); commit belum di-`git push`.
