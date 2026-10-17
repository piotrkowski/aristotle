#!/bin/sh
# Install the Aristotle agent: one upstart job on the root filesystem, the rest
# in /var/local/aristotle. Run as root on the device (from KUAL or SSH).
# Idempotent.
#
# Refuses to run, and changes nothing, unless the device is a Kindle
# Paperwhite 4 (2018) on firmware 5.13.x (../guard.sh). ARISTOTLE_FORCE=1
# skips that check, for developers trying another model or firmware on
# purpose, at their own risk.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
EXT=$(dirname "$HERE")
# R stands in for / in the host-side test (devloop/kindle/tests/agent.sh); it
# is empty on the device.
R=${AGENT_TEST_ROOT:-}
S=$R/var/local/aristotle
JOB=$R/etc/upstart/aristotle-agent.conf
PIDFILE=$R/var/run/aristotle-dropbear.pid

say() {
    printf '%s\n' "$*"
    eips 0 1 "$*                                        " > /dev/null 2>&1
}

[ -f "$EXT/guard.sh" ] || { say "aristotle: guard.sh missing, not installing"; exit 1; }
. "$EXT/guard.sh"
device_guard || exit 1

# Two agents would fight over the USB port. One installed before the project's
# rename is moved over by migrate.sh, which can roll back.
if [ -e "$R/etc/upstart/aristotle.conf" ]; then
    say "aristotle: earlier agent found; use Migrate agent instead"
    exit 1
fi
if [ ! -s "$EXT/authorized_keys" ] && [ ! -s "$S/authorized_keys" ]; then
    say "aristotle: no authorized_keys, not installing"
    exit 1
fi

mkdir -p "$S"
cp "$HERE/boot.sh" "$HERE/aristotlectl" "$HERE/net.sh" "$S/"
chmod 755 "$S/boot.sh" "$S/aristotlectl" "$S/net.sh"
if [ -s "$EXT/authorized_keys" ]; then
    cp "$EXT/authorized_keys" "$S/authorized_keys"
fi

# The single root-filesystem write: the upstart job. mntroot is the stock tool
# for a temporary read-write root.
if ! cmp -s "$HERE/aristotle-agent.conf" "$JOB" 2> /dev/null; then
    mntroot rw || { say "aristotle: mntroot rw failed"; exit 1; }
    cp "$HERE/aristotle-agent.conf" "$JOB"
    sync
    mntroot ro
fi
initctl reload-configuration 2> /dev/null
# Hand the USB port over from a manual "Start" session, if one is running.
[ -f "$PIDFILE" ] && kill "$(cat "$PIDFILE")" 2> /dev/null
rm -f "$PIDFILE"
stop aristotle-agent > /dev/null 2>&1
start aristotle-agent && say "aristotle agent installed and running"
