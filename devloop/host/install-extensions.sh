#!/bin/sh
# Copy aristotle's KUAL extensions (and the reader app, if built) to a Kindle
# mounted over USB mass storage, then eject it so the device leaves drive mode
# while the cable stays plugged in. After installing the agent from KUAL once,
# everything else goes over SSH.
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
KINDLE=${KINDLE_MOUNT:-/run/media/$USER/Kindle}
# Dedicated dev-loop key first; the owner's main key as a fallback.
KEYS="${ARISTOTLE_PUBKEY:-$HOME/.cache/aristotle/ssh/kindle_ed25519.pub} $HOME/.ssh/id_ed25519.pub"
APP_BUNDLE=${APP_BUNDLE:-$HOME/.cache/aristotle/reader-work/aristotle-app-device.tar.gz}

[ -d "$KINDLE/extensions" ] || { echo "Kindle not mounted at $KINDLE" >&2; exit 1; }
dev=$(findmnt -no SOURCE "$KINDLE")
if findmnt -no OPTIONS "$KINDLE" | grep -q '^ro'; then
    udisksctl unmount -b "$dev" > /dev/null
    udisksctl mount -b "$dev" > /dev/null
fi

cp -r "$REPO/probe/device/aristotle-probe" "$KINDLE/extensions/"
cp -r "$REPO/devloop/kindle/aristotle-usbnet" "$KINDLE/extensions/"
cp "$REPO/devloop/kindle/takeover.sh" "$KINDLE/extensions/aristotle-usbnet/"
: > "$KINDLE/extensions/aristotle-usbnet/authorized_keys"
for key in $KEYS; do
    [ -s "$key" ] && cat "$key" >> "$KINDLE/extensions/aristotle-usbnet/authorized_keys"
done
if [ -s "$APP_BUNDLE" ]; then
    tar -xzf "$APP_BUNDLE" -C "$KINDLE"
    echo "Unpacked the reader app to $KINDLE/aristotle"
fi
sync

# Eject (not power off): the device leaves drive mode, the cable stays usable.
udisksctl unmount -b "$dev" > /dev/null
drive=$(udisksctl info -b "$dev" | awk '/^ +Drive:/ {print $2}' | tr -d "'")
gdbus call --system --dest org.freedesktop.UDisks2 --object-path "$drive" \
    --method org.freedesktop.UDisks2.Drive.Eject '{}' > /dev/null
echo "Installed and ejected. On the Kindle: KUAL > Aristotle > Install agent (SSH on every boot)."
