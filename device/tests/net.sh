#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Host-side test of the agent's Wi-Fi policy generator (agent/net.sh).
#
#   sh device/tests/net.sh          compare with net/expected-*
#   UPDATE=1 sh device/tests/net.sh write net/expected-* instead
#
# Runs the pure stages on fixtures, with a stub getent on PATH: the allowlist
# parser, resolution, and the iptables commands for "scoped" (with
# /etc/resolv.conf from net/resolv.conf) and "off". Each output is written to
# a fresh directory under $TMPDIR and diffed against the expected file.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
NET="sh $HERE/../extension/agent/net.sh"
FIX=$HERE/net
WORK=$(mktemp -d "${TMPDIR:-/tmp}/aristotle-net-test.XXXXXX")
PATH="$FIX:$PATH"
export PATH
failures=0

# A fake root, so that the resolvers come from the fixture.
mkdir -p "$WORK/root/etc"
cp "$FIX/resolv.conf" "$WORK/root/etc/resolv.conf"
export AGENT_TEST_ROOT="$WORK/root"

$NET parse < "$FIX/net-allow" > "$WORK/parse" 2> "$WORK/parse-errors"
$NET resolve < "$WORK/parse" > "$WORK/resolve" 2> "$WORK/resolve-errors"
$NET resolvers > "$WORK/resolvers"
# What apply feeds the generator: the resolvers, then the resolved hosts.
cat "$WORK/resolvers" "$WORK/resolve" | $NET rules scoped > "$WORK/rules-scoped"
$NET rules off < /dev/null > "$WORK/rules-off"

for out in parse parse-errors resolve resolve-errors resolvers rules-scoped rules-off; do
    if [ "${UPDATE:-}" = 1 ]; then
        cp "$WORK/$out" "$FIX/expected-$out"
        echo "  wrote expected-$out"
    elif diff -u "$FIX/expected-$out" "$WORK/$out"; then
        echo "  ok    $out"
    else
        echo "  FAIL  $out"
        failures=$((failures + 1))
    fi
done

# The generated rules must parse as shell, as the agent runs them with sh -e.
for out in rules-scoped rules-off; do
    sh -n "$WORK/$out" || { echo "  FAIL  $out is not valid sh"; failures=$((failures + 1)); }
done

# apply, end to end, against the iptables model in net/iptables: scoped twice
# (the second run replaces the first), then off, which must leave the stock
# chains exactly as they were.
check() {
    if eval "$2"; then
        echo "  ok    $1"
    else
        echo "  FAIL  $1"
        failures=$((failures + 1))
    fi
}
S=$AGENT_TEST_ROOT/var/local/aristotle
T=$AGENT_TEST_ROOT/ipt
mkdir -p "$S" "$T"
cp "$FIX/net-allow" "$S/net-allow"
printf '%s\n' '-i wlan0 -p tcp -m state --state RELATED,ESTABLISHED -j ACCEPT' '-i usb0 -j ACCEPT' > "$T/INPUT"
printf '%s\n' '-o usb0 -j ACCEPT' > "$T/OUTPUT"
cp "$T/INPUT" "$WORK/stock-INPUT"
cp "$T/OUTPUT" "$WORK/stock-OUTPUT"
echo scoped > "$S/net-mode"
$NET apply
check "scoped: apply succeeds" "[ $? = 0 ]"
$NET apply
check "scoped: applying again succeeds" "[ $? = 0 ]"
check "scoped: one jump on top of INPUT" "[ \"\$(head -n 1 $T/INPUT)\" = '-i wlan0 -j ARISTOTLE_IN' ] && [ \$(grep -c ARISTOTLE $T/INPUT) = 1 ]"
check "scoped: one jump on top of OUTPUT" "[ \"\$(head -n 1 $T/OUTPUT)\" = '-o wlan0 -j ARISTOTLE_OUT' ] && [ \$(grep -c ARISTOTLE $T/OUTPUT) = 1 ]"
check "scoped: no leftover chains" "[ \"\$(ls $T | tr '\n' ' ')\" = 'ARISTOTLE_IN ARISTOTLE_OUT INPUT OUTPUT ' ]"
check "scoped: ARISTOTLE_OUT as generated" "grep '^iptables -A ARISTOTLE_OUT_NEW' $FIX/expected-rules-scoped | sed 's/^iptables -A ARISTOTLE_OUT_NEW //' | diff - $T/ARISTOTLE_OUT"
check "scoped: hosts resolved" "diff $FIX/expected-resolve $S/net-resolved"
sort "$S/net-resolved" > "$WORK/resolved-sorted"
FAKE_DNS_DOWN=dash.example.org $NET apply
check "scoped: a host that fails to resolve keeps its addresses" "sort $S/net-resolved | diff - $WORK/resolved-sorted"
grep -v '^cdn' "$FIX/net-allow" > "$S/net-allow"
$NET apply
check "scoped: a host removed from net-allow loses its rules" "! grep -q cdn $S/net-resolved && ! grep -q 192.0.2.4 $T/ARISTOTLE_OUT"
echo off > "$S/net-mode"
$NET apply
check "off: apply succeeds" "[ $? = 0 ]"
check "off: stock INPUT and OUTPUT restored" "diff $WORK/stock-INPUT $T/INPUT && diff $WORK/stock-OUTPUT $T/OUTPUT"
check "off: chains removed" "[ ! -e $T/ARISTOTLE_IN ] && [ ! -e $T/ARISTOTLE_OUT ]"

# aristotlectl net allow, as the app's settings call it: tagged lines in
# net-allow, the user's own lines left alone, applied at once in scoped
# mode only.
cp "$HERE/../extension/agent/net.sh" "$S/net.sh"
CTL="sh $HERE/../extension/agent/aristotlectl"
printf '%s\n' 'calibre.lan:8080   # my own line' > "$S/net-allow"
$CTL net allow add calibre.lan:8080 opds > /dev/null
check "allow add: tagged line appended" "grep -qx 'calibre.lan:8080 *# aristotle:opds' $S/net-allow && grep -q '# my own line' $S/net-allow"
$CTL net allow add calibre.lan:8080 opds > /dev/null
check "allow add: twice adds one line" "[ \$(grep -c 'aristotle:opds' $S/net-allow) = 1 ]"
check "allow add: the line parses" "$NET parse < $S/net-allow 2>&1 | sort -u | grep -qx 'calibre.lan 8080 tcp'"
check "allow add: off mode applies nothing" "[ ! -e $T/ARISTOTLE_OUT ]"
for bad in 'bad host:80 opds' 'calibre.lan:99999 opds' 'calibre.lan:80 Opds!' 'calibre.lan:80'; do
    # shellcheck disable=SC2086 # the words are the arguments
    $CTL net allow add $bad > /dev/null 2>&1
    check "allow add: refuses '$bad'" "[ $? = 2 ]"
done
check "allow add: refused entries left no trace" "[ \$(grep -c . $S/net-allow) = 2 ]"
echo scoped > "$S/net-mode"
$CTL net allow add dash.example.org:8443 sync > /dev/null
check "allow add: scoped mode applies at once" "grep -q -- '-d 203.0.113.7 -p tcp --dport 8443 -j ACCEPT' $T/ARISTOTLE_OUT"
$CTL net allow remove calibre.lan:8080 opds > /dev/null
check "allow remove: only the tagged line goes" "! grep -q aristotle:opds $S/net-allow && grep -q '# my own line' $S/net-allow"
$CTL net allow remove dash.example.org:8443 sync > /dev/null
check "allow remove: scoped rules follow" "! grep -q 'dport 8443' $T/ARISTOTLE_OUT"
$CTL net allow remove dash.example.org:8443 sync > /dev/null
check "allow remove: twice is harmless" "[ $? = 0 ]"
echo off > "$S/net-mode"
$NET apply
check "off again: stock INPUT and OUTPUT restored" "diff $WORK/stock-INPUT $T/INPUT && diff $WORK/stock-OUTPUT $T/OUTPUT"

if [ $failures = 0 ]; then
    echo "net: all checks passed"
else
    echo "net: $failures checks failed (output in $WORK)"
    exit 1
fi
