#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# KUAL > Aristotle > Start Aristotle: run the app in /mnt/us/aristotle in
# place of the stock GUI (run-aristotle.sh: takeover, app, stock GUI back
# when it quits). No computer needed.
#
# KUAL runs this as a child of the Java framework, which the takeover stops;
# upstart then signals the framework's whole process group. So the launcher
# starts in a session of its own (setsid) and this script returns at once.
#
# The app's log goes to /mnt/us/.aristotle/app.log (the previous run's to
# app.log.1): on the USB drive, where you can read it from a computer.
set -u
# R stands in for / in the host-side test (device/tests/agent.sh); it
# is empty on the device.
R=${AGENT_TEST_ROOT:-}
APP=${ARISTOTLE_APP_DIR:-$R/mnt/us/aristotle}
LOGDIR=$R/mnt/us/.aristotle

say() {
    printf '%s\n' "$*"
    eips 0 1 "$*                                        " > /dev/null 2>&1
}

if [ ! -x "$APP/aristotle" ] || [ ! -f "$APP/run-aristotle.sh" ]; then
    say "aristotle: no app in /mnt/us/aristotle"
    exit 1
fi
if pgrep -x aristotle > /dev/null; then
    say "aristotle: already running"
    exit 0
fi
mkdir -p "$LOGDIR"
[ -f "$LOGDIR/app.log" ] && mv -f "$LOGDIR/app.log" "$LOGDIR/app.log.1"
say "Starting Aristotle..."
ARISTOTLE_LOG=$LOGDIR/app.log setsid sh "$APP/run-aristotle.sh" < /dev/null > /dev/null 2>&1 &
exit 0
