#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Gate DNS to test concurrent off and uninstall against the iptables model.
set -eu
REPO=$(cd "$(dirname "$0")/../.." && pwd)
. "$REPO/tools/env.sh"
T=$(mktemp -d "$TMPDIR/aristotle-net-overlap.XXXXXX")
owner= waiter=
cleanup() {
    result=$?
    for p in "$owner" "$waiter"; do
        [ -z "$p" ] || kill "$p" 2> /dev/null || :
    done
    p=$(cat "$T/gate/pid" 2> /dev/null || :)
    if [ -n "$p" ] && grep -qF "$T/bin/getent" "/proc/$p/cmdline" 2> /dev/null; then
        kill "$p" 2> /dev/null || :
    fi
    [ -z "$owner" ] || wait "$owner" 2> /dev/null || :
    [ -z "$waiter" ] || wait "$waiter" 2> /dev/null || :
    [ "$result" = 0 ] || cat "$T"/*.log >&2 2> /dev/null || :
    rm -rf "$T"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
export NET_TEST_DIR="$T" NET_TEST_FLOCK="$(command -v flock)" NET_TEST_REPO="$REPO"
export AGENT_TEST_ROOT="$T/root" AGENT_TEST_OWNER="$(id -u)"
mkdir -p "$T/bin" "$AGENT_TEST_ROOT/etc" "$AGENT_TEST_ROOT/proc/sys/kernel/random" "$AGENT_TEST_ROOT/var/run" "$AGENT_TEST_ROOT/ipt"
cat > "$T/bin/getent" <<'EOF'
#!/bin/sh
if [ -d "$NET_TEST_DIR/gate" ] && [ ! -e "$NET_TEST_DIR/gate/passed" ]; then
    echo $$ > "$NET_TEST_DIR/gate/pid"
    touch "$NET_TEST_DIR/gate/started"
    read -r release < "$NET_TEST_DIR/gate/release"
    touch "$NET_TEST_DIR/gate/passed"
fi
echo '192.0.2.10 STREAM dash.example.org'
EOF
cat > "$T/bin/flock" <<'EOF'
#!/bin/sh
[ -z "${NET_TEST_WAIT_MARKER:-}" ] || printf '%s\n' "$*" > "$NET_TEST_WAIT_MARKER"
exec "$NET_TEST_FLOCK" "$@"
EOF
cat > "$T/bin/iptables" <<'EOF'
#!/bin/sh
if [ "${NET_TEST_TEARDOWN_FAIL:-0}" = 1 ]; then
    case "$1" in -D | -F | -X) exit 1 ;; esac
fi
exec "$NET_TEST_REPO/device/tests/net/iptables" "$@"
EOF
cat > "$T/bin/device-stub" <<'EOF'
#!/bin/sh
case "${0##*/}:$1" in initctl:status) exit 1 ;; esac
exit 0
EOF
chmod +x "$T/bin/getent" "$T/bin/flock" "$T/bin/iptables" "$T/bin/device-stub"
for command in stop start initctl eips mntroot sync ifconfig rmmod umount lsmod lipc-set-prop; do
    ln -s device-stub "$T/bin/$command"
done
PATH="$T/bin:$PATH"
export PATH
S=$AGENT_TEST_ROOT/var/local/aristotle
IPT=$AGENT_TEST_ROOT/ipt
NET=$REPO/device/extension/agent/net.sh
CTL=$REPO/device/extension/agent/aristotlectl
UNINSTALL=$REPO/device/extension/agent/uninstall.sh
cp "$REPO/device/tests/net/resolv.conf" "$AGENT_TEST_ROOT/etc/resolv.conf"
printf '1000.0 0.0\n' > "$AGENT_TEST_ROOT/proc/uptime"
printf 'test-boot\n' > "$AGENT_TEST_ROOT/proc/sys/kernel/random/boot_id"
printf '%s\n' '-i usb0 -p tcp --dport 2222 -j ACCEPT' > "$T/stock-INPUT"
printf '%s\n' '-o usb0 -p tcp --sport 2222 -j ACCEPT' > "$T/stock-OUTPUT"

prepare() {
    mkdir -p "$S"
    chmod 700 "$S"
    cp "$NET" "$S/net.sh"
    cp "$T/stock-INPUT" "$IPT/INPUT"
    cp "$T/stock-OUTPUT" "$IPT/OUTPUT"
    printf 'dash.example.org\n' > "$S/net-allow"
    echo scoped > "$S/net-mode"
}
await_file() {
    i=0
    until [ -e "$1" ]; do
        i=$((i + 1))
        [ "$i" -lt 500 ] || { echo "Timeout waiting for $1" >&2; return 1; }
        sleep 0.01
    done
}
for action in off uninstall; do
    prepare
    mkdir "$T/gate"
    mkfifo "$T/gate/release"
    sh "$NET" apply > "$T/owner-$action.log" 2>&1 &
    owner=$!
    await_file "$T/gate/started"
    if [ "$action" = off ]; then
        NET_TEST_WAIT_MARKER="$T/waiting" sh "$CTL" net off > "$T/off.log" 2>&1 &
    else
        NET_TEST_WAIT_MARKER="$T/waiting" sh "$UNINSTALL" > "$T/uninstall.log" 2>&1 &
    fi
    waiter=$!
    await_file "$T/waiting"
    grep -qx 9 "$T/waiting"
    [ -f "$S/net.sh" ]
    kill -0 "$waiter"
    printf 'continue\n' > "$T/gate/release"
    wait "$owner"
    owner=
    wait "$waiter"
    waiter=
    diff "$T/stock-INPUT" "$IPT/INPUT"
    diff "$T/stock-OUTPUT" "$IPT/OUTPUT"
    [ ! -e "$IPT/ARISTOTLE_IN" ] && [ ! -e "$IPT/ARISTOTLE_OUT" ]
    if [ "$action" = off ]; then [ "$(cat "$S/net-mode")" = off ]; else [ ! -e "$S" ]; fi
    rm -rf "$T/gate" "$T/waiting"
    echo "  ok    $action waits for DNS owner and restores stock and USB rules"
done

prepare
sh "$NET" apply > "$T/initial.log" 2>&1
mkdir -p "$AGENT_TEST_ROOT/etc/upstart"
touch "$AGENT_TEST_ROOT/etc/upstart/aristotle-agent.conf"
printf 'test key\n' > "$S/authorized_keys"
if NET_TEST_TEARDOWN_FAIL=1 sh "$UNINSTALL" > "$T/failed-uninstall.log" 2>&1; then
    echo 'Uninstall ignored a teardown failure' >&2
    exit 1
fi
[ -f "$S/net.sh" ] && [ -f "$S/authorized_keys" ]
[ -f "$AGENT_TEST_ROOT/etc/upstart/aristotle-agent.conf" ]
grep -q 'files kept, retry removal' "$T/failed-uninstall.log"
sh "$UNINSTALL" > "$T/retried-uninstall.log" 2>&1
[ ! -e "$S" ]
diff "$T/stock-INPUT" "$IPT/INPUT"
diff "$T/stock-OUTPUT" "$IPT/OUTPUT"
echo '  ok    failed teardown keeps jobs, keys and scripts for a successful retry'
echo 'net-overlap: all checks passed'
