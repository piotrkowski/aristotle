#!/bin/sh
# Build the workspace members that link Plato (crates/aristotle-render and
# the reader app, crates/aristotle-reader) and assemble their bundle:
#
#   $OUT/aristotle-render      static binary: the renderer CLI
#   $OUT/aristotle             static binary: the reader app
#   $OUT/data/fonts/           the fonts Plato's engine loads unconditionally
#   $OUT/data/serif/           Literata, the body and chrome font
#                              (fonts cut to $FONT_SUBSET, see below)
#   $OUT/data/css/            Plato's default stylesheets
#   $OUT/data/hyphenation-patterns/   Liang patterns ($HYPH_LANGS)
#   $OUT/licenses/             the licences of everything above
#                              (tools/bundle-licenses.py)
#   $OUT/SOURCE.txt            where the corresponding source is
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
# Hyphenation pattern sets to ship: Polish, English, Ancient Greek and Latin
# by default. Plato loads every
# file present when the first paragraph is hyphenated, so fewer files means a
# faster first page. Text in a language without patterns is not hyphenated.
HYPH_LANGS=${HYPH_LANGS:-pl en-us en-gb grc la}
# FONT_SUBSET=latin-greek (default) cuts the fonts to Latin and Greek;
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

SRC="$REPO/third_party/plato"
if [ ! -f "$SRC/crates/core/Cargo.toml" ]; then
    echo "third_party/plato is not checked out; run: git submodule update --init" >&2
    exit 1
fi

# plato-core's build.rs (the fork's "core: lib dirs and host musl") links the
# archives from these directories (libmupdf_wrapper.a sits in
# target/mupdf_wrapper/Kindle in older builds).
export PLATO_LIB_DIRS="$B/plato/libs:$B/plato/target/mupdf_wrapper/Kindle"
# zstd-sys (the reader app's page snapshots) compiles C with the cc crate:
# use Zig too.
T_=$(printf %s "$TARGET" | tr - _)
export "CC_$T_=$B/bin/$PFX-cc" "CXX_$T_=$B/bin/$PFX-c++" "AR_$T_=$B/bin/$PFX-ar" "CFLAGS_$T_=-O2"

# CARGO_CMD=clippy or CARGO_CMD=test runs that instead, in the same
# environment, and skips the bundle (tests need the host target).
CARGO_CMD=${CARGO_CMD:-build}
cargo "$CARGO_CMD" --config "$REPO/.cargo/config.toml" --release --target "$TARGET" \
    --manifest-path "$REPO/Cargo.toml" -p aristotle-render -p aristotle-reader
[ "$CARGO_CMD" = build ] || exit 0

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
# subset SRC DST UNICODES [OPTION...]. The output keeps the source's mtime,
# and copies keep theirs, so the app's snapshot key (sizes and mtimes of
# data/) only changes when the data does.
subset() {
    if [ "$FONT_SUBSET" = none ]; then
        cp -p "$1" "$2"
        return
    fi
    _src=$1 _dst=$2 _uni=$3
    shift 3
    PYTHONPATH="$DL/$FONTTOOLS" python3 -m fontTools.subset "$_src" --output-file="$_dst" \
        --unicodes="$_uni" --layout-features='*' --name-IDs='*' --name-languages='*' \
        --notdef-outline "$@"
    touch -r "$_src" "$_dst"
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
    cp -p "$SRC/fonts/$f" "$OUT/data/fonts/"
done
for f in "$REPO/crates/aristotle-render/assets/fonts/"Literata-*.ttf; do
    subset "$f" "$OUT/data/serif/$(basename "$f")" "$TEXT_UNICODES"
done
cp -p "$REPO/crates/aristotle-render/assets/fonts/Literata-OFL.txt" "$OUT/data/serif/"
cp -p "$SRC/css/epub.css" "$SRC/css/html.css" "$OUT/data/css/"

# Glyph fallback for Plato's engine when MuPDF has no Noto fonts (the fork's
# "core: mupdf tofu fallback"): DejaVu Sans cut to arrows, mathematical
# operators, box drawing, geometric shapes, miscellaneous symbols up to the music signs, dingbats up
# to the checkmarks, crosses and stars, and the white squares (about 105 KB
# unhinted), and Noto Sans Coptic from MuPDF's tree. Neither licence
# reserves the names used here. CJK stays without a font.
DEJAVU=dejavu-fonts-ttf-2.37
fetch "$DEJAVU.tar.bz2" \
    "https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/$DEJAVU.tar.bz2" \
    fa9ca4d13871dd122f61258a80d01751d603b4d3ee14095d65453b4e846e17d7
if [ ! -f "$DL/$DEJAVU/ttf/DejaVuSans.ttf" ]; then
    tar -xjf "$DL/$DEJAVU.tar.bz2" -C "$DL" "$DEJAVU/ttf/DejaVuSans.ttf" "$DEJAVU/LICENSE"
fi
subset "$DL/$DEJAVU/ttf/DejaVuSans.ttf" "$OUT/data/fonts/DejaVuSans-Symbols.ttf" \
    U+2190-21FF,U+2200-22C5,U+2500-257F,U+25A0-25FF,U+2600-266F,U+2700-2775,U+2B00-2B2F \
    --no-hinting
cp -p "$DL/$DEJAVU/LICENSE" "$OUT/data/fonts/DejaVuSans-LICENSE.txt"
NOTO="$B/plato/thirdparty/mupdf/resources/fonts/noto"
cp -p "$NOTO/NotoSansCoptic-Regular.otf" "$OUT/data/fonts/"
cp -p "$NOTO/COPYING" "$OUT/data/fonts/Noto-OFL.txt"

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

# Licences. The patterns' authors and licences are in the headers of their
# hyph-utf8 master files, at the commit kl-hyphenate's README names; the
# bundle ships those files.
HYPH_REV=49706f9cfa97f6ead26b473ec10d23d5a651318a
HYPH_DL=hyph-utf8-49706f9
mkdir -p "$DL/$HYPH_DL"
for lang in $HYPH_LANGS; do
    case $lang in
        pl) sum=83ddbbd21bc06a44bc4f4a4c49c38782e809cff5f026b764c044daa29e818843 ;;
        en-us) sum=f4ffcd96c5cbc886bdad23f95dcae8edc3cd3620eae62f7946eceda97c4e68f8 ;;
        en-gb) sum=e95bb4ab350f620c41c231c4785d7bdb4f91949d3e631b30e08b3ead3c340a52 ;;
        grc) sum=dee55cf442ba4f4c5655622571ee796bede243a65ef2286858b28db7f1b40d78 ;;
        la) sum=84e0be1877beaf6d1d7a83d5639fb65a0e0645cd2447b0dbccfa5e63565abdfd ;;
        *) echo "no pinned hyph-utf8 notice for $lang; add its SHA-256 here" >&2; exit 1 ;;
    esac
    fetch "$HYPH_DL/hyph-$lang.tex" \
        "https://raw.githubusercontent.com/hyphenation/tex-hyphen/$HYPH_REV/hyph-utf8/tex/generic/hyph-utf8/patterns/tex/hyph-$lang.tex" \
        "$sum"
done
rm -rf "$OUT/licenses" "$OUT/SOURCE.txt"
DJVU_LINKED=0
[ ! -f "$B/plato/libs/libdjvulibre.a" ] || DJVU_LINKED=1
PYTHONPATH="$DL/$FONTTOOLS" python3 "$REPO/tools/bundle-licenses.py" "$OUT" "$REPO" \
    "$B/plato" "$TARGET" "$TOOLS/zig" "$DL/$HYPH_DL" "$DJVU_LINKED"

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
    tar -czf "$BUNDLE.part" -C "$OUT" --transform 's,^\.,aristotle,' ./aristotle-render ./data \
        ./licenses ./SOURCE.txt
    mv -f "$BUNDLE.part" "$BUNDLE"
    APP_BUNDLE=${APP_BUNDLE:-$HOME/.cache/aristotle/render/aristotle-app-device.tar.gz}
    cp "$REPO/devloop/kindle/run-aristotle.sh" "$REPO/devloop/kindle/takeover.sh" "$OUT/"
    tar -czf "$APP_BUNDLE.part" -C "$OUT" --transform 's,^\.,aristotle,' \
        ./aristotle ./data ./run-aristotle.sh ./takeover.sh ./licenses ./SOURCE.txt
    mv -f "$APP_BUNDLE.part" "$APP_BUNDLE"
    ls -l "$BUNDLE" "$APP_BUNDLE"
fi
