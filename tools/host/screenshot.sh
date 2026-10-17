#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Save a PNG of the Kindle's framebuffer over the USB network.
#   tools/host/screenshot.sh [NAME]     -> $ARISTOTLE_CACHE/screens/NAME-<stamp>.png
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
. "$REPO/tools/env.sh"
NAME=${1:-screen}
OUT="$ARISTOTLE_CACHE/screens"
CFG="$REPO/tools/host/ssh_config"
PY="$ARISTOTLE_CACHE/venv/bin/python"

mkdir -p "$OUT"
stamp=$(date -u '+%Y%m%dT%H%M%SZ')
raw="$OUT/$NAME-$stamp.raw"
# Visible area only; stride is 1088 bytes per row.
ssh -F "$CFG" kindle "dd if=/dev/fb0 bs=1088 count=1448 2>/dev/null" | dd of="$raw" status=none
"$PY" -I "$REPO/tools/host/fb2png.py" "$raw" "$OUT/$NAME-$stamp.png"
echo "$OUT/$NAME-$stamp.png"
