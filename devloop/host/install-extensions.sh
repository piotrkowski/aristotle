#!/bin/sh
# Copy aristotle's KUAL extensions to a Kindle mounted over USB mass storage.
# This is the last step that needs drive mode; after it, use SSH over USB.
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
KINDLE=${KINDLE_MOUNT:-/run/media/$USER/Kindle}
PUBKEY=${ARISTOTLE_PUBKEY:-$HOME/.cache/aristotle/ssh/kindle_ed25519.pub}

[ -d "$KINDLE/extensions" ] || { echo "Kindle not mounted at $KINDLE" >&2; exit 1; }
if findmnt -no OPTIONS "$KINDLE" | grep -q '^ro'; then
    dev=$(findmnt -no SOURCE "$KINDLE")
    udisksctl unmount -b "$dev" > /dev/null
    udisksctl mount -b "$dev" > /dev/null
fi

cp -r "$REPO/probe/device/aristotle-probe" "$KINDLE/extensions/"
cp -r "$REPO/devloop/kindle/aristotle-usbnet" "$KINDLE/extensions/"
cp "$PUBKEY" "$KINDLE/extensions/aristotle-usbnet/authorized_keys"
sync
echo "Installed aristotle-probe and aristotle-usbnet. Eject, then run KUAL > Aristotle USB network > Start."
