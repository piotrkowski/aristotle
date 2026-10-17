#!/bin/sh
# Build a crate for the Kindle, copy it into the running PW4 emulator, run it
# over SSH and save a screenshot of the e-ink scanout.
#   devloop/emulator/run.sh [PACKAGE] [ARGS...]     (default: aristotle-hello)
# Start the emulator first with the run command that setup.sh prints; the
# screenshot needs its --qmp-socket.
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
EMU=${ARISTOTLE_EMU:-$HOME/.cache/aristotle/emulator}/eink-emulator
MACHINE=${ARISTOTLE_EMU_MACHINE:-pw4}
PKG=${1:-aristotle-hello}
[ $# -gt 0 ] && shift
TARGET=armv7-unknown-linux-musleabihf
BIN="$HOME/.cache/aristotle/target/$TARGET/release/$PKG"
OUT="$HOME/.cache/aristotle/emulator/screens"
SSH="ssh -q -i $EMU/build/ssh/id_ed25519 -p 2222 -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=$HOME/.cache/aristotle/emu_known_hosts root@127.0.0.1"

export TMPDIR="$HOME/.cache/aristotle/tmp"
export CARGO_TARGET_DIR="$HOME/.cache/aristotle/target"
(cd "$REPO" && cargo build --release --target "$TARGET" -p "$PKG")

# The rootfs is read-only; /var/tmp is writable on the stock system.
$SSH "cat > /var/tmp/$PKG && chmod 755 /var/tmp/$PKG" < "$BIN"
$SSH "/var/tmp/$PKG $*"

mkdir -p "$OUT"
shot="$OUT/$PKG-$(date -u '+%Y%m%dT%H%M%SZ').png"
python3 -I "$EMU/scripts/eink-qmp.py" --machine "$MACHINE" screendump "$shot" > /dev/null
echo "screenshot: $shot"
