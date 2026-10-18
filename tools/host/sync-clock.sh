#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Set the runtime clock over USB without changing NTP or the hardware clock.
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
. "$REPO/tools/env.sh"
case "${1:-}" in
'') action=set ;;
--check) action=check ;;
*) echo "usage: $0 [--check]" >&2; exit 2 ;;
esac
[ $# -le 1 ] || { echo "usage: $0 [--check]" >&2; exit 2; }

epoch=$(date -u +%s)
case "$epoch" in '' | *[!0-9]*) echo 'Invalid host time' >&2; exit 1 ;; esac
if [ "$epoch" -lt 1704067200 ] || [ "$epoch" -gt 2147483647 ]; then
    echo 'Host time must be between 2024-01-01 and 2038-01-19' >&2
    exit 1
fi

{
    cat "$REPO/device/extension/guard.sh"
    cat <<'REMOTE'

set -eu
device_guard || exit 1
action=$1 epoch=$2
before=$(date -u +%s)
printf 'Device minus host: %s seconds\n' "$((before - epoch))"
if [ "$action" = set ]; then
    date -u -s "@$epoch" '+Runtime clock: %Y-%m-%d %H:%M:%S UTC'
fi
REMOTE
} | ssh -F "$REPO/tools/host/ssh_config" kindle "sh -s -- $action $epoch"
