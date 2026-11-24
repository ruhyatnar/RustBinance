# Universal Linux Packaging Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Menjadikan instalasi dependensi, sistem, dan operasional bot universal di seluruh distro Linux utama (Desktop, Headless VPS, dan Container/non-`systemd`).

**Architecture:** Memisahkan deteksi package manager ke `scripts/deps.sh` (dengan `--headless` dan `--dry-run` agar mudah diuji tanpa menyentuh host), menambah pre-flight check & deteksi `nologin` di `scripts/install-system.sh`, serta menyediakan `packaging/Dockerfile` + `packaging/compose.yaml` multi-stage non-root untuk server/non-`systemd`.

**Tech Stack:** Bash (`set -euo pipefail`), Docker/OCI multi-stage build (Rust + Node.js + Debian slim), Docker Compose, Markdown.

**Spec:** `docs/superpowers/specs/2026-11-24-universal-linux-packaging-design.md`

## Global Constraints
- Default mode selalu `paper`; tidak ada `mode=live` atau rahasia (`*.pem`, `api_key*`, `.env`) di dalam image container maupun repo.
- `scripts/install-system.sh` tidak boleh auto-enable atau auto-start layanan `systemd`.
- Semua skrip shell wajib lulus `bash -n` (dan `shellcheck` bila tersedia).
- Tidak ada perubahan pada kode inti Rust (`crates/`) atau tes Qt C++ (`desktop-ui/tests/`).
- Pengujian tidak boleh menjalankan `sudo` atau memodifikasi direktori sistem host (`/etc`, `/var`, `/usr`).

## Review Focus
- Distro tidak dikenal (tidak ada `pacman`/`apt-get`/`dnf`/`zypper`) → `scripts/deps.sh` harus keluar dengan exit code non-zero dan pesan error jelas (tidak crash diam-diam).
- Flag `--headless` pada `scripts/deps.sh` → tidak boleh menyertakan paket Qt6/Kirigami di perintah instalasi.
- Binary release belum dibuild saat menjalankan `scripts/install-system.sh` → pre-flight check harus gagal lebih awal dengan pesan agar menjalankan `./scripts/build.sh` dulu (sebelum membuat user/direktori).
- Path `nologin` berbeda antar distro (`/usr/sbin/nologin` vs `/usr/bin/nologin`) → `install-system.sh` harus memilih path yang benar secara dinamis.
- `packaging/Dockerfile` → wajib berjalan sebagai user non-root (`USER binancebot`) dan tidak menyalin file kunci/rahasia ke dalam image.

---

### Task 1: Universal Dependency Installer (`scripts/deps.sh` & wrapper `scripts/deps-arch.sh`)

**Files:**
- Create: `scripts/deps.sh`
- Modify: `scripts/deps-arch.sh`
- Create: `scripts/tests/test_packaging.sh`

**Interfaces:**
- Produces:
  - `scripts/deps.sh [--headless] [--dry-run]` (juga mendukung env `BOT_PKG_MGR=pacman|apt|dnf|zypper|none` untuk pengujian terisolasi).
  - `scripts/deps-arch.sh` meneruskan argumen ke `scripts/deps.sh "$@"`.
  - `scripts/tests/test_packaging.sh` (skrip uji mandiri, exit 0 bila semua lulus).

- [ ] **Step 1: Write the failing tests in `scripts/tests/test_packaging.sh`**

Buat `scripts/tests/test_packaging.sh` (`chmod +x`) dengan kasus uji berikut:
- `test_deps_syntax`: `bash -n scripts/deps.sh` dan `bash -n scripts/deps-arch.sh`.
- `test_deps_dry_run_all_managers`: jalankan `BOT_PKG_MGR=<mgr> scripts/deps.sh --dry-run` untuk `pacman`, `apt`, `dnf`, `zypper`; pastikan exit 0 dan output memuat perintah package manager terkait + paket Qt6.
- `test_deps_headless_skips_qt`: jalankan `BOT_PKG_MGR=apt scripts/deps.sh --headless --dry-run` dan `BOT_PKG_MGR=pacman scripts/deps.sh --headless --dry-run`; pastikan exit 0 dan output **tidak** memuat `qt6` maupun `kirigami`.
- `test_deps_unsupported_manager`: jalankan `BOT_PKG_MGR=none scripts/deps.sh --dry-run`; pastikan exit code != 0 dan stderr memuat pesan `Unsupported`.

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/tests/test_packaging.sh`
Expected: FAIL karena `scripts/deps.sh` belum ada.

- [ ] **Step 3: Implement `scripts/deps.sh` and update `scripts/deps-arch.sh`**

- Di `scripts/deps.sh`:
  - Parse flag `--headless`, `--dry-run`, `--help`.
  - Deteksi `PKG_MGR` dari `BOT_PKG_MGR` (jika diset) atau `command -v pacman / apt-get / dnf / zypper`. Jika tidak ditemukan, cetak error ke stderr dan `exit 1`.
  - Definisikan daftar paket base (build tools, git, cmake, ninja, pkg-config, openssl dev, rust/cargo, nodejs, npm, chrony) dan paket desktop (Qt6 + KDE Kirigami/Frameworks 6) untuk masing-masing `pacman`, `apt`, `dnf`, `zypper`.
  - Jika `--headless`, pasang hanya paket base.
  - Jika `--dry-run`, cetak perintah yang akan dijalankan tanpa mengeksekusi `sudo`.
  - Aktifkan NTP (`chronyd` di Arch/Fedora/openSUSE, `chrony` di Debian/Ubuntu) hanya jika `systemctl` tersedia dan `/run/systemd/system` ada (skip saat `--dry-run`).
- Di `scripts/deps-arch.sh`: ganti isi menjadi wrapper tipis `exec "$(dirname "$0")/deps.sh" "$@"`.

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/tests/test_packaging.sh`
Expected: PASS (semua tes Task 1 hijau).

- [ ] **Step 5: Commit**

```bash
git add scripts/deps.sh scripts/deps-arch.sh scripts/tests/test_packaging.sh
git commit -m "feat(packaging): add multi-distro deps.sh with headless mode"
```

---

### Task 2: Harden `scripts/install-system.sh` (Pre-flight, Dynamic `nologin`, Non-Systemd Fallback)

**Files:**
- Modify: `scripts/install-system.sh`
- Modify: `scripts/tests/test_packaging.sh`

**Interfaces:**
- Consumes: `scripts/tests/test_packaging.sh` dari Task 1.
- Produces: `scripts/install-system.sh` dengan fungsi helper internal yang bisa dicek pre-flight & sintaksnya.

- [ ] **Step 1: Add failing tests to `scripts/tests/test_packaging.sh`**

Tambahkan ke `scripts/tests/test_packaging.sh`:
- `test_install_syntax`: `bash -n scripts/install-system.sh`.
- `test_install_no_hardcoded_nologin`: pastikan `scripts/install-system.sh` tidak lagi men-hardcode `--shell /usr/bin/nologin` secara statis, melainkan memakai variabel hasil deteksi (`NOLOGIN_SHELL`).
- `test_install_preflight_binaries_check`: pastikan `scripts/install-system.sh` memeriksa keberadaan `$ROOT/target/release/binance-botd` dan `binance-botctl` sebelum `useradd`.

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/tests/test_packaging.sh`
Expected: FAIL pada `test_install_no_hardcoded_nologin` dan `test_install_preflight_binaries_check`.

- [ ] **Step 3: Update `scripts/install-system.sh`**

- Tambahkan pre-flight check tepat setelah inisialisasi `ROOT`: jika `$ROOT/target/release/binance-botd` atau `$ROOT/target/release/binance-botctl` tidak executable, cetak pesan jelas (`Jalankan ./scripts/build.sh terlebih dahulu`) ke stderr dan `exit 1`.
- Deteksi `NOLOGIN_SHELL`: `command -v nologin` → `/usr/sbin/nologin` → `/usr/bin/nologin` → `/bin/false`, lalu pakai `--shell "$NOLOGIN_SHELL"` di `useradd`.
- Pada bagian `systemctl daemon-reload`: jika `systemd` tidak aktif, cetak catatan bahwa unit systemd tetap dipasang di `$SYSTEMD_DIR`, tetapi di lingkungan non-`systemd` daemon dapat dijalankan langsung via `sudo -u "$SVC" "$BIN_DIR/binance-botd" --config "$CONF/bot.toml"`.

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/tests/test_packaging.sh`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add scripts/install-system.sh scripts/tests/test_packaging.sh
git commit -m "fix(packaging): add pre-flight checks and dynamic nologin in install-system.sh"
```

---

### Task 3: Headless Container Packaging (`packaging/Dockerfile` & `packaging/compose.yaml`)

**Files:**
- Create: `packaging/Dockerfile`
- Create: `packaging/compose.yaml`
- Modify: `scripts/tests/test_packaging.sh`

**Interfaces:**
- Produces:
  - `packaging/Dockerfile` (multi-stage: `rust-builder`, `web-builder`, `runtime`).
  - `packaging/compose.yaml` (service `binance-botd`, volume config/data, restart policy `unless-stopped`, read-only config mount).

- [ ] **Step 1: Add failing tests to `scripts/tests/test_packaging.sh`**

Tambahkan ke `scripts/tests/test_packaging.sh`:
- `test_dockerfile_security_invariants`: pastikan `packaging/Dockerfile` ada, berisi `USER binancebot`, tidak menyalin `*.pem` / `api_key*` / `.env`, dan memakai `ENTRYPOINT ["/usr/local/bin/binance-botd", "--config", "/etc/binance-bot/bot.toml"]`.
- `test_compose_invariants`: pastikan `packaging/compose.yaml` ada, me-mount `/etc/binance-bot` sebagai `:ro` dan `/var/lib/binance-bot`, serta tidak men-set `mode=live`.

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/tests/test_packaging.sh`
Expected: FAIL karena `packaging/Dockerfile` dan `packaging/compose.yaml` belum ada.

- [ ] **Step 3: Create `packaging/Dockerfile` and `packaging/compose.yaml`**

- `packaging/Dockerfile`:
  - Stage `rust-builder` (`rust:1-bookworm`): build `--release --bin binance-botd --bin binance-botctl`.
  - Stage `web-builder` (`node:22-bookworm-slim`): `npm ci && npm run build` di `web-ui/`.
  - Stage `runtime` (`debian:bookworm-slim`): pasang `ca-certificates`, buat user sistem `binancebot`, salin binary ke `/usr/local/bin/` dan aset web ke `/usr/share/binance-bot/web`, buat direktori `/etc/binance-bot` & `/var/lib/binance-bot`, set `USER binancebot`, `ENTRYPOINT ["/usr/local/bin/binance-botd", "--config", "/etc/binance-bot/bot.toml"]`.
- `packaging/compose.yaml`:
  - Build dari `context: ..`, `dockerfile: packaging/Dockerfile`.
  - `restart: unless-stopped`, `read_only: true`, `tmpfs: [/tmp, /run/binance-bot]`, volumes `./config:/etc/binance-bot:ro` dan `bot_data:/var/lib/binance-bot`.

- [ ] **Step 4: Run test to verify it passes**

Run: `bash scripts/tests/test_packaging.sh`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packaging/Dockerfile packaging/compose.yaml scripts/tests/test_packaging.sh
git commit -m "feat(packaging): add multi-stage headless Dockerfile and compose.yaml"
```

---

### Task 4: Update Documentation (`README.md`, `CHANGELOG.md`) & Final Verification

**Files:**
- Modify: `README.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Update `README.md`**

Perbarui bagian instalasi agar mencakup 3 jalur yang jelas:
1. **Linux Desktop (Arch/CachyOS, Debian/Ubuntu, Fedora, openSUSE):** `./scripts/deps.sh` → `./scripts/build.sh` → `sudo ./scripts/install-system.sh`.
2. **Linux Server / Headless VPS:** `./scripts/deps.sh --headless` → `./scripts/build.sh` → `sudo ./scripts/install-system.sh`.
3. **Container (Docker / Podman — termasuk distro non-`systemd`):** `docker compose -f packaging/compose.yaml up -d --build`.

- [ ] **Step 2: Update `CHANGELOG.md`**

Tambahkan entri di bagian `[Unreleased]` yang mencatat penambahan dukungan multi-distro Linux (`scripts/deps.sh`, perbaikan `scripts/install-system.sh`, dan `packaging/Dockerfile` + `compose.yaml`).

- [ ] **Step 3: Run full verification**

Run: `bash scripts/tests/test_packaging.sh`
Run: `git status --short`
Expected: Semua tes lulus; tidak ada file sementara/rahasia yang ikut ter-stage.

- [ ] **Step 4: Commit**

```bash
git add README.md CHANGELOG.md
git commit -m "docs: document universal Linux desktop, headless, and container setup"
```
