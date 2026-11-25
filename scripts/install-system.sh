#!/usr/bin/env bash
# Stage 2 system installer (§12.1–§12.2): run as root via sudo.
# Installs binaries, web assets, config, separate Ed25519 keypairs per venue,
# and hardened systemd unit. Does NOT auto-enable or start the service.
set -euo pipefail

if [ "${EUID:-$(id -u)}" -ne 0 ]; then
  echo "ERROR: Jalankan dengan sudo (root)." >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
if [ ! -x "$ROOT/target/release/binance-botd" ] || [ ! -x "$ROOT/target/release/binance-botctl" ]; then
  echo "ERROR: Binary release belum ditemukan di $ROOT/target/release/." >&2
  echo "Jalankan ./scripts/build.sh terlebih dahulu (sebagai user biasa, tanpa sudo)." >&2
  exit 1
fi

SVC="${BOT_SVC_USER:-binancebot}"
CONF="${BOT_CONF_DIR:-/etc/binance-bot}"
DATA="${BOT_DATA_DIR:-/var/lib/binance-bot}"
BIN_DIR="${BOT_BIN_DIR:-/usr/local/bin}"
WEB_DIR="${BOT_WEB_DIR:-/usr/share/binance-bot/web}"
SYSTEMD_DIR="${BOT_SYSTEMD_DIR:-/etc/systemd/system}"
DESKTOP_DIR="${BOT_DESKTOP_DIR:-/usr/share/applications}"

NOLOGIN_SHELL="$(command -v nologin 2>/dev/null || true)"
if [ -z "$NOLOGIN_SHELL" ]; then
  if [ -x /usr/sbin/nologin ]; then NOLOGIN_SHELL="/usr/sbin/nologin"
  elif [ -x /usr/bin/nologin ]; then NOLOGIN_SHELL="/usr/bin/nologin"
  else NOLOGIN_SHELL="/bin/false"
  fi
fi

id -u "$SVC" &>/dev/null || useradd --system --home-dir "$DATA" \
  --create-home --shell "$NOLOGIN_SHELL" "$SVC"

if [ -n "${SUDO_USER:-}" ] && id -u "$SUDO_USER" &>/dev/null; then
  usermod -aG "$SVC" "$SUDO_USER"
fi

install -d -m 0750 -o "$SVC" -g "$SVC" "$DATA" "$DATA/backup"
install -d -m 0750 -o "$SVC" -g "$SVC" "$CONF"
install -d -m 0700 -o "$SVC" -g "$SVC" "$CONF/keys"
install -d -m 0755 "$WEB_DIR" "$BIN_DIR" "$SYSTEMD_DIR" "$DESKTOP_DIR"

install -m 0755 "$ROOT/target/release/binance-botd"   "$BIN_DIR/"
install -m 0755 "$ROOT/target/release/binance-botctl" "$BIN_DIR/"
if [ -x "$ROOT/desktop-ui/build/binance-bot-ui" ]; then
  install -m 0755 "$ROOT/desktop-ui/build/binance-bot-ui" "$BIN_DIR/"
fi
if [ -d "$ROOT/web-ui/build" ]; then
  rm -rf "${WEB_DIR:?}/"*
  cp -r "$ROOT/web-ui/build/." "$WEB_DIR/"
fi

if [ ! -f "$CONF/bot.toml" ]; then
  install -m 0640 -o "$SVC" -g "$SVC" "$ROOT/packaging/bot.toml.example" "$CONF/bot.toml"
fi

for V in spot futures; do
  if [ ! -f "$CONF/keys/binance_${V}_ed25519.pem" ]; then
    sudo -u "$SVC" "$BIN_DIR/binance-botctl" keygen --out "$CONF/keys" --name "binance_${V}_ed25519"
  fi
done

if [ ! -f "$CONF/keys/web_token" ]; then
  (
    umask 0177
    head -c 32 /dev/urandom | base64 | tr -d '\n' > "$CONF/keys/web_token"
  )
  chown "$SVC:$SVC" "$CONF/keys/web_token"
  chmod 0600 "$CONF/keys/web_token"
fi

install -m 0644 "$ROOT/packaging/binance-botd.service" "$SYSTEMD_DIR/"
install -m 0644 "$ROOT/packaging/binance-trading-bot.desktop" "$DESKTOP_DIR/"

if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
  systemctl daemon-reload
else
  echo "INFO: systemd tidak aktif di host ini. Untuk distro non-systemd, jalankan daemon secara manual:"
  echo "  sudo -u $SVC $BIN_DIR/binance-botd --config $CONF/bot.toml"
  echo "  (atau gunakan packaging/Dockerfile & packaging/compose.yaml)"
fi

cat <<MSG
Instalasi selesai (mode default = paper; layanan BELUM diaktifkan).
1) Daftarkan public key ke Binance API Management (Ed25519), satu API key per venue:
   cat $CONF/keys/binance_spot_ed25519.pub.pem
   (dan $CONF/keys/binance_futures_ed25519.pub.pem bila memakai Futures)
2) Simpan API key ke $CONF/keys/api_key_spot.txt (dan api_key_futures.txt), mode 0600 pemilik $SVC,
   whitelist IP, WITHDRAWAL & TRANSFER = OFF.
3) Validasi konfigurasi:
   sudo -u $SVC $BIN_DIR/binance-botctl config validate --config $CONF/bot.toml
4) Mulai layanan (tidak auto-enable):
   sudo systemctl enable --now binance-botd
5) Pantau log:
   journalctl -u binance-botd -f -o cat
Logout/login ulang agar keanggotaan grup '$SVC' aktif untuk UI desktop.
MSG
