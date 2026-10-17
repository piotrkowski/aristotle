#!/bin/sh
# Reversible takeover of the Kindle: stop Amazon's GUI stack so Aristotle owns
# the screen and most of the RAM, and bring it back afterwards.
#
#   ssh kindle sh -s start < devloop/kindle/takeover.sh
#   ssh kindle sh -s stop  < devloop/kindle/takeover.sh
#   ssh kindle sh -s status < devloop/kindle/takeover.sh
#
# Nothing is written to the root filesystem: everything here is runtime state,
# so a reboot always returns the stock system. Kept running on purpose: powerd
# (power keys, battery, suspend), volumd (USB mass storage), wifid, lipcd and
# dbus, plus our USB network and SSH.

# Jobs stopped by `stop lab126_gui`: x, framework (cvm), pillow, kb,
# kppreader, webreader, kfxreader, ... Their "stop on" stanzas follow it.
GUI=lab126_gui
# Background services that only serve the stock UI or Amazon's cloud.
# Same list KOReader stops to reclaim RAM, plus telemetry.
EXTRA="stored todo tmd rcm archive scanner otav3 otaupd dmld fastmetrics clickstream_logging perfd"

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
    mem
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
    for job in $EXTRA; do
        start "$job" > /dev/null 2>&1
    done
    start "$GUI" > /dev/null 2>&1
    echo "takeover: stock GUI starting (takes about 20 s)"
    ;;
status)
    state
    mem
    ;;
*)
    echo "usage: takeover.sh start|stop|status" >&2
    exit 2
    ;;
esac
