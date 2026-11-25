#!/usr/bin/env bash
# Unified interactive Linux installer for Binance Rust Trading Bot.
# Detects the Linux distribution & package manager, then orchestrates:
#   1) scripts/deps.sh           (OS packages + Rust >= 1.89)
#   2) scripts/build.sh          (Rust release + Web UI + optional Qt6 Desktop UI)
#   3) sudo scripts/install-system.sh (binaries, config, Ed25519 keys, systemd)
# Or deploys via Docker/Podman Compose on non-systemd / unsupported distros.
# Default mode is ALWAYS paper; systemd service is never auto-enabled.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"

PROFILE=""        # desktop | headless | container
DO_DEPS=1
DO_BUILD=1
DO_INSTALL=1
ASSUME_YES=0
DRY_RUN=0

usage() {
  cat <<'EOF'
Binance Rust Trading Bot — Installer Terpadu Interaktif

Penggunaan:
  ./install.sh [opsi]

Opsi:
  --desktop       Profil Full Desktop (daemon + CLI + Web UI + Desktop Qt6/QML)
  --headless      Profil Headless / VPS (daemon + CLI + Web UI, tanpa Qt6/KDE)
  --container     Profil Container (Docker / Podman Compose, cocok untuk Alpine/non-systemd)
  --skip-deps     Lewati pemasangan dependensi sistem (scripts/deps.sh)
  --skip-build    Lewati proses kompilasi (scripts/build.sh)
  --skip-install  Lewati instalasi ke sistem (sudo scripts/install-system.sh)
  -y, --yes       Mode non-interaktif (gunakan pilihan default / flag tanpa prompt)
  --dry-run       Tampilkan rencana perintah tanpa mengeksekusi perubahan sistem
  -h, --help      Tampilkan bantuan ini
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --desktop)      PROFILE="desktop" ;;
    --headless)     PROFILE="headless" ;;
    --container)    PROFILE="container" ;;
    --skip-deps)    DO_DEPS=0 ;;
    --skip-build)   DO_BUILD=0 ;;
    --skip-install) DO_INSTALL=0 ;;
    -y|--yes)       ASSUME_YES=1 ;;
    --dry-run)      DRY_RUN=1 ;;
    -h|--help)      usage; exit 0 ;;
    *)
      echo "ERROR: Opsi tidak dikenal: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

# Warna hanya aktif di terminal interaktif.
if [ -t 1 ]; then
  C_BOLD=$'\033[1m'; C_CYAN=$'\033[36m'; C_GREEN=$'\033[32m'
  C_YELLOW=$'\033[33m'; C_RED=$'\033[31m'; C_RESET=$'\033[0m'
else
  C_BOLD=""; C_CYAN=""; C_GREEN=""; C_YELLOW=""; C_RED=""; C_RESET=""
fi

if [ "${EUID:-$(id -u)}" -eq 0 ] && [ "$DRY_RUN" -eq 0 ]; then
  echo "${C_RED}ERROR:${C_RESET} Jangan jalankan ./install.sh sebagai root." >&2
  echo "Jalankan sebagai user biasa; sudo akan diminta otomatis hanya saat dibutuhkan." >&2
  exit 1
fi

detect_os_info() {
  local os_file="${BOT_OS_RELEASE:-/etc/os-release}"
  OS_NAME="Linux (tidak diketahui)"
  OS_ID="unknown"
  OS_LIKE=""
  if [ -r "$os_file" ]; then
    OS_NAME="$(awk -F= '/^PRETTY_NAME=/{gsub(/"/,"",$2); print $2; exit}' "$os_file" || true)"
    OS_ID="$(awk -F= '/^ID=/{gsub(/"/,"",$2); print tolower($2); exit}' "$os_file" || true)"
    OS_LIKE="$(awk -F= '/^ID_LIKE=/{gsub(/"/,"",$2); print tolower($2); exit}' "$os_file" || true)"
  fi
  if [ -z "$OS_NAME" ]; then
    OS_NAME="Linux ($OS_ID)"
  fi
}

detect_pkg_mgr() {
  if [ -n "${BOT_PKG_MGR:-}" ]; then
    echo "$BOT_PKG_MGR"; return
  fi
  if command -v pacman >/dev/null 2>&1; then echo "pacman"
  elif command -v apt-get >/dev/null 2>&1; then echo "apt"
  elif command -v dnf >/dev/null 2>&1; then echo "dnf"
  elif command -v zypper >/dev/null 2>&1; then echo "zypper"
  else echo "none"
  fi
}

detect_distro_family() {
  case "$PKG_MGR" in
    pacman) echo "Arch / CachyOS / Manjaro (pacman)" ;;
    apt)    echo "Debian / Ubuntu / Mint (apt)" ;;
    dnf)    echo "Fedora / RHEL / Rocky (dnf)" ;;
    zypper) echo "openSUSE (zypper)" ;;
    *)      echo "Lainnya / Non-standar (direkomendasikan Container)" ;;
  esac
}

detect_container_cmd() {
  if command -v docker >/dev/null 2>&1; then echo "docker compose"
  elif command -v podman >/dev/null 2>&1; then echo "podman compose"
  else echo ""
  fi
}

prompt_yn() {
  local prompt="$1" default="$2" ans=""
  if [ "$ASSUME_YES" -eq 1 ] || [ ! -t 0 ]; then
    [ "$default" = "y" ]
    return
  fi
  local hint="[Y/n]"
  if [ "$default" = "n" ]; then hint="[y/N]"; fi
  read -r -p "$prompt $hint: " ans
  ans="$(printf '%s' "${ans:-$default}" | tr '[:upper:]' '[:lower:]')"
  [ "$ans" = "y" ] || [ "$ans" = "ya" ] || [ "$ans" = "yes" ]
}

run_step() {
  if [ "$DRY_RUN" -eq 1 ]; then
    echo "${C_YELLOW}[dry-run]${C_RESET} $*"
  else
    "$@"
  fi
}

detect_os_info
PKG_MGR="$(detect_pkg_mgr)"
FAMILY="$(detect_distro_family)"
HAS_SYSTEMD=0; if [ -d /run/systemd/system ]; then HAS_SYSTEMD=1; fi
HAS_GUI=0; if [ -n "${WAYLAND_DISPLAY:-}" ] || [ -n "${DISPLAY:-}" ]; then HAS_GUI=1; fi

# Pilih profil default cerdas sesuai sistem terdeteksi.
DEFAULT_PROFILE="headless"
if [ "$PKG_MGR" = "none" ]; then
  DEFAULT_PROFILE="container"
elif [ "$HAS_GUI" -eq 1 ]; then
  DEFAULT_PROFILE="desktop"
fi

echo "${C_BOLD}${C_CYAN}========================================================${C_RESET}"
echo "${C_BOLD}  Binance Rust Trading Bot — Installer Terpadu Linux${C_RESET}"
echo "${C_BOLD}${C_CYAN}========================================================${C_RESET}"
echo "  OS terdeteksi   : ${C_BOLD}$OS_NAME${C_RESET}"
echo "  Basis Linux     : $FAMILY"
echo "  Init systemd    : $(if [ "$HAS_SYSTEMD" -eq 1 ]; then echo "Aktif"; else echo "Tidak terdeteksi"; fi)"
echo "  Lingkungan GUI  : $(if [ "$HAS_GUI" -eq 1 ]; then echo "Terdeteksi"; else echo "Headless / Server"; fi)"
echo "  Mode keamanan   : ${C_GREEN}paper (default, layanan tidak auto-enable)${C_RESET}"
echo ""

if [ -z "$PROFILE" ]; then
  if [ "$ASSUME_YES" -eq 1 ] || [ ! -t 0 ]; then
    PROFILE="$DEFAULT_PROFILE"
  else
    echo "${C_BOLD}Pilih profil instalasi:${C_RESET}"
    echo "  1) Full Desktop — Daemon + CLI + Web UI + Desktop UI (Qt6/QML)"
    echo "  2) Headless / VPS — Daemon + CLI + Web UI (tanpa Qt6/KDE)"
    echo "  3) Container — Docker / Podman Compose (Alpine / non-systemd / terisolasi)"
    default_num="2"
    if [ "$DEFAULT_PROFILE" = "desktop" ]; then default_num="1"; fi
    if [ "$DEFAULT_PROFILE" = "container" ]; then default_num="3"; fi
    read -r -p "Pilihan [1-3, default $default_num]: " choice
    case "${choice:-$default_num}" in
      1) PROFILE="desktop" ;;
      2) PROFILE="headless" ;;
      3) PROFILE="container" ;;
      *) echo "${C_RED}ERROR:${C_RESET} Pilihan tidak valid." >&2; exit 1 ;;
    esac
  fi
fi

if [ "$PROFILE" != "container" ] && [ "$PKG_MGR" = "none" ] && [ "$DO_DEPS" -eq 1 ]; then
  echo "${C_RED}ERROR:${C_RESET} Package manager native tidak didukung untuk OS '$OS_NAME'." >&2
  echo "Gunakan profil --container (Docker/Podman) atau lewati dependensi dengan --skip-deps." >&2
  exit 1
fi

if [ "$PROFILE" != "container" ] && [ "$ASSUME_YES" -eq 0 ] && [ -t 0 ]; then
  echo ""
  echo "${C_BOLD}Sesuaikan langkah instalasi:${C_RESET}"
  if prompt_yn "  1) Pasang dependensi sistem & pastikan Rust >= 1.89 ($PKG_MGR)?" "y"; then DO_DEPS=1; else DO_DEPS=0; fi
  if prompt_yn "  2) Kompilasi proyek (Rust release + UI)?" "y"; then DO_BUILD=1; else DO_BUILD=0; fi
  if prompt_yn "  3) Instal ke sistem (/usr/local/bin, /etc/binance-bot, keypair Ed25519)?" "y"; then DO_INSTALL=1; else DO_INSTALL=0; fi
fi

echo ""
echo "${C_BOLD}Ringkasan Rencana Instalasi:${C_RESET}"
echo "  Profil          : $PROFILE"
if [ "$PROFILE" = "container" ]; then
  echo "  Aksi            : Siapkan packaging/config/bot.toml (mode=paper) & build container"
else
  DEPS_HINT=""; if [ "$PROFILE" = "headless" ]; then DEPS_HINT=" --headless"; fi
  echo "  1. Dependensi   : $(if [ "$DO_DEPS" -eq 1 ]; then echo "Ya (scripts/deps.sh$DEPS_HINT)"; else echo "Lewati"; fi)"
  echo "  2. Build        : $(if [ "$DO_BUILD" -eq 1 ]; then echo "Ya (scripts/build.sh$DEPS_HINT)"; else echo "Lewati"; fi)"
  echo "  3. Instal sistem: $(if [ "$DO_INSTALL" -eq 1 ]; then echo "Ya (sudo scripts/install-system.sh)"; else echo "Lewati"; fi)"
fi
echo ""

if [ "$ASSUME_YES" -eq 0 ] && [ -t 0 ]; then
  if ! prompt_yn "Lanjutkan instalasi?" "y"; then
    echo "Instalasi dibatalkan oleh pengguna."
    exit 0
  fi
fi

if [ "$PROFILE" = "container" ]; then
  COMPOSE_CMD="$(detect_container_cmd)"
  if [ -z "$COMPOSE_CMD" ] && [ "$DRY_RUN" -eq 0 ]; then
    echo "${C_RED}ERROR:${C_RESET} Docker atau Podman tidak ditemukan. Pasang docker/podman terlebih dahulu." >&2
    exit 1
  fi
  if [ -z "$COMPOSE_CMD" ]; then COMPOSE_CMD="docker compose"; fi
  run_step mkdir -p "$ROOT/packaging/config"
  if [ ! -f "$ROOT/packaging/config/bot.toml" ]; then
    run_step cp "$ROOT/packaging/bot.toml.example" "$ROOT/packaging/config/bot.toml"
  fi
  # shellcheck disable=SC2086
  run_step $COMPOSE_CMD -f "$ROOT/packaging/compose.yaml" up -d --build
  echo "${C_GREEN}OK:${C_RESET} Container selesai disiapkan (mode=paper)."
  exit 0
fi

if [ "$DO_DEPS" -eq 1 ]; then
  echo "${C_CYAN}==> [1/3] Memasang dependensi sistem ($PKG_MGR)...${C_RESET}"
  DEPS_ARGS=()
  if [ "$PROFILE" = "headless" ]; then DEPS_ARGS+=(--headless); fi
  if [ "$DRY_RUN" -eq 1 ]; then DEPS_ARGS+=(--dry-run); fi
  BOT_PKG_MGR="$PKG_MGR" bash "$ROOT/scripts/deps.sh" "${DEPS_ARGS[@]}"
fi

if [ "$DO_BUILD" -eq 1 ]; then
  echo "${C_CYAN}==> [2/3] Membangun binary & UI...${C_RESET}"
  BUILD_ARGS=()
  if [ "$PROFILE" = "headless" ]; then BUILD_ARGS+=(--headless); fi
  run_step bash "$ROOT/scripts/build.sh" "${BUILD_ARGS[@]}"
fi

if [ "$DO_INSTALL" -eq 1 ]; then
  echo "${C_CYAN}==> [3/3] Menginstal ke sistem (membutuhkan sudo)...${C_RESET}"
  run_step sudo bash "$ROOT/scripts/install-system.sh"
fi

echo ""
echo "${C_GREEN}${C_BOLD}Instalasi terpadu selesai!${C_RESET}"
echo "Mode default adalah ${C_BOLD}paper${C_RESET} dan layanan ${C_BOLD}belum${C_RESET} diaktifkan otomatis."
echo "Langkah berikutnya:"
echo "  1) Validasi config : sudo -u binancebot binance-botctl config validate --config /etc/binance-bot/bot.toml"
if [ "$HAS_SYSTEMD" -eq 1 ]; then
  echo "  2) Jalankan daemon : sudo systemctl enable --now binance-botd"
  echo "  3) Pantau log      : journalctl -u binance-botd -f -o cat"
else
  echo "  2) Jalankan manual : sudo -u binancebot binance-botd --config /etc/binance-bot/bot.toml"
fi
