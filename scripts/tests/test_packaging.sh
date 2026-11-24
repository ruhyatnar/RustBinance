#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PASS=0
FAIL=0

assert_ok() {
  local name="$1"; shift
  if "$@"; then
    echo "PASS: $name"
    PASS=$((PASS + 1))
  else
    echo "FAIL: $name" >&2
    FAIL=$((FAIL + 1))
  fi
}

assert_fail() {
  local name="$1"; shift
  if "$@" >/dev/null 2>&1; then
    echo "FAIL: $name (expected failure, got exit 0)" >&2
    FAIL=$((FAIL + 1))
  else
    echo "PASS: $name"
    PASS=$((PASS + 1))
  fi
}

test_deps_syntax() {
  bash -n "$ROOT/scripts/deps.sh"
  bash -n "$ROOT/scripts/deps-arch.sh"
}

test_deps_dry_run_all_managers() {
  local out
  for mgr in pacman apt dnf zypper; do
    out="$(BOT_PKG_MGR="$mgr" bash "$ROOT/scripts/deps.sh" --dry-run)"
    case "$mgr" in
      pacman) grep -q "pacman -S" <<<"$out" && grep -q "qt6-base" <<<"$out" ;;
      apt)    grep -q "apt-get install" <<<"$out" && grep -q "qt6-base-dev" <<<"$out" ;;
      dnf)    grep -q "dnf install" <<<"$out" && grep -q "qt6-qtbase-devel" <<<"$out" ;;
      zypper) grep -q "zypper install" <<<"$out" && grep -q "qt6-base-devel" <<<"$out" ;;
    esac
  done
}

test_deps_headless_skips_qt() {
  local out
  for mgr in pacman apt dnf zypper; do
    out="$(BOT_PKG_MGR="$mgr" bash "$ROOT/scripts/deps.sh" --headless --dry-run)"
    if grep -qiE "(qt6|kirigami)" <<<"$out"; then
      echo "Headless output for $mgr unexpectedly included Qt/Kirigami: $out" >&2
      return 1
    fi
  done
}

test_deps_unsupported_manager() {
  local err
  err="$(BOT_PKG_MGR=none bash "$ROOT/scripts/deps.sh" --dry-run 2>&1 || true)"
  grep -qi "unsupported" <<<"$err"
}

test_install_syntax() {
  bash -n "$ROOT/scripts/install-system.sh"
}

test_install_no_hardcoded_nologin() {
  ! grep -q -- "--shell /usr/bin/nologin" "$ROOT/scripts/install-system.sh"
  grep -q 'NOLOGIN_SHELL' "$ROOT/scripts/install-system.sh"
}

test_install_preflight_binaries_check() {
  grep -q 'target/release/binance-botd' "$ROOT/scripts/install-system.sh"
  grep -q 'scripts/build.sh' "$ROOT/scripts/install-system.sh"
}

test_dockerfile_security_invariants() {
  local df="$ROOT/packaging/Dockerfile"
  [ -f "$df" ]
  grep -q '^USER binancebot' "$df"
  ! grep -qiE 'COPY.*(\.pem|api_key|\.env)' "$df"
  grep -q 'ENTRYPOINT \["/usr/local/bin/binance-botd", "--config", "/etc/binance-bot/bot.toml"\]' "$df"
}

test_compose_invariants() {
  local cf="$ROOT/packaging/compose.yaml"
  [ -f "$cf" ]
  grep -q '/etc/binance-bot:ro' "$cf"
  grep -q '/var/lib/binance-bot' "$cf"
  ! grep -qi 'mode=live' "$cf"
}

assert_ok "test_deps_syntax" test_deps_syntax
assert_ok "test_deps_dry_run_all_managers" test_deps_dry_run_all_managers
assert_ok "test_deps_headless_skips_qt" test_deps_headless_skips_qt
assert_fail "test_deps_unsupported_manager_exits_nonzero" env BOT_PKG_MGR=none bash "$ROOT/scripts/deps.sh" --dry-run
assert_ok "test_deps_unsupported_manager_message" test_deps_unsupported_manager
assert_ok "test_install_syntax" test_install_syntax
assert_ok "test_install_no_hardcoded_nologin" test_install_no_hardcoded_nologin
assert_ok "test_install_preflight_binaries_check" test_install_preflight_binaries_check
assert_ok "test_dockerfile_security_invariants" test_dockerfile_security_invariants
assert_ok "test_compose_invariants" test_compose_invariants

echo "Summary: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
