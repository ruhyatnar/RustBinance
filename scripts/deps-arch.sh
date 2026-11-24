#!/usr/bin/env bash
set -euo pipefail
sudo pacman -S --needed --noconfirm \
  base-devel git cmake ninja extra-cmake-modules pkgconf openssl \
  rust nodejs npm chrony \
  qt6-base qt6-declarative qt6-svg qt6-graphs \
  kirigami ki18n knotifications kstatusnotifieritem qqc2-desktop-style breeze
sudo systemctl enable --now chronyd     # nama unit di Arch: chronyd (bukan chrony)
rustc --version   # harus >= 1.86 (syarat SDK Binance)
