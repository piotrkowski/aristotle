#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Package Aristotle. This is the one definition of what goes on the Kindle's
# USB drive; the release workflow, CI and tools/host/install-extensions.sh
# all use it.
#
# usage: tools/package.sh tree DIR
#        tools/package.sh zip [FILE]
#        tools/package.sh source [FILE]
#        tools/package.sh version
#
#   tree    Lays out DIR (which must not exist yet) as the root of the drive:
#             DIR/aristotle/             the app bundle of tools/build-render.sh
#                                        device (APP_BUNDLE), with licenses/
#                                        and SOURCE.txt; left out if not built
#             DIR/extensions/aristotle/  the KUAL extension (device/extension/
#                                        and takeover.sh)
#   zip     The same tree as one zip that users unpack to the root of the
#           drive; the app is required. Default FILE:
#           $ARISTOTLE_CACHE/dist/aristotle-VERSION.zip
#   source  The corresponding source of a release: the files of HEAD,
#           third_party/plato at the commit HEAD records, and REVISION and
#           C-SOURCES.txt at the top. Default FILE:
#           $ARISTOTLE_CACHE/dist/aristotle-VERSION-source.tar.gz
#   version Prints VERSION.
#
# VERSION is workspace.package.version in Cargo.toml. APP_BUNDLE defaults to
# $ARISTOTLE_CACHE/render/aristotle-device.tar.gz. File times in the zip and
# the tarball are the commit's (or SOURCE_DATE_EPOCH), so the same inputs give
# the same bytes.
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
. "$REPO/tools/env.sh"
APP_BUNDLE=${APP_BUNDLE:-$ARISTOTLE_CACHE/render/aristotle-device.tar.gz}
VERSION=$(python3 -c 'import sys, tomllib
print(tomllib.load(open(sys.argv[1], "rb"))["workspace"]["package"]["version"])' "$REPO/Cargo.toml")
EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$REPO" log -1 --format=%ct 2>/dev/null || echo 315532800)}

usage() {
    echo "usage: $0 tree DIR | zip [FILE] | source [FILE] | version" >&2
    exit 1
}

# lay_out DIR [app-required]
lay_out() {
    [ ! -e "$1" ] || { echo "$1 exists; give a new directory" >&2; exit 1; }
    if [ -s "$APP_BUNDLE" ]; then
        mkdir -p "$1"
        tar -xzf "$APP_BUNDLE" -C "$1"
        for f in aristotle run-aristotle.sh SOURCE.txt licenses/README.txt; do
            [ -e "$1/aristotle/$f" ] || { echo "$APP_BUNDLE has no aristotle/$f" >&2; exit 1; }
        done
        echo "app: $APP_BUNDLE"
    elif [ "${2:-}" = app-required ]; then
        echo "no app bundle at $APP_BUNDLE; run tools/build-render.sh device or set APP_BUNDLE" >&2
        exit 1
    else
        echo "app: not built ($APP_BUNDLE), left out"
    fi
    mkdir -p "$1/extensions/aristotle"
    cp -R "$REPO/device/extension/." "$1/extensions/aristotle/"
    cp "$REPO/device/app/takeover.sh" "$1/extensions/aristotle/"
}

work() {
    WORK=$(mktemp -d "$TMPDIR/package.XXXXXX")
    trap 'rm -rf "$WORK"' EXIT
}

case ${1:-} in
    version)
        echo "$VERSION"
        ;;
    tree)
        [ $# -eq 2 ] || usage
        lay_out "$2"
        ;;
    zip)
        [ $# -le 2 ] || usage
        OUT=${2:-$ARISTOTLE_CACHE/dist/aristotle-$VERSION.zip}
        mkdir -p "$(dirname "$OUT")"
        work
        lay_out "$WORK/root" app-required
        # Sorted entries, fixed times, Unix modes kept (the binary and the
        # scripts stay executable for unzip on a computer).
        python3 - "$WORK/root" "$WORK/out.zip" "$EPOCH" <<'EOF'
import os, sys, time, zipfile
root, out, epoch = sys.argv[1], sys.argv[2], int(sys.argv[3])
stamp = time.gmtime(max(epoch, 315532800))[:6]
entries = []
for d, dirs, files in os.walk(root):
    entries += [os.path.relpath(os.path.join(d, n), root) + "/" for n in dirs]
    entries += [os.path.relpath(os.path.join(d, n), root) for n in files]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
    for rel in sorted(entries):
        path = os.path.join(root, rel)
        info = zipfile.ZipInfo(rel, stamp)
        info.external_attr = (os.stat(path).st_mode & 0xFFFF) << 16
        if rel.endswith("/"):
            info.external_attr |= 0x10
            z.writestr(info, b"")
        else:
            info.compress_type = zipfile.ZIP_DEFLATED
            with open(path, "rb") as f:
                z.writestr(info, f.read())
EOF
        mv -f "$WORK/out.zip" "$OUT"
        ls -l "$OUT"
        ;;
    source)
        [ $# -le 2 ] || usage
        OUT=${2:-$ARISTOTLE_CACHE/dist/aristotle-$VERSION-source.tar.gz}
        mkdir -p "$(dirname "$OUT")"
        P=aristotle-$VERSION
        COMMIT=$(git -C "$REPO" rev-parse HEAD)
        PLATO=$(git -C "$REPO" rev-parse HEAD:third_party/plato)
        TAG=$(git -C "$REPO" describe --tags --exact-match HEAD 2>/dev/null || true)
        git -C "$REPO/third_party/plato" cat-file -e "$PLATO^{commit}" 2>/dev/null || {
            echo "third_party/plato lacks commit $PLATO; run: git submodule update --init" >&2
            exit 1
        }
        if [ -n "$(git -C "$REPO" status --porcelain --untracked-files=no)" ]; then
            echo "note: uncommitted changes are not in the tarball; it packs HEAD ($COMMIT)" >&2
        fi
        work
        git -C "$REPO" archive --format=tar --prefix="$P/" HEAD > "$WORK/src.tar"
        git -C "$REPO/third_party/plato" archive --format=tar --prefix="$P/third_party/plato/" \
            "$PLATO" > "$WORK/plato.tar"
        mkdir -p "$WORK/top/$P"
        {
            echo "version $VERSION"
            [ -z "$TAG" ] || echo "tag $TAG"
            echo "aristotle $COMMIT"
            echo "plato $PLATO"
        } > "$WORK/top/$P/REVISION"
        {
            echo "The C libraries in Aristotle $VERSION, as tools/build-plato.sh downloads"
            echo "and checks them (name, URL, SHA-256). MuPDF gets the patches in"
            echo "tools/mupdf-patches/ and Plato's kobo.patch; DjVuLibre is built only"
            echo "with PLATO_DJVU=1. tools/build-render.sh pins the fonts and hyphenation"
            echo "patterns it adds to the bundle the same way."
            echo
            sed -n "/^done <<'EOF'\$/,/^EOF\$/p" "$REPO/tools/build-plato.sh" | sed '1d;$d' |
                awk '{ print $1 "\n  " $3 "\n  sha256 " $4 }'
        } > "$WORK/top/$P/C-SOURCES.txt"
        tar -cf "$WORK/top.tar" -C "$WORK/top" --sort=name --owner=0 --group=0 --numeric-owner \
            --mtime="@$EPOCH" "$P/REVISION" "$P/C-SOURCES.txt"
        tar -Af "$WORK/src.tar" "$WORK/plato.tar"
        tar -Af "$WORK/src.tar" "$WORK/top.tar"
        gzip -9n < "$WORK/src.tar" > "$WORK/out.tar.gz"
        mv -f "$WORK/out.tar.gz" "$OUT"
        ls -l "$OUT"
        ;;
    *) usage ;;
esac
