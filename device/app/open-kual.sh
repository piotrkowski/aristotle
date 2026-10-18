#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Open the installed KUAL booklet after the stock framework is ready.
set -eu
R=${AGENT_TEST_ROOT:-}
WAIT=${KUAL_WAIT:-90}
case "$WAIT" in '' | *[!0-9]*) echo 'invalid KUAL wait budget' >&2; exit 1 ;; esac
if [ "${#WAIT}" -gt 3 ] || [ "$WAIT" -lt 1 ] || [ "$WAIT" -gt 180 ]; then
    echo 'KUAL wait budget must be 1 to 180 seconds' >&2
    exit 1
fi
if [ ! -f "$R/opt/amazon/ebook/booklet/KUALBooklet.jar" ]; then
    echo "KUAL is not installed; the Kindle interface stays active" >&2
    exit 1
fi
command -v timeout > /dev/null || exit 1
now() { cut -d. -f1 /proc/uptime; }
deadline=$(($(now) + WAIT))
next_request=0
bounded() {
    left=$((deadline - $(now)))
    [ "$left" -gt 0 ] || return 1
    [ "$left" -lt 3 ] || left=3
    timeout -t "$left" -s KILL "$@"
}
while [ "$(now)" -lt "$deadline" ]; do
    if bounded initctl status framework 2> /dev/null | grep -q start/running; then
        pid=$(bounded lipc-get-prop com.lab126.appmgrd activeAppPid 2> /dev/null) || pid=
        active=$(bounded lipc-get-prop com.lab126.appmgrd activeApp 2> /dev/null) || active=
        case "$pid" in
        '' | 0 | *[!0-9]*) ;;
        *)
            if kill -0 "$pid" 2> /dev/null; then
                if [ "$active" = com.mobileread.ixtab.kindlelauncher ]; then
                    echo "KUAL launch confirmed"
                    exit 0
                fi
                if [ -n "$active" ] && [ "$(now)" -ge "$next_request" ]; then
                    if bounded lipc-set-prop com.lab126.appmgrd start app://com.mobileread.ixtab.kindlelauncher; then
                        next_request=$(($(now) + 5))
                        continue
                    fi
                    next_request=$(($(now) + 2))
                fi
            fi
            ;;
        esac
    fi
    sleep 1
done
echo "KUAL did not become available; the Kindle interface stays active" >&2
exit 1
