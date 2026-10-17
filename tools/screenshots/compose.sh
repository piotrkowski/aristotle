#!/bin/sh
# Re-render every framed screenshot listed in manifest.toml.
#   tools/screenshots/compose.sh [manifest.toml]
# Set SCREENSHOTS_BASE to read captures from another directory.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
cache="$HOME/.cache/aristotle"
export TMPDIR="$cache/tmp"
export UV_CACHE_DIR="$cache/uv-cache"
export UV_PROJECT_ENVIRONMENT="$cache/screenshots-venv"
mkdir -p "$TMPDIR" "$UV_CACHE_DIR"

exec uv run --locked --project "$here" python -I "$here/compose.py" "${1:-$here/manifest.toml}"
