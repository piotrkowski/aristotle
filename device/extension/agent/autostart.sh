#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Manage the internal boot copy without changing root filesystem jobs.
set -eu
umask 077
HERE=$(cd "$(dirname "$0")" && pwd)
EXT=$(dirname "$HERE")
R=${AGENT_TEST_ROOT:-}
S=$R/var/local/aristotle
APP=${ARISTOTLE_APP_DIR:-$R/mnt/us/aristotle}
LOCK=$R/var/run/aristotle-app-update.lock
PRESERVE_STAGE=0

say() {
    printf '%s\n' "$*"
    eips 0 1 "$*                                        " > /dev/null 2>&1 || :
}

cleanup() {
    [ "$PRESERVE_STAGE" = 1 ] || rm -rf "$S/app.new"
    rmdir "$LOCK"
}

lock() {
    if ! mkdir "$LOCK" 2> /dev/null; then
        say "aristotle: another app update is active; retry after it finishes or reboot"
        exit 1
    fi
    trap cleanup EXIT
    trap 'exit 1' HUP INT TERM
}

prepare() {
    [ -f "$EXT/guard.sh" ] || { say "aristotle: guard.sh missing"; exit 1; }
    . "$EXT/guard.sh"
    device_guard || exit 1
    if [ ! -f "$R/etc/upstart/aristotle-ui.conf" ] || [ ! -f "$S/ui-boot.sh" ]; then
        say "aristotle: install the agent first"
        exit 1
    fi
    lock
    if initctl status aristotle-ui 2> /dev/null | grep -q start/running; then
        say "aristotle: quit the boot app before changing its copy"
        exit 1
    fi
    if [ -s "$S/app.pid" ] && kill -0 "$(cat "$S/app.pid")" 2> /dev/null; then
        say "aristotle: the app is still running; stop it first"
        exit 1
    fi
    for path in "$S/app" "$S/app.new" "$S/app.previous"; do
        if [ -L "$path" ] || { [ -e "$path" ] && [ ! -d "$path" ]; }; then
            say "aristotle: unexpected app path: $path"
            exit 1
        fi
    done
    if [ ! -d "$S/app" ] && [ -d "$S/app.previous" ]; then
        mv "$S/app.previous" "$S/app"
    fi
}

activate() {
    sync
    rm -rf "$S/app.previous"
    if [ -d "$S/app" ]; then
        mv "$S/app" "$S/app.previous"
    fi
    if ! mv "$S/app.new" "$S/app"; then
        if [ -d "$S/app.previous" ] && ! mv "$S/app.previous" "$S/app"; then
            PRESERVE_STAGE=1
            say "aristotle: restore failed; app.previous and app.new were kept"
            exit 1
        fi
        say "aristotle: activation failed; the previous app was kept"
        exit 1
    fi
    echo 0 > "$S/ui-fails"
    : > "$S/autostart"
    sync
}

case "${1:-status}" in
on)
    prepare
    if [ ! -x "$APP/aristotle" ] || [ ! -d "$APP/data" ] || [ ! -s "$APP/SOURCE.txt" ]; then
        say "aristotle: incomplete app in /mnt/us/aristotle"
        exit 1
    fi
    rm -rf "$S/app.new"
    mkdir "$S/app.new"
    if ! cp -r "$APP/aristotle" "$APP/data" "$APP/SOURCE.txt" "$S/app.new/"; then
        say "aristotle: copying the app failed (space on /var/local?)"
        exit 1
    fi
    cp "$APP/SOURCE.txt" "$S/app.new/USB-SOURCE.txt"
    chmod 700 "$S/app.new/aristotle"
    activate
    say "Aristotle starts at the next boot; the previous app is kept"
    ;;
rollback)
    prepare
    if [ ! -x "$S/app.previous/aristotle" ] || [ ! -d "$S/app.previous/data" ]; then
        say "aristotle: no previous boot app; nothing changed"
        exit 1
    fi
    rm -rf "$S/app.new"
    mv "$S/app.previous" "$S/app.new"
    if [ -s "$APP/SOURCE.txt" ]; then
        cp "$APP/SOURCE.txt" "$S/app.new/USB-SOURCE.txt"
    fi
    activate
    say "Previous boot app restored; reading data was not changed"
    ;;
off)
    lock
    rm -f "$S/autostart" "$S/ui-fails"
    sync
    say "Autostart is off; internal app copies are kept for rollback"
    ;;
status)
    if [ -e "$S/autostart" ]; then echo "on"; else echo "off"; fi
    ;;
*)
    echo "usage: autostart.sh on|off|rollback|status" >&2
    exit 2
    ;;
esac
