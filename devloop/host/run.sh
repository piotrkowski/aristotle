#!/bin/sh
# Build a crate for the Kindle, copy it over the USB network, run it, and save
# a PNG of the e-ink framebuffer.
#   devloop/host/run.sh [PACKAGE[:BINARY]] [ARGS...]     (default: aristotle-hello)
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
SPEC=${1:-aristotle-hello}
[ $# -gt 0 ] && shift
PKG=${SPEC%%:*}
NAME=${SPEC#*:}
TARGET=armv7-unknown-linux-musleabihf
BIN="$HOME/.cache/aristotle/target/$TARGET/release/$NAME"
OUT="$HOME/.cache/aristotle/screens"
CFG="$REPO/devloop/host/ssh_config"
PY="$HOME/.cache/aristotle/venv/bin/python"

export TMPDIR="$HOME/.cache/aristotle/tmp"
export CARGO_TARGET_DIR="$HOME/.cache/aristotle/target"
(cd "$REPO" && cargo build --release --target "$TARGET" -p "$PKG" --bin "$NAME")

# The rootfs is read-only; /var/tmp is writable and wiped on reboot.
scp -q -F "$CFG" "$BIN" "kindle:/var/tmp/$NAME"
ssh -F "$CFG" kindle "chmod 755 /var/tmp/$NAME && /var/tmp/$NAME $*"

mkdir -p "$OUT"
stamp=$(date -u '+%Y%m%dT%H%M%SZ')
raw="$OUT/$NAME-$stamp.raw"
# Visible area only; stride is 1088 bytes per row.
ssh -F "$CFG" kindle "dd if=/dev/fb0 bs=1088 count=1448 2>/dev/null" | dd of="$raw" status=none
"$PY" -I "$REPO/devloop/host/fb2png.py" "$raw" "$OUT/$NAME-$stamp.png"
echo "screenshot: $OUT/$NAME-$stamp.png"
