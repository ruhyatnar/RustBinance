#!/usr/bin/env bash
# Stage 1 build script (§12.2): run as a regular (non-root) user.
set -euo pipefail

if [ "${EUID:-$(id -u)}" -eq 0 ]; then
  echo "ERROR: Jangan jalankan scripts/build.sh sebagai root. Jalankan sebagai user biasa." >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Building Rust workspace (release, locked)"
cargo build --release --locked --workspace

if [ -d "$ROOT/web-ui" ] && command -v npm >/dev/null 2>&1; then
  echo "==> Building web-ui"
  ( cd "$ROOT/web-ui" && npm ci && npm run build )
fi

if [ -d "$ROOT/desktop-ui" ] && command -v cmake >/dev/null 2>&1; then
  echo "==> Building desktop-ui (Qt6/QML)"
  cmake -S "$ROOT/desktop-ui" -B "$ROOT/desktop-ui/build" -G Ninja -DCMAKE_BUILD_TYPE=Release
  cmake --build "$ROOT/desktop-ui/build"
fi

echo "OK: build selesai."
