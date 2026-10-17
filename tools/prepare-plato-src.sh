#!/bin/sh
# Materialise the patched Plato source that crates/aristotle-render builds
# against, at a fixed path inside the repo: third_party/plato-patched
# (gitignored).
#
# It is `git archive` of the commit the third_party/plato submodule pins, plus
# tools/plato-patches/plato-*.patch: the same tree tools/build-plato.sh builds,
# minus the C dependencies, which aristotle-render links from a build-plato.sh
# run instead (PLATO_LIB_DIRS). The submodule itself is never modified.
#
# Idempotent: a stamp records the commit and a hash of the patches, and the
# tree is rebuilt only when either changes.
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
SUB="$REPO/third_party/plato"
OUT="$REPO/third_party/plato-patched"
PATCHES="$REPO/tools/plato-patches"

# The pinned commit is the submodule's gitlink, so this follows submodule bumps.
REV=$(git -C "$REPO" rev-parse HEAD:third_party/plato)
SUM=$(cat "$PATCHES"/plato-*.patch | sha256sum | cut -c1-16)
STAMP="$REV $SUM"

if [ -f "$OUT/.aristotle-stamp" ] && [ "$(cat "$OUT/.aristotle-stamp")" = "$STAMP" ]; then
    exit 0
fi

if ! git -C "$SUB" cat-file -e "$REV^{commit}" 2>/dev/null; then
    echo "third_party/plato does not have $REV; run: git submodule update --init third_party/plato" >&2
    exit 1
fi

NEW="$OUT.new.$$"
mkdir "$NEW"
git -C "$SUB" archive --format=tar "$REV" | tar -x -C "$NEW"
for p in "$PATCHES"/plato-*.patch; do
    echo "applying $(basename "$p")"
    patch -s -d "$NEW" -p1 < "$p"
done
printf '%s\n' "$STAMP" > "$NEW/.aristotle-stamp"

# Generated tree inside the repo, never under ~/.cache, so replacing it is safe.
if [ -e "$OUT" ]; then
    rm -r "$OUT"
fi
mv "$NEW" "$OUT"
echo "patched Plato source: $OUT ($REV)"
