#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Install the Aristotle agent: three upstart jobs on the root filesystem, the
# rest in /var/local/aristotle. Run as root on the device (from KUAL or SSH).
# Idempotent.
#
#   aristotle-agent.conf  USB network, SSH, watchdog, Wi-Fi policy (boot.sh)
#   aristotle-ota.conf    keeps Amazon's OTA daemons from running (ota-stub)
#   aristotle-ui.conf     Aristotle at boot; idle until autostart.sh on
#
# /var/local/aristotle is made root's alone: owner root, mode 000. The stock
# framework job gives /var/local to its user's group on every start
# (chgrp -R, chmod -R g=u); with no permission bits for the owner, the group
# gets none either, and root does not need them (device/README.md,
# "Security").
#
# Refuses to run, and changes nothing, unless the device is a Kindle
# Paperwhite 4 (2018) on firmware 5.13.x (../guard.sh). ARISTOTLE_FORCE=1
# skips that check, for developers trying another model or firmware on
# purpose, at their own risk.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
EXT=$(dirname "$HERE")
# R stands in for / in the host-side test (device/tests/agent.sh); it
# is empty on the device.
R=${AGENT_TEST_ROOT:-}
S=$R/var/local/aristotle
JOBS="aristotle-agent aristotle-ota aristotle-ui"
KO=$R/mnt/us/koreader
# The manual USB network's dropbear (bin/start.sh).
MANUAL_PID=$R/var/run/aristotle-dropbear.pid

say() {
    printf '%s\n' "$*"
    eips 0 1 "$*                                        " > /dev/null 2>&1
}

[ -f "$EXT/guard.sh" ] || { say "aristotle: guard.sh missing, not installing"; exit 1; }
. "$EXT/guard.sh"
device_guard || exit 1

if [ ! -s "$EXT/authorized_keys" ] && [ ! -s "$S/authorized_keys" ]; then
    say "aristotle: no authorized_keys, not installing"
    exit 1
fi

umask 077
# A link or a file in our place is not ours to fill.
if [ -L "$S" ] || { [ -e "$S" ] && [ ! -d "$S" ]; }; then
    say "aristotle: $S is not a directory; remove it first"
    exit 1
fi
untrusted=0
if [ -d "$S" ]; then
    st=$(stat -c '%u %a' "$S")
    if [ "${st% *}" != "${AGENT_TEST_OWNER:-0}" ] || [ $((0${st#* } & 077)) != 0 ]; then
        untrusted=1
    fi
fi
mkdir -p "$S"
chown 0:0 "$S" 2> /dev/null
# (The host-side test runs as an ordinary user, who needs the owner's bits.)
chmod "${AGENT_TEST_DIR_MODE:-000}" "$S"
# Others could write here until now (earlier versions left the directory to
# the framework's group): keep the settings and the log, as plain files only,
# and replace everything else.
if [ $untrusted = 1 ]; then
    for f in "$S"/* "$S"/.[!.]*; do
        [ -e "$f" ] || [ -L "$f" ] || continue
        case ${f##*/} in
        log | log.1 | net-mode | net-allow | usb-mode)
            if [ -L "$f" ] || [ ! -f "$f" ]; then rm -rf "$f"; fi
            ;;
        *) rm -rf "$f" ;;
        esac
    done
fi
cp "$HERE/boot.sh" "$HERE/aristotlectl" "$HERE/net.sh" "$HERE/ui-boot.sh" "$S/"
cp "$EXT/takeover.sh" "$EXT/guard.sh" "$S/"
chmod 700 "$S/boot.sh" "$S/aristotlectl" "$S/net.sh" "$S/ui-boot.sh" "$S/takeover.sh"
# The stub runs as Amazon's daemon through a read-only bind mount; no write
# bits, so the framework's chmod g=u cannot add any.
cp "$HERE/ota-stub" "$S/ota-stub"
chmod 555 "$S/ota-stub"
if [ -s "$EXT/authorized_keys" ]; then
    cp "$EXT/authorized_keys" "$S/authorized_keys"
fi
if [ ! -s "$S/authorized_keys" ]; then
    say "aristotle: no authorized_keys, not installing"
    exit 1
fi
# SSH runs from the agent's own copy of KOReader's dropbear, with its own
# settings directory; KOReader's host key comes along so known_hosts stays
# valid.
if [ -x "$KO/dropbear" ]; then
    cp "$KO/dropbear" "$S/dropbear.new" && chmod 700 "$S/dropbear.new" && mv -f "$S/dropbear.new" "$S/dropbear"
    mkdir -p "$S/ssh/settings/SSH"
    for k in "$KO"/settings/SSH/dropbear_*_host_key; do
        [ -f "$k" ] && [ ! -f "$S/ssh/settings/SSH/${k##*/}" ] && cp "$k" "$S/ssh/settings/SSH/"
    done
fi
# Older versions of the agent left these behind.
rm -f "$S/takeover"

# The only root-filesystem write: the upstart jobs, in one mntroot window,
# skipped when they are unchanged. mntroot is the stock tool for a temporary
# read-write root.
changed=
for j in $JOBS; do
    cmp -s "$HERE/$j.conf" "$R/etc/upstart/$j.conf" 2> /dev/null || changed="$changed $j"
done
if [ -n "$changed" ]; then
    mntroot rw || { say "aristotle: mntroot rw failed"; exit 1; }
    for j in $changed; do
        cp "$HERE/$j.conf" "$R/etc/upstart/$j.conf"
        chmod 644 "$R/etc/upstart/$j.conf"
    done
    sync
    mntroot ro
fi
initctl reload-configuration 2> /dev/null

# Block the OTA daemons now: restart those that run, so the new job puts
# the stub in their place.
for j in otaupd otav3; do
    if initctl status "$j" 2> /dev/null | grep -q start/running; then
        stop "$j" > /dev/null 2>&1
        start "$j" > /dev/null 2>&1
    fi
done

# Hand the USB port over from a manual "Start" session, or from an earlier
# version of the agent, which kept its pid file there too. Open sessions
# stay up; only the listener goes.
[ -f "$MANUAL_PID" ] && kill "$(cat "$MANUAL_PID")" 2> /dev/null
rm -f "$MANUAL_PID"
stop aristotle-agent > /dev/null 2>&1
start aristotle-agent && say "aristotle agent installed and running"
