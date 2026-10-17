#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Run the Aristotle reader in place of the stock UI, then bring the stock UI
# back. Lives next to the `aristotle` binary and its data/ directory (the app
# bundle from tools/build-render.sh unpacks all of them into one folder):
#
#   ssh kindle sh /mnt/us/aristotle/run-aristotle.sh [aristotle options]
#   ssh kindle sh /mnt/us/aristotle/run-aristotle.sh --turn-mode 7
#
# The screen is taken with `takeover.sh start` and given back with
# `takeover.sh stop`, which restarts the stock GUI (30 to 60 s of boot splash).
# `takeover.sh pause`/`resume` (SIGSTOP) is deliberately not used: on our
# firmware the frozen Java framework was killed by Amazon's watchdog and the
# GUI did not come back without a reboot (seen on the device, 2026-10-17).
#
# The app's stderr (timings as `[perf ...]` lines) goes to $ARISTOTLE_LOG
# (default /var/tmp/aristotle.log) and is printed when the app exits. Hangups
# are ignored, so the app keeps running if the SSH session drops; the stock
# GUI comes back whenever it exits. Quit by holding the "Biblioteka" title for
# a second, or with `killall aristotle` (progress is saved either way).
#
# KUAL > Aristotle > Start Aristotle runs this script detached
# (extension/bin/aristotle-start.sh), with the log on the USB drive.
DIR=$(cd "$(dirname "$0")" && pwd)
LOG=${ARISTOTLE_LOG:-/var/tmp/aristotle.log}
# R stands in for / in the host-side test (device/tests/agent.sh); it is
# empty on the device.
R=${AGENT_TEST_ROOT:-}

# The app installs its own SIGINT/SIGTERM handlers (save and leave); the
# script itself must survive them to restore the stock GUI.
trap '' HUP INT TERM

# An Aristotle started at boot (or by `aristotlectl app start`) has the
# screen: stop it first, without counting it as a crash and without bringing
# the stock GUI back in between.
if initctl status aristotle-ui 2> /dev/null | grep -q start/running; then
    sh "$R/var/local/aristotle/aristotlectl" app stop --keep-takeover
fi

start=0
[ -f "$LOG" ] && start=$(wc -l < "$LOG")
echo "=== aristotle $(date) $*" >> "$LOG"
# takeover.sh refuses anything but a PW4 on firmware 5.13.x; then the app does
# not start either.
sh "$DIR/takeover.sh" start || exit 1
"$DIR/aristotle" "$@" >> "$LOG" 2>&1
status=$?
echo "=== exit $status" >> "$LOG"
sh "$DIR/takeover.sh" stop
tail -n "+$((start + 1))" "$LOG"
exit $status
