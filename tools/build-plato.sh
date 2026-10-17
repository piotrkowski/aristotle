#!/bin/sh
# Build Plato (third_party/plato, our fork) as a fully static armv7-unknown-linux-musleabihf
# binary for the Kindle Paperwhite 4, with user-space tools only.
#
# Zig is the cross C/C++ compiler (musl, hard float, Cortex-A9 + NEON). Every C
# dependency is built as a static archive from the versions Plato pins in
# thirdparty/download.sh, then Cargo links the binary with rust-lld through the
# repo's .cargo/config.toml (+crt-static).
#
# Output: $PLATO_BUILD/out/plato (default PLATO_BUILD: $ARISTOTLE_CACHE/plato-build/main;
# ARISTOTLE_CACHE comes from devloop/env.sh).
# Static C archives, also used by crates/aristotle-render: $PLATO_BUILD/plato/libs.
#
# PLATO_TARGET=x86_64-unknown-linux-musl builds the same C libraries for the PC
# (default PLATO_BUILD: $ARISTOTLE_CACHE/plato-build/host-x86_64), so
# aristotle-render can render preview pages on the host with exactly the code
# that runs on the device. PLATO_LIBS_ONLY=1 stops after the C libraries.
#
# Source: the commit checked out in the third_party/plato submodule, exported
# with `git archive` to $PLATO_BUILD/plato. The C sources are unpacked and
# built there too, so nothing is written into the submodule. MuPDF gets
# tools/mupdf-patches/*.patch on top of Plato's kobo.patch.
#
# Needs on the host: curl, tar, xz, openssl, xxd, make, cmake, bash, a host cc
# (FreeType's build runs a small host tool), cargo and the rustup target
# armv7-unknown-linux-musleabihf. Nothing is installed system-wide.
#
# What goes in (sizes and reasons: docs/reader.md, "Size budget"):
#
#   MUPDF_FONTS=latin-greek  (default) MuPDF embeds the 14 standard PDF fonts
#                        (URW Nimbus, cut to Latin, monotonic Greek and
#                        Cyrillic), the substitutes for PDFs that do not
#                        embed theirs, and Noto Serif, its fallback for
#                        polytonic Greek in TXT and MOBI (mupdf-0001).
#                        No other Noto fonts, no CJK font or CMaps, no
#                        Charis SIL (MuPDF's HTML layout then uses Times);
#                        Plato's engine gets no fallback fonts (fork
#                        commit "core: mupdf tofu fallback").
#   MUPDF_FONTS=base14   the same without Noto Serif.
#   MUPDF_FONTS=full     Plato's set: Noto for every script, Droid Sans
#                        Fallback for CJK, Charis SIL, the CJK CMaps (+17 MB).
#   MUPDF_FORMATS=minimal (default) MuPDF opens PDF, images (EPUB pictures),
#                        MOBI (fallback for books mobi.rs can't convert) and
#                        TXT; Plato's own engine reads EPUB and HTML. No XPS,
#                        SVG, CBZ, FB2, Office, HTML or EPUB in MuPDF, and no
#                        MuPDF hyphenation (used only with CSS hyphens: auto).
#   MUPDF_FORMATS=full   everything MuPDF reads.
#   PLATO_DJVU=0         (default) no DjVuLibre: DjVu files are not opened
#                        ("core: optional djvu"). PLATO_DJVU=1 builds and
#                        links it.
#
# The script is restartable: finished steps leave a stamp in $PLATO_BUILD/stamps.
# MuPDF is built once per MUPDF_FONTS/MUPDF_FORMATS pair, so switching back
# and forth reuses earlier builds. For a clean rebuild point PLATO_BUILD at a
# new directory.
set -eu

REPO=$(cd "$(dirname "$0")/.." && pwd)
. "$REPO/devloop/env.sh"
SUB="$REPO/third_party/plato"
MUPDF_PATCHES="$REPO/tools/mupdf-patches"
TARGET=${PLATO_TARGET:-armv7-unknown-linux-musleabihf}
case $TARGET in
    armv7-unknown-linux-musleabihf)
        DEFAULT_BUILD=main
        HOST=arm-linux-musleabihf
        PFX=arm
        CPU_FLAGS="-mcpu=cortex_a9 -mfpu=neon"
        CMAKE_CPU=arm ;;
    x86_64-unknown-linux-musl)
        DEFAULT_BUILD=host-x86_64
        HOST=x86_64-linux-musl
        PFX=x86_64
        CPU_FLAGS=
        CMAKE_CPU=x86_64 ;;
    *) echo "unsupported PLATO_TARGET: $TARGET" >&2; exit 1 ;;
esac
B=${PLATO_BUILD:-$ARISTOTLE_CACHE/plato-build/$DEFAULT_BUILD}
TOOLS=${ARISTOTLE_TOOLS:-$ARISTOTLE_CACHE/tools}
JOBS=${JOBS:-2}

MUPDF_FONTS=${MUPDF_FONTS:-latin-greek}
MUPDF_FORMATS=${MUPDF_FORMATS:-minimal}
PLATO_DJVU=${PLATO_DJVU:-0}
case $MUPDF_FONTS in
    latin-greek) MUPDF_XCFLAGS="-DTOFU -DTOFU_CJK -DNO_CJK -DKEEP_NOTO_SERIF" ;;
    base14) MUPDF_XCFLAGS="-DTOFU -DTOFU_CJK -DNO_CJK" ;;
    full) MUPDF_XCFLAGS= ;;
    *) echo "MUPDF_FONTS must be latin-greek, base14 or full" >&2; exit 1 ;;
esac
case $MUPDF_FORMATS in
    minimal) MUPDF_XCFLAGS="$MUPDF_XCFLAGS -DFZ_ENABLE_XPS=0 -DFZ_ENABLE_SVG=0 -DFZ_ENABLE_CBZ=0
        -DFZ_ENABLE_HTML=0 -DFZ_ENABLE_FB2=0 -DFZ_ENABLE_EPUB=0 -DFZ_ENABLE_OFFICE=0
        -DFZ_ENABLE_HYPHEN=0 -DFZ_ENABLE_DOCX_OUTPUT=0" ;;
    full) ;;
    *) echo "MUPDF_FORMATS must be minimal or full" >&2; exit 1 ;;
esac
case $PLATO_DJVU in
    0|1) ;;
    *) echo "PLATO_DJVU must be 0 or 1" >&2; exit 1 ;;
esac
MUPDF_CONF=$MUPDF_FONTS-$MUPDF_FORMATS

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ARISTOTLE_CACHE/target-plato}
export ZIG_GLOBAL_CACHE_DIR="$B/zig-cache"
export ZIG_LOCAL_CACHE_DIR="$B/zig-cache"
DL="$TOOLS/downloads"
mkdir -p "$TMPDIR" "$DL" "$B/stamps" "$B/bin"

log() { printf '\n=== %s\n' "$*"; }
done_() { [ -f "$B/stamps/$1" ]; }
stamp() { touch "$B/stamps/$1"; }

# fetch DIR NAME URL SHA256
fetch() {
    _f="$1/$2"
    if [ ! -f "$_f" ] || ! printf '%s  %s\n' "$4" "$_f" | sha256sum -c --status; then
        curl -fL --retry 3 -o "$_f.part" "$3"
        printf '%s  %s\n' "$4" "$_f.part" | sha256sum -c --status || {
            echo "checksum mismatch: $2" >&2
            exit 1
        }
        mv "$_f.part" "$_f"
    fi
}

# minisign_verify FILE SIGFILE PUBKEY: Ed25519 over BLAKE2b-512 ("ED" prehashed
# signatures), plus the global signature over the trusted comment.
minisign_verify() {
    d=$(mktemp -d "$TMPDIR/minisign.XXXXXX")
    printf '%s' "$3" | base64 -d | tail -c 32 > "$d/pk.raw"
    printf '302a300506032b6570032100' | xxd -r -p | cat - "$d/pk.raw" > "$d/pk.der"
    openssl pkey -pubin -inform DER -in "$d/pk.der" -out "$d/pk.pem"
    sed -n 2p "$2" | base64 -d > "$d/sigline"
    [ "$(head -c 2 "$d/sigline")" = ED ] || { echo "unsupported minisign signature" >&2; exit 1; }
    [ "$(head -c 10 "$d/sigline" | tail -c 8 | xxd -p)" = \
      "$(printf '%s' "$3" | base64 -d | head -c 10 | tail -c 8 | xxd -p)" ] || {
        echo "minisign key id mismatch" >&2; exit 1; }
    tail -c 64 "$d/sigline" > "$d/sig.raw"
    openssl dgst -blake2b512 -binary "$1" > "$d/hash"
    openssl pkeyutl -verify -pubin -inkey "$d/pk.pem" -rawin -in "$d/hash" -sigfile "$d/sig.raw"
    sed -n 3p "$2" | sed 's/^trusted comment: //' | tr -d '\n' > "$d/tc"
    cat "$d/sig.raw" "$d/tc" > "$d/gmsg"
    sed -n 4p "$2" | base64 -d > "$d/gsig"
    openssl pkeyutl -verify -pubin -inkey "$d/pk.pem" -rawin -in "$d/gmsg" -sigfile "$d/gsig"
}

# --- Zig ---------------------------------------------------------------------
ZIG_VERSION=0.17.0
ZIG_TARBALL=zig-x86_64-linux-$ZIG_VERSION.tar.xz
ZIG_SHA256=1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026
ZIG_PUBKEY=RWSGOq2NVecA2UPNdBUZykf1CCb147pkmdtYxgb3Ti+JO/wCYvhbAb/U  # ziglang.org/download
ZIG="$TOOLS/zig/zig"
if [ ! -x "$ZIG" ] || [ "$("$ZIG" version)" != "$ZIG_VERSION" ]; then
    log "Zig $ZIG_VERSION"
    fetch "$DL" "$ZIG_TARBALL" \
        "https://ziglang.org/download/$ZIG_VERSION/$ZIG_TARBALL" "$ZIG_SHA256"
    curl -fsSL --retry 3 -o "$DL/$ZIG_TARBALL.minisig.$$" \
        "https://ziglang.org/download/$ZIG_VERSION/$ZIG_TARBALL.minisig"
    minisign_verify "$DL/$ZIG_TARBALL" "$DL/$ZIG_TARBALL.minisig.$$" "$ZIG_PUBKEY"
    [ ! -e "$TOOLS/zig" ] || { echo "$TOOLS/zig exists but is not Zig $ZIG_VERSION; move it away" >&2; exit 1; }
    mkdir "$TOOLS/zig"
    tar -xJf "$DL/$ZIG_TARBALL" -C "$TOOLS/zig" --strip-components=1
fi

# Compiler wrappers. Zig only accepts its own CPU names (cortex_a9), so map the
# GCC spelling that MuPDF's Makerules passes (-mcpu=cortex-a9), and drop the
# Rust-style --target=armv7-... that the cc crate adds when it detects clang.
# For the host target, the ARM flags that MuPDF's OS=kobo rules add are dropped.
if [ "$PFX" = arm ]; then
    # shellcheck disable=SC2016 # expanded by the generated wrapper
    CPU_CASE='-mcpu=*) a=-mcpu=$(printf %s "${a#-mcpu=}" | tr - _) ;;'
else
    CPU_CASE='-mcpu=*|-mfpu=*) continue ;;'
fi
for tool in cc c++; do
    w="$B/bin/$PFX-$tool"
    cat > "$w" <<EOF
#!/bin/sh
for a do
    shift
    case \$a in
        --target=*) continue ;;
        $CPU_CASE
    esac
    set -- "\$@" "\$a"
done
exec "$ZIG" $tool -target $HOST $CPU_FLAGS "\$@"
EOF
    chmod +x "$w"
done
for tool in ar ranlib; do
    w="$B/bin/$PFX-$tool"
    printf '#!/bin/sh\nexec "%s" %s "$@"\n' "$ZIG" "$tool" > "$w"
    chmod +x "$w"
done
export PATH="$B/bin:$PATH"
# One section per function and object, so the final link (--gc-sections)
# drops what nothing calls; MuPDF's own rules already do this.
CFLAGS="-O2 -ffunction-sections -fdata-sections"
XCC="$B/bin/$PFX-cc"
XCXX="$B/bin/$PFX-c++"
XAR="$B/bin/$PFX-ar"
XRANLIB="$B/bin/$PFX-ranlib"

# --- Sources -----------------------------------------------------------------
PLATO_REV=$(git -C "$SUB" rev-parse HEAD 2>/dev/null) || {
    echo "third_party/plato is not checked out; run: git submodule update --init" >&2
    exit 1
}
if ! done_ plato-src; then
    log "Plato $PLATO_REV"
    [ ! -e "$B/plato" ] || { echo "$B/plato exists without a stamp; use a fresh PLATO_BUILD" >&2; exit 1; }
    mkdir "$B/plato"
    git -C "$SUB" archive --format=tar "$PLATO_REV" | tar -x -C "$B/plato"
    printf '%s\n' "$PLATO_REV" >> "$B/stamps/plato-src"
fi
# A build directory belongs to one Plato commit (stamps written before the
# fork are empty and accepted as they are).
BUILT_REV=$(cat "$B/stamps/plato-src")
if [ -n "$BUILT_REV" ] && [ "$BUILT_REV" != "$PLATO_REV" ]; then
    echo "$B was built from Plato $BUILT_REV, the submodule is at $PLATO_REV; use a fresh PLATO_BUILD" >&2
    exit 1
fi
P="$B/plato"
T="$P/thirdparty"

# name file url sha256 (versions as pinned by Plato's thirdparty/download.sh)
# The zlib URL moved to GitHub because zlib.net only keeps the latest release.
while read -r name file url sha; do
    [ "$name" != djvulibre ] || [ "$PLATO_DJVU" = 1 ] || continue
    fetch "$DL" "$file" "$url" "$sha"
    if ! done_ "src-$name"; then
        mkdir -p "$T/$name"
        tar -xzf "$DL/$file" --strip-components=1 -C "$T/$name"
        if [ -f "$T/$name/kobo.patch" ] && [ "$name" != djvulibre ]; then
            patch -d "$T/$name" -p1 < "$T/$name/kobo.patch"
        fi
        if [ "$name" = mupdf ]; then
            for p in "$MUPDF_PATCHES"/*.patch; do
                echo "applying $(basename "$p")"
                patch -d "$T/$name" -p1 < "$p"
            done
        fi
        stamp "src-$name"
    fi
done <<'EOF'
zlib zlib-1.3.1.tar.gz https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz 9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23
bzip2 bzip2-1.0.8.tar.gz https://sourceware.org/pub/bzip2/bzip2-1.0.8.tar.gz ab5a03176ee106d3f0fa90e381da478ddae405918153cca248e682cd0c4a2269
libpng libpng-1.6.53.tar.gz https://download.sourceforge.net/libpng/libpng-1.6.53.tar.gz da0b045cbb1d06a8fc9696f9441359f70645f280ff24ae453ccb7c722353654f
libjpeg jpegsrc.v9f.tar.gz https://www.ijg.org/files/jpegsrc.v9f.tar.gz 04705c110cb2469caa79fb71fba3d7bf834914706e9641a4589485c1f832565b
openjpeg openjpeg-2.5.4.tar.gz https://github.com/uclouvain/openjpeg/archive/v2.5.4.tar.gz a695fbe19c0165f295a8531b1e4e855cd94d0875d2f88ec4b61080677e27188a
jbig2dec jbig2dec-0.20.tar.gz https://github.com/ArtifexSoftware/jbig2dec/releases/download/0.20/jbig2dec-0.20.tar.gz 7b63ff6470289547e7a3a0f145cb8ea6c2afffdd65645b7d87d3b7febc96fb3a
freetype2 freetype-2.14.1.tar.gz https://download.savannah.gnu.org/releases/freetype/freetype-2.14.1.tar.gz 174d9e53402e1bf9ec7277e22ec199ba3e55a6be2c0740cb18c0ee9850fc8c34
harfbuzz harfbuzz-12.3.0.tar.gz https://github.com/harfbuzz/harfbuzz/archive/12.3.0.tar.gz e93af4816128fc0a02d2e84106fdfe36a3fde01086b723be8f0656a65562ca9e
gumbo gumbo-0.10.1.tar.gz https://github.com/google/gumbo-parser/archive/v0.10.1.tar.gz 28463053d44a5dfbc4b77bcf49c8cee119338ffa636cc17fc3378421d714efad
djvulibre djvulibre-3.5.29.tar.gz https://downloads.sourceforge.net/djvu/djvulibre-3.5.29.tar.gz d3b4b03ae2bdca8516a36ef6eb27b777f0528c9eda26745d9962824a3fdfeccf
mupdf mupdf-1.27.0-source.tar.gz https://casper.mupdf.com/downloads/archive/mupdf-1.27.0-source.tar.gz ae2442416de499182d37a526c6fa2bacc7a3bed5a888d113ca04844484dfe7c6
EOF

# --- C libraries (static) ----------------------------------------------------
# Output paths follow Plato's build-kobo.sh scripts, which MuPDF's kobo.patch
# relies on (../freetype2/objs/.libs and so on).

if ! done_ zlib; then
    log zlib
    (cd "$T/zlib" && CHOST=$HOST CC=$XCC AR=$XAR RANLIB=$XRANLIB CFLAGS=$CFLAGS ./configure --static \
        && make -j"$JOBS" libz.a)
    stamp zlib
fi

if ! done_ bzip2; then
    log bzip2
    (cd "$T/bzip2" \
        && for f in blocksort huffman crctable randtable compress decompress bzlib; do
               $XCC $CFLAGS -D_FILE_OFFSET_BITS=64 -c $f.c -o $f.o
           done \
        && $XAR rcs libbz2.a blocksort.o huffman.o crctable.o randtable.o compress.o decompress.o bzlib.o)
    stamp bzip2
fi

if ! done_ libpng; then
    log libpng
    (cd "$T/libpng" && ./configure --host=$HOST CC=$XCC AR=$XAR RANLIB=$XRANLIB CFLAGS="$CFLAGS" \
            CPPFLAGS="-I$T/zlib" LDFLAGS="-L$T/zlib" --disable-shared --enable-static \
        && make -j"$JOBS" libpng16.la)
    stamp libpng
fi

if ! done_ libjpeg; then
    log libjpeg
    (cd "$T/libjpeg" && ./configure --host=$HOST CC=$XCC AR=$XAR RANLIB=$XRANLIB CFLAGS="$CFLAGS" \
            --disable-shared --enable-static \
        && make -j"$JOBS" libjpeg.la \
        && { [ -L include ] || ln -s . include; } && { [ -L lib ] || ln -s .libs lib; })
    stamp libjpeg
fi

if ! done_ openjpeg; then
    log openjpeg
    (cd "$T/openjpeg" && cmake -S . -B build -DCMAKE_BUILD_TYPE=Release \
            -DCMAKE_SYSTEM_NAME=Linux -DCMAKE_SYSTEM_PROCESSOR=$CMAKE_CPU \
            -DCMAKE_C_COMPILER="$XCC" -DCMAKE_AR="$XAR" -DCMAKE_RANLIB="$XRANLIB" \
            -DCMAKE_C_FLAGS="$CFLAGS" -DBUILD_SHARED_LIBS=OFF -DBUILD_STATIC_LIBS=ON \
            -DBUILD_CODEC=OFF -DBUILD_TESTING=OFF \
        && cmake --build build -j"$JOBS" \
        && cp build/src/lib/openjp2/opj_config.h src/lib/openjp2/)
    stamp openjpeg
fi

if ! done_ jbig2dec; then
    log jbig2dec
    (cd "$T/jbig2dec" && ./configure --host=$HOST CC=$XCC AR=$XAR RANLIB=$XRANLIB CFLAGS="$CFLAGS" \
            --disable-shared --enable-static --without-libpng \
        && make -j"$JOBS" libjbig2dec.la)
    stamp jbig2dec
fi

if ! done_ freetype2; then
    log freetype2
    # Only the font formats PDFs and books embed (TrueType, CFF, Type 1,
    # CID, OpenType): no PFR, Type 42, Windows FNT, PCF, BDF, the LZW and
    # bzip2 readers for compressed PCF, OT-SVG or SDF rendering. gzip stays:
    # sfnt needs it for WOFF.
    sed -i -E 's/^((FONT|RASTER|AUX)_MODULES \+= (pfr|type42|winfonts|pcf|bdf|svg|sdf|lzw|bzip2))$/# \1/' \
        "$T/freetype2/modules.cfg"
    (cd "$T/freetype2" && ZLIB_CFLAGS="-I$T/zlib" ZLIB_LIBS="-L$T/zlib -lz" \
            BZIP2_CFLAGS="-I$T/bzip2" BZIP2_LIBS="-L$T/bzip2 -lbz2" \
            LIBPNG_CFLAGS="-I$T/libpng" LIBPNG_LIBS="-L$T/libpng/.libs -lpng16" \
            ./configure --host=$HOST CC=$XCC AR=$XAR RANLIB=$XRANLIB CFLAGS="$CFLAGS" \
            --with-zlib=yes --with-png=yes --with-bzip2=yes --with-harfbuzz=no --with-brotli=no \
            --disable-shared --enable-static \
        && make -j"$JOBS")
    stamp freetype2
fi

# HarfBuzz: Plato uses meson; we compile the single-file amalgamation with the
# same feature set (FreeType on; glib, ICU, cairo off).
if ! done_ harfbuzz; then
    log harfbuzz
    (cd "$T/harfbuzz" && mkdir -p build/src \
        && $XCXX $CFLAGS -std=c++17 -fno-exceptions -fno-rtti -fno-threadsafe-statics \
            -fvisibility-inlines-hidden -DHAVE_FREETYPE=1 -DHAVE_PTHREAD=1 \
            -DHAVE_FT_GET_VAR_BLEND_COORDINATES=1 -DHAVE_FT_SET_VAR_BLEND_COORDINATES=1 \
            -DHAVE_FT_DONE_MM_VAR=1 -DHAVE_FT_GET_TRANSFORM=1 \
            -DHAVE_UNISTD_H=1 -DHAVE_SYS_MMAN_H=1 -DHAVE_MMAP=1 \
            -I"$T/freetype2/include" -c src/harfbuzz.cc -o build/src/harfbuzz.o \
        && $XAR rcs build/src/libharfbuzz.a build/src/harfbuzz.o)
    stamp harfbuzz
fi

# Gumbo: plain C99 sources, no configure needed.
if ! done_ gumbo; then
    log gumbo
    (cd "$T/gumbo" && mkdir -p .libs \
        && for f in src/*.c; do
               $XCC $CFLAGS -std=gnu99 -Isrc -c "$f" -o ".libs/$(basename "$f" .c).o"
           done \
        && $XAR rcs .libs/libgumbo.a .libs/*.o)
    stamp gumbo
fi

# DjVuLibre: only libdjvu is built (what Plato's kobo.patch achieves by
# editing Makefile.am, without needing autoreconf).
if [ "$PLATO_DJVU" = 1 ] && ! done_ djvulibre; then
    log djvulibre
    (cd "$T/djvulibre" && ./configure --host=$HOST CC=$XCC CXX=$XCXX AR=$XAR RANLIB=$XRANLIB \
            CFLAGS="$CFLAGS" CXXFLAGS="$CFLAGS" --disable-xmltools --disable-desktopfiles \
            --with-jpeg="$T/libjpeg" --disable-shared --enable-static \
        && make -j"$JOBS" -C libdjvu libdjvulibre.la)
    stamp djvulibre
fi

# MuPDF: Plato's kobo.patch adds OS=kobo with system-library paths; the
# toolchain variables are overridden on the command line. Each font and
# format choice gets its own output directory.
MUPDF_OUT=build/aristotle-$MUPDF_CONF
if ! done_ "mupdf-$MUPDF_CONF" || [ ! -f "$T/mupdf/$MUPDF_OUT/libmupdf.a" ]; then
    log "mupdf ($MUPDF_CONF)"
    # shellcheck disable=SC2086 # MUPDF_XCFLAGS is a list of flags
    (cd "$T/mupdf" && make generate XCFLAGS="$(echo $MUPDF_XCFLAGS)" \
        && make -j"$JOBS" verbose=yes mujs=no tesseract=no extract=no archive=no brotli=no \
            barcode=no commercial=no USE_SYSTEM_LIBS=yes OS=kobo build=release \
            OUT="$MUPDF_OUT" XCFLAGS="$(echo $MUPDF_XCFLAGS)" \
            CC="$XCC" CXX="$XCXX" AR="$XAR" LD=false libs)
    stamp "mupdf-$MUPDF_CONF"
fi

if ! done_ mupdf_wrapper; then
    log mupdf_wrapper
    (cd "$P/mupdf_wrapper" && TARGET_OS=Kindle CC=$XCC AR=$XAR CFLAGS=$CFLAGS ./build.sh)
    stamp mupdf_wrapper
fi

# Zig builds libc++/libc++abi for the target on first C++ link; pick them out
# of its cache. libunwind and libc come from Rust's self-contained musl.
if ! done_ libcxx; then
    log "libc++ runtime"
    mkdir -p "$B/cxxrt"
    printf 'int main(void){return 0;}\n' >> "$B/cxxrt/main.cpp"
    $XCXX -O2 -static "$B/cxxrt/main.cpp" -o "$B/cxxrt/main" --verbose 2>> "$B/cxxrt/link.log"
    for lib in libc++.a libc++abi.a; do
        path=$(tr ' ' '\n' < "$B/cxxrt/link.log" | grep "/$lib\$" | tail -n 1)
        [ -n "$path" ] || { echo "could not find $lib from zig" >&2; exit 1; }
        cp "$path" "$B/cxxrt/$lib"
    done
    stamp libcxx
fi

# Collect the archives where crates/core/build.rs looks for them.
log "collect static libraries"
mkdir -p "$P/libs"
for a in "$T/zlib/libz.a" "$T/bzip2/libbz2.a" "$T/libpng/.libs/libpng16.a" \
         "$T/libjpeg/.libs/libjpeg.a" "$T/openjpeg/build/bin/libopenjp2.a" \
         "$T/jbig2dec/.libs/libjbig2dec.a" "$T/freetype2/objs/.libs/libfreetype.a" \
         "$T/harfbuzz/build/src/libharfbuzz.a" "$T/gumbo/.libs/libgumbo.a" \
         "$T/mupdf/$MUPDF_OUT/libmupdf.a" "$T/mupdf/$MUPDF_OUT/libmupdf-third.a" \
         "$B/cxxrt/libc++.a" "$B/cxxrt/libc++abi.a" \
         "$P/target/mupdf_wrapper/Kindle/libmupdf_wrapper.a"; do
    cp "$a" "$P/libs/"
done
# plato-core's build.rs (fork commits "core: mupdf tofu fallback" and
# "core: optional djvu") reads the choices from here.
if [ "$PLATO_DJVU" = 1 ]; then
    cp "$T/djvulibre/libdjvu/.libs/libdjvulibre.a" "$P/libs/"
else
    rm -f "$P/libs/libdjvulibre.a"
fi
printf '%s\n' "$MUPDF_FONTS" > "$P/libs/mupdf-fonts"

if [ "${PLATO_LIBS_ONLY:-0}" = 1 ]; then
    ls -l "$P/libs"
    exit 0
fi

# --- Rust ----------------------------------------------------------------------
log "cargo build -p plato"
# The cc crate (zstd-sys, pulled in by zip) also needs the cross compiler.
T_=$(printf %s "$TARGET" | tr - _)
export "CC_$T_=$XCC" "CXX_$T_=$XCXX" "AR_$T_=$XAR" "CFLAGS_$T_=$CFLAGS"
(cd "$P" && cargo build --config "$REPO/.cargo/config.toml" --release --target "$TARGET" -p plato)

mkdir -p "$B/out"
cp "$CARGO_TARGET_DIR/$TARGET/release/plato" "$B/out/plato"
if command -v llvm-strip >/dev/null; then
    llvm-strip -o "$B/out/plato.stripped" "$B/out/plato"
fi
file "$B/out/plato"
readelf -A "$B/out/plato" | grep Tag_ABI_VFP_args
if readelf -lW "$B/out/plato" | grep -q INTERP; then
    echo "error: $B/out/plato has a program interpreter (not static)" >&2
    exit 1
fi
ls -l "$B/out"
