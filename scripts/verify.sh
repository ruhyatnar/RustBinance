#!/usr/bin/env bash
# Pemeriksaan kualitas lengkap. Bagian yang belum ada (web-ui/desktop-ui) dilewati otomatis.
set -euo pipefail
cd "$(dirname "$0")/.."
echo "== rustfmt";  cargo fmt --all -- --check
echo "== clippy";   cargo clippy --workspace --all-targets -- -D warnings
echo "== test";     cargo test --workspace
if command -v cargo-deny >/dev/null 2>&1; then echo "== deny"; cargo deny check; else echo "!! cargo-deny belum terpasang (cargo install cargo-deny --locked)"; fi
if [ -f web-ui/package.json ]; then echo "== web-ui"; ( cd web-ui && npm ci && npm run check && npm test && npm run build ); fi
if [ -f desktop-ui/CMakeLists.txt ]; then echo "== desktop-ui"; cmake -S desktop-ui -B desktop-ui/build -G Ninja && cmake --build desktop-ui/build && QT_QPA_PLATFORM=offscreen ctest --test-dir desktop-ui/build --output-on-failure; fi
echo "== secret scan (sederhana)"
if git rev-parse --git-dir >/dev/null 2>&1 && git ls-files | grep -Ei '\.(pem|key)$|api_key|\.env$' ; then echo "!! file rahasia ter-track git"; exit 1; fi
echo "OK: semua pemeriksaan lulus"
