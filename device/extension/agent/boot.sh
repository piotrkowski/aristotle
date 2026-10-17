#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Aristotle agent, started by /etc/upstart/aristotle-agent.conf on every boot.
#
# Kill switch: a file named aristotle-disable at the root of the USB drive
# (/mnt/us/aristotle-disable) stops the agent before it does anything on this
# boot: no USB network, no SSH, no watchdog, no Wi-Fi policy. USB stays with
# volumd and the stock system. It is the way out without SSH: connect the
# Kindle as a USB drive, create the file, eject and reboot. While the agent
# runs, the USB port is a network link, so switch it to drive mode first
# (KUAL > Aristotle > USB drive mode for 10 minutes, or the reader's
# settings). Delete the file and reboot to have the agent back.
#
# Otherwise it keeps the device reachable and healthy without anyone touching
# it:
# - USB network (g_ether, usb0 at 192.168.15.244) and key-only SSH (KOReader's
#   dropbear) whenever the USB mode is "net", the default;
# - "storage" mode for a limited time on request (aristotlectl mode storage N),
#   then back to net automatically;
# - a watchdog that restarts the stock GUI if it stays dead for 5 minutes, and
#   reboots at most once an hour if that does not help. It stays out of the
#   way while Aristotle has taken over (/var/run/aristotle-takeover exists;
#   /var is a tmpfs, so a reboot clears it and the watchdog is back);
# - the Wi-Fi policy (net.sh, device/README.md, "Network"): nothing in "off", the default;
#   in "scoped", wlan0 limited to the hosts in net-allow, re-applied when the
#   address, the resolvers or the list change.
#
# State and logs live in /var/local/aristotle (persistent ext3). Nothing here
# writes to the root filesystem.
set -u
umask 022

# R stands in for / in the host-side test (device/tests/agent.sh); it
# is empty on the device.
R=${AGENT_TEST_ROOT:-}
S=$R/var/local/aristotle
LOG=$S/log
KO=$R/mnt/us/koreader
PIDFILE=$R/var/run/aristotle-dropbear.pid
DISABLE=$R/mnt/us/aristotle-disable
TAKEOVER=$R/var/run/aristotle-takeover
# How long to wait for the userstore (/mnt/us) before giving up on the kill
# switch check.
MOUNT_WAIT=${MOUNT_WAIT:-60}
DEV_IP=192.168.15.244
# Fixed MACs for both ends of the USB link, so the host's NetworkManager
# profile (tools/host/setup-host-net.sh) can match the interface: 02, the
# locally administered unicast prefix, then "aris" in ASCII and the end of
# the link. Any locally administered unicast address would do. bin/start.sh
# and setup-host-net.sh repeat them; change all three together.
DEV_MAC=02:61:72:69:73:01
HOST_MAC=02:61:72:69:73:02
TICK=15
GUI_GRACE=300

log() {
    printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG"
    # Keep the log small: one rotated copy.
    if [ "$(wc -c < "$LOG")" -gt 262144 ]; then
        mv -f "$LOG" "$LOG.1"
    fi
}

usb_mode() {
    set -- $(cat "$S/usb-mode" 2> /dev/null)
    case "${1:-net}" in
    storage)
        # "storage UNTIL_EPOCH": fall back to net once the time is up.
        if [ "$(date +%s)" -lt "${2:-0}" ]; then echo storage; else echo net; fi
        ;;
    *) echo net ;;
    esac
}

dropbear_alive() {
    [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2> /dev/null
}

net_up() {
    # Tell volumd the USB port is for networking, so it doesn't start drive
    # mode when the cable is plugged in.
    lipc-set-prop com.lab126.volumd useUsbForNetwork 1 2> /dev/null
    if lsmod | grep -q '^g_mass_storage'; then
        rmmod g_mass_storage 2>> "$LOG" && log "net: unloaded g_mass_storage"
    fi
    if ! lsmod | grep -q '^g_ether'; then
        if modprobe g_ether dev_addr="$DEV_MAC" host_addr="$HOST_MAC" 2>> "$LOG"; then
            log "net: g_ether loaded"
        else
            log "net: g_ether failed to load"
            return 1
        fi
    fi
    if ! ifconfig usb0 2> /dev/null | grep -q "inet addr:$DEV_IP"; then
        ifconfig usb0 "$DEV_IP" netmask 255.255.255.0 up 2>> "$LOG" && log "net: usb0 up"
    fi
    iptables -C INPUT -i usb0 -j ACCEPT 2> /dev/null || iptables -I INPUT -i usb0 -j ACCEPT
    iptables -C OUTPUT -o usb0 -j ACCEPT 2> /dev/null || iptables -I OUTPUT -o usb0 -j ACCEPT
    if ! dropbear_alive; then
        if [ ! -x "$KO/dropbear" ]; then
            log "ssh: $KO/dropbear not available yet"
            return 1
        fi
        mkdir -p "$KO/settings/SSH"
        cp "$S/authorized_keys" "$KO/settings/SSH/authorized_keys" 2>> "$LOG"
        chmod 600 "$KO/settings/SSH/authorized_keys"
        # KOReader's dropbear reads settings/SSH/authorized_keys relative to
        # its working directory and keeps its host keys there (-R). -s: keys
        # only; -p ADDRESS:PORT: listen on usb0's address only, never on
        # Wi-Fi.
        if (cd "$KO" && ./dropbear -E -R -s -p "$DEV_IP:22" -P "$PIDFILE" >> "$LOG" 2>&1); then
            # dropbear writes its pid file after forking; give it a moment.
            sleep 1
            log "ssh: dropbear started (pid $(cat "$PIDFILE" 2> /dev/null))"
        else
            log "ssh: dropbear failed to start"
        fi
    fi
}

net_down() {
    [ -f "$PIDFILE" ] && kill "$(cat "$PIDFILE")" 2> /dev/null
    rm -f "$PIDFILE"
    ifconfig usb0 down 2> /dev/null
    lsmod | grep -q '^g_ether' && rmmod g_ether
}

storage_up() {
    lsmod | grep -q '^g_ether' && net_down && log "storage: network down"
    lipc-set-prop com.lab126.volumd useUsbForNetwork 0 2> /dev/null
    # Same module and ids volumd loads for drive mode on Rex.
    if ! lsmod | grep -q '^g_mass_storage'; then
        modprobe g_mass_storage file="" removable=1 idVendor=0x1949 idProduct=0x0324 2>> "$LOG" &&
            log "storage: g_mass_storage loaded"
    fi
}

# The framework process is /usr/java/bin/cvm; match the path, the short name
# alone proved unreliable on the device.
gui_alive() {
    pgrep -f /usr/java/bin/cvm > /dev/null && pgrep -x Xorg > /dev/null
}

gui_bad=0
gui_good=0
gui_check() {
    if [ -e "$TAKEOVER" ]; then
        gui_bad=0
        return
    fi
    if gui_alive; then
        gui_bad=0
        gui_good=$((gui_good + TICK))
        return
    fi
    up=$(cut -d. -f1 "$R/proc/uptime")
    [ "$up" -lt "$GUI_GRACE" ] && return
    gui_bad=$((gui_bad + TICK))
    [ "$gui_bad" -lt "$GUI_GRACE" ] && return
    gui_bad=0
    last=$(cat "$S/last-reboot" 2> /dev/null || echo 0)
    now=$(date +%s)
    if [ -f "$S/gui-restarted" ] && [ $((now - last)) -gt 3600 ]; then
        log "watchdog: stock GUI still dead after a restart; rebooting"
        echo "$now" > "$S/last-reboot"
        rm -f "$S/gui-restarted"
        sync
        reboot
    else
        log "watchdog: stock GUI dead for ${GUI_GRACE}s (cvm or Xorg missing); restarting it"
        touch "$S/gui-restarted"
        stop lab126_gui > /dev/null 2>&1
        sleep 5
        start lab126_gui > /dev/null 2>&1
    fi
}

log "agent: boot (uptime $(cut -d' ' -f1 "$R/proc/uptime") s)"

# The kill switch, before anything else. The userstore may be mounted a
# moment after the agent starts.
i=0
until grep -q ' /mnt/us ' "$R/proc/mounts" 2> /dev/null; do
    if [ $i -ge "$MOUNT_WAIT" ]; then
        log "agent: /mnt/us not mounted after ${MOUNT_WAIT}s; kill switch not checked"
        break
    fi
    sleep 1
    i=$((i + 1))
done
if [ -e "$DISABLE" ]; then
    log "agent: /mnt/us/aristotle-disable exists; not starting (no USB network, SSH, watchdog or Wi-Fi policy). Delete it and reboot to enable the agent."
    # A network setting the agent left must not keep volumd from drive mode.
    lipc-set-prop com.lab126.volumd useUsbForNetwork 0 2> /dev/null
    # Idle instead of exiting: upstart respawns a job whose process ends.
    exec sleep 2147483647
fi

# Earlier versions kept the takeover flag here, on persistent storage, where a
# reboot during a takeover left it behind and the watchdog off for good.
if [ -e "$S/takeover" ]; then
    rm -f "$S/takeover"
    log "agent: removed a takeover flag left in $S"
fi
# iptables rules do not survive a reboot; make the first tick apply the
# Wi-Fi policy again.
rm -f "$S/net-sig"
mode_was=""
while true; do
    mode=$(usb_mode)
    if [ "$mode" != "$mode_was" ]; then
        log "mode: $mode"
        mode_was=$mode
    fi
    case "$mode" in
    net) net_up ;;
    storage) storage_up ;;
    esac
    gui_check
    [ -f "$S/net.sh" ] && sh "$S/net.sh" tick 2>> "$LOG"
    if [ "$gui_good" -gt 600 ]; then
        rm -f "$S/gui-restarted"
        gui_good=0
    fi
    sleep "$TICK"
done
