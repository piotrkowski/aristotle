#!/bin/sh
# One-time host setup: a NetworkManager profile that gives the Kindle's USB
# network interface (matched by the fixed MAC set in aristotle-usbnet) the address
# 192.168.15.201/24. Safe to re-run; it replaces only its own profile.
set -eu

NAME=aristotle-kindle-usbnet
HOST_MAC=02:61:72:69:73:02

if nmcli -t -f NAME connection show | grep -qx "$NAME"; then
    nmcli connection delete "$NAME" > /dev/null
fi
nmcli connection add type ethernet con-name "$NAME" ifname '*' \
    ethernet.mac-address "$HOST_MAC" \
    ipv4.method manual ipv4.addresses 192.168.15.201/24 ipv4.never-default yes \
    ipv6.method disabled connection.autoconnect yes > /dev/null
echo "NetworkManager profile $NAME ready."
