#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Read-only runtime probe for the Kindle PW4. Collects kernel, hardware,
# process, upstart and lipc state into /mnt/us/aristotle-probe/<UTC stamp>/.
# Writes nothing outside that directory and changes no system state.
#
#   probe.sh        the snapshot; the e-ink waveforms are listed, not copied
#   probe.sh wfm    also copy the waveform files from /mnt/wfm
#
# The output identifies the device and its owner: the logs, dmesg and every
# lipc property (lipc-probe -a -v) can hold the serial number, MAC addresses,
# Wi-Fi network names and registration details. A README.txt beside it says
# so. Review it before sharing any of it; never attach it as is to a public
# issue. The waveforms are Amazon's proprietary data and stay private.
set -u
umask 022

COPY_WFM=0
[ "${1:-}" = wfm ] && COPY_WFM=1

OUT_ROOT=${ARISTOTLE_PROBE_OUT:-/mnt/us/aristotle-probe}
STAMP=$(date -u '+%Y%m%dT%H%M%SZ')
OUT="$OUT_ROOT/.partial-$STAMP"
mkdir -p "$OUT"
exec 2>> "$OUT/probe-errors.log"

# run NAME CMD...: capture stdout+stderr of a command into NAME.txt
run() {
    name=$1
    shift
    { printf '$ %s\n' "$*"; "$@"; printf '\n[exit %s]\n' "$?"; } > "$OUT/$name.txt" 2>&1
}

# grab FILE...: copy readable files, keeping their absolute path under files/
grab() {
    for f in "$@"; do
        [ -r "$f" ] && [ -f "$f" ] || continue
        mkdir -p "$OUT/files$(dirname "$f")"
        cat "$f" > "$OUT/files$f" 2> /dev/null
    done
}

# sysdump DIR NAME: print every readable regular attribute file under DIR
sysdump() {
    find "$1" -maxdepth 3 -type f 2> /dev/null | sort | while read -r f; do
        case "$f" in */power/*|*/subsystem/*) continue ;; esac
        v=$(head -c 512 "$f" 2> /dev/null | tr '\n' ' ') || continue
        printf '%s = %s\n' "$f" "$v"
    done > "$OUT/$2.txt"
}

eips 0 0 "aristotle probe running..." > /dev/null 2>&1

cat > "$OUT/README.txt" << EOF
Aristotle probe output, $STAMP

This is a read-only snapshot of one Kindle: kernel and system logs (dmesg,
/var/log/messages), every lipc property (lipc-probe -a -v), processes,
network state, upstart jobs, the app registry and more.

IT IDENTIFIES THE DEVICE AND ITS OWNER. The files can contain the serial
number, MAC addresses, Wi-Fi network names and addresses, and account or
registration details.

- Review every file before you share any of it, and share only the files
  that are needed, with the identifiers removed.
- Never attach this directory, or files from it as they are, to a public
  issue, pull request, forum post or chat.
EOF
if [ $COPY_WFM = 1 ]; then
    cat >> "$OUT/README.txt" << EOF
- files/mnt/wfm/ holds the e-ink waveforms copied from this device. They
  are Amazon's proprietary data: keep them to yourself, never share them.
EOF
fi

# Kernel and boot
run uname uname -a
run uptime cat /proc/uptime
run cmdline cat /proc/cmdline
run cpuinfo cat /proc/cpuinfo
run meminfo cat /proc/meminfo
run iomem cat /proc/iomem
run interrupts cat /proc/interrupts
run partitions cat /proc/partitions
run mounts cat /proc/mounts
run df df -k
run lsmod cat /proc/modules
run dmesg dmesg
run devices cat /proc/devices
run misc cat /proc/misc
grab /proc/config.gz /proc/version /etc/version.txt /etc/prettyversion.txt \
     /mnt/us/system/version.txt

# Input, display, light, power
run input-devices cat /proc/bus/input/devices
run dev-input ls -l /dev/input /dev/input/by-path /dev/input/by-id
run fb-sys sh -c 'for f in /sys/class/graphics/fb0/*; do [ -f "$f" ] && printf "%s = %s\n" "$f" "$(head -c 256 "$f" | tr "\n" " ")"; done'
[ -x /mnt/us/libkh/bin/fbink ] && run fbink /mnt/us/libkh/bin/fbink -e
sysdump /sys/class/backlight backlight
sysdump /sys/class/power_supply power-supply
sysdump /sys/bus/platform/drivers/hall_sensor hall
run thermal sh -c 'for z in /sys/class/thermal/*/; do printf "%s %s %s\n" "$z" "$(cat "$z/type" 2>/dev/null)" "$(cat "$z/temp" 2>/dev/null)"; done'
run cpufreq sh -c 'grep -H . /sys/devices/system/cpu/cpu0/cpufreq/* 2>/dev/null'
run i2c ls -l /sys/bus/i2c/devices/
run i2c-names sh -c 'for d in /sys/bus/i2c/devices/*; do printf "%s %s\n" "$d" "$(cat "$d/name" 2>/dev/null)"; done'
run platform-devices ls /sys/bus/platform/devices/

# E-ink waveform partition: list and checksums; the files themselves only
# with "wfm", for offline study.
run wfm sh -c 'ls -lR /mnt/wfm; find /mnt/wfm -type f -exec md5sum {} \;'
if [ $COPY_WFM = 1 ]; then
    find /mnt/wfm -type f -size -8192k 2> /dev/null | while read -r f; do grab "$f"; done
fi

# Storage layout
run mmc sh -c 'ls -l /dev/mmcblk* /dev/block 2>/dev/null; for b in /sys/block/mmcblk*; do printf "%s size=%s\n" "$b" "$(cat "$b/size")"; ls "$b"; done'

# Processes and memory
run ps ps
run proc-mem sh -c 'for p in /proc/[0-9]*; do n=$(tr "\0" " " < "$p/cmdline" 2>/dev/null | cut -c1-120); r=$(awk "/VmRSS/ {print \$2}" "$p/status" 2>/dev/null); u=$(awk "/^Uid/ {print \$2}" "$p/status" 2>/dev/null); printf "%s\t%s\t%s\t%s\n" "${p#/proc/}" "${r:-0}" "$u" "$n"; done | sort -t "	" -k2 -n -r'
run initctl initctl list
run listening sh -c 'netstat -anp 2>/dev/null || netstat -an'
run ip sh -c 'ip addr; ip route'

# lipc: every published service and its properties
run lipc-services lipc-probe -a
run lipc-props lipc-probe -a -v

# Persistent config and app registry (no book contents)
run var-local ls -la /var/local
if command -v sqlite3 > /dev/null 2>&1; then
    run appreg sqlite3 /var/local/appreg.db .dump
    run cc-schema sqlite3 /var/local/cc.db .schema
fi
grab /var/local/system/userpasswdenabled /etc/upstart/env /etc/lipc-daemon-props.conf \
     /etc/lipc-daemon-events.conf /var/log/messages /tmp/pillow.err

# Tool availability on device
run tools sh -c 'for t in sqlite3 strace gdb gdbserver lsof iw iwconfig wpa_cli i2cutil i2cdump evtest dropbear sshd busybox python lua; do printf "%s: %s\n" "$t" "$(command -v $t)"; done; busybox --list 2>/dev/null | tr "\n" " "'

printf 'complete\n' > "$OUT/COMPLETE"
mv "$OUT" "$OUT_ROOT/$STAMP"
printf '%s\n' "$STAMP" > "$OUT_ROOT/LATEST"
sync
eips 0 0 "aristotle probe done: $STAMP   " > /dev/null 2>&1
