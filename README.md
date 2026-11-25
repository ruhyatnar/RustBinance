# Binance Rust Trading Bot (Spot ⇄ USDⓈ-M Futures)

Bot trading crypto Binance 24/7 berbasis Rust dengan UI Web (SvelteKit) dan Desktop (Qt6/QML, KDE Plasma 6), mendukung seluruh distro Linux utama (desktop, VPS headless, dan container/non-`systemd`). Mode default selalu **`paper`**.

## Instalasi Cepat & Interaktif (Satu Perintah — Semua Distro Linux)
Gunakan installer terpadu interaktif yang mendeteksi otomatis distro Linux Anda (`Arch/CachyOS`, `Debian/Ubuntu`, `Fedora/RHEL`, `openSUSE`, atau fallback Container untuk Alpine/non-`systemd`), memasang dependensi + memastikan Rust ≥ 1.89, mem-build proyek, dan menginstal ke sistem:
```bash
./install.sh
```
Opsi non-interaktif yang tersedia:
```bash
./install.sh --desktop --yes      # Full Desktop (Qt6/QML + Web UI + Daemon)
./install.sh --headless --yes     # Headless / VPS (tanpa Qt6/KDE)
./install.sh --container --yes    # Docker / Podman Compose
./install.sh --dry-run            # Pratinjau rencana perintah tanpa mengubah sistem
```
Layanan `binance-botd.service` **tidak** diaktifkan otomatis dan selalu berjalan dalam mode default `paper`.

## Instalasi Manual Bertahap (Arch/CachyOS, Debian/Ubuntu, Fedora, openSUSE)

### 1. Pasang dependensi sistem
```bash
# Workstation Linux (full desktop Qt6/KDE + web + daemon)
./scripts/deps.sh

# VPS / server headless (tanpa Qt6/KDE)
./scripts/deps.sh --headless
```

### 2. Build (sebagai user biasa, bukan root)
```bash
./scripts/build.sh                # atau ./scripts/build.sh --headless
```

### 3. Instal ke sistem (sebagai root)
```bash
sudo ./scripts/install-system.sh
```

### 4. Konfigurasi & mulai layanan
```bash
# Validasi konfigurasi
sudo -u binancebot binance-botctl config validate --config /etc/binance-bot/bot.toml

# Aktifkan & jalankan daemon (systemd)
sudo systemctl enable --now binance-botd

# Pantau log JSON
journalctl -u binance-botd -f -o cat
```

## Instalasi Container (Docker / Podman — Headless & Non-`systemd`)
Cocok untuk VPS atau distro non-`systemd` (Alpine, Void, Gentoo OpenRC, dll.). Image berjalan sebagai user non-root `binancebot` dengan default `mode=paper` tanpa menyertakan rahasia di dalam image:
```bash
mkdir -p packaging/config
cp packaging/bot.toml.example packaging/config/bot.toml
docker compose -f packaging/compose.yaml up -d --build
```

## Verifikasi & Pengembangan
```bash
./scripts/verify.sh
./scripts/tests/test_packaging.sh
```
