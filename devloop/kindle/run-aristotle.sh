#!/bin/sh
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
DIR=$(cd "$(dirname "$0")" && pwd)
LOG=${ARISTOTLE_LOG:-/var/tmp/aristotle.log}

# The app installs its own SIGINT/SIGTERM handlers (save and leave); the
# script itself must survive them to restore the stock GUI.
trap '' HUP INT TERM

start=0
[ -f "$LOG" ] && start=$(wc -l < "$LOG")
echo "=== aristotle $(date) $*" >> "$LOG"
sh "$DIR/takeover.sh" start
"$DIR/aristotle" "$@" >> "$LOG" 2>&1
status=$?
echo "=== exit $status" >> "$LOG"
sh "$DIR/takeover.sh" stop
tail -n "+$((start + 1))" "$LOG"
exit $status
