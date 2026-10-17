#!/bin/sh
# Run Aristotle in place of the stock UI and give the screen back afterwards.
# Expects /var/tmp/aristotle-app and the takeover script uploaded to /var/tmp.
#   ssh kindle sh /var/tmp/run-app.sh
sh /var/tmp/takeover.sh start || exit 1
/var/tmp/aristotle-app
status=$?
sh /var/tmp/takeover.sh stop
exit $status
