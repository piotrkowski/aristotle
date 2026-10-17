# SPDX-License-Identifier: AGPL-3.0-or-later
# Host settings shared by the scripts in tools/. Source it:
#
#   . tools/env.sh
#
# ARISTOTLE_CACHE is the one directory for everything large or local that stays
# out of git: build directories, the C libraries, downloaded tools, the
# emulator, screenshots, probe output and the dev-loop SSH key
# ($ARISTOTLE_CACHE/ssh/kindle_ed25519). Default:
# ${XDG_CACHE_HOME:-$HOME/.cache}/aristotle. Set it before sourcing to put the
# cache elsewhere.
#
# TMPDIR goes there too: the C build, the emulator and the tests write large
# temporary files, and /tmp is a small RAM disk on many hosts.

: "${ARISTOTLE_CACHE:=${XDG_CACHE_HOME:-$HOME/.cache}/aristotle}"
export ARISTOTLE_CACHE
export TMPDIR="$ARISTOTLE_CACHE/tmp"
mkdir -p "$TMPDIR"
