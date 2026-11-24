#!/usr/bin/env bash
# Backward-compatible wrapper for scripts/deps.sh.
set -euo pipefail
exec "$(cd "$(dirname "$0")" && pwd)/deps.sh" "$@"
