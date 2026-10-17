#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Remove the Aristotle agent completely and give USB back to volumd. Run as
# root on the device: KUAL > Aristotle > Remove agent, or over SSH (the
# session drops at the end, when the network goes). Removes:
#
#  - the upstart jobs: /etc/upstart/aristotle-agent.conf, aristotle-ui.conf
#    (Aristotle at boot) and aristotle-ota.conf (the OTA block);
#  - the OTA block itself: the stubs over /usr/bin/otaupd and otav3 are
#    unmounted, and the daemons that were running run again for real;
#  - Aristotle started at boot, if it runs (it saves first; the stock GUI
#    comes back);
#  - /var/local/aristotle: the scripts, the state (USB mode, Wi-Fi policy and
#    allowlist, watchdog flags, autostart and its copy of the app), the
#    agent's copies of the SSH key and of dropbear, and the logs;
#  - /var/local/aristotle-migrate, if an earlier version of the agent left
#    one: backups from moving an older install over, with copies of the key;
#  - the agent's keys from KOReader's settings/SSH/authorized_keys (other keys
#    there stay) and the extension's authorized_keys (also under the
#    extension's earlier name, aristotle-usbnet);
#  - the runtime state: dropbear, usb0 and g_ether, the usb0 firewall rules,
#    the Wi-Fi policy chains.
#
# The logs are appended to /mnt/us/aristotle-agent.log on the USB drive
# first, for diagnosis; delete that file when it is no longer needed. Not
# guarded by the device check: removing always works. Safe to run twice.
set -u
# Keep going if the SSH session that started us goes away.
trap '' HUP PIPE

# R stands in for / in the host-side test (device/tests/agent.sh); it
# is empty on the device.
R=${AGENT_TEST_ROOT:-}
S=$R/var/local/aristotle
BACKUPS=$R/var/local/aristotle-migrate
JOBS="aristotle-agent aristotle-ui aristotle-ota"
EXT=$R/mnt/us/extensions/aristotle
# The extension's folder before it was renamed.
OLD_EXT=$R/mnt/us/extensions/aristotle-usbnet
KO_KEYS=$R/mnt/us/koreader/settings/SSH/authorized_keys
USB_LOG=$R/mnt/us/aristotle-agent.log

say() {
    printf '%s\n' "$*"
    eips 0 1 "$*                                        " > /dev/null 2>&1
}

running() {
    initctl status "$1" 2> /dev/null | grep -q start/running
}

# Aristotle started at boot: it saves on SIGTERM; then the stock GUI.
if running aristotle-ui; then
    stop aristotle-ui > /dev/null 2>&1
    sh "$S/takeover.sh" stop > /dev/null 2>&1
fi
stop aristotle-agent > /dev/null 2>&1
# Back to the stock firewall on Wi-Fi.
if [ -f "$S/net.sh" ]; then
    echo off > "$S/net-mode"
    sh "$S/net.sh" apply
fi
# The OTA block: the stubs go, and the daemons that ran (as stubs) run again
# for real.
for b in otaupd otav3; do
    grep -q " $R/usr/bin/$b " "$R/proc/mounts" 2> /dev/null || continue
    was=0
    if running "$b"; then
        was=1
        stop "$b" > /dev/null 2>&1
    fi
    umount "$R/usr/bin/$b" 2> /dev/null || umount -l "$R/usr/bin/$b"
    [ $was = 1 ] && start "$b" > /dev/null 2>&1
done

# The upstart jobs, in one mntroot window. Should mntroot fail, they stay
# but do nothing: each needs files in /var/local/aristotle, removed below.
job_left=0
present=
for j in $JOBS; do
    [ -e "$R/etc/upstart/$j.conf" ] && present="$present $j"
done
if [ -n "$present" ]; then
    if mntroot rw; then
        for j in $present; do
            rm -f "$R/etc/upstart/$j.conf"
        done
        sync
        mntroot ro
    else
        job_left=1
    fi
fi
initctl reload-configuration 2> /dev/null

# The logs, to the USB drive.
if [ -d "$R/mnt/us" ]; then
    for f in "$S/log.1" "$S/log" "$S/ui.log.1" "$S/ui.log" "$S/app.log.1" "$S/app.log" \
        "$BACKUPS/migrate.log"; do
        [ -s "$f" ] || continue
        printf '=== %s, saved by uninstall.sh on %s\n' "${f#"$R"}" "$(date '+%Y-%m-%d %H:%M:%S')" >> "$USB_LOG"
        cat "$f" >> "$USB_LOG"
    done
fi

# The agent's keys, wherever it put them. KOReader's file may hold keys of
# the owner's own; only lines equal to one of ours go.
keys=$R/var/run/aristotle-uninstall-keys
cat "$S/authorized_keys" "$EXT/authorized_keys" "$OLD_EXT/authorized_keys" 2> /dev/null | grep . > "$keys"
if [ -s "$keys" ] && [ -f "$KO_KEYS" ]; then
    grep -v -x -F -f "$keys" "$KO_KEYS" > "$KO_KEYS.new"
    if grep -q . "$KO_KEYS.new"; then
        mv -f "$KO_KEYS.new" "$KO_KEYS"
    else
        rm -f "$KO_KEYS.new" "$KO_KEYS"
    fi
fi
rm -f "$keys" "$EXT/authorized_keys" "$OLD_EXT/authorized_keys"

# The agent's dropbear, a manual USB network's, or an earlier version's
# (which used /var/run). Read before the directory goes.
pids=
for f in "$S/dropbear.pid" "$R/var/run/aristotle-dropbear.pid"; do
    p=$(cat "$f" 2> /dev/null)
    [ -n "$p" ] && grep -q dropbear "/proc/$p/cmdline" 2> /dev/null && pids="$pids $p"
done

rm -rf "$S" "$BACKUPS"

# Last, the network: an SSH session that started us ends here.
for p in $pids; do
    kill "$p" 2> /dev/null
done
rm -f "$R/var/run/aristotle-dropbear.pid" "$R/var/run/aristotle-takeover"
iptables -D INPUT -i usb0 -j ACCEPT 2> /dev/null
iptables -D OUTPUT -o usb0 -j ACCEPT 2> /dev/null
ifconfig usb0 down 2> /dev/null
lipc-set-prop com.lab126.volumd useUsbForNetwork 0 2> /dev/null
lsmod | grep -q '^g_ether' && rmmod g_ether

if [ $job_left = 1 ]; then
    say "aristotle agent removed; its jobs stay in /etc/upstart (mntroot failed) but no longer run"
    exit 1
fi
say "aristotle agent removed"
