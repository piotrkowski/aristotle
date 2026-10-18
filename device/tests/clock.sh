#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Exercise the USB clock helper without contacting or changing a device.
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
. "$REPO/tools/env.sh"
T=$(mktemp -d "$TMPDIR/aristotle-clock-test.XXXXXX")
trap 'rm -rf "$T"' EXIT
trap 'exit 1' HUP INT TERM
export TEST_CLOCK_DIR="$T"
mkdir -p "$T/bin" "$T/root/proc" "$T/root/etc"
printf 'G000PP0000000000\n' > "$T/root/proc/usid"
printf 'Kindle 5.13.6 (test)\n' > "$T/root/etc/prettyversion.txt"
cat > "$T/bin/ssh" <<'SSH'
#!/bin/sh
set -eu
printf '%s\n' "$@" > "$TEST_CLOCK_DIR/ssh-args"
[ "${TEST_CLOCK_SSH_FAIL:-0}" = 0 ] || exit 67
for arg do command=$arg; done
export AGENT_TEST_ROOT="$TEST_CLOCK_DIR/root" TEST_CLOCK_REMOTE=1
exec sh -c "$command"
SSH
cat > "$T/bin/date" <<'DATE'
#!/bin/sh
set -eu
if [ "${TEST_CLOCK_REMOTE:-0}" = 0 ]; then
    echo "${TEST_CLOCK_HOST_EPOCH:-1792306800}"
elif [ "$*" = '-u +%s' ]; then
    echo 1792306200
else
    [ "$1" = -u ] && [ "$2" = -s ] && [ "$3" = @1792306800 ] || exit 1
    printf '%s\n' "$*" > "$TEST_CLOCK_DIR/clock-set"
    echo 'Runtime clock: test'
fi
DATE
printf '#!/bin/sh\nexit 0\n' > "$T/bin/eips"
chmod +x "$T/bin/ssh" "$T/bin/date" "$T/bin/eips"
PATH="$T/bin:$PATH"
export PATH

sh "$REPO/tools/host/sync-clock.sh" --check > "$T/check"
grep -qx 'Device minus host: -600 seconds' "$T/check"
[ ! -e "$T/clock-set" ]
grep -qx "$REPO/tools/host/ssh_config" "$T/ssh-args"
echo '  ok    check reports clock drift without changing it'

sh "$REPO/tools/host/sync-clock.sh" > "$T/set"
[ -s "$T/clock-set" ]
echo '  ok    set uses the numeric host UTC epoch over USB'
rm "$T/clock-set"

printf 'Kindle 5.16.2 (test)\n' > "$T/root/etc/prettyversion.txt"
if sh "$REPO/tools/host/sync-clock.sh" > "$T/unsupported" 2>&1; then exit 1; fi
[ ! -e "$T/clock-set" ]
grep -q 'nothing changed' "$T/unsupported"
echo '  ok    the device guard refuses unsupported firmware before setting time'

rm "$T/ssh-args"
if TEST_CLOCK_HOST_EPOCH=0 sh "$REPO/tools/host/sync-clock.sh" > "$T/bad-clock" 2>&1; then exit 1; fi
[ ! -e "$T/ssh-args" ]
echo '  ok    an invalid host clock never reaches SSH'

if TEST_CLOCK_SSH_FAIL=1 sh "$REPO/tools/host/sync-clock.sh" --check > "$T/ssh-failure" 2>&1; then exit 1; else result=$?; fi
[ "$result" = 67 ]
echo '  ok    SSH failures reach the caller'
echo 'clock: all checks passed'
