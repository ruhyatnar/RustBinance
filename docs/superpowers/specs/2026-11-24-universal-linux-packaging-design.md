# Desain: Dukungan Universal Distro Linux (Desktop, Headless, & Container)

**Tanggal:** 2026-11-24  
**Status:** Menunggu persetujuan pemilik  
**Ukuran pekerjaan:** Perubahan kecil–menengah (packaging, skrip instalasi, container, dokumentasi)

## 1. Tujuan & Alasan
Kode inti Rust (`crates/`) sudah 100% distro-agnostik, tetapi skrip instalasi dan dokumentasi saat ini masih mengasumsikan CachyOS/Arch Linux (`pacman`, `chronyd`, `/usr/bin/nologin`). Perubahan ini menjadikan instalasi dan operasional bot universal di seluruh distro Linux utama — baik untuk **workstation desktop** (`botd` + `web-ui` + `desktop-ui` Qt6/KDE) maupun **VPS/server headless dan distro non-`systemd`** (`botd` + `web-ui` via native atau container).

## 2. Ruang Lingkup
- **Termasuk (Goals):**
  - `scripts/deps.sh`: deteksi otomatis package manager (`pacman`, `apt`, `dnf`, `zypper`) dan layanan NTP (`chronyd` vs `chrony`), dengan flag `--headless` untuk melewati dependensi Qt6/KDE.
  - `scripts/deps-arch.sh`: dipertahankan sebagai wrapper tipis ke `scripts/deps.sh` agar backward-compatible.
  - `scripts/install-system.sh`: pre-flight check keberadaan binary release (`binance-botd`, `binance-botctl`), deteksi dinamis `nologin` (`/usr/sbin/nologin` vs `/usr/bin/nologin`), dan fallback informatif bila `systemd` tidak tersedia.
  - `packaging/Dockerfile` & `packaging/compose.yaml`: multi-stage build untuk `binance-botd`, `binance-botctl`, dan `web-ui` (headless), berjalan sebagai user non-root, `mode=paper` secara default, tanpa menyimpan rahasia di dalam image.
  - `README.md`: diperbarui agar mendokumentasikan instalasi universal Linux (Desktop, Headless, dan Docker/Podman).
- **Tidak termasuk (Non-goals):**
  - Tidak mengubah perilaku trading, risk gate, atau kode Rust inti (`crates/`).
  - Tidak membuat skrip init native terpisah untuk OpenRC/runit (ditangani oleh container).
  - Tidak mengubah file tes Qt C++ (`private slots:` adalah sintaks standar Qt; false positive `tree-sitter-cpp`).
- **Anti-goals:**
  - Tidak pernah mengaktifkan `mode=live` secara default atau menyertakan file kunci/API key ke dalam image container.
  - Tidak pernah auto-enable atau auto-start layanan `systemd` saat instalasi.

## 3. Batasan & Aturan Keras
- Mematuhi seluruh Aturan Keras `AGENTS.md` (default `mode=paper`, tanpa menyentuh kunci rahasia, tanpa perintah destruktif).
- Image container harus menjalankan proses sebagai user non-root (`binancebot`) dan me-mount konfigurasi/kunci/data sebagai volume read-only/read-write terpisah.
- Skrip shell harus lulus `bash -n` (dan `shellcheck` bila tersedia di environment).

## 4. Pendekatan yang Dipertimbangkan
1. **Opsi A (Dipilih):** Skrip multi-distro `scripts/deps.sh` (`--headless`) + perbaikan `scripts/install-system.sh` + `packaging/Dockerfile` & `packaging/compose.yaml`.
   - *Trade-off:* Cakupan paling luas (desktop & server native di 4 keluarga distro utama + semua Linux/non-systemd via container) dengan beban perawatan rendah.
2. **Opsi B (Ditolak):** Native-only tanpa container.
   - *Alasan ditolak:* Pengguna VPS/Alpine/non-systemd tetap harus menyiapkan supervisor proses sendiri.
3. **Opsi C (Ditolak):** Native multi-distro + unit OpenRC & `runit`.
   - *Alasan ditolak:* Beban perawatan dan audit hardening lintas init system terlalu tinggi dibanding memakai container.

## 5. Detail Desain Singkat
- **`scripts/deps.sh [--headless]`:**
  - Memeriksa `pacman`, `apt-get`, `dnf`, atau `zypper` (gagal dengan pesan jelas jika tidak ada yang cocok).
  - Memasang toolchain dasar (Rust/build tools, Node.js/npm, OpenSSL/pkg-config, chrony) + paket Qt6/Kirigami hanya jika `--headless` **tidak** diberikan.
  - Mengaktifkan `chronyd` atau `chrony` hanya bila `systemctl` tersedia dan berjalan.
- **`scripts/install-system.sh`:**
  - Cek pre-flight: pastikan `$ROOT/target/release/binance-botd` dan `binance-botctl` ada sebelum membuat direktori/menyalin file.
  - Deteksi shell `nologin` via `command -v nologin` (fallback `/usr/sbin/nologin` → `/usr/bin/nologin` → `/bin/false`).
  - Jika `systemd` tidak aktif, tetap selesaikan instalasi binary/config/keys dan tampilkan cara menjalankan `binance-botd` secara manual.
- **`packaging/Dockerfile` & `packaging/compose.yaml`:**
  - Stage 1 (Rust builder): build `binance-botd` & `binance-botctl` (`--release`).
  - Stage 2 (Node builder): build `web-ui` statis.
  - Stage 3 (Runtime minimal, mis. `debian:bookworm-slim`): `ca-certificates`, user non-root `binancebot`, binary + aset web, volume `/etc/binance-bot` dan `/var/lib/binance-bot`, `ENTRYPOINT ["/usr/local/bin/binance-botd", "--config", "/etc/binance-bot/bot.toml"]`.

## 6. Yang Diserahkan ke Implementasi
- Pemetaan nama paket Qt6/KDE yang tepat per distro (`apt`, `dnf`, `zypper`, `pacman`).
- Penambahan tes otomatis untuk validasi sintaks shell, pre-flight check `install-system.sh`, dan verifikasi `Dockerfile` (tanpa mengubah host).
