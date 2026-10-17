#!/bin/sh
# Build the aristotle-render workspace (crates/aristotle-render and the
# reader app, crates/aristotle-reader) and assemble its bundle:
#
#   $OUT/aristotle-render      static binary: the renderer CLI
#   $OUT/aristotle             static binary: the reader app
#   $OUT/data/fonts/           the fonts Plato's engine loads unconditionally
#   $OUT/data/serif/           Literata, the body and chrome font
#                              (fonts cut to $FONT_SUBSET, see below)
#   $OUT/data/css/            Plato's default stylesheets
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
# OUT defaults to ~/.cache/aristotle/render/<device|host>; the device tarball
# goes to BUNDLE (default ~/.cache/aristotle/render/aristotle-render-device.tar.gz)
# and the reader app's to APP_BUNDLE (.../aristotle-app-device.tar.gz).
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
# Hyphenation pattern sets to ship: the owner's languages. Plato loads every
# file present when the first paragraph is hyphenated, so fewer files means a
# faster first page. Text in a language without patterns is not hyphenated.
HYPH_LANGS=${HYPH_LANGS:-pl en-us en-gb grc la}
# FONT_SUBSET=latin-greek (default) cuts the fonts to the owner's scripts;
# FONT_SUBSET=none ships them as they come.
FONT_SUBSET=${FONT_SUBSET:-latin-greek}
case $FONT_SUBSET in
    latin-greek|none) ;;
    *) echo "FONT_SUBSET must be latin-greek or none" >&2; exit 1 ;;
esac

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
# Any crate that compiles C with the cc crate uses Zig too (none does since
# zip lost its zstd codec, but the setup costs nothing).
T_=$(printf %s "$TARGET" | tr - _)
export "CC_$T_=$B/bin/$PFX-cc" "CXX_$T_=$B/bin/$PFX-c++" "AR_$T_=$B/bin/$PFX-ar" "CFLAGS_$T_=-O2"

# CARGO_CMD=clippy or CARGO_CMD=test runs that instead, in the same
# environment, and skips the bundle (tests need the host target).
CARGO_CMD=${CARGO_CMD:-build}
cargo "$CARGO_CMD" --config "$REPO/.cargo/config.toml" --release --target "$TARGET" \
    --manifest-path "$REPO/crates/aristotle-render/Cargo.toml" --workspace
[ "$CARGO_CMD" = build ] || exit 0

SRC="$REPO/third_party/plato-patched"
DL="$TOOLS/downloads"
mkdir -p "$DL"
# fetch NAME URL SHA256: download once into $DL, checked.
fetch() {
    if ! printf '%s  %s\n' "$3" "$DL/$1" | sha256sum -c --status 2>/dev/null; then
        curl -fL --retry 3 -o "$DL/$1.part" "$2"
        printf '%s  %s\n' "$3" "$DL/$1.part" | sha256sum -c --status || {
            echo "checksum mismatch: $1" >&2
            exit 1
        }
        mv "$DL/$1.part" "$DL/$1"
    fi
}

# The data directory is assembled from scratch, so nothing from an earlier
# configuration stays behind.
rm -rf "$OUT/data"
mkdir -p "$OUT/data/fonts" "$OUT/data/serif" "$OUT/data/css" "$OUT/data/hyphenation-patterns"
cp "$CARGO_TARGET_DIR/$TARGET/release/aristotle-render" "$OUT/aristotle-render"
cp "$CARGO_TARGET_DIR/$TARGET/release/aristotle" "$OUT/aristotle"

# Fonts. With FONT_SUBSET=latin-greek, Literata (the body font) and Noto Sans
# (CSS sans-serif) keep Latin with all its extensions, Greek with polytonic,
# combining marks, punctuation, super- and subscripts, currency, letterlike
# symbols, number forms, arrows, mathematical operators, geometric shapes and
# the f-ligatures, with all OpenType features; Cyrillic and the rest go.
# Literata keeps its hinting, which the reader app's UI uses (it maps these
# files instead of compiling the font in). Libertinus Serif, the
# engine's own serif family, is opened unconditionally but never drawn,
# because Book replaces the serif family with Literata, so it ships as a stub
# that only maps the space. Source Code Pro and Parisienne are shipped
# unmodified: their licences reserve their names for the original fonts.
# Delius is small already. Subsetting runs fontTools from its pure-Python
# wheel, pinned like the other downloads.
FONTTOOLS=fonttools-4.60.1-py3-none-any.whl
fetch "$FONTTOOLS" \
    "https://files.pythonhosted.org/packages/c7/93/0dd45cd283c32dea1545151d8c3637b4b8c53cdb3a625aeb2885b184d74d/$FONTTOOLS" \
    906306ac7afe2156fcf0042173d6ebbb05416af70f6b370967b47f8f00103bbb
TEXT_UNICODES=U+0000-036F,U+0370-03FF,U+1E00-1FFF,U+2000-22FF,U+25A0-25FF,U+FB00-FB06
# subset SRC DST UNICODES
subset() {
    if [ "$FONT_SUBSET" = none ]; then
        cp "$1" "$2"
        return
    fi
    PYTHONPATH="$DL/$FONTTOOLS" python3 -m fontTools.subset "$1" --output-file="$2" \
        --unicodes="$3" --layout-features='*' --name-IDs='*' --name-languages='*' \
        --notdef-outline
}
for f in LibertinusSerif-Regular.otf LibertinusSerif-Italic.otf LibertinusSerif-Bold.otf \
         LibertinusSerif-BoldItalic.otf; do
    subset "$SRC/fonts/$f" "$OUT/data/fonts/$f" U+0020
done
for f in NotoSans-Regular.ttf NotoSans-Italic.ttf NotoSans-Bold.ttf NotoSans-BoldItalic.ttf; do
    subset "$SRC/fonts/$f" "$OUT/data/fonts/$f" "$TEXT_UNICODES"
done
# The rest of what plato-core's default_fonts() opens; without any one of
# them the engine has no fonts at all.
for f in SourceCodeVariable-Roman.otf SourceCodeVariable-Italic.otf Parisienne-Regular.ttf \
         Delius-Regular.ttf; do
    cp "$SRC/fonts/$f" "$OUT/data/fonts/"
done
for f in "$REPO/crates/aristotle-render/assets/fonts/"Literata-*.ttf; do
    subset "$f" "$OUT/data/serif/$(basename "$f")" "$TEXT_UNICODES"
done
cp "$REPO/crates/aristotle-render/assets/fonts/Literata-OFL.txt" "$OUT/data/serif/"
cp "$SRC/css/epub.css" "$SRC/css/html.css" "$OUT/data/css/"

# Plato's hyphenation patterns are prebuilt (kl-hyphenate bincode) and only
# published in its release archive; take them from the release that matches
# the kl-hyphenate version plato-core pins.
ZIP=plato-0.9.45.zip
fetch "$ZIP" "https://github.com/baskerville/plato/releases/download/0.9.45/$ZIP" \
    d89b828ff02ae2c835e14476be58b5475c612310a101dae00ad367935cece8cb
for lang in $HYPH_LANGS; do
    unzip -o -q -j "$DL/$ZIP" "hyphenation-patterns/$lang.standard.bincode" \
        -d "$OUT/data/hyphenation-patterns"
done

file "$OUT/aristotle-render" "$OUT/aristotle"
ls -l "$OUT/aristotle-render" "$OUT/aristotle"
du -sh "$OUT/data"

# Device bundles: binary plus data, rebuilt from what was just built so an
# upload can never carry a stale binary (it once did). One per binary,
# because both together unpack to about 60 MB and /var on the device is a
# 64 MB tmpfs. Unpack streamed:
#   ssh kindle 'tar -xzf - -C /var/tmp' < ~/.cache/aristotle/render/aristotle-render-device.tar.gz
# The reader app's bundle (aristotle/ with the app, data/ and
# run-aristotle.sh) is meant for the userstore; see docs/reader.md:
#   ssh kindle 'tar -xzf - -C /mnt/us' < ~/.cache/aristotle/render/aristotle-app-device.tar.gz
if [ "$WHAT" = device ]; then
    BUNDLE=${BUNDLE:-$HOME/.cache/aristotle/render/aristotle-render-device.tar.gz}
    tar -czf "$BUNDLE.part" -C "$OUT" --transform 's,^\.,aristotle,' ./aristotle-render ./data
    mv -f "$BUNDLE.part" "$BUNDLE"
    APP_BUNDLE=${APP_BUNDLE:-$HOME/.cache/aristotle/render/aristotle-app-device.tar.gz}
    cp "$REPO/devloop/kindle/run-aristotle.sh" "$REPO/devloop/kindle/takeover.sh" "$OUT/"
    tar -czf "$APP_BUNDLE.part" -C "$OUT" --transform 's,^\.,aristotle,' \
        ./aristotle ./data ./run-aristotle.sh ./takeover.sh
    mv -f "$APP_BUNDLE.part" "$APP_BUNDLE"
    ls -l "$BUNDLE" "$APP_BUNDLE"
fi
