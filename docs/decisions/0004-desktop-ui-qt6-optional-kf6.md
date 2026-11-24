# ADR-0004: Arsitektur `desktop-ui` Qt6/QML Murni dengan Integrasi KF6 Opsional (`HAVE_KF6`)

- **Status:** Accepted
- **Tanggal:** 2026-11-23
- **Milestone:** M12 (`desktop-ui`)

## Konteks

`docs/milestones/M12-desktop-ui.md` dan `docs/spec/11-ui.md` §11.4 menargetkan klien desktop native untuk CachyOS / KDE Plasma 6 menggunakan Qt6/QML dan KDE Frameworks 6 (`StatusNotifierItem`, `Notifications`, `I18n`, tema Breeze terang/gelap), sekaligus menuntut `scripts/verify.sh` dan CI GitHub Actions (`.github/workflows/ci.yml` pada Ubuntu 24.04 LTS) tetap hijau penuh.

Fakta teknis yang ditemukan saat verifikasi lintas lingkungan:
1. Ubuntu 24.04 (`ubuntu-latest`) menyediakan Qt `6.4.2`, namun **belum memiliki paket KF6** maupun `Qt6Graphs` di repositori resminya, sementara CachyOS/Arch menyediakan Qt `6.12+` dan KF6 `6.32+`.
2. File QML tidak memiliki praprosesor `#ifdef`: menulis `import org.kde.kirigami` atau `import QtGraphs` di dalam QML akan langsung menggagalkan pemuatan aplikasi pada lingkungan tanpa modul tersebut atau antar versi Qt yang berbeda.
3. Baik `KStatusNotifierItem::setContextMenu(QMenu*)` di KF6 maupun `QSystemTrayIcon` di Qt6 membutuhkan `QMenu` dari modul `Qt6::Widgets`, sehingga aplikasi wajib dijalankan dengan `QApplication` (bukan `QGuiApplication`).
4. Pada Plasma 6 (CachyOS/Arch), aplikasi `QtQuick.Controls` standar secara otomatis mewarisi tema Breeze terang/gelap native melalui plugin platform `qqc2-desktop-style` dan `SystemPalette` tanpa memerlukan `import org.kde.kirigami` di dalam QML.

## Keputusan

1. **Satu Basis Kode QML Murni (`QtQuick.Controls` + `QtQuick.Layouts` + `Canvas`):**
   - Seluruh 9 layar dan komponen QML ditulis murni memakai modul inti Qt6 (`RESOURCE_PREFIX /qt/qml`, dimuat via `qrc:/qt/qml/BinanceBot/qml/Main.qml` agar kompatibel dari Qt 6.4 hingga Qt 6.12+).
   - Candlestick chart (`CandlesChart.qml`) diimplementasikan menggunakan `Canvas` 2D sesuai opsi resmi di §11.4, lengkap dengan garis posisi `entry`, `stop`, `tp1`, dan `tp2`.
   - Tema Breeze terang/gelap diikuti secara dinamis lewat singleton `Theme.qml` (`SystemPalette`) dan paket `qqc2-desktop-style` yang ditambahkan ke `scripts/deps-arch.sh`.
2. **Integrasi KF6 Opsional di Lapisan C++ (`#ifdef HAVE_KF6`):**
   - CMake mencari `KF6` (`StatusNotifierItem`, `Notifications`, `I18n`) secara otomatis bila tersedia (`BOT_ENABLE_KF6=ON`, serta opsi tegas `BOT_REQUIRE_KF6=ON` untuk verifikasi rilis Arch/CachyOS).
   - Bila `HAVE_KF6` aktif (CachyOS Plasma 6), `Notifier` memakai `KStatusNotifierItem` (native Wayland SNI) dan `KNotification` (dengan resource `:/knotifications6/binance-bot-ui.notifyrc`). Bila tidak tersedia (Ubuntu CI), `Notifier` otomatis memakai `QSystemTrayIcon` dari `Qt6::Widgets`.
   - Pada mode headless (`QT_QPA_PLATFORM=offscreen`) atau tanpa sesi D-Bus/tray, `Notifier` beroperasi sebagai *silent no-op* sehingga tes smoke otomatis berjalan bersih tanpa warning.

## Konsekuensi

- **Positif:** Portabilitas maksimal (berjalan native penuh dengan fitur KF6 di CachyOS Plasma 6, sekaligus dapat dikompilasi dan diuji `ctest` secara cepat di Ubuntu 24.04 CI maupun desktop Linux lain tanpa duplikasi file QML). Bebas patah akibat perubahan API `QtGraphs` atau `Kirigami` antar rilis rolling.
- **Negatif:** Memiliki cabang `#ifdef HAVE_KF6` berukuran kecil (~50 baris) di `Notifier.cpp` yang dimitigasi dengan menguji kedua profil build (Ubuntu `HAVE_KF6=0` dan container Arch Linux `-DBOT_REQUIRE_KF6=ON`).
