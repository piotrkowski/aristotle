#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Host-side test of the device scripts: the device guard
# (aristotle-usbnet/guard.sh) in install.sh, bin/start.sh and takeover.sh,
# uninstall.sh, the kill switch in boot.sh, and the takeover flag.
#
#   sh devloop/kindle/tests/agent.sh
#
# Each case builds a fake device root in a fresh directory under $TMPDIR: the
# KUAL extension copied to mnt/us/extensions/ as install-extensions.sh does,
# /proc/usid and /etc/prettyversion.txt, and stub device commands on PATH that
# record their calls in the root's `calls` file. The real scripts run against
# it (AGENT_TEST_ROOT). Nothing outside the fresh directory is touched.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
KINDLE=$(cd "$HERE/.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/aristotle-agent-test.XXXXXX")
failures=0

# A PW4 serial (device code 0PP in characters 4-6) and a PW5 one (1Q0); both
# made up for the test.
PW4_SN=G000PP0123456789
PW5_SN=G001Q00123456789

cleanup() {
    for p in $(cat "$WORK"/*/jobs/*.pid "$WORK"/*/var/run/*.pid 2> /dev/null); do
        kill "$p" 2> /dev/null
    done
}
trap cleanup EXIT

check() {
    if eval "$2"; then
        printf '  ok    %s\n' "$1"
    else
        printf '  FAIL  %s\n' "$1"
        failures=$((failures + 1))
    fi
}

# stubs DIR: the device commands the scripts call. Each one appends its
# command line to $AGENT_TEST_ROOT/calls.
stubs() {
    mkdir -p "$1"
    for c in eips mntroot lipc-set-prop iptables ifconfig modprobe rmmod sync reboot; do
        cat > "$1/$c" << 'EOF'
#!/bin/sh
echo "${0##*/} $*" >> "$AGENT_TEST_ROOT/calls"
case "${0##*/}:$1" in
# No rule or chain exists yet, as on a fresh boot.
iptables:-C | iptables:-D | iptables:-F | iptables:-X) exit 1 ;;
mntroot:*) [ "${FAKE_MNTROOT_FAIL:-}" != "$1" ] ;;
esac
EOF
    done
    printf '#!/bin/sh\necho "lsmod" >> "$AGENT_TEST_ROOT/calls"\ncat "$AGENT_TEST_ROOT/modules" 2> /dev/null\n' > "$1/lsmod"
    printf '#!/bin/sh\necho 0\n' > "$1/id"
    cat > "$1/initctl" << 'EOF'
#!/bin/sh
R=$AGENT_TEST_ROOT
echo "initctl $*" >> "$R/calls"
case "$1" in
reload-configuration) exit 0 ;;
status)
    [ -f "$R/etc/upstart/$2.conf" ] || [ -f "$R/jobs/$2.pid" ] || { echo "initctl: Unknown job: $2" >&2; exit 1; }
    if [ -f "$R/jobs/$2.pid" ] && kill -0 "$(cat "$R/jobs/$2.pid")" 2> /dev/null; then
        echo "$2 start/running, process $(cat "$R/jobs/$2.pid")"
    else
        echo "$2 stop/waiting"
    fi
    ;;
esac
EOF
    cat > "$1/start" << 'EOF'
#!/bin/sh
R=$AGENT_TEST_ROOT
echo "start $*" >> "$R/calls"
case "$1" in aristotle-agent) ;; *) exit 0 ;; esac
[ -f "$R/etc/upstart/$1.conf" ] || { echo "start: Unknown job: $1" >&2; exit 1; }
mkdir -p "$R/jobs"
sleep 300 > /dev/null 2>&1 &
echo $! > "$R/jobs/$1.pid"
EOF
    cat > "$1/stop" << 'EOF'
#!/bin/sh
R=$AGENT_TEST_ROOT
echo "stop $*" >> "$R/calls"
[ -f "$R/jobs/$1.pid" ] || exit 1
kill "$(cat "$R/jobs/$1.pid")" 2> /dev/null
rm -f "$R/jobs/$1.pid"
EOF
    chmod 755 "$1"/*
}

# device NAME SERIAL FIRMWARE: a fake root with the KUAL extension, KOReader's
# dropbear and the given /proc/usid and prettyversion (none when empty).
# Prints the root.
device() {
    r="$WORK/$1"
    mkdir -p "$r/proc" "$r/etc/upstart" "$r/var/local" "$r/var/run" "$r/mnt/us/extensions" \
        "$r/mnt/us/koreader" "$r/jobs"
    [ -n "$2" ] && printf '%s\n' "$2" > "$r/proc/usid"
    [ -n "$3" ] && printf 'Kindle %s (~~otaVersion~~)\n\n\n\n' "$3" > "$r/etc/prettyversion.txt"
    printf '10.00 5.00\n' > "$r/proc/uptime"
    cp -R "$KINDLE/aristotle-usbnet" "$r/mnt/us/extensions/"
    cp "$KINDLE/takeover.sh" "$r/mnt/us/extensions/aristotle-usbnet/"
    printf 'ssh-ed25519 AAAA test\n' > "$r/mnt/us/extensions/aristotle-usbnet/authorized_keys"
    cat > "$r/mnt/us/koreader/dropbear" << 'EOF'
#!/bin/sh
echo "dropbear $*" >> "$AGENT_TEST_ROOT/calls"
while [ $# -gt 0 ]; do [ "$1" = -P ] && pid=$2; shift; done
sleep 300 > /dev/null 2>&1 &
echo $! > "$pid"
EOF
    chmod 755 "$r/mnt/us/koreader/dropbear"
    echo "$r"
}

# run ROOT SCRIPT [ARGS]: SCRIPT relative to the extension on the fake root.
run() {
    r=$1
    s=$2
    shift 2
    AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" \
        sh "$r/mnt/us/extensions/aristotle-usbnet/$s" "$@" > "$r.out" 2>&1
}

running() {
    AGENT_TEST_ROOT=$1 "$WORK/bin/initctl" status "$2" 2> /dev/null | grep -q start/running
}

called() {
    grep -q "^$2" "$1/calls" 2> /dev/null
}

stubs "$WORK/bin"

echo "guard: install on a PW4 with 5.13.6"
r=$(device pw4 "$PW4_SN" 5.13.6)
run "$r" agent/install.sh
check "exit status 0" "[ $? = 0 ]"
check "job file installed" "[ -f $r/etc/upstart/aristotle-agent.conf ]"
check "agent scripts installed" "[ -x $r/var/local/aristotle/boot.sh ] && [ -s $r/var/local/aristotle/authorized_keys ]"
check "agent started" "called $r 'start aristotle-agent'"

# refused NAME SERIAL FIRMWARE SCRIPT REASON: SCRIPT must exit 1, say REASON
# on stdout/stderr and on screen, and change nothing.
refused() {
    r=$(device "$1" "$2" "$3")
    run "$r" $4
    check "exit status 1" "[ $? = 1 ]"
    check "says why" "grep -q '$5' $r.out"
    check "shows it on screen" "grep -q '^eips 0 1 aristotle: .*$5' $r/calls"
    check "nothing written" "[ ! -e $r/var/local/aristotle ] && [ ! -e $r/etc/upstart/aristotle-agent.conf ]"
    check "no root filesystem, module or job command" "! grep -q -e '^mntroot' -e '^modprobe' -e '^rmmod' -e '^start' -e '^stop' -e '^iptables' $r/calls"
    [ -n "$2" ] && check "serial number not printed" "! grep -q '$2' $r.out $r/calls"
}

echo "guard: install on a PW5"
refused pw5 "$PW5_SN" 5.13.6 agent/install.sh "not a Kindle Paperwhite 4 (device code 1Q0)"
echo "guard: install on a PW4 with 5.16.2"
refused fw "$PW4_SN" 5.16.2 agent/install.sh "firmware 5.16.2, not 5.13.x"
echo "guard: install without prettyversion.txt"
refused fwfmt "$PW4_SN" "" agent/install.sh "firmware unknown"
echo "guard: install on an old-scheme serial (Kindle Touch)"
refused old B011000000000000 5.3.7 agent/install.sh "device code 11"
echo "guard: install off a Kindle (no /proc/usid)"
refused host "" 5.13.6 agent/install.sh "device code unknown"
echo "guard: bin/start.sh on a PW4 with 5.16.2"
r=$(device start "$PW4_SN" 5.16.2)
run "$r" bin/start.sh
check "exit status 1" "[ $? = 1 ]"
check "says why in its log" "grep -q 'firmware 5.16.2' $r/mnt/us/extensions/aristotle-usbnet/usbnet.log"
check "no module, firewall or dropbear" "! grep -q -e '^modprobe' -e '^rmmod' -e '^iptables' -e '^dropbear' $r/calls"
echo "guard: takeover.sh start on a PW5"
refused takeover "$PW5_SN" 5.13.6 "takeover.sh start" "not a Kindle Paperwhite 4"
check "stock GUI not stopped" "! called $r 'stop lab126_gui'"
echo "guard: takeover.sh on stdin finds the installed extension's guard"
r=$(device stdin "$PW5_SN" 5.13.6)
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" sh -s start < "$KINDLE/takeover.sh" > "$r.out" 2>&1
check "exit status 1" "[ $? = 1 ]"
check "stock GUI not stopped" "! called $r 'stop lab126_gui'"
run "$r" takeover.sh stop
check "stop always runs" "[ $? = 0 ] && called $r 'start lab126_gui'"
echo "guard: ARISTOTLE_FORCE=1 overrides"
r=$(device force "$PW5_SN" 5.16.2)
ARISTOTLE_FORCE=1 run "$r" agent/install.sh
check "exit status 0" "[ $? = 0 ]"
check "job file installed" "[ -f $r/etc/upstart/aristotle-agent.conf ]"

# installed NAME: a PW4 with the agent installed and running, its dropbear
# up, g_ether loaded, a log and its rotated copy, a scoped Wi-Fi policy, the
# backup directory an earlier version could leave, and KOReader's authorized_keys holding the agent's key
# and one of the owner's.
installed() {
    r=$(device "$1" "$PW4_SN" 5.13.6)
    run "$r" agent/install.sh
    S=$r/var/local/aristotle
    printf '2026-01-01 00:00:00 agent: boot\n' > "$S/log"
    printf '2025-12-31 00:00:00 older lines\n' > "$S/log.1"
    echo scoped > "$S/net-mode"
    echo example.org > "$S/net-allow"
    mkdir -p "$r/mnt/us/koreader/settings/SSH" "$r/var/local/aristotle-migrate/20260101T000000Z"
    printf 'ssh-ed25519 OWNER own\nssh-ed25519 AAAA test\n' > "$r/mnt/us/koreader/settings/SSH/authorized_keys"
    printf 'ssh-ed25519 AAAA test\n' > "$r/var/local/aristotle-migrate/20260101T000000Z/authorized_keys"
    printf 'migrated\n' > "$r/var/local/aristotle-migrate/migrate.log"
    sleep 300 > /dev/null 2>&1 &
    echo $! > "$r/var/run/aristotle-dropbear.pid"
    echo 'g_ether 40960 0' > "$r/modules"
    echo "$r"
}

echo "uninstall removes everything"
r=$(installed uninstall)
dropbear=$(cat "$r/var/run/aristotle-dropbear.pid")
run "$r" agent/uninstall.sh
check "exit status 0" "[ $? = 0 ]"
check "job file removed" "[ ! -e $r/etc/upstart/aristotle-agent.conf ] && [ \"\$(grep '^mntroot' $r/calls | tail -n 2 | tr '\n' ' ')\" = 'mntroot rw mntroot ro ' ]"
check "agent stopped" "called $r 'stop aristotle-agent' && ! running $r aristotle-agent"
check "Wi-Fi policy back to off" "called $r 'iptables -F ARISTOTLE_IN'"
check "state directory removed, key included" "[ ! -e $r/var/local/aristotle ]"
check "migration backups removed" "[ ! -e $r/var/local/aristotle-migrate ]"
check "log saved to the USB drive" "grep -q 'agent: boot' $r/mnt/us/aristotle-agent.log && grep -q 'older lines' $r/mnt/us/aristotle-agent.log && grep -q migrated $r/mnt/us/aristotle-agent.log"
check "agent key gone from KOReader, the owner's kept" "[ \"\$(cat $r/mnt/us/koreader/settings/SSH/authorized_keys)\" = 'ssh-ed25519 OWNER own' ]"
check "extension key removed" "[ ! -e $r/mnt/us/extensions/aristotle-usbnet/authorized_keys ]"
check "dropbear stopped" "! kill -0 $dropbear 2> /dev/null && [ ! -e $r/var/run/aristotle-dropbear.pid ]"
check "USB back to volumd" "called $r 'lipc-set-prop com.lab126.volumd useUsbForNetwork 0' && called $r 'rmmod g_ether'"
check "usb0 firewall holes closed" "called $r 'iptables -D INPUT -i usb0 -j ACCEPT' && called $r 'iptables -D OUTPUT -o usb0 -j ACCEPT'"
check "nothing left in /var/run" "[ -z \"\$(ls $r/var/run)\" ]"
check "shows it on screen" "called $r 'eips 0 1 aristotle agent removed'"
cp "$r/mnt/us/aristotle-agent.log" "$r.usb-log"
run "$r" agent/uninstall.sh
check "running it again is harmless" "[ $? = 0 ] && cmp -s $r.usb-log $r/mnt/us/aristotle-agent.log"

echo "uninstall: KOReader's keys were only the agent's"
r=$(installed onlyours)
printf 'ssh-ed25519 AAAA test\n' > "$r/mnt/us/koreader/settings/SSH/authorized_keys"
run "$r" agent/uninstall.sh
check "KOReader's authorized_keys removed" "[ ! -e $r/mnt/us/koreader/settings/SSH/authorized_keys ] && [ ! -e $r/mnt/us/koreader/settings/SSH/authorized_keys.new ]"

echo "uninstall: mntroot rw fails"
r=$(installed mntroot)
FAKE_MNTROOT_FAIL=rw run "$r" agent/uninstall.sh
check "exit status 1" "[ $? = 1 ]"
check "says the job file stayed" "grep -q 'mntroot failed' $r.out"
check "state still removed, so the job cannot run" "[ ! -e $r/var/local/aristotle ]"

# boot ROOT [MOUNT_WAIT]: start the installed agent's boot.sh in the
# background, as upstart would; its pid goes where cleanup finds it.
boot() {
    MOUNT_WAIT=${2:-60} AGENT_TEST_ROOT=$1 PATH="$WORK/bin:$PATH" sh "$1/var/local/aristotle/boot.sh" > "$1.boot.out" 2>&1 &
    echo $! > "$1/jobs/boot.pid"
}

# wait_for FILE PATTERN: up to 10 s for PATTERN to show up in FILE.
wait_for() {
    i=0
    until grep -q "$2" "$1" 2> /dev/null; do
        [ $i -ge 50 ] && return 1
        sleep 0.2
        i=$((i + 1))
    done
}

# A booted device: the agent installed, the userstore mounted.
booted() {
    r=$(device "$1" "$PW4_SN" 5.13.6)
    run "$r" agent/install.sh
    # Only the upstart stub's stand-in ran; boot() runs the real script.
    kill "$(cat "$r/jobs/aristotle-agent.pid")" 2> /dev/null
    printf 'fsp /mnt/us fuse.fsp rw,nosuid,nodev,noatime 0 0\n' > "$r/proc/mounts"
    : > "$r/calls"
    echo "$r"
}

echo "boot: no kill switch, the agent starts"
r=$(booted boot)
boot "$r"
check "USB network and SSH come up" "wait_for $r/calls '^dropbear ' && called $r 'modprobe g_ether'"
check "with the fixed MACs" "called $r 'modprobe g_ether dev_addr=02:61:72:69:73:01 host_addr=02:61:72:69:73:02'"
check "mode logged" "grep -q 'mode: net' $r/var/local/aristotle/log"
kill "$(cat "$r/jobs/boot.pid")" 2> /dev/null

echo "boot: kill switch on the USB drive"
r=$(booted kill)
: > "$r/mnt/us/aristotle-disable"
boot "$r"
check "says why in the log" "wait_for $r/var/local/aristotle/log 'aristotle-disable exists; not starting'"
check "USB left to volumd" "called $r 'lipc-set-prop com.lab126.volumd useUsbForNetwork 0'"
sleep 1
check "no g_ether, dropbear, firewall or Wi-Fi policy" "! grep -q -e '^modprobe' -e '^rmmod' -e '^dropbear' -e '^iptables' $r/calls"
check "stays up idle, so upstart does not respawn it" "kill -0 \$(cat $r/jobs/boot.pid)"

echo "boot: the userstore comes late"
r=$(booted late)
: > "$r/mnt/us/aristotle-disable"
: > "$r/proc/mounts"
boot "$r"
sleep 1
check "waits for it" "! grep -q -e '^modprobe' -e '^dropbear' $r/calls"
printf 'fsp /mnt/us fuse.fsp rw 0 0\n' > "$r/proc/mounts"
check "then honours the kill switch" "wait_for $r/var/local/aristotle/log 'aristotle-disable exists' && ! called $r 'modprobe'"

echo "boot: the userstore never comes"
r=$(booted nomount)
: > "$r/proc/mounts"
boot "$r" 1
check "gives up on the check and starts" "wait_for $r/calls '^dropbear ' && grep -q 'kill switch not checked' $r/var/local/aristotle/log"

# persistent ROOT: the files outside the tmpfs (/var/local, /etc) with their
# checksums.
persistent() {
    find "$1/var/local" "$1/etc" -type f | sort | while read -r f; do
        cksum "$f"
    done
}

echo "takeover: the flag is runtime state only"
r=$(booted takeover)
persistent "$r" > "$r.before"
run "$r" takeover.sh start
check "exit status 0" "[ $? = 0 ]"
check "stock GUI stopped" "called $r 'stop lab126_gui'"
check "flag on the tmpfs" "[ -e $r/var/run/aristotle-takeover ]"
check "nothing persistent written, so a reboot is stock" "persistent $r | cmp -s - $r.before"
check "aristotlectl sees it" "AGENT_TEST_ROOT=$r sh $r/var/local/aristotle/aristotlectl status 2> /dev/null | grep -q 'takeover: on'"
run "$r" takeover.sh stop
check "stop clears it and restarts the stock GUI" "[ ! -e $r/var/run/aristotle-takeover ] && called $r 'start lab126_gui'"
AGENT_TEST_ROOT=$r sh "$r/var/local/aristotle/aristotlectl" takeover on
check "aristotlectl takeover on: on the tmpfs" "[ -e $r/var/run/aristotle-takeover ] && persistent $r | cmp -s - $r.before"
AGENT_TEST_ROOT=$r sh "$r/var/local/aristotle/aristotlectl" takeover off
check "aristotlectl takeover off" "[ ! -e $r/var/run/aristotle-takeover ]"

echo "takeover: a flag an earlier version left on persistent storage"
r=$(booted oldflag)
: > "$r/var/local/aristotle/takeover"
boot "$r"
check "boot removes it" "wait_for $r/var/local/aristotle/log 'removed a takeover flag' && [ ! -e $r/var/local/aristotle/takeover ]"

echo "takeover: with the agent from before the rename"
r=$(device legacy "$PW4_SN" 5.13.6)
mkdir -p "$r/var/local/aristotle"
printf 'old boot\n' > "$r/var/local/aristotle/boot.sh"
chmod 755 "$r/var/local/aristotle/boot.sh"
run "$r" takeover.sh start
check "its flag is set too" "[ -e $r/var/local/aristotle/takeover ] && [ -e $r/var/run/aristotle-takeover ]"
check "no new agent directory created" "[ ! -e $r/var/local/aristotle ]"
run "$r" takeover.sh stop
check "stop clears both" "[ ! -e $r/var/local/aristotle/takeover ] && [ ! -e $r/var/run/aristotle-takeover ]"

if [ $failures = 0 ]; then
    echo "agent: all checks passed"
else
    echo "agent: $failures checks failed (output in $WORK)"
    exit 1
fi
