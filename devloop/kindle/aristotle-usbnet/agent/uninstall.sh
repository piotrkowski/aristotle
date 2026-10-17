#!/bin/sh
# Remove the Aristotle agent completely and give USB back to volumd. Run as
# root on the device: KUAL > Aristotle > Remove agent, or over SSH (the
# session drops at the end, when the network goes). Removes:
#
#  - the upstart job, /etc/upstart/aristotle-agent.conf;
#  - /var/local/aristotle: the scripts, the state (USB mode, Wi-Fi policy and
#    allowlist, watchdog flags), the agent's copy of the SSH key and the log;
#  - /var/local/aristotle-migrate, the backups of a migration from an earlier
#    install (they hold copies of the key too);
#  - the agent's keys from KOReader's settings/SSH/authorized_keys (other keys
#    there stay) and the extension's authorized_keys;
#  - the runtime state: dropbear, usb0 and g_ether, the usb0 firewall rules,
#    the Wi-Fi policy chains.
#
# The log is appended to /mnt/us/aristotle-agent.log on the USB drive first,
# for diagnosis; delete that file when it is no longer needed. Not guarded by
# the device check: removing always works. Safe to run twice.
set -u
# Keep going if the SSH session that started us goes away.
trap '' HUP PIPE

# R stands in for / in the host-side test (devloop/kindle/tests/agent.sh); it
# is empty on the device.
R=${AGENT_TEST_ROOT:-}
S=$R/var/local/aristotle
BACKUPS=$R/var/local/aristotle-migrate
JOB=$R/etc/upstart/aristotle-agent.conf
PIDFILE=$R/var/run/aristotle-dropbear.pid
EXT=$R/mnt/us/extensions/aristotle-usbnet
KO_KEYS=$R/mnt/us/koreader/settings/SSH/authorized_keys
USB_LOG=$R/mnt/us/aristotle-agent.log

say() {
    printf '%s\n' "$*"
    eips 0 1 "$*                                        " > /dev/null 2>&1
}

stop aristotle-agent > /dev/null 2>&1
# Back to the stock firewall on Wi-Fi.
if [ -f "$S/net.sh" ]; then
    echo off > "$S/net-mode"
    sh "$S/net.sh" apply
fi

# The upstart job. Should mntroot fail, the job stays but does nothing: its
# pre-start check needs /var/local/aristotle/boot.sh, removed below.
job_left=0
if [ -e "$JOB" ]; then
    if mntroot rw; then
        rm -f "$JOB"
        sync
        mntroot ro
    else
        job_left=1
    fi
fi
initctl reload-configuration 2> /dev/null

# The log, to the USB drive.
if [ -d "$R/mnt/us" ]; then
    for f in "$S/log.1" "$S/log" "$BACKUPS/migrate.log"; do
        [ -s "$f" ] || continue
        printf '=== %s, saved by uninstall.sh on %s\n' "${f#"$R"}" "$(date '+%Y-%m-%d %H:%M:%S')" >> "$USB_LOG"
        cat "$f" >> "$USB_LOG"
    done
fi

# The agent's keys, wherever it put them. KOReader's file may hold keys of
# the owner's own; only lines equal to one of ours go.
keys=$R/var/run/aristotle-uninstall-keys
cat "$S/authorized_keys" "$EXT/authorized_keys" 2> /dev/null | grep . > "$keys"
if [ -s "$keys" ] && [ -f "$KO_KEYS" ]; then
    grep -v -x -F -f "$keys" "$KO_KEYS" > "$KO_KEYS.new"
    if grep -q . "$KO_KEYS.new"; then
        mv -f "$KO_KEYS.new" "$KO_KEYS"
    else
        rm -f "$KO_KEYS.new" "$KO_KEYS"
    fi
fi
rm -f "$keys" "$EXT/authorized_keys"

rm -rf "$S" "$BACKUPS"

# Last, the network: an SSH session that started us ends here.
[ -f "$PIDFILE" ] && kill "$(cat "$PIDFILE")" 2> /dev/null
rm -f "$PIDFILE"
iptables -D INPUT -i usb0 -j ACCEPT 2> /dev/null
iptables -D OUTPUT -o usb0 -j ACCEPT 2> /dev/null
ifconfig usb0 down 2> /dev/null
lipc-set-prop com.lab126.volumd useUsbForNetwork 0 2> /dev/null
lsmod | grep -q '^g_ether' && rmmod g_ether

if [ $job_left = 1 ]; then
    say "aristotle agent removed; $JOB left (mntroot failed), it no longer runs"
    exit 1
fi
say "aristotle agent removed"
