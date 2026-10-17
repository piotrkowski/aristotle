#!/bin/sh
# Switch the USB port from mass storage to a network gadget and start SSH,
# so development continues over the same cable without replugging.
# Everything here is runtime state only: a reboot restores stock behaviour.
set -u
umask 022

BASE=/mnt/us/extensions/aristotle-usbnet
KO=/mnt/us/koreader
PIDFILE=/var/run/aristotle-dropbear.pid
DEV_IP=192.168.15.244
# Locally administered MACs so the host can match the interface reliably.
DEV_MAC=02:61:72:69:73:01
HOST_MAC=02:61:72:69:73:02

exec >> "$BASE/usbnet.log" 2>&1
printf '\nstart %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"

say() {
    printf '%s\n' "$*"
    eips 0 1 "$*                              " > /dev/null 2>&1
}

fail() {
    say "usbnet: $*"
    exit 1
}

[ -x "$KO/dropbear" ] || fail "KOReader dropbear not found"
[ -s "$BASE/authorized_keys" ] || fail "authorized_keys missing in extension"

mkdir -p "$KO/settings/SSH"
cp "$BASE/authorized_keys" "$KO/settings/SSH/authorized_keys"
chmod 600 "$KO/settings/SSH/authorized_keys"

if lsmod | grep -q '^g_mass_storage'; then
    rmmod g_mass_storage || fail "cannot unload mass storage (eject on host first)"
fi
if ! lsmod | grep -q '^g_ether'; then
    modprobe g_ether dev_addr="$DEV_MAC" host_addr="$HOST_MAC" || fail "g_ether failed to load"
fi
ifconfig usb0 "$DEV_IP" netmask 255.255.255.0 up || fail "usb0 did not come up"

# The stock firewall drops unsolicited input; open usb0 only.
iptables -C INPUT -i usb0 -j ACCEPT 2> /dev/null || iptables -I INPUT -i usb0 -j ACCEPT
iptables -C OUTPUT -o usb0 -j ACCEPT 2> /dev/null || iptables -I OUTPUT -o usb0 -j ACCEPT

# Keep powerd from blanking to the screensaver or suspending while we work
# (property used by KOReader; reset by stop.sh or a reboot).
lipc-set-prop com.lab126.powerd preventScreenSaver 1 || say "usbnet: warning, screensaver still active"

if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2> /dev/null; then
    say "usbnet: already up, ssh root@$DEV_IP"
    exit 0
fi
# -s: public keys only. The KOReader build reads settings/SSH/authorized_keys
# relative to its working directory and stores host keys there (-R).
cd "$KO" && ./dropbear -E -R -s -p 22 -P "$PIDFILE" || fail "dropbear failed"
say "usbnet up: ssh root@$DEV_IP"
