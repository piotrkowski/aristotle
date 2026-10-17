#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Start Aristotle at boot instead of the stock GUI, or stop doing so. Run as
# root on the device: KUAL > Aristotle > Start Aristotle at boot: on/off.
#
#   autostart.sh on    copy the app from /mnt/us/aristotle to
#                      /var/local/aristotle/app (USB drive mode cannot take
#                      it away there), reset the failure count and switch
#                      autostart on; takes effect at the next boot
#   autostart.sh off   switch it off and delete the copy
#   autostart.sh status
#
# Needs the agent (Install agent), which installs the boot job,
# aristotle-ui.conf. Neither command writes to the root filesystem.
set -u
umask 077
HERE=$(cd "$(dirname "$0")" && pwd)
EXT=$(dirname "$HERE")
# R stands in for / in the host-side test (device/tests/agent.sh); it
# is empty on the device.
R=${AGENT_TEST_ROOT:-}
S=$R/var/local/aristotle
APP=${ARISTOTLE_APP_DIR:-$R/mnt/us/aristotle}

say() {
    printf '%s\n' "$*"
    eips 0 1 "$*                                        " > /dev/null 2>&1
}

case "${1:-status}" in
on)
    [ -f "$EXT/guard.sh" ] || { say "aristotle: guard.sh missing"; exit 1; }
    . "$EXT/guard.sh"
    device_guard || exit 1
    if [ ! -f "$R/etc/upstart/aristotle-ui.conf" ] || [ ! -f "$S/ui-boot.sh" ]; then
        say "aristotle: install the agent first"
        exit 1
    fi
    if [ ! -x "$APP/aristotle" ] || [ ! -d "$APP/data" ]; then
        say "aristotle: no app in /mnt/us/aristotle"
        exit 1
    fi
    # A fresh copy next to the old one, then swapped in, so a full disk
    # never leaves half an app behind.
    rm -rf "$S/app.new"
    mkdir -p "$S/app.new"
    if ! cp -r "$APP/aristotle" "$APP/data" "$S/app.new/"; then
        rm -rf "$S/app.new"
        say "aristotle: copying the app failed (space on /var/local?)"
        exit 1
    fi
    chmod 700 "$S/app.new/aristotle"
    rm -rf "$S/app"
    mv "$S/app.new" "$S/app"
    echo 0 > "$S/ui-fails"
    : > "$S/autostart"
    sync
    say "Aristotle starts at the next boot"
    ;;
off)
    rm -f "$S/autostart" "$S/ui-fails"
    # The copy goes too, unless Aristotle is running from it (over SSH:
    # `aristotlectl app stop` first, then off again).
    if initctl status aristotle-ui 2> /dev/null | grep -q start/running; then
        say "Off from the next boot; the running copy stays until it stops"
    else
        rm -rf "$S/app" "$S/app.new"
        say "The Kindle starts as usual at the next boot"
    fi
    sync
    ;;
status)
    if [ -e "$S/autostart" ]; then echo "on"; else echo "off"; fi
    ;;
*)
    echo "usage: autostart.sh on|off|status" >&2
    exit 2
    ;;
esac
