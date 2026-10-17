#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Undo start.sh: stop SSH, drop the firewall holes and unload the network gadget.
# Replug the cable (or reboot) to get USB drive mode back.
set -u

BASE=/mnt/us/extensions/aristotle-usbnet
PIDFILE=/var/run/aristotle-dropbear.pid

exec >> "$BASE/usbnet.log" 2>&1
printf '\nstop %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"

[ -f "$PIDFILE" ] && kill "$(cat "$PIDFILE")" 2> /dev/null
rm -f "$PIDFILE"
iptables -D INPUT -i usb0 -j ACCEPT 2> /dev/null
iptables -D OUTPUT -o usb0 -j ACCEPT 2> /dev/null
ifconfig usb0 down 2> /dev/null
lsmod | grep -q '^g_ether' && rmmod g_ether
lipc-set-prop com.lab126.powerd preventScreenSaver 0
eips 0 1 "usbnet stopped; replug for USB drive              " > /dev/null 2>&1
