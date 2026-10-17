#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# One-time host setup: a NetworkManager profile that gives the Kindle's USB
# network interface (matched by the fixed MAC that
# devloop/kindle/aristotle-usbnet/agent/boot.sh sets) the address
# 192.168.15.201/24. Safe to re-run: it replaces every profile bound to that
# MAC or the earlier one, including ones made under an earlier name, and
# nothing else.
set -eu

NAME=aristotle-kindle-usbnet
HOST_MAC=02:61:72:69:73:02
# The host-side MAC that earlier versions of the agent and of bin/start.sh
# set. A device keeps it until g_ether is loaded again: after a migration
# (agent/migrate.sh hands the running network over) or an update of the
# agent, that is the next reboot. A second profile keeps the host connected
# meanwhile. Drop it once no device can still run such a g_ether.
OLD_HOST_MAC=02:61:72:69:73:02

nmcli -t -f UUID,TYPE connection show | while IFS=: read -r uuid type; do
    [ "$type" = 802-3-ethernet ] || continue
    # -g escapes the colons and prints upper case; normalise both.
    mac=$(nmcli -g 802-3-ethernet.mac-address connection show "$uuid" | tr -d '\\' | tr 'A-F' 'a-f')
    case "$mac" in
    "$HOST_MAC" | "$OLD_HOST_MAC") nmcli connection delete "$uuid" > /dev/null ;;
    esac
    true
done
for p in "$NAME $HOST_MAC" "$NAME-old $OLD_HOST_MAC"; do
    set -- $p
    nmcli connection add type ethernet con-name "$1" ifname '*' \
        ethernet.mac-address "$2" \
        ipv4.method manual ipv4.addresses 192.168.15.201/24 ipv4.never-default yes \
        ipv6.method disabled connection.autoconnect yes > /dev/null
done
echo "NetworkManager profiles $NAME and $NAME-old ready."
