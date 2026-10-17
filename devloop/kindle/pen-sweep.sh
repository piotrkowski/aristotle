#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Replay a recorded pen session under several PenParams and print one latency
# line per variant. Upload aristotle-pen-bench and the recording to /var/tmp,
# then run on the device: ssh kindle sh -s < devloop/kindle/pen-sweep.sh
B=/var/tmp/aristotle-pen-bench
REC=/var/tmp/pen.rec
# Baseline: one update per frame, submitted from the input loop.
$B $REC backend=pipeline inflight=4 cleanup=none
# Display thread with latest-wins coalescing (the default).
$B $REC backend=display
$B $REC backend=display radius=2
$B $REC backend=display stroke=1
