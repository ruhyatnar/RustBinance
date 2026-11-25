#!/usr/bin/env bash
# Stage 1 build script (§12.2): run as a regular (non-root) user.
set -euo pipefail

if [ "${EUID:-$(id -u)}" -eq 0 ]; then
  echo "ERROR: Jangan jalankan scripts/build.sh sebagai root. Jalankan sebagai user biasa." >&2
  exit 1
fi

HEADLESS=0
for arg in "$@"; do
  case "$arg" in
    --headless) HEADLESS=1 ;;
    -h|--help)
      echo "Usage: $0 [--headless]"
      echo "  --headless  Lewati build desktop-ui (Qt6/QML)"
      exit 0
      ;;
    *)
      echo "ERROR: Unknown option: $arg" >&2
      exit 1
      ;;
  esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Pick up rustup toolchain if freshly installed in the same shell flow.
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

echo "==> Building Rust workspace (release, locked)"
cargo build --release --locked --workspace

if [ -d "$ROOT/web-ui" ] && command -v npm >/dev/null 2>&1; then
  echo "==> Building web-ui"
  ( cd "$ROOT/web-ui" && npm ci && npm run build )
fi

if [ "$HEADLESS" -eq 0 ] && [ -d "$ROOT/desktop-ui" ] && command -v cmake >/dev/null 2>&1; then
  if pkg-config --exists Qt6Core 2>/dev/null || command -v qmake6 >/dev/null 2>&1; then
    echo "==> Building desktop-ui (Qt6/QML)"
    cmake -S "$ROOT/desktop-ui" -B "$ROOT/desktop-ui/build" -G Ninja -DCMAKE_BUILD_TYPE=Release
    cmake --build "$ROOT/desktop-ui/build"
  else
    echo "INFO: Qt6 tidak ditemukan; build desktop-ui dilewati (gunakan --headless atau pasang Qt6)."
  fi
fi

echo "OK: build selesai."
