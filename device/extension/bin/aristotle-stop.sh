#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# KUAL > Aristotle > Return to the Kindle interface: stop Aristotle however
# it was started, and bring back everything the takeover stopped. Aristotle
# saves its progress first. From inside Aristotle, holding the "Library"
# title for a second does the same.
#
# - started at boot or by `aristotlectl app start`: aristotlectl app stop,
#   which is not counted as a crash;
# - started by KUAL or run-aristotle.sh: SIGTERM, after which its launcher
#   brings back the stock GUI;
# - nothing running, but the stock GUI or the jobs the takeover stops are
#   down: takeover.sh stop.
set -u
# R stands in for / in the host-side test (device/tests/agent.sh); it
# is empty on the device.
R=${AGENT_TEST_ROOT:-}
HERE=$(cd "$(dirname "$0")" && pwd)
EXT=$(dirname "$HERE")
CTL=$R/var/local/aristotle/aristotlectl

say() {
    printf '%s\n' "$*"
    eips 0 1 "$*                                        " > /dev/null 2>&1
}

if initctl status aristotle-ui 2> /dev/null | grep -q start/running; then
    say "Stopping Aristotle..."
    sh "$CTL" app stop
fi
if pgrep -x aristotle > /dev/null; then
    say "Stopping Aristotle..."
    killall -TERM aristotle 2> /dev/null
    i=0
    while pgrep -x aristotle > /dev/null && [ $i -lt 20 ]; do
        sleep 1
        i=$((i + 1))
    done
    # run-aristotle.sh brings the stock GUI back once the app has gone.
    sleep 2
fi
if ! initctl status lab126_gui 2> /dev/null | grep -q start/running || [ -e "$R/var/run/aristotle-takeover" ]; then
    sh "$EXT/takeover.sh" stop
fi
say "Back to the Kindle interface"
