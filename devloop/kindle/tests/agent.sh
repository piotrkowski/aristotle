#!/bin/sh
# Host-side test of the device scripts other than the migration: the device
# guard (aristotle-usbnet/guard.sh) in install.sh, bin/start.sh and
# takeover.sh.
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
case "$1" in aristotle-agent | aristotle) ;; *) exit 0 ;; esac
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

if [ $failures = 0 ]; then
    echo "agent: all checks passed"
else
    echo "agent: $failures checks failed (output in $WORK)"
    exit 1
fi
