#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copy the dev-loop KUAL extensions (and the reader app, if built) to a Kindle
# mounted over USB mass storage, then eject it so the device leaves drive mode
# while the cable stays plugged in. After installing the agent from KUAL once,
# everything else goes over SSH.
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
. "$REPO/tools/env.sh"
KINDLE=${KINDLE_MOUNT:-/run/media/$USER/Kindle}
# The device accepts exactly one key: the dedicated dev-loop key that
# tools/host/ssh_config uses. Create it once with
#   mkdir -p "$ARISTOTLE_CACHE/ssh" && ssh-keygen -t ed25519 -f "$ARISTOTLE_CACHE/ssh/kindle_ed25519"
# or point ARISTOTLE_PUBKEY at another public key.
PUBKEY=${ARISTOTLE_PUBKEY:-$ARISTOTLE_CACHE/ssh/kindle_ed25519.pub}
# The reader app comes from APP_BUNDLE (default: what tools/build-render.sh
# device writes); see tools/package.sh.

[ -s "$PUBKEY" ] || {
    echo "no public key at $PUBKEY" >&2
    echo "create one: ssh-keygen -t ed25519 -f ${PUBKEY%.pub}, or set ARISTOTLE_PUBKEY" >&2
    exit 1
}

[ -d "$KINDLE/extensions" ] || { echo "Kindle not mounted at $KINDLE" >&2; exit 1; }
dev=$(findmnt -no SOURCE "$KINDLE")
if findmnt -no OPTIONS "$KINDLE" | grep -q '^ro'; then
    udisksctl unmount -b "$dev" > /dev/null
    udisksctl mount -b "$dev" > /dev/null
fi

# The app and the extension as in the release zip (tools/package.sh), plus
# the read-only probe and the dev-loop key.
STAGE=$(mktemp -d "$TMPDIR/install.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
"$REPO/tools/package.sh" tree "$STAGE/root"
mkdir -p "$STAGE/root/extensions/aristotle-probe"
cp -r "$REPO/device/probe/." "$STAGE/root/extensions/aristotle-probe/"
cp "$PUBKEY" "$STAGE/root/extensions/aristotle/authorized_keys"
# The extension's folder before it was renamed: KUAL would show both menus,
# and it may hold an older copy of the key.
rm -rf "$KINDLE/extensions/aristotle-usbnet"
cp -r "$STAGE/root/." "$KINDLE/"
sync

# Eject (not power off): the device leaves drive mode, the cable stays usable.
udisksctl unmount -b "$dev" > /dev/null
drive=$(udisksctl info -b "$dev" | awk '/^ +Drive:/ {print $2}' | tr -d "'")
gdbus call --system --dest org.freedesktop.UDisks2 --object-path "$drive" \
    --method org.freedesktop.UDisks2.Drive.Eject '{}' > /dev/null
echo "Installed and ejected. On the Kindle: KUAL > Aristotle > Install agent (SSH on every boot)."
