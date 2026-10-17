#!/bin/sh
# Reversible takeover of the Kindle: stop Amazon's GUI stack so Aristotle owns
# the screen and most of the RAM, and bring it back afterwards.
#
#   ssh kindle sh -s start < devloop/kindle/takeover.sh
#   ssh kindle sh -s stop  < devloop/kindle/takeover.sh
#   ssh kindle sh -s status < devloop/kindle/takeover.sh
#   ssh kindle sh -s pause  < devloop/kindle/takeover.sh
#   ssh kindle sh -s resume < devloop/kindle/takeover.sh
#
# start/stop take the stock GUI down and bring it back: most RAM is freed, and
# the way back shows the boot splash for 30-60 s while Java restarts. This is
# the reliable path; launchers use it.
#
# pause/resume (EXPERIMENTAL) freeze the stock GUI instead (SIGSTOP, as KOReader
# and slint-kindle-backend do). On our PW4 the frozen framework was killed,
# most likely by Amazon's pmond watchdog (the poll_daemons job). The GUI then
# failed to come back (no Xorg, awesome or pillow) until a reboot. pause now
# refuses unless the stock UI is fully up, and holds poll_daemons while frozen.
# It is still unproven; don't build on it.
#
# Nothing is written to persistent storage: everything here is runtime state,
# so a reboot in the middle of a takeover, or after it, always returns the
# stock system. The flag that keeps the agent's watchdog from restarting the
# stock GUI, /var/run/aristotle-takeover, is on the /var tmpfs too. Kept running on purpose: powerd
# (power keys, battery, suspend), volumd (USB mass storage), wifid, lipcd and
# dbus, plus our USB network and SSH.
#
# start and pause refuse to run, and change nothing, unless the device is a
# Kindle Paperwhite 4 (2018) on firmware 5.13.x (aristotle-usbnet/guard.sh,
# looked for next to this script, then in the installed KUAL extension).
# ARISTOTLE_FORCE=1 skips that check, for developers trying another model or
# firmware on purpose, at their own risk. stop, resume and status always run.

# R stands in for / in the host-side test (devloop/kindle/tests/agent.sh); it
# is empty on the device.
R=${AGENT_TEST_ROOT:-}

# Jobs stopped by `stop lab126_gui`: x, framework (cvm), pillow, kb,
# kppreader, webreader, kfxreader, ... Their "stop on" stanzas follow it.
GUI=lab126_gui
# Background services that only serve the stock UI or Amazon's cloud.
# Same list KOReader stops to reclaim RAM, plus telemetry.
EXTRA="stored todo tmd rcm archive scanner otav3 otaupd dmld fastmetrics clickstream_logging perfd"
TAKEOVER=$R/var/run/aristotle-takeover
# An agent installed before the project's rename (until the device has run
# agent/migrate.sh) looks for the flag in its persistent state directory
# instead, where a reboot does not clear it; its watchdog then stays off
# until the next stop. Drop this once no device runs it.
LEGACY=$R/var/local/aristotle
# The device guard: beside this script in the app bundle and the KUAL
# extension, one level down in the repository, or in the installed extension
# when the script comes on stdin (sh -s).
guard() {
    [ "${ARISTOTLE_FORCE:-}" = 1 ] && return 0
    here=
    case "$0" in */takeover.sh) here=${0%/*} ;; takeover.sh) here=. ;; esac
    for g in ${here:+"$here/guard.sh" "$here/aristotle-usbnet/guard.sh"} \
        "$R/mnt/us/extensions/aristotle-usbnet/guard.sh"; do
        if [ -f "$g" ]; then
            . "$g"
            device_guard
            return
        fi
    done
    echo "takeover: guard.sh not found; refusing (ARISTOTLE_FORCE=1 overrides)" >&2
    return 1
}
# Processes frozen by `pause`: everything that would draw over us.
FROZEN="awesome cvm AaMenuApplication KindleContentDownloadManagerApplication"

mem() {
    awk '/MemTotal/ {t=$2} /MemFree/ {f=$2} /^Buffers/ {b=$2} /^Cached/ {c=$2}
         END {printf "RAM: %d MB total, %d MB available\n", t/1024, (f+b+c)/1024}' /proc/meminfo
}

state() {
    for job in $GUI x framework pillow kb $EXTRA powerd volumd wifid; do
        printf '%-20s %s\n' "$job" "$(initctl status "$job" 2>/dev/null | cut -d' ' -f2 | tr -d ,)"
    done
}

case "${1:-status}" in
start)
    guard || exit 1
    mem
    # Tell the agent's watchdog that the stock GUI is down on purpose.
    touch "$TAKEOVER"
    [ -x "$LEGACY/boot.sh" ] && touch "$LEGACY/takeover"
    stop "$GUI" > /dev/null 2>&1
    for job in $EXTRA; do
        stop "$job" > /dev/null 2>&1
    done
    # Let the framework finish tearing down before we draw.
    i=0
    while pgrep -x cvm > /dev/null && [ $i -lt 30 ]; do
        sleep 1
        i=$((i + 1))
    done
    pgrep -x cvm > /dev/null && echo "warning: cvm still running after 30 s"
    # Keep powerd from blanking to the screensaver while we own the screen.
    lipc-set-prop com.lab126.powerd preventScreenSaver 1
    echo "takeover: stock GUI stopped"
    mem
    ;;
stop)
    rm -f "$TAKEOVER" "$LEGACY/takeover" "$R/var/local/aristotle/takeover"
    for job in $EXTRA; do
        start "$job" > /dev/null 2>&1
    done
    start "$GUI" > /dev/null 2>&1
    echo "takeover: stock GUI starting (takes about 20 s)"
    ;;
pause)
    guard || exit 1
    # Freezing a GUI that is still starting wedged it until reboot: require
    # the framework, the window manager and Pillow to be running.
    for p in cvm awesome pillowd; do
        pgrep -x "$p" > /dev/null || { echo "takeover: $p not running; refusing to pause (use start)" >&2; exit 1; }
    done
    # pmond kills processes that stop answering, which includes a frozen cvm.
    stop poll_daemons > /dev/null 2>&1
    # Hide the stock chrome, then freeze everything that draws: the window
    # manager, the Java framework and the React Native apps.
    lipc-set-prop com.lab126.pillow disableEnablePillow disable 2> /dev/null
    for p in $FROZEN; do
        killall -STOP "$p" 2> /dev/null
    done
    lipc-set-prop com.lab126.powerd preventScreenSaver 1
    echo "takeover: stock GUI frozen"
    ;;
resume)
    for p in $FROZEN; do
        killall -CONT "$p" 2> /dev/null
    done
    start poll_daemons > /dev/null 2>&1
    lipc-set-prop com.lab126.pillow disableEnablePillow enable 2> /dev/null
    # The stock UI does not know we drew over it. Reopen Home so the booklet
    # repaints, and ask Pillow to show its bars again, as KOReader does: the
    # status bar and toolbar are Pillow windows that do not repaint on their own.
    lipc-set-prop com.lab126.appmgrd start app://com.lab126.booklet.home 2> /dev/null
    for bar in default_status_bar search_bar; do
        lipc-set-prop com.lab126.pillow interrogatePillow \
            "{\"pillowId\": \"$bar\", \"function\": \"nativeBridge.showMe();\"}" 2> /dev/null
    done
    echo "takeover: stock GUI resumed"
    ;;
status)
    state
    mem
    ;;
*)
    echo "usage: takeover.sh start|stop|pause|resume|status" >&2
    exit 2
    ;;
esac
