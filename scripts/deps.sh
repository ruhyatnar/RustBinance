#!/usr/bin/env bash
# Universal Linux dependency installer for Binance Rust Trading Bot.
# Supports Arch/CachyOS (pacman), Debian/Ubuntu (apt), Fedora/RHEL (dnf),
# and openSUSE (zypper). Use --headless to skip Qt6/KDE desktop UI packages.
set -euo pipefail

HEADLESS=0
DRY_RUN=0

for arg in "$@"; do
  case "$arg" in
    --headless) HEADLESS=1 ;;
    --dry-run)  DRY_RUN=1 ;;
    -h|--help)
      echo "Usage: $0 [--headless] [--dry-run]"
      echo "  --headless  Skip Qt6/KDE desktop UI dependencies (for VPS/servers)"
      echo "  --dry-run   Print package install commands without executing them"
      exit 0
      ;;
    *)
      echo "ERROR: Unknown option: $arg" >&2
      exit 1
      ;;
  esac
done

detect_pkg_mgr() {
  if [ -n "${BOT_PKG_MGR:-}" ]; then
    echo "$BOT_PKG_MGR"
    return
  fi
  if command -v pacman >/dev/null 2>&1; then echo "pacman"
  elif command -v apt-get >/dev/null 2>&1; then echo "apt"
  elif command -v dnf >/dev/null 2>&1; then echo "dnf"
  elif command -v zypper >/dev/null 2>&1; then echo "zypper"
  else echo "none"
  fi
}

run_cmd() {
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "[dry-run] $*"
  else
    "$@"
  fi
}

PKG_MGR="$(detect_pkg_mgr)"
NTP_UNIT="chronyd"

case "$PKG_MGR" in
  pacman)
    BASE_PKGS=(base-devel git cmake ninja pkgconf openssl rust nodejs npm chrony)
    DESKTOP_PKGS=(extra-cmake-modules qt6-base qt6-declarative qt6-svg qt6-graphs kirigami ki18n knotifications kstatusnotifieritem qqc2-desktop-style breeze)
    PKGS=("${BASE_PKGS[@]}")
    [ "$HEADLESS" -eq 0 ] && PKGS+=("${DESKTOP_PKGS[@]}")
    run_cmd sudo pacman -S --needed --noconfirm "${PKGS[@]}"
    ;;
  apt)
    NTP_UNIT="chrony"
    BASE_PKGS=(build-essential git cmake ninja-build pkg-config libssl-dev nodejs npm chrony)
    DESKTOP_PKGS=(extra-cmake-modules qt6-base-dev qt6-declarative-dev qt6-svg-dev qml6-module-org-kde-kirigami libkf6i18n-dev libkf6notifications-dev libkf6statusnotifieritem-dev)
    PKGS=("${BASE_PKGS[@]}")
    [ "$HEADLESS" -eq 0 ] && PKGS+=("${DESKTOP_PKGS[@]}")
    run_cmd sudo apt-get update
    run_cmd sudo apt-get install -y "${PKGS[@]}"
    ;;
  dnf)
    BASE_PKGS=(gcc gcc-c++ make git cmake ninja-build pkgconf-pkg-config openssl-devel rust cargo nodejs npm chrony)
    DESKTOP_PKGS=(extra-cmake-modules qt6-qtbase-devel qt6-qtdeclarative-devel qt6-qtsvg-devel kf6-kirigami-devel kf6-ki18n-devel kf6-knotifications-devel kf6-kstatusnotifieritem-devel)
    PKGS=("${BASE_PKGS[@]}")
    [ "$HEADLESS" -eq 0 ] && PKGS+=("${DESKTOP_PKGS[@]}")
    run_cmd sudo dnf install -y "${PKGS[@]}"
    ;;
  zypper)
    BASE_PKGS=(gcc gcc-c++ make git cmake ninja pkg-config libopenssl-devel rust cargo nodejs npm chrony)
    DESKTOP_PKGS=(extra-cmake-modules qt6-base-devel qt6-declarative-devel qt6-svg-devel kf6-kirigami-devel kf6-ki18n-devel kf6-knotifications-devel kf6-kstatusnotifieritem-devel)
    PKGS=("${BASE_PKGS[@]}")
    [ "$HEADLESS" -eq 0 ] && PKGS+=("${DESKTOP_PKGS[@]}")
    run_cmd sudo zypper install -y "${PKGS[@]}"
    ;;
  *)
    echo "ERROR: Unsupported package manager ('$PKG_MGR'). Supported: pacman, apt, dnf, zypper." >&2
    echo "For Alpine/non-systemd or other distros, use packaging/Dockerfile." >&2
    exit 1
    ;;
esac

if [ "$DRY_RUN" -eq 0 ] && command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
  sudo systemctl enable --now "$NTP_UNIT" || true
fi

if [ "$DRY_RUN" -eq 0 ] && command -v rustc >/dev/null 2>&1; then
  rustc --version
fi
