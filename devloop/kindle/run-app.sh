#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Run Aristotle in place of the stock UI and give the screen back afterwards.
# Expects /var/tmp/aristotle-app and the takeover script uploaded to /var/tmp.
#   ssh kindle sh /var/tmp/run-app.sh
sh /var/tmp/takeover.sh start
/var/tmp/aristotle-app
status=$?
sh /var/tmp/takeover.sh stop
exit $status
