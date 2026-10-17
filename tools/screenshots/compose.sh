#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Re-render every framed screenshot listed in manifest.toml.
#   tools/screenshots/compose.sh [manifest.toml]
# Set SCREENSHOTS_BASE to read captures from another directory.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
. "$here/../env.sh"
export UV_CACHE_DIR="$ARISTOTLE_CACHE/uv-cache"
export UV_PROJECT_ENVIRONMENT="$ARISTOTLE_CACHE/screenshots-venv"
mkdir -p "$UV_CACHE_DIR"

exec uv run --locked --project "$here" python -I "$here/compose.py" "${1:-$here/manifest.toml}"
