#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Host-side test of the device scripts: the device guard
# (device/extension/guard.sh) in install.sh, bin/start.sh and takeover.sh,
# uninstall.sh, the kill switch in boot.sh, the takeover flag, the upstart
# jobs' own scripts (run from the .conf files), Aristotle at boot
# (ui-boot.sh, autostart.sh, aristotlectl app), the OTA block, the KUAL
# start and stop items, and the ownership check on /var/local/aristotle.
#
#   sh device/tests/agent.sh
#
# Each case builds a fake device root in a fresh directory under $TMPDIR: the
# KUAL extension copied to mnt/us/extensions/ as install-extensions.sh does,
# /proc/usid and /etc/prettyversion.txt, and stub device commands on PATH that
# record their calls in the root's `calls` file. The real scripts run against
# it (AGENT_TEST_ROOT). Nothing outside the fresh directory is touched.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
DEVICE=$(cd "$HERE/.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/aristotle-agent-test.XXXXXX")
failures=0

# A PW4 serial (device code 0PP in characters 4-6) and a PW5 one (1Q0); both
# made up for the test.
PW4_SN=G000PP0123456789
PW5_SN=G001Q00123456789

cleanup() {
    for p in $(cat "$WORK"/*/jobs/*.pid "$WORK"/*/var/run/*.pid 2> /dev/null); do
        pkill -TERM -P "$p" 2> /dev/null
        kill "$p" 2> /dev/null
    done
}
trap 'cleanup; rm -rf "$WORK"' EXIT

check() {
    if eval "$2"; then
        printf '  ok    %s\n' "$1"
    else
        printf '  FAIL  %s\n' "$1"
        failures=$((failures + 1))
    fi
}

# The scripts check that /var/local/aristotle belongs to root and gives
# nothing to group or others; here the owner is whoever runs the test, who
# also needs the owner's bits to enter it.
AGENT_TEST_OWNER=$(id -u)
export AGENT_TEST_OWNER
export AGENT_TEST_DIR_MODE=700

# stubs DIR: the device commands the scripts call. Each one appends its
# command line to $AGENT_TEST_ROOT/calls.
stubs() {
    mkdir -p "$1"
    for c in eips mntroot lipc-set-prop lipc-send-event logger iptables ifconfig modprobe rmmod \
        sync reboot mount umount; do
        cat > "$1/$c" << 'EOF'
#!/bin/sh
echo "${0##*/} $*" >> "$AGENT_TEST_ROOT/calls"
case "${0##*/}:$1" in
# No rule or chain exists yet, as on a fresh boot.
iptables:-C | iptables:-D | iptables:-F | iptables:-X) exit 1 ;;
mntroot:*) [ "${FAKE_MNTROOT_FAIL:-}" != "$1" ] ;;
# A bind mount shows up in /proc/mounts, as on the device.
mount:--bind) echo "/dev/mmcblk1p9 $3 ext3 rw 0 0" >> "$AGENT_TEST_ROOT/proc/mounts" ;;
esac
EOF
    done
    printf '#!/bin/sh\necho "lsmod" >> "$AGENT_TEST_ROOT/calls"\ncat "$AGENT_TEST_ROOT/modules" 2> /dev/null\n' > "$1/lsmod"
    printf '#!/bin/sh\necho 0\n' > "$1/id"
    cat > "$1/cp" << 'EOF'
#!/bin/sh
for last do :; done
case "$last:${FAKE_APP_COPY_FAIL:-0}" in */app.new/:1) exit 1 ;; esac
command -p cp "$@"
EOF
    cat > "$1/mv" << 'EOF'
#!/bin/sh
case "$1:${FAKE_APP_MOVE_FAIL:-0}" in */app.new:1) exit 1 ;; esac
case "$1:${FAKE_APP_RETIRE_FAIL:-0}" in */app:1) exit 1 ;; esac
case "$1:${FAKE_APP_RESTORE_FAIL:-0}" in */app.previous:1) exit 1 ;; esac
command -p mv "$@"
EOF
    cat > "$1/initctl" << 'EOF'
#!/bin/sh
R=$AGENT_TEST_ROOT
echo "initctl $*" >> "$R/calls"
case "$1" in
reload-configuration) exit 0 ;;
status)
    # The stock jobs always exist; ours only once installed.
    case "$2" in
    aristotle-*) [ -f "$R/etc/upstart/$2.conf" ] || [ -f "$R/jobs/$2.pid" ] || { echo "initctl: Unknown job: $2" >&2; exit 1; } ;;
    esac
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
    mkdir -p "$r/proc/sys/kernel/random" "$r/etc/upstart" "$r/var/local" "$r/var/run" \
        "$r/mnt/us/extensions" "$r/mnt/us/koreader/settings/SSH" "$r/jobs" "$r/usr/bin"
    echo "boot-$1" > "$r/proc/sys/kernel/random/boot_id"
    echo "host key" > "$r/mnt/us/koreader/settings/SSH/dropbear_ed25519_host_key"
    [ -n "$2" ] && printf '%s\n' "$2" > "$r/proc/usid"
    [ -n "$3" ] && printf 'Kindle %s (~~otaVersion~~)\n\n\n\n' "$3" > "$r/etc/prettyversion.txt"
    printf '10.00 5.00\n' > "$r/proc/uptime"
    cp -R "$DEVICE/extension" "$r/mnt/us/extensions/aristotle"
    cp "$DEVICE/app/takeover.sh" "$r/mnt/us/extensions/aristotle/"
    printf 'ssh-ed25519 AAAA test\n' > "$r/mnt/us/extensions/aristotle/authorized_keys"
    cat > "$r/mnt/us/koreader/dropbear" << 'EOF'
#!/bin/sh
echo "dropbear $* (in $PWD)" >> "$AGENT_TEST_ROOT/calls"
while [ $# -gt 0 ]; do [ "$1" = -P ] && pid=$2; shift; done
# A process with "dropbear" in its command line, as the scripts check.
sh -c 'sleep 300; :' fake-dropbear > /dev/null 2>&1 &
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
        sh "$r/mnt/us/extensions/aristotle/$s" "$@" > "$r.out" 2>&1
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
check "says why in its log" "grep -q 'firmware 5.16.2' $r/mnt/us/extensions/aristotle/usbnet.log"
check "no module, firewall or dropbear" "! grep -q -e '^modprobe' -e '^rmmod' -e '^iptables' -e '^dropbear' $r/calls"
echo "guard: takeover.sh start on a PW5"
refused takeover "$PW5_SN" 5.13.6 "takeover.sh start" "not a Kindle Paperwhite 4"
check "stock GUI not stopped" "! called $r 'stop lab126_gui'"
echo "guard: takeover.sh on stdin finds the installed extension's guard"
r=$(device stdin "$PW5_SN" 5.13.6)
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" sh -s start < "$DEVICE/app/takeover.sh" > "$r.out" 2>&1
check "exit status 1" "[ $? = 1 ]"
check "stock GUI not stopped" "! called $r 'stop lab126_gui'"
run "$r" takeover.sh stop
check "stop always runs" "[ $? = 0 ] && called $r 'start lab126_gui'"
echo "guard: takeover.sh in the repository finds ../extension/guard.sh"
r=$(device repo "$PW5_SN" 5.13.6)
rm "$r/mnt/us/extensions/aristotle/guard.sh"
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" sh "$DEVICE/app/takeover.sh" start > "$r.out" 2>&1
check "exit status 1" "[ $? = 1 ]"
check "says why" "grep -q 'not a Kindle Paperwhite 4' $r.out"
check "stock GUI not stopped" "! called $r 'stop lab126_gui'"
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
    sh -c 'sleep 300; :' fake-dropbear > /dev/null 2>&1 &
    echo $! > "$r/var/run/aristotle-dropbear.pid"
    echo 'g_ether 40960 0' > "$r/modules"
    echo "$r"
}

echo "uninstall removes everything"
r=$(installed uninstall)
dropbear=$(cat "$r/var/run/aristotle-dropbear.pid")
# The extension under its earlier name, as an older install left it.
mkdir -p "$r/mnt/us/extensions/aristotle-usbnet"
printf 'ssh-ed25519 AAAA test\n' > "$r/mnt/us/extensions/aristotle-usbnet/authorized_keys"
run "$r" agent/uninstall.sh
check "exit status 0" "[ $? = 0 ]"
check "job file removed" "[ ! -e $r/etc/upstart/aristotle-agent.conf ] && [ \"\$(grep '^mntroot' $r/calls | tail -n 2 | tr '\n' ' ')\" = 'mntroot rw mntroot ro ' ]"
check "agent stopped" "called $r 'stop aristotle-agent' && ! running $r aristotle-agent"
check "Wi-Fi policy back to off" "called $r 'iptables -F ARISTOTLE_IN'"
check "state directory removed, key included" "[ ! -e $r/var/local/aristotle ]"
check "migration backups removed" "[ ! -e $r/var/local/aristotle-migrate ]"
check "log saved to the USB drive" "grep -q 'agent: boot' $r/mnt/us/aristotle-agent.log && grep -q 'older lines' $r/mnt/us/aristotle-agent.log && grep -q migrated $r/mnt/us/aristotle-agent.log"
check "agent key gone from KOReader, the owner's kept" "[ \"\$(cat $r/mnt/us/koreader/settings/SSH/authorized_keys)\" = 'ssh-ed25519 OWNER own' ]"
check "extension key removed" "[ ! -e $r/mnt/us/extensions/aristotle/authorized_keys ]"
check "earlier extension's key removed" "[ ! -e $r/mnt/us/extensions/aristotle-usbnet/authorized_keys ]"
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
check "stays idle when the kill switch cannot be checked" "wait_for $r/var/local/aristotle/log 'cannot check kill switch; staying idle' && ! called $r modprobe && ! called $r dropbear && ! called $r iptables"
check "does not respawn and leaves USB to volumd" "kill -0 \$(cat $r/jobs/boot.pid) && called $r 'lipc-set-prop com.lab126.volumd useUsbForNetwork 0'"

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
kill "$(cat "$r/jobs/boot.pid")" 2> /dev/null
: > "$r/var/local/aristotle/takeover"
run "$r" takeover.sh stop
check "takeover.sh stop removes it too" "[ ! -e $r/var/local/aristotle/takeover ]"

echo "takeover: without the agent"
r=$(device noagent "$PW4_SN" 5.13.6)
run "$r" takeover.sh start
check "flag on the tmpfs" "[ -e $r/var/run/aristotle-takeover ]"
check "no agent directory created" "[ ! -e $r/var/local/aristotle ]"
run "$r" takeover.sh stop
check "stop clears it" "[ ! -e $r/var/run/aristotle-takeover ]"

# --- The upstart jobs' own scripts --------------------------------------

# section CONF NAME: the body of a .conf's "pre-start script" or "script"
# section, as upstart runs it (with sh -e).
section() {
    awk -v want="$2" '
        $0 == want " script" || ($0 == "script" && want == "main") { on = 1; next }
        on && $0 == "end script" { exit }
        on { print }' "$DEVICE/extension/agent/$1.conf"
}

# job ROOT CONF NAME: run that section of an installed job against ROOT.
job() {
    section "$2" "$3" > "$1.$2.$3.sh"
    AGENT_TEST_ROOT=$1 PATH="$WORK/bin:$PATH" sh -e "$1.$2.$3.sh" > "$1.job.out" 2>&1
}

echo "jobs: the directory check"
r=$(booted jobs)
job "$r" aristotle-agent pre-start
check "agent: root's directory passes" "[ $? = 0 ]"
chmod 775 "$r/var/local/aristotle"
job "$r" aristotle-agent pre-start
check "agent: a group-writable directory is refused" "[ $? != 0 ]"
check "logged on the USB drive" "grep -q 'agent: .* is not root.s alone (owner [0-9]*, mode 775)' $r/mnt/us/aristotle-refused.log"
check "and in syslog" "called $r 'logger -t aristotle agent:'"
job "$r" aristotle-agent main
check "agent: the main script refuses too" "[ $? != 0 ] && ! called $r 'lipc-set-prop com.lab126.volumd'"
chmod 2700 "$r/var/local/aristotle"
job "$r" aristotle-agent pre-start
check "agent: a setgid bit alone (the framework's chmod g+s) passes" "[ $? = 0 ]"
chmod g-s "$r/var/local/aristotle"
mv "$r/var/local/aristotle" "$r/var/local/real"
ln -s real "$r/var/local/aristotle"
job "$r" aristotle-agent pre-start
check "agent: a link in its place is refused" "[ $? != 0 ]"
rm "$r/var/local/aristotle"
mv "$r/var/local/real" "$r/var/local/aristotle"
job "$r" aristotle-ui pre-start
check "ui: idle without autostart" "[ $? != 0 ]"
: > "$r/var/local/aristotle/autostart"
job "$r" aristotle-ui pre-start
check "ui: starts with autostart" "[ $? = 0 ]"
cp "$r/proc/sys/kernel/random/boot_id" "$r/var/local/aristotle/ui-booted"
job "$r" aristotle-ui pre-start
check "ui: but once per boot" "[ $? != 0 ]"
: > "$r/var/local/aristotle/ui-request"
job "$r" aristotle-ui pre-start
check "ui: started by hand, again" "[ $? = 0 ]"
rm "$r/var/local/aristotle/ui-request"
echo other-boot > "$r/var/local/aristotle/ui-booted"
chmod 750 "$r/var/local/aristotle"
job "$r" aristotle-ui pre-start
check "ui: a directory the group can enter is refused" "[ $? != 0 ] && grep -q 'ui: .*mode 750' $r/mnt/us/aristotle-refused.log"
chmod 700 "$r/var/local/aristotle"

echo "jobs: the OTA block"
r=$(booted ota)
job "$r" aristotle-ota main
check "binds the stub over both daemons" "called $r 'mount --bind ./ota-stub $r/usr/bin/otaupd' && called $r 'mount --bind ./ota-stub $r/usr/bin/otav3'"
check "read-only" "called $r 'mount -o remount,ro,bind $r/usr/bin/otav3'"
check "the stub has no write bits" "[ \"\$(stat -c %a $r/var/local/aristotle/ota-stub)\" = 555 ]"
: > "$r/calls"
job "$r" aristotle-ota main
check "once per boot" "[ $? = 0 ] && ! called $r 'mount --bind'"
r=$(booted otabad)
chmod 755 "$r/var/local/aristotle"
job "$r" aristotle-ota main
check "nothing bound from a directory others can enter" "! called $r 'mount --bind'"
r=$(booted otanone)
rm "$r/var/local/aristotle/ota-stub"
job "$r" aristotle-ota main
check "nothing without the stub" "[ $? = 0 ] && ! called $r 'mount'"

echo "install: three jobs, one root filesystem window"
r=$(device install3 "$PW4_SN" 5.13.6)
mkdir -p "$r/jobs"
sleep 300 > /dev/null 2>&1 &
echo $! > "$r/jobs/otav3.pid"
run "$r" agent/install.sh
check "exit status 0" "[ $? = 0 ]"
check "all three job files" "[ -f $r/etc/upstart/aristotle-agent.conf ] && [ -f $r/etc/upstart/aristotle-ui.conf ] && [ -f $r/etc/upstart/aristotle-ota.conf ]"
check "one mntroot window" "[ \"\$(grep '^mntroot' $r/calls | tr '\n' ' ')\" = 'mntroot rw mntroot ro ' ]"
check "a running OTA daemon is restarted (to run as the stub)" "called $r 'stop otav3' && called $r 'start otav3' && ! called $r 'stop otaupd'"
check "the directory's mode is set" "[ \"\$(stat -c %a $r/var/local/aristotle)\" = 700 ]"
check "dropbear and the host key copied" "[ -x $r/var/local/aristotle/dropbear ] && [ -f $r/var/local/aristotle/ssh/settings/SSH/dropbear_ed25519_host_key ]"
check "ui-boot.sh, takeover.sh and the guard copied" "[ -f $r/var/local/aristotle/ui-boot.sh ] && [ -f $r/var/local/aristotle/takeover.sh ] && [ -f $r/var/local/aristotle/guard.sh ]"
check "autostart stays off" "[ ! -e $r/var/local/aristotle/autostart ]"
: > "$r/calls"
run "$r" agent/install.sh
check "again: no root filesystem write" "! called $r mntroot"

echo "install: over a directory others could write"
r=$(device untrusted "$PW4_SN" 5.13.6)
S=$r/var/local/aristotle
mkdir -p "$S/app"
chmod 775 "$S"
echo planted > "$S/boot.sh"
echo planted > "$S/app/aristotle"
: > "$S/autostart"
echo scoped > "$S/net-mode"
echo example.org > "$S/net-allow"
echo 'old line' > "$S/log"
ln -s /etc/passwd "$S/usb-mode"
run "$r" agent/install.sh
check "exit status 0" "[ $? = 0 ]"
check "locked" "[ \"\$(stat -c %a $S)\" = 700 ]"
check "planted files gone or replaced" "[ ! -e $S/app ] && [ ! -e $S/autostart ] && ! grep -q planted $S/boot.sh"
check "settings and log kept" "grep -q scoped $S/net-mode && grep -q example.org $S/net-allow && grep -q 'old line' $S/log"
check "a link among them removed" "[ ! -L $S/usb-mode ]"

echo "aristotlectl: the directory check"
r=$(booted ctl)
chmod 770 "$r/var/local/aristotle"
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" sh "$r/var/local/aristotle/aristotlectl" mode net > "$r.out" 2>&1
check "refuses a directory the group can write" "[ $? = 1 ] && grep -q 'not root.s alone' $r.out"
chmod 700 "$r/var/local/aristotle"
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" sh "$r/var/local/aristotle/aristotlectl" mode storage 5 > "$r.out" 2>&1
check "works on root's own" "[ $? = 0 ] && grep -q '^storage ' $r/var/local/aristotle/usb-mode"

# --- Aristotle at boot ------------------------------------------------------

# fake_app ROOT: an app in /mnt/us/aristotle that records its runs, exits
# with $FAKE_RC after $FAKE_SECS seconds, and saves and exits 0 on SIGTERM,
# as the real one does.
fake_app() {
    mkdir -p "$1/mnt/us/aristotle/data"
    echo 'test release 1' > "$1/mnt/us/aristotle/SOURCE.txt"
    cat > "$1/mnt/us/aristotle/aristotle" << 'EOF'
#!/bin/sh
echo "app $*" >> "$AGENT_TEST_ROOT/calls"
echo $$ > "$AGENT_TEST_ROOT/jobs/app-$$.pid"
trap 'echo "app saved" >> "$AGENT_TEST_ROOT/calls"; rm -f "$AGENT_TEST_ROOT/jobs/app-$$.pid"; exit 0' TERM
i=0
while [ $i -lt "${FAKE_SECS:-0}" ]; do sleep 1; i=$((i + 1)); done
exit "${FAKE_RC:-0}"
EOF
    chmod 755 "$1/mnt/us/aristotle/aristotle"
    cp "$DEVICE/app/run-aristotle.sh" "$DEVICE/app/takeover.sh" "$DEVICE/extension/guard.sh" "$1/mnt/us/aristotle/"
}

# autostarted NAME: a booted device with autostart on.
autostarted() {
    r=$(booted "$1")
    fake_app "$r"
    run "$r" agent/autostart.sh on
    : > "$r/calls"
    echo "$r"
}

# ui_boot ROOT: the ui job's main script in the background, as upstart
# would run it; its pid in jobs/aristotle-ui.pid for initctl.
ui_boot() {
    section aristotle-ui main > "$1.ui.sh"
    AGENT_TEST_ROOT=$1 PATH="$WORK/bin:$PATH" sh -e "$1.ui.sh" > "$1.ui.out" 2>&1 &
    echo $! > "$1/jobs/aristotle-ui.pid"
}

# done_ui ROOT: wait up to 20 s for the ui job to end.
done_ui() {
    i=0
    while kill -0 "$(cat "$1/jobs/aristotle-ui.pid")" 2> /dev/null; do
        [ $i -ge 100 ] && return 1
        sleep 0.2
        i=$((i + 1))
    done
}

echo "autostart: on and off"
r=$(booted auto)
run "$r" agent/autostart.sh on
check "refused without an app" "[ $? = 1 ] && [ ! -e $r/var/local/aristotle/autostart ]"
fake_app "$r"
echo 3 > "$r/var/local/aristotle/ui-fails"
run "$r" agent/autostart.sh on
check "on: the app copied, the count reset" "[ $? = 0 ] && [ -x $r/var/local/aristotle/app/aristotle ] && [ -d $r/var/local/aristotle/app/data ] && [ \"\$(cat $r/var/local/aristotle/ui-fails)\" = 0 ] && [ -e $r/var/local/aristotle/autostart ]"
check "no root filesystem write" "! called $r mntroot"
echo 'test release 2' > "$r/mnt/us/aristotle/SOURCE.txt"
run "$r" agent/autostart.sh on
check "update: both release identities kept" "[ $? = 0 ] && grep -q 'release 2' $r/var/local/aristotle/app/SOURCE.txt && grep -q 'release 1' $r/var/local/aristotle/app.previous/SOURCE.txt"
run "$r" agent/autostart.sh rollback
check "rollback: previous becomes current" "[ $? = 0 ] && grep -q 'release 1' $r/var/local/aristotle/app/SOURCE.txt && grep -q 'release 2' $r/var/local/aristotle/app.previous/SOURCE.txt"
FAKE_APP_COPY_FAIL=1 run "$r" agent/autostart.sh on
check "failed copy: current and previous kept" "[ $? = 1 ] && grep -q 'release 1' $r/var/local/aristotle/app/SOURCE.txt && grep -q 'release 2' $r/var/local/aristotle/app.previous/SOURCE.txt && [ ! -e $r/var/local/aristotle/app.new ]"
FAKE_APP_MOVE_FAIL=1 run "$r" agent/autostart.sh on
check "failed activation: current restored" "[ $? = 1 ] && grep -q 'release 1' $r/var/local/aristotle/app/SOURCE.txt && [ ! -e $r/var/local/aristotle/app.new ]"
check "failed activation: lock removed" "[ ! -e $r/var/run/aristotle-app-update.lock ]"
FAKE_APP_RETIRE_FAIL=1 run "$r" agent/autostart.sh on
check "failed old-copy rename: current kept" "[ $? = 1 ] && grep -q 'release 1' $r/var/local/aristotle/app/SOURCE.txt"
FAKE_APP_MOVE_FAIL=1 FAKE_APP_RESTORE_FAIL=1 run "$r" agent/autostart.sh on
check "failed restore: both complete copies kept" "[ $? = 1 ] && [ ! -e $r/var/local/aristotle/app ] && grep -q 'release 1' $r/var/local/aristotle/app.previous/SOURCE.txt && grep -q 'release 2' $r/var/local/aristotle/app.new/SOURCE.txt"
mv "$r/var/local/aristotle/app.previous" "$r/var/local/aristotle/app"
mkdir "$r/var/run/aristotle-app-update.lock"
run "$r" agent/autostart.sh on
check "concurrent update: refused without taking the lock" "[ $? = 1 ] && [ -d $r/var/run/aristotle-app-update.lock ] && grep -q 'release 1' $r/var/local/aristotle/app/SOURCE.txt"
rmdir "$r/var/run/aristotle-app-update.lock"
echo $$ > "$r/jobs/aristotle-ui.pid"
run "$r" agent/autostart.sh on
check "running boot app: update refused" "[ $? = 1 ] && grep -q 'quit the boot app' $r.out && grep -q 'release 1' $r/var/local/aristotle/app/SOURCE.txt"
rm "$r/jobs/aristotle-ui.pid"
run "$r" agent/autostart.sh off
check "off: marker gone, copy retained" "[ ! -e $r/var/local/aristotle/autostart ] && [ -x $r/var/local/aristotle/app/aristotle ]"
r=$(device noagent2 "$PW4_SN" 5.13.6)
fake_app "$r"
run "$r" agent/autostart.sh on
check "on: refused without the agent" "[ $? = 1 ] && grep -q 'install the agent first' $r.out"

echo "ui-boot: the kill switch"
r=$(autostarted uikill)
printf 'fsp %s/mnt/us fuse.fsp rw 0 0\n' "$r" > "$r/proc/mounts"
: > "$r/mnt/us/aristotle-disable"
ui_boot "$r"
done_ui "$r"
check "stays on the stock GUI" "! called $r 'stop lab126_gui' && ! called $r 'app' && grep -q 'aristotle-disable exists' $r/var/local/aristotle/ui.log"
check "remembers this boot" "cmp -s $r/proc/sys/kernel/random/boot_id $r/var/local/aristotle/ui-booted"

echo "ui-boot: interrupted app activation"
r=$(autostarted uiinterrupted)
printf 'fsp %s/mnt/us fuse.fsp rw 0 0\n' "$r" > "$r/proc/mounts"
mv "$r/var/local/aristotle/app" "$r/var/local/aristotle/app.previous"
ui_boot "$r"
done_ui "$r"
check "runs the complete previous copy" "called $r app && grep -q 'activation interrupted' $r/var/local/aristotle/ui.log"

echo "ui-boot: cannot check the kill switch"
r=$(autostarted uiunmounted)
: > "$r/proc/mounts"
MOUNT_WAIT=0 ui_boot "$r"
done_ui "$r"
check "leaves stock running without touching the app" "! called $r 'stop lab126_gui' && ! called $r app && grep -q 'cannot check the kill switch' $r/var/local/aristotle/ui.log"

echo "ui-boot: USB app replaced without refreshing autostart"
r=$(autostarted uistale)
printf 'fsp %s/mnt/us fuse.fsp rw 0 0\n' "$r" > "$r/proc/mounts"
echo 'test release 2' > "$r/mnt/us/aristotle/SOURCE.txt"
ui_boot "$r"
done_ui "$r"
check "stays stock instead of silently launching an older app" "! called $r 'stop lab126_gui' && ! called $r app && grep -q 'USB app changed' $r/var/local/aristotle/ui.log"
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" sh "$r/var/local/aristotle/aristotlectl" app status > "$r.out" 2>&1
check "status shows both release identities and the mismatch" "grep -q 'release 1' $r.out && grep -q 'release 2' $r.out && grep -q 'changed or untracked' $r.out"
run "$r" agent/autostart.sh on
run "$r" agent/autostart.sh rollback
: > "$r/calls"
ui_boot "$r"
done_ui "$r"
check "explicit rollback still launches the selected older app" "called $r app && grep -q 'release 1' $r/var/local/aristotle/app/SOURCE.txt"

echo "ui-boot: app update in progress"
r=$(autostarted uiupdating)
printf 'fsp %s/mnt/us fuse.fsp rw 0 0\n' "$r" > "$r/proc/mounts"
mkdir "$r/var/run/aristotle-app-update.lock"
ui_boot "$r"
done_ui "$r"
check "does not stop stock or launch the app" "! called $r 'stop lab126_gui' && ! called $r app"

echo "ui-boot: Aristotle quits"
r=$(autostarted uiquit)
printf 'fsp %s/mnt/us fuse.fsp rw 0 0\n' "$r" > "$r/proc/mounts"
ui_boot "$r"
done_ui "$r"
check "takes the screen" "called $r 'stop lab126_gui' && called $r 'lipc-set-prop com.lab126.blanket unload splash' && called $r 'lipc-send-event -r 5 com.lab126.hal bootSplashCleanup'"
check "runs the copy" "called $r app"
check "then the stock GUI, screensaver back" "called $r 'start lab126_gui' && called $r 'lipc-set-prop com.lab126.blanket load screensaver'"
check "count 0" "[ \"\$(cat $r/var/local/aristotle/ui-fails)\" = 0 ]"
check "takeover flag gone" "[ ! -e $r/var/run/aristotle-takeover ]"

echo "ui-boot: crash loop"
r=$(autostarted uicrash)
printf 'fsp %s/mnt/us fuse.fsp rw 0 0\n' "$r" > "$r/proc/mounts"
FAKE_RC=1 ui_boot "$r"
done_ui "$r"
check "three launches, then the stock GUI" "[ \"\$(grep -c '^app' $r/calls)\" = 3 ] && called $r 'start lab126_gui'"
check "count 3" "[ \"\$(cat $r/var/local/aristotle/ui-fails)\" = 3 ]"
: > "$r/calls"
echo other-boot > "$r/var/local/aristotle/ui-booted"
FAKE_RC=1 ui_boot "$r"
done_ui "$r"
check "next boot: stays stock, nothing stopped" "! called $r 'stop lab126_gui' && ! called $r app && grep -q 'launches failed in a row' $r/var/local/aristotle/ui.log"
run "$r" agent/autostart.sh on
check "autostart on again resets it" "[ \"\$(cat $r/var/local/aristotle/ui-fails)\" = 0 ]"

echo "ui-boot: a launch that stays up is a good one"
r=$(autostarted uihealthy)
printf 'fsp %s/mnt/us fuse.fsp rw 0 0\n' "$r" > "$r/proc/mounts"
echo 1 > "$r/var/local/aristotle/ui-fails"
HEALTHY_AFTER=1 FAKE_SECS=3 FAKE_RC=1 ui_boot "$r"
sleep 2
check "count back to 0 while it runs" "[ \"\$(cat $r/var/local/aristotle/ui-fails)\" = 0 ]"
kill "$(cat "$r/jobs/aristotle-ui.pid")" 2> /dev/null
done_ui "$r"

echo "aristotlectl app stop and start"
r=$(autostarted uistop)
printf 'fsp %s/mnt/us fuse.fsp rw 0 0\n' "$r" > "$r/proc/mounts"
FAKE_SECS=60 ui_boot "$r"
i=0
until [ -s "$r/var/local/aristotle/app.pid" ] || [ $i -ge 50 ]; do sleep 0.2; i=$((i + 1)); done
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" sh "$r/var/local/aristotle/aristotlectl" app status > "$r.out" 2>&1
check "status: running" "grep -q 'running: yes' $r.out && grep -q 'autostart: on' $r.out"
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" sh "$r/var/local/aristotle/aristotlectl" app stop > "$r.out" 2>&1
check "stop: the app saved and the job ended" "[ $? = 0 ] && called $r 'app saved' && ! kill -0 \$(cat $r/jobs/aristotle-ui.pid) 2> /dev/null"
check "not counted as a crash" "[ \"\$(cat $r/var/local/aristotle/ui-fails)\" = 0 ]"
check "the stock GUI back" "called $r 'start lab126_gui'"
: > "$r/calls"
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" sh "$r/var/local/aristotle/aristotlectl" app start > "$r.out" 2>&1
check "start: asks upstart, once this boot is over" "[ $? = 0 ] && called $r 'start aristotle-ui' && [ -e $r/var/local/aristotle/ui-request ]"
job "$r" aristotle-ui pre-start
check "and the job may run again in this boot" "[ $? = 0 ]"
FAKE_SECS=60 ui_boot "$r"
i=0
until [ -s "$r/var/local/aristotle/app.pid" ] || [ $i -ge 50 ]; do sleep 0.2; i=$((i + 1)); done
check "the request is used up" "[ ! -e $r/var/local/aristotle/ui-request ]"
: > "$r/calls"
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" sh "$r/var/local/aristotle/aristotlectl" app stop --keep-takeover > "$r.out" 2>&1
check "stop --keep-takeover: the stock GUI stays down" "[ $? = 0 ] && called $r 'app saved' && ! called $r 'start lab126_gui' && [ -e $r/var/run/aristotle-takeover ]"

echo "run-aristotle.sh stops Aristotle started at boot"
r=$(autostarted uirun)
printf 'fsp %s/mnt/us fuse.fsp rw 0 0\n' "$r" > "$r/proc/mounts"
FAKE_SECS=60 ui_boot "$r"
i=0
until [ -s "$r/var/local/aristotle/app.pid" ] || [ $i -ge 50 ]; do sleep 0.2; i=$((i + 1)); done
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" ARISTOTLE_LOG=$r/app.log sh "$r/mnt/us/aristotle/run-aristotle.sh" > "$r.out" 2>&1
check "the boot one saved first, not counted" "called $r 'app saved' && [ \"\$(cat $r/var/local/aristotle/ui-fails)\" = 0 ]"
check "then its own run, then the stock GUI" "[ \"\$(grep -c '^app' $r/calls)\" = 3 ] && called $r 'start lab126_gui'"
check "the stock GUI was not started in between" "[ \"\$(grep -c '^start lab126_gui' $r/calls)\" = 1 ]"

# --- KUAL items -------------------------------------------------------------

# No fake app may still run: the KUAL item looks for one by name.
cleanup
sleep 1

echo "KUAL: Start Aristotle detaches"
r=$(device kual "$PW4_SN" 5.13.6)
fake_app "$r"
# The launcher leaves a mark with its session id.
cat > "$r/mnt/us/aristotle/run-aristotle.sh" << 'EOF'
#!/bin/sh
ps -o sid= -p $$ > "$AGENT_TEST_ROOT/launcher-sid"
echo "log=$ARISTOTLE_LOG" >> "$AGENT_TEST_ROOT/launcher-sid"
sleep 2
echo finished >> "$AGENT_TEST_ROOT/launcher-sid"
EOF
t0=$(date +%s)
run "$r" bin/aristotle-start.sh
check "returns at once" "[ $? = 0 ] && [ \$((\$(date +%s) - $t0)) -lt 2 ]"
wait_for "$r/launcher-sid" log=
check "in a session of its own" "[ \"\$(head -n 1 $r/launcher-sid | tr -d ' ')\" != \"\$(ps -o sid= -p \$\$ | tr -d ' ')\" ]"
check "log on the USB drive" "grep -q 'log=$r/mnt/us/.aristotle/app.log' $r/launcher-sid"
wait_for "$r/launcher-sid" finished
check "outlives its caller" "grep -q finished $r/launcher-sid"

echo "KUAL: Return to the Kindle interface"
r=$(autostarted kualstop)
printf 'fsp %s/mnt/us fuse.fsp rw 0 0\n' "$r" > "$r/proc/mounts"
FAKE_SECS=60 ui_boot "$r"
i=0
until [ -s "$r/var/local/aristotle/app.pid" ] || [ $i -ge 50 ]; do sleep 0.2; i=$((i + 1)); done
run "$r" bin/aristotle-stop.sh
check "stops Aristotle started at boot, the stock GUI back" "called $r 'app saved' && called $r 'start lab126_gui' && [ \"\$(cat $r/var/local/aristotle/ui-fails)\" = 0 ]"

echo "uninstall: Aristotle at boot and the OTA block"
r=$(autostarted uninst)
job "$r" aristotle-ota main
sleep 300 > /dev/null 2>&1 &
echo $! > "$r/jobs/otav3.pid"
: > "$r/calls"
run "$r" agent/uninstall.sh
check "exit status 0" "[ $? = 0 ]"
check "all three job files removed, one window" "[ -z \"\$(ls $r/etc/upstart)\" ] && [ \"\$(grep '^mntroot' $r/calls | tr '\n' ' ')\" = 'mntroot rw mntroot ro ' ]"
check "stubs unmounted" "called $r 'umount $r/usr/bin/otav3' && called $r 'umount $r/usr/bin/otaupd'"
check "the running daemon restarted for real" "called $r 'stop otav3' && called $r 'start otav3'"
check "state and app copy gone" "[ ! -e $r/var/local/aristotle ]"

if [ $failures = 0 ]; then
    echo "agent: all checks passed"
else
    echo "agent: $failures checks failed (output in $WORK)"
    exit 1
fi
