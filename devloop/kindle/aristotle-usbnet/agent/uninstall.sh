#!/bin/sh
# Remove the aristotle agent completely and give USB back to volumd.
set -u
stop aristotle > /dev/null 2>&1
if [ -e /etc/upstart/aristotle.conf ]; then
    mntroot rw && rm -f /etc/upstart/aristotle.conf && sync && mntroot ro
fi
initctl reload-configuration 2> /dev/null
[ -f /var/run/aristotle-dropbear.pid ] && kill "$(cat /var/run/aristotle-dropbear.pid)" 2> /dev/null
rm -f /var/run/aristotle-dropbear.pid
lipc-set-prop com.lab126.volumd useUsbForNetwork 0 2> /dev/null
lsmod | grep -q '^g_ether' && rmmod g_ether
# Keep the log for diagnosis; drop the scripts that the job would run.
rm -f /var/local/aristotle/boot.sh /var/local/aristotle/aristotlectl
eips 0 1 "aristotle agent removed                         " > /dev/null 2>&1
echo "aristotle agent removed"
