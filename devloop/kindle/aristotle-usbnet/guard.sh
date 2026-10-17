# Device guard, sourced (not run) by every script that changes the device
# outside /mnt/us: agent/install.sh, agent/migrate.sh, bin/start.sh and
# takeover.sh. Aristotle is built and tested on one model and one firmware
# line only, the Kindle Paperwhite 4 (2018) on firmware 5.13.x; its scripts
# load kernel modules, open the firewall and write the root filesystem the way
# that device needs. device_guard refuses everything else:
#
#   . "$EXT/guard.sh"
#   device_guard || exit 1
#
# On a mismatch it prints why, shows it on screen with eips and returns 1, so
# the caller exits before it has changed anything.
#
#   ARISTOTLE_FORCE=1   skip the check. For developers trying another model or
#                       firmware on purpose, at their own risk.
#
# The model: the device code in the serial number, /proc/usid. Serials that
# start with B or 9 are the older scheme (code in characters 3-4) and are
# never a PW4; the others carry the code in characters 4-6. The PW4 codes are
# KOReader's (frontend/device/kindle/device.lua, pw4_set); "0PP" is verified
# on our device. Only the code is ever printed, never the serial.
# The firmware: the first line of /etc/prettyversion.txt, "Kindle 5.13.6 (...)".
#
# AGENT_TEST_ROOT stands in for / in the host-side tests (devloop/kindle/tests/).

PW4_CODES="0PP 0T1 0T2 0T3 0T4 0T5 0T6 0T7 0TJ 0TK 0TL 0TM 0TN 102 103 16Q 16R 16S 16T 16U 16V"

# The device code from /proc/usid; empty when it cannot be read.
device_code() {
    guard_sn=$(head -n 1 "${AGENT_TEST_ROOT:-}/proc/usid" 2> /dev/null)
    case "$guard_sn" in
    '') ;;
    B* | 9*) printf '%s\n' "$guard_sn" | cut -c3-4 ;;
    *) printf '%s\n' "$guard_sn" | cut -c4-6 ;;
    esac
}

# The firmware version, e.g. 5.13.6; empty when it cannot be read.
firmware_version() {
    sed -n '1s/^Kindle \([0-9][0-9.]*\).*/\1/p' "${AGENT_TEST_ROOT:-}/etc/prettyversion.txt" 2> /dev/null
}

device_guard() {
    [ "${ARISTOTLE_FORCE:-}" = 1 ] && return 0
    guard_code=$(device_code)
    guard_fw=$(firmware_version)
    guard_why=""
    case " $PW4_CODES " in
    *" ${guard_code:-none} "*) ;;
    *) guard_why="not a Kindle Paperwhite 4 (device code ${guard_code:-unknown})" ;;
    esac
    if [ -z "$guard_why" ]; then
        case "$guard_fw" in
        5.13 | 5.13.*) ;;
        *) guard_why="firmware ${guard_fw:-unknown}, not 5.13.x" ;;
        esac
    fi
    [ -z "$guard_why" ] && return 0
    printf 'aristotle: %s; nothing changed\n' "$guard_why" >&2
    printf 'aristotle: only the PW4 (2018) on 5.13.x is supported; ARISTOTLE_FORCE=1 overrides\n' >&2
    eips 0 1 "aristotle: $guard_why                    " > /dev/null 2>&1
    eips 0 2 "only PW4 on 5.13.x; nothing changed                 " > /dev/null 2>&1
    return 1
}
