#!/bin/sh
# Host-side test of the agent migration (agent/migrate.sh and rollback.sh).
#
#   sh devloop/kindle/tests/migrate.sh
#
# Each case builds a fake device root in a fresh directory under $TMPDIR, with
# stub upstart, mntroot, ifconfig and eips commands on PATH, then runs the real
# scripts against it (AGENT_TEST_ROOT). Background `sleep`s stand in for the
# jobs and dropbear. Nothing outside the fresh directory is touched.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
AGENT=$(cd "$HERE/../aristotle-usbnet/agent" && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/aristotle-migrate-test.XXXXXX")
failures=0

# The fake jobs and dropbears, wherever the scripts left their pid files.
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

# stubs DIR: the device commands the scripts call.
stubs() {
    mkdir -p "$1"
    cat > "$1/initctl" << 'EOF'
#!/bin/sh
R=$AGENT_TEST_ROOT
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
[ -f "$R/etc/upstart/$1.conf" ] || { echo "start: Unknown job: $1" >&2; exit 1; }
mkdir -p "$R/jobs"
if [ "${FAKE_JOB_DIES:-}" = "$1" ]; then sleep 2 > /dev/null 2>&1 & else sleep 300 > /dev/null 2>&1 & fi
echo $! > "$R/jobs/$1.pid"
echo "$1" >> "$R/jobs/started"
EOF
    cat > "$1/stop" << 'EOF'
#!/bin/sh
R=$AGENT_TEST_ROOT
[ -f "$R/jobs/$1.pid" ] || exit 1
kill "$(cat "$R/jobs/$1.pid")" 2> /dev/null
rm -f "$R/jobs/$1.pid"
EOF
    cat > "$1/mntroot" << 'EOF'
#!/bin/sh
echo "$1" >> "$AGENT_TEST_ROOT/mntroot.log"
[ "${FAKE_MNTROOT_FAIL:-}" != "$1" ]
EOF
    cat > "$1/ifconfig" << 'EOF'
#!/bin/sh
[ "${FAKE_USB0_DOWN:-}" = 1 ] && exit 1
echo "usb0      Link encap:Ethernet  HWaddr 02:61:72:69:73:01"
echo "          inet addr:192.168.15.244  Bcast:192.168.15.255  Mask:255.255.255.0"
echo "          UP BROADCAST RUNNING MULTICAST  MTU:1500  Metric:1"
EOF
    printf '#!/bin/sh\nexit 0\n' > "$1/eips"
    printf '#!/bin/sh\necho 0\n' > "$1/id"
    chmod 755 "$1"/*
}

# device NAME: a fake root with the old agent installed and running, its
# dropbear listening on the USB address. Prints the root.
device() {
    r="$WORK/$1"
    mkdir -p "$r/var/local/aristotle" "$r/etc/upstart" "$r/var/run" "$r/proc/net" \
        "$r/mnt/us/extensions/aristotle-usbnet" "$r/jobs"
    printf 'description "old agent"\n' > "$r/etc/upstart/aristotle.conf"
    printf 'ssh-ed25519 AAAA test\n' > "$r/var/local/aristotle/authorized_keys"
    printf 'old boot\n' > "$r/var/local/aristotle/boot.sh"
    printf 'old ctl\n' > "$r/var/local/aristotle/aristotlectl"
    printf '2026-01-01 00:00:00 agent: boot\n' > "$r/var/local/aristotle/log"
    touch "$r/var/local/aristotle/takeover"
    printf '<extension/>\n' > "$r/mnt/us/extensions/aristotle-usbnet/config.xml"
    # A PW4 on 5.13.6 (made-up serial with the device code 0PP), which the
    # device guard accepts.
    printf 'G000PP0123456789\n' > "$r/proc/usid"
    printf 'Kindle 5.13.6 (~~otaVersion~~)\n' > "$r/etc/prettyversion.txt"
    # Background processes must not hold the caller's $(...) pipe open.
    sleep 300 > /dev/null 2>&1 &
    echo $! > "$r/jobs/aristotle.pid"
    sleep 300 > /dev/null 2>&1 &
    echo $! > "$r/var/run/aristotle-dropbear.pid"
    printf '  sl  local_address rem_address   st\n   0: F40FA8C0:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 1 1\n' \
        > "$r/proc/net/tcp"
    echo "$r"
}

run() {
    AGENT_TEST_ROOT=$1 PATH="$WORK/bin:$PATH" VERIFY_SECS=2 POLL_SECS=1 GRACE_SECS=0 \
        CONFIRM=yes sh "$AGENT/$2" > "$1.$2.out" 2>&1
}

running() {
    AGENT_TEST_ROOT=$1 "$WORK/bin/initctl" status "$2" 2> /dev/null | grep -q start/running
}

# After a rollback the device must look as it did before the migration.
old_state() {
    r=$1
    check "old job file restored" "[ \"\$(cat $r/etc/upstart/aristotle.conf)\" = 'description \"old agent\"' ]"
    check "new job file gone" "[ ! -e $r/etc/upstart/aristotle-agent.conf ]"
    check "new state moved into the backup" "[ ! -e $r/var/local/aristotle ] && ls -d $r/var/local/aristotle-migrate/*/new-state.1 > /dev/null"
    check "old state intact" "[ -f $r/var/local/aristotle/aristotlectl ] && [ -e $r/var/local/aristotle/takeover ]"
    check "dropbear pid file handed back" "[ \"\$(cat $r/var/run/aristotle-dropbear.pid)\" = $2 ] && [ ! -e $r/var/run/aristotle-dropbear.pid ]"
    check "old job running" "running $r aristotle"
    check "new job stopped" "! running $r aristotle-agent"
    check "old extension enabled" "[ -f $r/mnt/us/extensions/aristotle-usbnet/config.xml ]"
    check "root filesystem back to read-only" "[ \"\$(tail -n 1 $r/mntroot.log)\" = ro ]"
}

stubs "$WORK/bin"

echo "migration succeeds"
r=$(device ok)
dropbear=$(cat "$r/var/run/aristotle-dropbear.pid")
run "$r" migrate.sh
check "exit status 0" "[ $? = 0 ]"
check "new job file installed" "cmp -s $AGENT/aristotle-agent.conf $r/etc/upstart/aristotle-agent.conf"
check "old job file removed" "[ ! -e $r/etc/upstart/aristotle.conf ]"
check "one read-write window" "[ \"\$(cat $r/mntroot.log | tr '\n' ' ')\" = 'rw ro ' ]"
check "agent scripts installed" "[ -x $r/var/local/aristotle/boot.sh ] && [ -x $r/var/local/aristotle/aristotlectl ]"
check "state copied" "cmp -s $r/var/local/aristotle/authorized_keys $r/var/local/aristotle/authorized_keys && [ -e $r/var/run/aristotle-takeover ] && [ ! -e $r/var/local/aristotle/takeover ] && grep -q 'agent: boot' $r/var/local/aristotle/log"
check "old state kept" "[ -f $r/var/local/aristotle/aristotlectl ]"
check "dropbear handed over, not restarted" "[ \"\$(cat $r/var/run/aristotle-dropbear.pid)\" = $dropbear ] && [ ! -e $r/var/run/aristotle-dropbear.pid ]"
check "new job running, old one stopped" "running $r aristotle-agent && ! running $r aristotle"
check "old extension hidden from KUAL" "[ -f $r/mnt/us/extensions/aristotle-usbnet/config.xml.migrated ] && [ ! -e $r/mnt/us/extensions/aristotle-usbnet/config.xml ]"
check "backup holds the old job and state" "ls $r/var/local/aristotle-migrate/*/old-job.conf $r/var/local/aristotle-migrate/*/old-state/aristotlectl > /dev/null"
check "journal complete" "[ \"\$(tail -n 1 $r/var/local/aristotle-migrate/*/journal)\" = done ]"

echo "manual rollback after a successful migration"
run "$r" rollback.sh
check "exit status 0" "[ $? = 0 ]"
old_state "$r" "$dropbear"

echo "usb0 never comes up: automatic rollback"
r=$(device usb0)
dropbear=$(cat "$r/var/run/aristotle-dropbear.pid")
FAKE_USB0_DOWN=1 run "$r" migrate.sh
check "exit status 1" "[ $? = 1 ]"
check "reason logged" "grep -q 'usb0 is not up' $r.migrate.sh.out"
old_state "$r" "$dropbear"

echo "new job dies: automatic rollback"
r=$(device dies)
dropbear=$(cat "$r/var/run/aristotle-dropbear.pid")
FAKE_JOB_DIES=aristotle-agent VERIFY_SECS=4 run "$r" migrate.sh
check "exit status 1" "[ $? = 1 ]"
check "reason logged" "grep -q 'stopped or restarted' $r.migrate.sh.out"
old_state "$r" "$dropbear"

echo "mntroot rw fails: automatic rollback"
r=$(device mntroot)
dropbear=$(cat "$r/var/run/aristotle-dropbear.pid")
FAKE_MNTROOT_FAIL=rw run "$r" migrate.sh
check "exit status 1" "[ $? = 1 ]"
check "old job file untouched" "[ -f $r/etc/upstart/aristotle.conf ]"
check "old job running again" "running $r aristotle"
check "new state moved away" "[ ! -e $r/var/local/aristotle ]"

echo "USB drive mode on: refused, nothing changed"
r=$(device storage)
echo "storage $(($(date +%s) + 600))" > "$r/var/local/aristotle/usb-mode"
run "$r" migrate.sh
check "exit status 1" "[ $? = 1 ]"
check "no backup, no new state" "! ls -d $r/var/local/aristotle-migrate/2* > /dev/null 2>&1 && [ ! -e $r/var/local/aristotle ]"
check "root filesystem untouched" "[ ! -e $r/mntroot.log ]"
check "old job still running" "running $r aristotle"

echo "not a PW4: refused, nothing changed"
r=$(device guard)
printf 'G001Q00123456789\n' > "$r/proc/usid"
run "$r" migrate.sh
check "exit status 1" "[ $? = 1 ]"
check "says why" "grep -q 'not a Kindle Paperwhite 4' $r.migrate.sh.out"
check "nothing changed" "[ ! -e $r/var/local/aristotle-migrate ] && [ ! -e $r/var/local/aristotle ] && [ ! -e $r/mntroot.log ]"
check "old job still running" "running $r aristotle"

echo "without CONFIRM=yes: refused"
r=$(device confirm)
AGENT_TEST_ROOT=$r PATH="$WORK/bin:$PATH" sh "$AGENT/migrate.sh" > /dev/null 2>&1
check "exit status 2" "[ $? = 2 ]"
check "nothing changed" "[ ! -e $r/var/local/aristotle-migrate ]"

if [ $failures = 0 ]; then
    echo "migrate: all checks passed"
else
    echo "migrate: $failures checks failed (output in $WORK)"
    exit 1
fi
