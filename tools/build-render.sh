#!/bin/sh
# Build aristotle-render (crates/aristotle-render) and assemble its bundle:
#
#   $OUT/aristotle-render      static binary
#   $OUT/data/fonts/           the fonts Plato's engine loads unconditionally
#   $OUT/data/serif/           Literata, the body and chrome font
#   $OUT/data/css/             Plato's default stylesheets
#   $OUT/data/hyphenation-patterns/   Liang patterns ($HYPH_LANGS)
#
# usage: tools/build-render.sh [device|host]
#
#   device  armv7-unknown-linux-musleabihf, for the PW4 (default)
#   host    x86_64-unknown-linux-musl, for preview PNGs on the PC
#
# Needs the static C libraries of a tools/build-plato.sh run for the same
# target: PLATO_BUILD (default ~/.cache/aristotle/plato-build/main for device,
# .../host-x86_64 for host; the host one only needs PLATO_LIBS_ONLY=1).
# OUT defaults to ~/.cache/aristotle/render/<device|host>.
# CARGO_CMD=clippy|test runs that cargo command instead of building.
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
WHAT=${1:-device}
case $WHAT in
    device) TARGET=armv7-unknown-linux-musleabihf PFX=arm DEFAULT_BUILD=main ;;
    host) TARGET=x86_64-unknown-linux-musl PFX=x86_64 DEFAULT_BUILD=host-x86_64 ;;
    *) echo "usage: $0 [device|host]" >&2; exit 1 ;;
esac
B=${PLATO_BUILD:-$HOME/.cache/aristotle/plato-build/$DEFAULT_BUILD}
OUT=${OUT:-$HOME/.cache/aristotle/render/$WHAT}
TOOLS=${ARISTOTLE_TOOLS:-$HOME/.cache/aristotle/tools}
# Hyphenation pattern sets to ship. Plato loads every file present when the
# first paragraph is hyphenated, so fewer files means a faster first page.
HYPH_LANGS=${HYPH_LANGS:-pl en-us en-gb de-1996 fr}

if [ ! -f "$B/plato/libs/libmupdf.a" ]; then
    echo "no Plato C libraries in $B/plato/libs" >&2
    echo "run: PLATO_TARGET=$TARGET PLATO_LIBS_ONLY=1 tools/build-plato.sh, or set PLATO_BUILD" >&2
    exit 1
fi

export TMPDIR="$HOME/.cache/aristotle/tmp"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/.cache/aristotle/target-render}
mkdir -p "$TMPDIR"

sh "$REPO/tools/prepare-plato-src.sh"

# plato-core's patched build.rs links the archives from these directories
# (libmupdf_wrapper.a sits in target/mupdf_wrapper/Kindle in older builds).
export PLATO_LIB_DIRS="$B/plato/libs:$B/plato/target/mupdf_wrapper/Kindle"
# zstd-sys (pulled in by zip) compiles C with the cc crate: use Zig too.
T_=$(printf %s "$TARGET" | tr - _)
export "CC_$T_=$B/bin/$PFX-cc" "CXX_$T_=$B/bin/$PFX-c++" "AR_$T_=$B/bin/$PFX-ar" "CFLAGS_$T_=-O2"

# CARGO_CMD=clippy or CARGO_CMD=test runs that instead, in the same
# environment, and skips the bundle (tests need the host target).
CARGO_CMD=${CARGO_CMD:-build}
cargo "$CARGO_CMD" --config "$REPO/.cargo/config.toml" --release --target "$TARGET" \
    --manifest-path "$REPO/crates/aristotle-render/Cargo.toml"
[ "$CARGO_CMD" = build ] || exit 0

SRC="$REPO/third_party/plato-patched"
mkdir -p "$OUT/data/fonts" "$OUT/data/serif" "$OUT/data/css" "$OUT/data/hyphenation-patterns"
cp "$CARGO_TARGET_DIR/$TARGET/release/aristotle-render" "$OUT/aristotle-render"
# Exactly the files plato-core's default_fonts() opens; without any one of
# them the engine has no fonts at all.
for f in LibertinusSerif-Regular.otf LibertinusSerif-Italic.otf LibertinusSerif-Bold.otf \
         LibertinusSerif-BoldItalic.otf NotoSans-Regular.ttf NotoSans-Italic.ttf \
         NotoSans-Bold.ttf NotoSans-BoldItalic.ttf SourceCodeVariable-Roman.otf \
         SourceCodeVariable-Italic.otf Parisienne-Regular.ttf Delius-Regular.ttf; do
    cp "$SRC/fonts/$f" "$OUT/data/fonts/"
done
cp "$REPO/crates/aristotle-render/assets/fonts/"Literata-* "$OUT/data/serif/"
cp "$SRC/css/epub.css" "$SRC/css/html.css" "$OUT/data/css/"

# Plato's hyphenation patterns are prebuilt (kl-hyphenate bincode) and only
# published in its release archive; take them from the release that matches
# the kl-hyphenate version plato-core pins.
ZIP=plato-0.9.45.zip
ZIP_SHA256=d89b828ff02ae2c835e14476be58b5475c612310a101dae00ad367935cece8cb
DL="$TOOLS/downloads"
mkdir -p "$DL"
if ! printf '%s  %s\n' "$ZIP_SHA256" "$DL/$ZIP" | sha256sum -c --status 2>/dev/null; then
    curl -fL --retry 3 -o "$DL/$ZIP.part" \
        "https://github.com/baskerville/plato/releases/download/0.9.45/$ZIP"
    printf '%s  %s\n' "$ZIP_SHA256" "$DL/$ZIP.part" | sha256sum -c --status || {
        echo "checksum mismatch: $ZIP" >&2
        exit 1
    }
    mv "$DL/$ZIP.part" "$DL/$ZIP"
fi
for lang in $HYPH_LANGS; do
    unzip -o -q -j "$DL/$ZIP" "hyphenation-patterns/$lang.standard.bincode" \
        -d "$OUT/data/hyphenation-patterns"
done

file "$OUT/aristotle-render"
ls -l "$OUT/aristotle-render"
du -sh "$OUT/data"
