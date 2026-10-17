#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Show each page-turn strategy for judging by eye. Each group starts with one
# full flash, then turns the page 4 times, 3 s apart. Run with the stock GUI
# stopped (takeover.sh start) and a text page dump at /var/tmp/page.raw.
#   ssh kindle sh -s < devloop/kindle/pageturn-demo.sh
for s in gl16 glr16 du du+gl16 a2+gl16; do
    echo "== $s"
    /var/tmp/aristotle-pageturn /var/tmp/page.raw 4 3000 "$s"
    sleep 2
done
