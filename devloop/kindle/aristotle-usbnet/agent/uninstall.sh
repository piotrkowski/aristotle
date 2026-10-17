#!/bin/sh
# Remove the Aristotle agent completely and give USB back to volumd.
set -u
S=/var/local/aristotle
JOB=/etc/upstart/aristotle-agent.conf
PIDFILE=/var/run/aristotle-dropbear.pid

stop aristotle-agent > /dev/null 2>&1
# Back to the stock firewall on Wi-Fi.
if [ -f "$S/net.sh" ]; then
    echo off > "$S/net-mode"
    sh "$S/net.sh" apply
fi
if [ -e "$JOB" ]; then
    mntroot rw && rm -f "$JOB" && sync && mntroot ro
fi
initctl reload-configuration 2> /dev/null
[ -f "$PIDFILE" ] && kill "$(cat "$PIDFILE")" 2> /dev/null
rm -f "$PIDFILE"
lipc-set-prop com.lab126.volumd useUsbForNetwork 0 2> /dev/null
lsmod | grep -q '^g_ether' && rmmod g_ether
# Keep the log for diagnosis; drop the scripts that the job would run.
rm -f "$S/boot.sh" "$S/aristotlectl" "$S/net.sh"
eips 0 1 "aristotle agent removed                       " > /dev/null 2>&1
echo "aristotle agent removed"
