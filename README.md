# Binance Rust Trading Bot (Spot ⇄ USDⓈ-M Futures)

Bot trading crypto Binance 24/7 berbasis Rust dengan UI Web (SvelteKit) dan Desktop (Qt6/QML, KDE Plasma 6), target OS CachyOS/Arch Linux. Mode default selalu **`paper`**.

## Instalasi Bersih (CachyOS / Arch Linux)

### 1. Pasang dependensi sistem
```bash
./scripts/deps-arch.sh
```

### 2. Build (sebagai user biasa, bukan root)
```bash
./scripts/build.sh
```

### 3. Instal ke sistem (sebagai root)
```bash
sudo ./scripts/install-system.sh
```
Layanan `binance-botd.service` **tidak** diaktifkan otomatis dan berjalan dalam mode default `paper`.

### 4. Konfigurasi & mulai layanan
```bash
# Validasi konfigurasi
sudo -u binancebot binance-botctl config validate --config /etc/binance-bot/bot.toml

# Aktifkan & jalankan daemon
sudo systemctl enable --now binance-botd

# Pantau log JSON
journalctl -u binance-botd -f -o cat
```

## Verifikasi & Pengembangan
```bash
./scripts/verify.sh
```
