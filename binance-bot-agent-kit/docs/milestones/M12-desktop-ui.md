# M12 — desktop-ui (Qt6/QML, Plasma 6)

**Tujuan:** Cockpit desktop native di CachyOS/Plasma 6 sebagai klien tipis.

## Baca dulu (hanya ini)
- docs/spec/11-ui.md §11.1–11.4
- docs/spec/10-ipc.md
- docs/spec/12-deployment.md §12.2

## Tugas
1. CMake (Qt6 Core/Gui/Qml/Quick/QuickControls2/Network + KF6 Kirigami/I18n/Notifications/StatusNotifierItem) — **verifikasi nama paket Arch** (`pacman -Ss`) dan perbarui `scripts/deps-arch.sh`.
2. `IpcClient` (QLocalSocket, JSON-lines, auto-reconnect, gap `seq` → snapshot), `StateStore`, model QML (`PositionsModel`, `OrdersModel`, `SignalsModel`), `CommandService` (nonce flow).
3. Halaman: Cockpit, Positions, Signals, Risk, Logs, System (+ Market/Performance sederhana); komponen `ModeBanner`, `RiskGauge`, `HoldToConfirmButton`.
4. Tray (KStatusNotifierItem) berwarna per mode + notifikasi (KNotification); Panic **tidak** di menu tray.
5. Tema Breeze terang/gelap; ikon+teks (bukan hanya warna).

## Acceptance (harus dapat dibuktikan dengan tes/perintah)
- Build CMake bersih di CachyOS tanpa warning kritis.
- Tes smoke: aplikasi tersambung ke daemon paper, menampilkan snapshot, reconnect otomatis saat daemon restart.
- Tidak ada angka hardcode di QML; semua dari `StateStore`.

## Di luar cakupan (JANGAN dikerjakan di milestone ini)
- Venue Switcher (M16); logika trading di UI.

## `[VERIFY]` yang harus diselesaikan & dicatat di `docs/verified-facts.md` sebelum coding
- Nama paket KF6/Qt6 pada Arch/CachyOS.

## Penutup (wajib)
- `scripts/verify.sh` hijau; perbarui `docs/STATUS.md` (centang M12, isi catatan sesi), `CHANGELOG.md`.
- Ikuti `docs/spec/C-definition-of-done.md`. Berhenti setelah selesai; tunggu perintah pemilik untuk milestone berikutnya.
