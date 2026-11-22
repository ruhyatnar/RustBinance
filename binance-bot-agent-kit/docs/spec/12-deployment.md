## 12. Deployment & Operasi (CachyOS/Arch-first) `[FIXED-v3]`

### 12.1. Tata Letak Sistem

| Item | Lokasi | Pemilik / Mode |
|---|---|---|
| Binary | `/usr/local/bin/{binance-botd,binance-botctl,binance-bot-ui}` | root 0755 |
| Aset web | `/usr/share/binance-bot/web` | root 0755 |
| Konfigurasi | `/etc/binance-bot/bot.toml` | `binancebot:binancebot` 0640 |
| Kunci | `/etc/binance-bot/keys/` | `binancebot` 0700; file 0600 |
| Data | `/var/lib/binance-bot/` (SQLite, backup) | `binancebot` 0750 |
| Socket | `/run/binance-bot/bot.sock` | `binancebot:binancebot` 0660 |
| Log | journald (`journalctl -u binance-botd`) | — |

Pengguna desktop ditambahkan ke grup `binancebot` agar UI Qt dapat membuka socket.

### 12.2. Skrip (dua tahap: build sebagai user biasa, instal sebagai root)

`scripts/deps-arch.sh` (CachyOS/Arch):
```bash
#!/usr/bin/env bash
set -euo pipefail
sudo pacman -S --needed --noconfirm \
  base-devel git cmake ninja extra-cmake-modules pkgconf openssl \
  rust nodejs npm chrony \
  qt6-base qt6-declarative qt6-svg qt6-graphs \
  kirigami ki18n knotifications kstatusnotifieritem breeze
sudo systemctl enable --now chronyd     # nama unit di Arch: chronyd (bukan chrony)
rustc --version   # harus >= 1.86 (syarat SDK Binance) [VERIFY]
```
> Nama paket KF6 bisa berubah antar rilis; bila `pacman` gagal, agen **MUST** mencari nama yang benar (`pacman -Ss kirigami`) lalu memperbarui skrip — bukan melewati langkah.

`scripts/build.sh` (jalankan sebagai user, **bukan root**):
```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked --workspace
( cd web-ui && npm ci && npm run build )
cmake -S desktop-ui -B desktop-ui/build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build desktop-ui/build
```

`scripts/install-system.sh` (root):
```bash
#!/usr/bin/env bash
set -euo pipefail
[ "$EUID" -eq 0 ] || { echo "Jalankan dengan sudo"; exit 1; }
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SVC=binancebot
CONF=/etc/binance-bot

id -u "$SVC" &>/dev/null || useradd --system --home-dir /var/lib/binance-bot \
    --create-home --shell /usr/bin/nologin "$SVC"
[ -n "${SUDO_USER:-}" ] && usermod -aG "$SVC" "$SUDO_USER"

install -d -m 0750 -o "$SVC" -g "$SVC" /var/lib/binance-bot /var/lib/binance-bot/backup
install -d -m 0750 -o "$SVC" -g "$SVC" "$CONF"
install -d -m 0700 -o "$SVC" -g "$SVC" "$CONF/keys"
install -d -m 0755 /usr/share/binance-bot/web

install -m 0755 "$ROOT/target/release/binance-botd"   /usr/local/bin/
install -m 0755 "$ROOT/target/release/binance-botctl" /usr/local/bin/
[ -x "$ROOT/desktop-ui/build/binance-bot-ui" ] && install -m 0755 "$ROOT/desktop-ui/build/binance-bot-ui" /usr/local/bin/
cp -r "$ROOT/web-ui/build/." /usr/share/binance-bot/web/

[ -f "$CONF/bot.toml" ] || install -m 0640 -o "$SVC" -g "$SVC" "$ROOT/packaging/bot.toml.example" "$CONF/bot.toml"
for V in spot futures; do   # keypair TERPISAH per venue
  [ -f "$CONF/keys/binance_${V}_ed25519.pem" ] || sudo -u "$SVC" /usr/local/bin/binance-botctl keygen --out "$CONF/keys" --name "binance_${V}_ed25519"
done
[ -f "$CONF/keys/web_token" ] || { head -c 32 /dev/urandom | base64 | tr -d '\n' > "$CONF/keys/web_token"; chown "$SVC:$SVC" "$CONF/keys/web_token"; chmod 0600 "$CONF/keys/web_token"; }

install -m 0644 "$ROOT/packaging/binance-botd.service" /etc/systemd/system/
install -m 0644 "$ROOT/packaging/binance-trading-bot.desktop" /usr/share/applications/
systemctl daemon-reload

cat <<MSG
Instalasi selesai (mode default = paper; layanan BELUM diaktifkan).
1) Daftarkan public key ke Binance API Management (Ed25519), satu API key per venue: cat $CONF/keys/binance_spot_ed25519.pub.pem  (dan ..._futures_... bila memakai Futures)
2) Simpan API key ke $CONF/keys/api_key_spot.txt dan api_key_futures.txt (0600, pemilik $SVC), whitelist IP, WITHDRAWAL & TRANSFER = OFF. Futures: set enabled=true di [venues.futures] hanya setelah testnet lulus.
3) Validasi: sudo -u $SVC binance-botctl config validate
4) Mulai: sudo systemctl enable --now binance-botd
5) Pantau: journalctl -u binance-botd -f -o cat
Logout/login ulang agar keanggotaan grup '$SVC' aktif untuk UI desktop.
MSG
```

### 12.3. Keygen Ed25519 (`binance-botctl keygen`) — perbaikan izin file

```rust
use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt, path::Path};
fn write_secret(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    // create_new → gagal bila sudah ada (tidak menimpa kunci lama); mode diterapkan SAAT dibuat (tanpa race)
    let mut f = OpenOptions::new().write(true).create_new(true).mode(mode).open(path)?;
    f.write_all(data)
}
// private: write_secret(priv_path, priv_pem.as_bytes(), 0o600); public: mode 0o644
// ed25519-dalek 2.x: SigningKey::generate(&mut rand_core::OsRng) + to_pkcs8_pem / to_public_key_pem (SPKI). Zeroize memori kunci.
```
Alternatif manual: `openssl genpkey -algorithm ED25519 -out k.pem && chmod 600 k.pem && openssl pkey -in k.pem -pubout -out k.pub.pem`.

### 12.4. `packaging/binance-botd.service`

```ini
[Unit]
Description=Binance Rust Trading Engine (binance-botd)
After=network-online.target time-sync.target
Wants=network-online.target time-sync.target

[Service]
Type=notify
User=binancebot
Group=binancebot
ExecStart=/usr/local/bin/binance-botd --config /etc/binance-bot/bot.toml
Restart=on-failure
RestartSec=5s
WatchdogSec=30
TimeoutStopSec=30
RuntimeDirectory=binance-bot
RuntimeDirectoryMode=0770
StateDirectory=binance-bot
LimitNOFILE=65536
UMask=0007
# Hardening
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
RestrictNamespaces=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallFilter=@system-service
ReadWritePaths=/var/lib/binance-bot

[Install]
WantedBy=multi-user.target
```
**Perilaku SIGTERM** `[FIXED-v3]`: pada shutdown, daemon membatalkan **order entry yang belum terisi** saja; **proteksi (OCO/stop/algo) TIDAK boleh dibatalkan** — posisi tetap terlindungi saat bot mati. (v2 membatalkan semua order pending: berbahaya.)

### 12.5. Operasional

| Kebutuhan | Perintah |
|---|---|
| Start/Stop/Restart | `sudo systemctl {start\|stop\|restart} binance-botd` |
| Status | `systemctl status binance-botd` / `binance-botctl status` |
| Log langsung | `journalctl -u binance-botd -f -o cat` |
| Jeda entry | `binance-botctl pause` / `resume` |
| Panic (darurat) | UI: tombol PANIC (tahan) · CLI: `binance-botctl panic-flatten --confirm` |
| Validasi konfig | `sudo -u binancebot binance-botctl config validate` |
| Sinkron waktu | `chronyc tracking` (CachyOS: layanan `chronyd`) |

Runbook ringkas: `-1021` → cek `chronyc tracking`, restart `chronyd`; `-2015` → cocokkan public key/IP whitelist/izin; WS sering putus → cek MTU/ISP, naikkan jitter reconnect; `Halted` karena loss harian → tidak ada aksi (reset otomatis 00:00 UTC), tinjau jurnal; `Killed` → investigasi, **baru** `reset-kill`.
