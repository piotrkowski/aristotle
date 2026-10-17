#!/bin/sh
# One-time migration of a device agent installed before the project's rename
# (upstart job "aristotle", state in /var/local/aristotle) to the Aristotle agent
# (job "aristotle-agent", state in /var/local/aristotle). Run as root, from
# KUAL (Aristotle > Migrate agent from an earlier install) or over SSH:
#
#   CONFIRM=yes sh /mnt/us/extensions/aristotle-usbnet/agent/migrate.sh
#
# An SSH session survives it: the running dropbear is handed to the new agent,
# not restarted.
#
#  1. Checks: root, an old agent present, no new one, USB mode "net", the new
#     agent's files beside this script. Nothing has changed if one fails.
#  2. Backup to /var/local/aristotle-migrate/<UTC stamp>/: the old job file
#     and a copy of /var/local/aristotle.
#  3. Copies the state (key, USB mode, takeover and watchdog flags, log) into
#     /var/local/aristotle and installs the agent's scripts there. The old
#     directory is left as it was.
#  4. Stops the old job and hands its dropbear pid file to the new agent.
#  5. In one `mntroot rw` window: installs /etc/upstart/aristotle-agent.conf
#     and removes /etc/upstart/aristotle.conf.
#  6. Starts the new job and checks for 30 s that it keeps running (same pid),
#     that dropbear runs and listens on the USB address, and that usb0 is up.
#  7. Disables the old KUAL extension (its config.xml is renamed).
#
# A failure in steps 3-6 runs rollback.sh on the backup, which restores the
# old job and state and starts it again. rollback.sh can also be run by hand
# later. Everything is logged to /var/local/aristotle-migrate/migrate.log.
set -u
umask 022
# Keep going if the SSH session that started us goes away.
trap '' HUP PIPE

HERE=$(cd "$(dirname "$0")" && pwd)
EXT=$(dirname "$HERE")

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
DEV_IP=192.168.15.244
# FILES: what install.sh puts in $S. STATE: what the old agent kept there.
FILES="boot.sh aristotlectl net.sh"
STATE="authorized_keys usb-mode takeover last-reboot gui-restarted log log.1"
VERIFY_SECS=${VERIFY_SECS:-30}
POLL_SECS=${POLL_SECS:-3}
# The agent ticks every 15 s; dropbear and usb0 may need one tick.
GRACE_SECS=${GRACE_SECS:-15}

BACKUPS=$R/var/local/aristotle-migrate
B=$BACKUPS/$(date -u '+%Y%m%dT%H%M%SZ')
LOG=$BACKUPS/migrate.log

say() {
    printf '%s\n' "$*" 2> /dev/null
    printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG" 2> /dev/null
    eips 0 1 "$*                                        " > /dev/null 2>&1
}

# Before anything has changed.
refuse() {
    say "migrate: $*; nothing changed"
    exit 1
}

# After the backup: undo everything and stop.
fail() {
    say "migrate: $*; rolling back"
    CONFIRM=yes sh "$HERE/rollback.sh" "$B"
    exit 1
}

journal() {
    echo "$*" >> "$B/journal"
}

job_status() {
    initctl status "$1" 2> /dev/null
}

# "aristotle-agent start/running, process 1234" -> 1234; empty when not running.
job_pid() {
    case "$(job_status "$1")" in
    *start/running*) job_status "$1" | sed -n 's/.*process \([0-9]*\).*/\1/p' ;;
    esac
}

# dropbear from the new pid file is alive and has a listening socket on
# DEV_IP:22. /proc/net/tcp prints the address as a little-endian hex word.
dropbear_ok() {
    [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2> /dev/null || return 1
    hex=$(echo "$DEV_IP" | awk -F. '{printf "%02X%02X%02X%02X", $4, $3, $2, $1}')
    grep -q " $hex:0016 00000000:0000 0A " "$R/proc/net/tcp"
}

usb0_ok() {
    ifconfig usb0 2> /dev/null | grep -q "inet addr:$DEV_IP" &&
        ifconfig usb0 2> /dev/null | grep -q ' UP '
}

# 1. Checks.
case "${CONFIRM:-}:${1:-}" in
yes:* | *:kual) ;;
*) echo "This replaces the agent's upstart job on the root filesystem. Run with CONFIRM=yes." >&2; exit 2 ;;
esac
mkdir -p "$BACKUPS" || { echo "cannot create $BACKUPS" >&2; exit 1; }
say "migrate: start ($HERE)"
[ "$(id -u)" = 0 ] || refuse "must run as root"
[ -e "$OLD_CONF" ] || [ -d "$OLD_S" ] || refuse "no agent from an earlier install; use Install agent"
[ ! -e "$CONF" ] || refuse "$CONF already exists"
[ ! -e "$S" ] || refuse "$S already exists"
for f in $FILES aristotle-agent.conf rollback.sh; do
    [ -f "$HERE/$f" ] || refuse "$HERE/$f missing"
done
[ -s "$OLD_S/authorized_keys" ] || [ -s "$EXT/authorized_keys" ] || refuse "no authorized_keys"
set -- $(cat "$OLD_S/usb-mode" 2> /dev/null)
if [ "${1:-net}" = storage ] && [ "$(date +%s)" -lt "${2:-0}" ]; then
    refuse "USB drive mode is on; switch to net first"
fi

# 2. Backup.
mkdir -p "$B" || refuse "cannot create $B"
journal "start $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
if [ -e "$OLD_CONF" ]; then
    cp -p "$OLD_CONF" "$B/old-job.conf" || refuse "cannot back up $OLD_CONF"
fi
if [ -d "$OLD_S" ]; then
    cp -a "$OLD_S" "$B/old-state" || refuse "cannot back up $OLD_S"
fi
job_status "$OLD_JOB" > "$B/old-job.status"
[ -n "$(job_pid "$OLD_JOB")" ] && journal "old-job-was-running"
say "migrate: backup in $B"

# 3. State.
journal "new-state"
mkdir -p "$S" || fail "cannot create $S"
for f in $STATE; do
    if [ -e "$OLD_S/$f" ]; then
        cp -p "$OLD_S/$f" "$S/$f" || fail "cannot copy $f"
    fi
done
if [ ! -s "$S/authorized_keys" ]; then
    cp "$EXT/authorized_keys" "$S/authorized_keys" || fail "cannot copy authorized_keys"
fi
for f in $FILES; do
    cp "$HERE/$f" "$S/$f" && chmod 755 "$S/$f" || fail "cannot install $f"
done
printf '%s migrate: state copied from %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$OLD_S" >> "$S/log"

# 4. Stop the old job. dropbear runs in its own session and survives; the new
# agent adopts it through the pid file.
journal "old-job-stopped"
stop "$OLD_JOB" > /dev/null 2>&1
i=0
while [ -n "$(job_pid "$OLD_JOB")" ] && [ $i -lt 10 ]; do
    sleep 1
    i=$((i + 1))
done
[ -z "$(job_pid "$OLD_JOB")" ] || fail "old job does not stop"
if [ -f "$OLD_PID" ] && kill -0 "$(cat "$OLD_PID")" 2> /dev/null; then
    journal "pid-handed"
    mv -f "$OLD_PID" "$PIDFILE" || fail "cannot hand over $OLD_PID"
fi

# 5. The root filesystem, in one read-write window.
journal "root-write"
mntroot rw || fail "mntroot rw failed"
ok=0
cp "$HERE/aristotle-agent.conf" "$CONF" && rm -f "$OLD_CONF" && sync && ok=1
mntroot ro || say "migrate: warning, mntroot ro failed"
[ $ok = 1 ] || fail "writing the upstart job failed"
initctl reload-configuration 2> /dev/null

# 6. Start and watch.
journal "new-job-started"
start "$JOB" > /dev/null 2>&1 || fail "start $JOB failed"
pid0=$(job_pid "$JOB")
[ -n "$pid0" ] || fail "$JOB is not running"
t=0
while [ $t -lt "$VERIFY_SECS" ]; do
    sleep "$POLL_SECS"
    t=$((t + POLL_SECS))
    pid=$(job_pid "$JOB")
    [ "$pid" = "$pid0" ] || fail "$JOB stopped or restarted (pid $pid0, now ${pid:-none})"
    [ $t -lt $GRACE_SECS ] && continue
    dropbear_ok || fail "dropbear is not listening on $DEV_IP:22"
    usb0_ok || fail "usb0 is not up at $DEV_IP"
done
dropbear_ok && usb0_ok || fail "dropbear or usb0 not up after ${VERIFY_SECS}s"

# 7. The old KUAL extension would reinstall the old job; hide it from KUAL.
if [ -f "$OLD_EXT/config.xml" ]; then
    journal "old-extension-disabled"
    mv -f "$OLD_EXT/config.xml" "$OLD_EXT/config.xml.migrated"
fi
journal "done"
say "migrate: done; $JOB running, SSH at $DEV_IP"
say "migrate: kept $OLD_S and $B for rollback.sh"
