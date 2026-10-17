#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Undo migrate.sh: bring back the agent from before the project's rename (job
# "aristotle", state in /var/local/aristotle) from a migration backup. migrate.sh
# runs it by itself when a step fails; run it by hand to go back later:
#
#   CONFIRM=yes sh /mnt/us/extensions/aristotle-usbnet/agent/rollback.sh [BACKUP_DIR]
#
# BACKUP_DIR defaults to the newest /var/local/aristotle-migrate/<stamp>/.
# Every step checks the current state first, so running it twice is harmless.
#  1. Stops aristotle-agent and hands dropbear's pid file back.
#  2. In one `mntroot rw` window: removes /etc/upstart/aristotle-agent.conf and
#     restores the old job file from the backup.
#  3. Moves /var/local/aristotle (if the migration created it) into the backup
#     directory, carrying the USB mode and the takeover flag
#     (/var/run/aristotle-takeover) back; restores
#     /var/local/aristotle from the backup if it is gone.
#  4. Re-enables the old KUAL extension and starts the old job.
set -u
umask 022
trap '' HUP PIPE

# R stands in for / in the host-side test (devloop/kindle/tests/migrate.sh);
# it is empty on the device.
R=${AGENT_TEST_ROOT:-}
OLD_S=$R/var/local/aristotle
OLD_JOB=aristotle
OLD_CONF=$R/etc/upstart/aristotle.conf
OLD_PID=$R/var/run/aristotle-dropbear.pid
OLD_EXT=$R/mnt/us/extensions/aristotle-usbnet

S=$R/var/local/aristotle
JOB=aristotle-agent
CONF=$R/etc/upstart/aristotle-agent.conf
PIDFILE=$R/var/run/aristotle-dropbear.pid

BACKUPS=$R/var/local/aristotle-migrate
LOG=$BACKUPS/migrate.log

say() {
    printf '%s\n' "$*" 2> /dev/null
    printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG" 2> /dev/null
    eips 0 1 "$*                                        " > /dev/null 2>&1
}

case "${CONFIRM:-}:${1:-}" in
yes:* | *:kual) ;;
*) echo "This rewrites the agent's upstart job on the root filesystem. Run with CONFIRM=yes." >&2; exit 2 ;;
esac
[ "${1:-}" = kual ] && shift
[ "$(id -u)" = 0 ] || { say "rollback: must run as root"; exit 1; }

B=${1:-$(ls -d "$BACKUPS"/[0-9]*/ 2> /dev/null | sort | tail -n 1)}
B=${B%/}
[ -n "$B" ] && [ -f "$B/journal" ] || { say "rollback: no migration backup found in $BACKUPS"; exit 1; }
say "rollback: from $B"
echo "rollback $(date -u '+%Y-%m-%dT%H:%M:%SZ')" >> "$B/journal"

# 1. The new job and its dropbear.
stop "$JOB" > /dev/null 2>&1
if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2> /dev/null; then
    mv -f "$PIDFILE" "$OLD_PID"
fi

# 2. The root filesystem, only if something there differs from the backup.
need_root=0
[ -e "$CONF" ] && need_root=1
if [ -f "$B/old-job.conf" ] && ! cmp -s "$B/old-job.conf" "$OLD_CONF"; then
    need_root=1
fi
if [ $need_root = 1 ]; then
    if mntroot rw; then
        rm -f "$CONF"
        [ -f "$B/old-job.conf" ] && cp -p "$B/old-job.conf" "$OLD_CONF"
        sync
        mntroot ro || say "rollback: warning, mntroot ro failed"
    else
        say "rollback: mntroot rw failed; the upstart jobs are unchanged"
    fi
fi
initctl reload-configuration 2> /dev/null

# 3. State directories.
[ -d "$OLD_S" ] || { [ -d "$B/old-state" ] && cp -a "$B/old-state" "$OLD_S"; }
if [ -d "$S" ] && grep -qx new-state "$B/journal"; then
    if [ -d "$OLD_S" ]; then
        if [ -e "$S/usb-mode" ]; then cp -p "$S/usb-mode" "$OLD_S/usb-mode"; else rm -f "$OLD_S/usb-mode"; fi
        if [ -e "$R/var/run/aristotle-takeover" ]; then touch "$OLD_S/takeover"; else rm -f "$OLD_S/takeover"; fi
    fi
    n=1
    while [ -e "$B/new-state.$n" ]; do n=$((n + 1)); done
    mv "$S" "$B/new-state.$n" && say "rollback: moved $S to $B/new-state.$n"
    rm -f "$R/var/run/aristotle-takeover"
fi

# 4. The old extension and job.
if [ -f "$OLD_EXT/config.xml.migrated" ] && [ ! -e "$OLD_EXT/config.xml" ]; then
    mv "$OLD_EXT/config.xml.migrated" "$OLD_EXT/config.xml"
fi
if [ -e "$OLD_CONF" ]; then
    start "$OLD_JOB" > /dev/null 2>&1
    i=0
    until initctl status "$OLD_JOB" 2> /dev/null | grep -q start/running; do
        i=$((i + 1))
        [ $i -ge 10 ] && { say "rollback: $OLD_JOB did not start"; exit 1; }
        sleep 1
    done
    say "rollback: done; $OLD_JOB running again"
else
    say "rollback: done; no old job to start (there was none before the migration)"
fi
