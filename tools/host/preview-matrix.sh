#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Render synthetic screen sizes on the host, without a Kindle or firmware.
set -eu

REPO=$(cd "$(dirname "$0")/../.." && pwd)
. "$REPO/tools/env.sh"
export CARGO_TARGET_DIR="$ARISTOTLE_CACHE/target"
TARGET=x86_64-unknown-linux-musl
DEST=${1:-$ARISTOTLE_CACHE/ui-preview/geometry}
mkdir -p "$DEST"
DEST=$(cd "$DEST" && pwd)
BUNDLE="$DEST/bundle"
BOOKS="$DEST/books"
mkdir -p "$BOOKS"
cd "$REPO"

python3 tools/host/make-reading-test-epub.py "$BOOKS/test.epub"
cargo build --release --target "$TARGET" --locked -p aristotle-ui --bin aristotle-ui-preview
OUT="$BUNDLE" CARGO_FLAGS=--locked tools/build-render.sh host

for screen_case in 600x800:167 758x1024:212 800x1200:300 1072x1448:300 1236x1648:300; do
    size=${screen_case%:*}
    dpi=${screen_case#*:}
    case_out="$DEST/${size}-${dpi}dpi"
    "$CARGO_TARGET_DIR/$TARGET/release/aristotle-ui-preview" --size "$size" --dpi "$dpi" --lang all "$case_out/ui"
    for lang in en pl; do
        ARISTOTLE_ACCOUNTS_DIR="$case_out/reading/$lang/accounts" \
            "$BUNDLE/aristotle" --data "$BUNDLE/data" --lang "$lang" --dpi "$dpi" \
            --library "$case_out/reading/$lang" \
            --preview-layout "$case_out/reading/$lang" "$BOOKS/test.epub" "${size%x*}" "${size#*x}"
    done
done

printf 'Preview matrix passed: %s\n' "$DEST"
