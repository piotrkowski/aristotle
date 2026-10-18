#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Generate original test text without using the owner's library.
set -eu
REPO=$(cd "$(dirname "$0")/../.." && pwd)
. "$REPO/tools/env.sh"
OUT=${1:-$ARISTOTLE_CACHE/latency-fixture}
mkdir -p "$OUT/books"
awk 'BEGIN {
    print "Latency fixture\n"
    for (i = 1; i <= 240; i++) {
        printf "Paragraph %d. ", i
        print "This text was written for the Aristotle latency benchmark. A reader opens a page, moves forward, and returns to the starting position. Each numbered paragraph provides a visible reference when checking the screen. The words belong to the test, not to a personal library. Repeat the same gestures with the same settings when comparing two builds.\n"
    }
}' > "$OUT/books/Latency fixture.txt"
printf '%s\n' "$OUT"
