#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Download pinned host-side reverse-engineering tools into $ARISTOTLE_TOOLS
# (default $ARISTOTLE_CACHE/tools, see devloop/env.sh). No root needed; nothing is installed system-wide.
# Each archive is checked against the SHA-256 published by its upstream release.
set -eu

. "$(cd "$(dirname "$0")/.." && pwd)/devloop/env.sh"
TOOLS=${ARISTOTLE_TOOLS:-$ARISTOTLE_CACHE/tools}
DL="$TOOLS/downloads"
mkdir -p "$DL"

fetch() {
    name=$1 url=$2 sha=$3
    file="$DL/$name"
    if [ ! -f "$file" ] || ! printf '%s  %s\n' "$sha" "$file" | sha256sum -c --status; then
        curl -fL --retry 3 -o "$file.part" "$url"
        printf '%s  %s\n' "$sha" "$file.part" | sha256sum -c --status || {
            echo "checksum mismatch: $name" >&2
            exit 1
        }
        mv "$file.part" "$file"
    fi
}

fetch jdk21.tar.gz \
    https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12.1%2B1/OpenJDK21U-jdk_x64_linux_hotspot_21.0.12.1_1.tar.gz \
    ce79869e1307ed8ee1e2baa86a412b1eb5b75d10a01006d788a6f968bcfaee94
fetch ghidra_12.1.4.zip \
    https://github.com/NationalSecurityAgency/ghidra/releases/download/Ghidra_12.1.4_build/ghidra_12.1.4_PUBLIC_20260921.zip \
    ddac49f903da9d5bac833e5cc79395098b9c33cfd3279be5f31bd00387d2d4db
fetch jadx-1.5.6.zip \
    https://github.com/skylot/jadx/releases/download/v1.5.6/jadx-1.5.6.zip \
    545ea2be9c242511bc145755cf4bda2485ade42966e096f8b4d3da2a230e8974

[ -d "$TOOLS/jdk21" ] || {
    mkdir "$TOOLS/jdk21"
    tar -xzf "$DL/jdk21.tar.gz" -C "$TOOLS/jdk21" --strip-components=1
}
[ -d "$TOOLS/ghidra" ] || {
    unzip -q "$DL/ghidra_12.1.4.zip" -d "$TOOLS/ghidra.tmp"
    mv "$TOOLS/ghidra.tmp"/ghidra_* "$TOOLS/ghidra"
    rmdir "$TOOLS/ghidra.tmp"
}
[ -d "$TOOLS/jadx" ] || unzip -q "$DL/jadx-1.5.6.zip" -d "$TOOLS/jadx"

cat > "$TOOLS/env.sh" <<EOF
export JAVA_HOME="$TOOLS/jdk21"
export GHIDRA_INSTALL_DIR="$TOOLS/ghidra"
export PATH="$TOOLS/jdk21/bin:$TOOLS/ghidra/support:$TOOLS/jadx/bin:\$PATH"
EOF
echo "Tools ready. Run: . $TOOLS/env.sh"
