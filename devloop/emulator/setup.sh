#!/bin/sh
# Set up katadelos/eink-emulator with a Kindle Paperwhite 4 instance running
# stock firmware 5.13.6.
#
#   ARISTOTLE_EMU     where the emulator goes (default $ARISTOTLE_CACHE/emulator)
#   ARISTOTLE_PW4_FW  an unpacked 5.13.6 update package, the directory holding
#                     imx6sll_rex/ and rootfs.img (default
#                     $ARISTOTLE_CACHE/firmware/pw4-5.13.6); see
#                   docs/pw4-reference.md for how it is unpacked
#   KINDLETOOL_DIR    directory with the kindletool binary, if it is not on PATH
set -eu

. "$(cd "$(dirname "$0")/../.." && pwd)/devloop/env.sh"
EMU=${ARISTOTLE_EMU:-$ARISTOTLE_CACHE/emulator}
FW=${ARISTOTLE_PW4_FW:-$ARISTOTLE_CACHE/firmware/pw4-5.13.6}
INSTANCE=${1:-pw4}
PATCH=$(cd "$(dirname "$0")" && pwd)/eink-emulator-pw4.patch
[ -z "${KINDLETOOL_DIR:-}" ] || export PATH="$KINDLETOOL_DIR:$PATH"
[ -f "$FW/rootfs.img" ] || {
    echo "no unpacked PW4 firmware in $FW (set ARISTOTLE_PW4_FW)" >&2
    exit 1
}
mkdir -p "$EMU" "$TMPDIR"

if [ ! -d "$EMU/eink-emulator" ]; then
    git clone --depth 1 https://github.com/katadelos/eink-emulator "$EMU/eink-emulator"
fi
cd "$EMU/eink-emulator"

# The submodule URL is SSH; fetch the pinned QEMU fork commit over HTTPS instead.
git submodule init qemu > /dev/null
git config submodule.qemu.url https://github.com/katadelos/qemu.git
git submodule update --init --depth 1 qemu

# PW4 has no recovery-package import; place the five files directly.
dest=firmware/kindle-paperwhite-4
mkdir -p "$dest"
for f in imx6sll_rex/u-boot.bin imx6sll_rex/boot.img imx6sll_rex/s-bios.bin imx6sll_rex/bios.bin rootfs.img; do
    [ -f "$dest/$(basename "$f")" ] || cp "$FW/$f" "$dest/"
done

# Without these fixes PW4 never reaches Home: the default board ID boots it as
# platform=yoshi, and kb crash loops reboot the guest. See the patch header.
git apply --reverse --check "$PATCH" 2> /dev/null || git apply "$PATCH"

# Builds QEMU on first use, then prepares a disk with KUAL, MRPI and the jailbreak.
# Instances created before the patch keep their old disk; recreate them with
# ./eink delete NAME, which needs lsof.
./eink list | grep -q "^$INSTANCE " ||
    ./eink create "$INSTANCE" --model kindle-paperwhite-4 --profile production --mrpi

# Home appears about 2.5 minutes after start; SSH is up after about 30 s.
# run.sh needs --qmp-socket for its screenshots; drop --headless to see the screen.
echo "Run with: cd $EMU/eink-emulator && ./eink run $INSTANCE --headless --qmp-socket --serial-socket"
