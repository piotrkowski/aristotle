#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Aristotle at boot, started by /etc/upstart/aristotle-ui.conf once the stock
# GUI's job has started, with /var/local/aristotle (checked) as the current
# directory. Also started by `aristotlectl app start`.
#
# It stops the stock GUI as takeover.sh does, tells powerd and blanket what
# the stock framework would have (the boot splash is gone), and runs the copy
# of the app in /var/local/aristotle/app, so USB drive mode cannot pull the
# binary away. Back to the stock GUI for this boot when:
#
# - /mnt/us/aristotle-disable exists (checked before anything is stopped);
# - three launches in a row did not stay up for two minutes (the count
#   survives reboots, so three forced restarts in a row also count); the
#   next boots stay stock until autostart is turned on again or the app is
#   started by hand;
# - Aristotle exits normally (the user quit);
# - `aristotlectl app stop` asks for it (not counted as a failure).
#
# Logs to ui.log, the app's output to app.log.
set -u
umask 077

# R stands in for / in the host-side test (device/tests/agent.sh); it is
# empty on the device.
R=${AGENT_TEST_ROOT:-}
S=${ARISTOTLE_DIR:-$R/var/local/aristotle}
LOG=$S/ui.log
FAILS=$S/ui-fails
MAX_FAILS=${MAX_FAILS:-3}
HEALTHY_AFTER=${HEALTHY_AFTER:-120}
APP=$S/app/aristotle
TAKEOVER=$R/var/run/aristotle-takeover

log() {
    printf '%s ui: %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG"
    if [ "$(wc -c < "$LOG")" -gt 262144 ]; then
        mv -f "$LOG" "$LOG.1"
    fi
}

uptime_s() {
    cut -d' ' -f1 "$R/proc/uptime"
}

fails() {
    n=$(cat "$FAILS" 2> /dev/null)
    case "$n" in '' | *[!0-9]*) n=0 ;; esac
    echo "$n"
}

set_fails() {
    echo "$1" > "$FAILS"
    sync
}

# Give the device back to the stock GUI and end the job; the job does not
# come back in this boot (aristotle-ui.conf).
stock() {
    log "stock GUI: $*"
    sh "$S/takeover.sh" stop >> "$LOG" 2>&1
    exit 0
}

manual=0
if [ -e "$S/ui-request" ]; then
    manual=1
    rm -f "$S/ui-request"
fi
# Once per boot (aristotle-ui.conf compares this with the current boot id).
cat "$R/proc/sys/kernel/random/boot_id" > "$S/ui-booted" 2> /dev/null
log "start (uptime $(uptime_s) s$([ $manual = 1 ] && echo ', by hand'))"

# The kill switch. The userstore is mounted long before the stock GUI
# starts; wait a little in case it is not.
i=0
until grep -q " $R/mnt/us " "$R/proc/mounts" 2> /dev/null || [ $i -ge 30 ]; do
    sleep 1
    i=$((i + 1))
done
if [ -e "$R/mnt/us/aristotle-disable" ]; then
    log "/mnt/us/aristotle-disable exists; staying on the stock GUI"
    exit 0
fi
n=$(fails)
if [ "$n" -ge "$MAX_FAILS" ] && [ $manual = 0 ]; then
    log "$n launches failed in a row; staying on the stock GUI (turn autostart on again to retry)"
    exit 0
fi
[ $manual = 1 ] && set_fails 0
if [ ! -x "$APP" ]; then
    log "no app at $APP; staying on the stock GUI"
    exit 0
fi

# Take the screen: the same steps as takeover.sh start, which is proven on
# the device.
if ! sh "$S/takeover.sh" start >> "$LOG" 2>&1; then
    log "takeover refused"
    stock "takeover failed"
fi
# The framework may have been starting when its parent job stopped.
i=0
while [ $i -lt 10 ]; do
    initctl status framework 2> /dev/null | grep -q 'stop/waiting' &&
        ! pgrep -f /usr/java/bin/cvm > /dev/null && break
    stop framework > /dev/null 2>&1
    sleep 1
    i=$((i + 1))
done
# What framework_setup does for powerd and blanket once Java is up: no boot
# splash any more, and powerd may handle the power button.
lipc-set-prop com.lab126.blanket unload splash 2> /dev/null
lipc-send-event -r 5 com.lab126.hal bootSplashCleanup 2> /dev/null
log "stock GUI stopped (uptime $(uptime_s) s)"

pid=
stop_app() {
    [ -n "$pid" ] && kill -TERM "$pid" 2> /dev/null && wait "$pid"
    rm -f "$S/app.pid"
    log "stopped by upstart"
    exit 0
}
trap stop_app TERM INT
trap '' HUP

while :; do
    n=$(($(fails) + 1))
    set_fails "$n"
    [ -f "$S/app.log" ] && mv -f "$S/app.log" "$S/app.log.1"
    log "launch $n (uptime $(uptime_s) s)"
    # A launch that stays up for HEALTHY_AFTER seconds is a good one.
    (sleep "$HEALTHY_AFTER" && set_fails 0) &
    healthy=$!
    "$APP" "$@" >> "$S/app.log" 2>&1 &
    pid=$!
    echo "$pid" > "$S/app.pid"
    wait "$pid"
    rc=$?
    pid=
    kill "$healthy" 2> /dev/null
    rm -f "$S/app.pid"
    if [ -e "$S/ui-stop" ]; then
        how=$(cat "$S/ui-stop")
        rm -f "$S/ui-stop"
        # Asked to stop: not a failure (the count is 0 already if this
        # launch got healthy).
        cur=$(fails)
        [ "$cur" -gt 0 ] && set_fails $((cur - 1))
        if [ "$how" = keep ]; then
            log "stopped by aristotlectl (exit $rc); screen left to the caller"
            exit 0
        fi
        stock "stopped by aristotlectl (exit $rc)"
    fi
    if [ "$rc" = 0 ]; then
        set_fails 0
        stock "Aristotle quit"
    fi
    log "Aristotle exited with $rc (launch $n)"
    [ "$(fails)" -ge "$MAX_FAILS" ] && stock "$MAX_FAILS failed launches in a row"
    sleep 2
done
