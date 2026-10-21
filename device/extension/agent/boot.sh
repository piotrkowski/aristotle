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
# State and logs live in /var/local/aristotle (persistent ext3), which only
# root may enter (device/README.md, "Security"). The job starts this script
# with that directory as the current one and ARISTOTLE_DIR pointing at it
# through /proc, so what runs cannot be swapped later in the boot. SSH runs
# the agent's own copy of KOReader's dropbear from there, with its own keys:
# nothing on the USB drive, which every process may write, runs as root.
# Nothing here writes to the root filesystem.
set -u
umask 077

# R stands in for / in the host-side test (device/tests/agent.sh); it
# is empty on the device.
R=${AGENT_TEST_ROOT:-}
S=${ARISTOTLE_DIR:-$R/var/local/aristotle}
LOG=$S/log
KO=$R/mnt/us/koreader
PIDFILE=$S/dropbear.pid
DISABLE=$R/mnt/us/aristotle-disable
TAKEOVER=$R/var/run/aristotle-takeover
# Wait for the userstore before allowing any agent action.
MOUNT_WAIT=${MOUNT_WAIT:-60}
DEV_IP=192.168.15.244
# Fixed MACs for both ends of the USB link, so the host's NetworkManager
# profile (tools/host/setup-host-net.sh) can match the interface: 02, the
# locally administered unicast prefix, then "aris" in ASCII and the end of
# the link. Any locally administered unicast address would do. bin/start.sh
# and setup-host-net.sh repeat them; change all three together.
DEV_MAC=02:61:72:69:73:01
HOST_MAC=02:61:72:69:73:02
TICK=60
GUI_GRACE=300

log() {
    printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG"
    # Keep the log small: one rotated copy.
    if [ "$(wc -c < "$LOG")" -gt 262144 ]; then
        mv -f "$LOG" "$LOG.1"
    fi
}

usb_mode() {
    requested=net until_epoch=0
    [ ! -r "$S/usb-mode" ] || read -r requested until_epoch < "$S/usb-mode"
    case "$requested" in
    storage)
        # "storage UNTIL_EPOCH": fall back to net once the time is up.
        if [ "$(date +%s)" -lt "${until_epoch:-0}" ]; then echo storage; else echo net; fi
        ;;
    *) echo net ;;
    esac
}

# The pid file is on persistent storage: after a reboot its pid may belong to
# another process, so check the name too.
dropbear_alive() {
    p=
    [ ! -r "$PIDFILE" ] || read -r p < "$PIDFILE"
    [ -n "$p" ] && kill -0 "$p" 2> /dev/null && grep -q dropbear "/proc/$p/cmdline" 2> /dev/null
}

net_healthy() {
    flags=0
    [ -r "$R/sys/class/net/usb0/flags" ] || return 1
    read -r flags < "$R/sys/class/net/usb0/flags" || return 1
    [ $((flags & 1)) -ne 0 ] && dropbear_alive
}

net_up() {
    # Tell volumd the USB port is for networking, so it doesn't start drive
    # mode when the cable is plugged in.
    if ! timeout -t 3 -s KILL lipc-set-prop com.lab126.volumd useUsbForNetwork 1 2> /dev/null; then
        log "net: volumd did not accept network mode; retrying later"
        return 1
    fi
    if lsmod | grep -q '^g_mass_storage'; then
        if ! rmmod g_mass_storage 2>> "$LOG"; then
            log "net: storage gadget is busy; eject the host drive before retrying"
            return 1
        fi
        log "net: unloaded g_mass_storage"
    fi
    if ! restore_userstore; then
        return 1
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
    dropbear_alive && return 0
    # The agent's copy (install.sh); KOReader's if there is none, or if the
    # copy does not start, so SSH never depends on the copy alone.
    if [ -x "$S/dropbear" ]; then
        start_dropbear "$S/dropbear" "$S/ssh" && return 0
        log "ssh: the agent's copy of dropbear did not start; trying KOReader's"
    fi
    if [ -x "$KO/dropbear" ]; then
        [ -x "$S/dropbear" ] || log "ssh: no copy of dropbear in $S; using KOReader's (install the agent again)"
        start_dropbear "$KO/dropbear" "$KO" && return 0
    else
        log "ssh: no dropbear available yet"
    fi
    return 1
}

base_mounted() {
    [ -r "$R/proc/mounts" ] || return 2
    awk -v point="$R/mnt/base-us" '$2 == point { found=1 } END { exit !found }' "$R/proc/mounts"
}

storage_gadget_absent() {
    modules=$(lsmod 2>> "$LOG") || { log "net: cannot read module state; not mounting userstore"; return 1; }
    if printf '%s\n' "$modules" | grep -q '^g_mass_storage'; then
        log "net: storage gadget present; not mounting userstore"
        return 1
    fi
}

restore_userstore() {
    deadline=$(($(cut -d. -f1 /proc/uptime) + 5))
    while :; do
        storage_gadget_absent || return 1
        if [ "$(cut -d. -f1 /proc/uptime)" -ge "$deadline" ]; then
            log "net: drive mode did not clear; not mounting userstore"
            return 1
        fi
        drive=$(timeout -t 1 -s KILL lipc-get-prop com.lab126.volumd driveModeState 2> /dev/null) || drive=unknown
        [ "$drive" = 0 ] && break
        sleep 1
    done
    if base_mounted; then
        :
    else
        if [ "$?" != 1 ]; then
            log "net: cannot read backing mount state; retrying later"
            return 1
        fi
        storage_gadget_absent || return 1
        if ! "$R/usr/sbin/mntus" mount >> "$LOG" 2>&1; then
            log "net: backing userstore mount failed; retrying later"
            return 1
        fi
        log "net: requested backing userstore mount"
    fi
    if ! base_mounted || [ ! -d "$R/mnt/us/." ]; then
        log "net: userstore is not ready after mount; retrying later"
        return 1
    fi
}

# start_dropbear BINARY HOME
start_dropbear() {
    mkdir -p "$2/settings/SSH"
    cp "$S/authorized_keys" "$2/settings/SSH/authorized_keys" 2>> "$LOG"
    chmod 600 "$2/settings/SSH/authorized_keys"
    # KOReader's dropbear reads settings/SSH/authorized_keys relative to its
    # working directory and keeps its host keys there (-R). -s: keys only;
    # -p ADDRESS:PORT: listen on usb0's address only, never on Wi-Fi.
    if (cd "$2" && "$1" -E -R -s -p "$DEV_IP:22" -P "$PIDFILE" >> "$LOG" 2>&1); then
        # dropbear writes its pid file after forking; give it a moment.
        sleep 1
        if dropbear_alive; then
            log "ssh: dropbear started from $1 (pid $(cat "$PIDFILE" 2> /dev/null))"
            return 0
        fi
    fi
    log "ssh: $1 failed to start"
    return 1
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
        log "agent: /mnt/us not mounted after ${MOUNT_WAIT}s; cannot check kill switch; staying idle"
        lipc-set-prop com.lab126.volumd useUsbForNetwork 0 2> /dev/null
        exec sleep 2147483647
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
net_ready=0
while true; do
    mode=$(usb_mode)
    if [ "$mode" != "$mode_was" ]; then
        log "mode: $mode"
        mode_was=$mode
        net_ready=0
    fi
    case "$mode" in
    net)
        if [ "$net_ready" = 0 ] || ! net_healthy; then
            net_ready=0
            net_up && net_ready=1
        fi
        ;;
    storage)
        if [ "$net_ready" = 0 ]; then
            storage_up && net_ready=1
        fi
        ;;
    esac
    gui_check
    policy=off
    [ ! -r "$S/net-mode" ] || read -r policy < "$S/net-mode"
    [ "$policy" != scoped ] || sh "$S/net.sh" tick 2>> "$LOG"
    if [ "$gui_good" -gt 600 ]; then
        rm -f "$S/gui-restarted"
        gui_good=0
    fi
    sleep "$TICK"
done
