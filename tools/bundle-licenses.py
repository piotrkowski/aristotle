#!/usr/bin/env python3
"""Write the licence notices and SOURCE.txt of a build-render.sh bundle.

usage: bundle-licenses.py OUT REPO PLATO_TREE TARGET ZIG_DIR HYPH_DIR DJVU

OUT         the bundle directory; writes OUT/licenses/ and OUT/SOURCE.txt
REPO        the repository root
PLATO_TREE  the tools/build-plato.sh tree the C libraries were built from
TARGET      the Rust target triple
ZIG_DIR     the Zig installation whose libc++ was linked
HYPH_DIR    directory with the hyph-utf8 master files (hyph-<lang>.tex)
DJVU        1 if DjVuLibre was linked

Everything the binaries link or the data directory ships gets its notice:
Aristotle and Plato (AGPL-3.0), the C libraries, the Rust crates (from
`cargo metadata`), the fonts (their copyright and licence name records) and
the hyphenation patterns. SOURCE.txt says where the corresponding source is
(AGPL-3.0 section 6).
"""

import json
import shutil
import subprocess
import sys
from pathlib import Path

OUT, REPO, PLATO, TARGET, ZIG, HYPH, DJVU = sys.argv[1:8]
OUT, REPO, PLATO, ZIG, HYPH = map(Path, (OUT, REPO, PLATO, ZIG, HYPH))
LIC = OUT / "licenses"
REPO_URL = "https://github.com/piotrkowski/aristotle"

# C and C++ libraries linked into the binaries: name, licence, files.
T = PLATO / "thirdparty"
C_LIBS = [
    ("mupdf", "AGPL-3.0-or-later", [T / "mupdf/COPYING"]),
    ("lcms2 (in MuPDF)", "MIT", [T / "mupdf/thirdparty/lcms2/LICENSE"]),
    ("jbig2dec", "AGPL-3.0-or-later", [T / "jbig2dec/COPYING", T / "jbig2dec/LICENSE"]),
    (
        "freetype",
        "FTL or GPL-2.0-or-later",
        [T / "freetype2/LICENSE.TXT", T / "freetype2/docs/FTL.TXT", T / "freetype2/docs/GPLv2.TXT"],
    ),
    ("harfbuzz", "MIT (Old MIT)", [T / "harfbuzz/COPYING"]),
    ("libjpeg", "IJG (see the legal issues in README)", [T / "libjpeg/README"]),
    ("libpng", "libpng-2.0", [T / "libpng/LICENSE"]),
    ("openjpeg", "BSD-2-Clause", [T / "openjpeg/LICENSE"]),
    ("zlib", "Zlib", [T / "zlib/LICENSE"]),
    ("bzip2", "bzip2-1.0.6", [T / "bzip2/LICENSE"]),
    ("gumbo", "Apache-2.0", [T / "gumbo/COPYING"]),
    ("libc++", "Apache-2.0 WITH LLVM-exception", [ZIG / "lib/libcxx/LICENSE.TXT"]),
    ("libc++abi", "Apache-2.0 WITH LLVM-exception", [ZIG / "lib/libcxxabi/LICENSE.TXT"]),
    ("libunwind", "Apache-2.0 WITH LLVM-exception", [ZIG / "lib/libunwind/LICENSE.TXT"]),
    ("musl", "MIT", [ZIG / "lib/libc/musl/COPYRIGHT"]),
]
if DJVU == "1":
    C_LIBS.append(("djvulibre", "GPL-2.0-or-later", [T / "djvulibre/COPYING"]))

# Licence files a crate may carry, and C sources vendored inside crates.
LICENCE_PREFIXES = ("license", "licence", "copying", "copyright", "notice", "unlicense")
VENDORED = {"zstd-sys": ["zstd/LICENSE", "zstd/COPYING"]}


def copy(src: Path, dst_dir: Path, name: str | None = None) -> None:
    if not src.is_file():
        sys.exit(f"bundle-licenses: missing {src}")
    dst_dir.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, dst_dir / (name or src.name))


def git(*args: str) -> str:
    r = subprocess.run(["git", "-C", str(REPO), *args], capture_output=True, text=True)
    return r.stdout.strip()


def rust_crates() -> list[dict]:
    """Packages the two binaries depend on for TARGET (normal and build)."""
    manifest = REPO / "crates/aristotle-render/Cargo.toml"
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--locked",
             "--filter-platform", TARGET, "--manifest-path", str(manifest)],
            check=True, capture_output=True, text=True,
        ).stdout
    )
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    seen, todo = set(), list(meta["workspace_members"])
    while todo:
        pid = todo.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(k["kind"] in (None, "build") for k in dep["dep_kinds"]):
                todo.append(dep["pkg"])
    return sorted(
        (p for p in meta["packages"] if p["id"] in seen),
        key=lambda p: (p["name"], p["version"]),
    )


def font_notice(fonts: list[Path]) -> str:
    """Copyright, licence and licence URL name records of each shipped font."""
    from fontTools.ttLib import TTFont  # from build-render.sh's PYTHONPATH

    out = []
    for f in fonts:
        names = TTFont(f, lazy=True)["name"]

        def rec(i: int) -> str:
            r = " ".join((names.getDebugName(i) or "-").split())
            # Some fonts carry their whole licence here; its file is next to this one.
            return r if len(r) <= 300 else r[:240] + " [...]"

        out.append(f"{f.parent.name}/{f.name}\n  copyright: {rec(0)}\n"
                   f"  licence: {rec(13)}\n  licence URL: {rec(14)}\n")
    return "\n".join(out)


def main() -> None:
    LIC.mkdir(parents=True, exist_ok=False)

    # Aristotle and Plato.
    copy(REPO / "LICENSE", LIC, "Aristotle-LICENSE.txt")
    copy(PLATO / "LICENSE-AGPLv3", LIC / "plato", "LICENSE-AGPLv3.txt")

    # C and C++ libraries.
    lines = []
    for name, licence, files in C_LIBS:
        d = LIC / "c" / name.split()[0]
        for f in files:
            copy(f, d)
        lines.append(f"{name}: {licence}")
    (LIC / "c" / "LIBRARIES.txt").write_text(
        "C and C++ libraries linked statically into the binaries:\n\n"
        + "\n".join(lines) + "\n"
    )

    # Rust crates.
    rows = []
    for p in rust_crates():
        if p["source"] is None:  # our crates and plato-core: AGPL, above
            rows.append(f"{p['name']} {p['version']}: {p.get('license') or 'see above'} (in this source tree)")
            continue
        root = Path(p["manifest_path"]).parent
        d = LIC / "rust" / f"{p['name']}-{p['version']}"
        found = [f for f in sorted(root.iterdir())
                 if f.is_file() and f.name.lower().startswith(LICENCE_PREFIXES)]
        found += [root / v for v in VENDORED.get(p["name"], [])]
        for f in found:
            copy(f, d, f.name if f.parent == root else f"{f.parent.name}-{f.name}")
        rows.append(f"{p['name']} {p['version']}: {p.get('license') or p.get('license_file')}"
                    f"{'' if found else ' (no licence file in the package)'}"
                    f" {p.get('repository') or ''}".rstrip())
    rows.append("Rust standard library: MIT OR Apache-2.0")
    (LIC / "rust" / "CRATES.txt").write_text(
        "Rust crates compiled into the binaries, with their licences (texts in the\n"
        "directory of each crate):\n\n" + "\n".join(rows) + "\n"
    )

    # Fonts in data/.
    fonts = sorted(
        f for d in ("data/fonts", "data/serif") for f in (OUT / d).iterdir()
        if f.suffix.lower() in (".ttf", ".otf")
    )
    fd = LIC / "fonts"
    for f in sorted((OUT / "data").glob("*/*")):
        if f.suffix == ".txt":
            copy(f, fd)
    (fd / "FONTS.txt").write_text(
        "Fonts in data/, with the copyright and licence records they carry. Most\n"
        "are under the SIL Open Font License 1.1 (full text: Literata-OFL.txt,\n"
        "Noto-OFL.txt); DejaVu Sans is under its own licence\n"
        "(DejaVuSans-LICENSE.txt). The binaries also embed MuPDF's fonts: the\n"
        "URW base 14 (under MuPDF's licence, c/mupdf) and Noto Serif (OFL 1.1).\n\n"
        + font_notice(fonts)
    )

    # Hyphenation patterns: the hyph-utf8 master files, which carry their
    # authors and licences in their headers.
    hd = LIC / "hyphenation"
    tex = sorted(HYPH.glob("hyph-*.tex"))
    for f in tex:
        copy(f, hd)
    (hd / "README.txt").write_text(
        "The hyphenation patterns in data/hyphenation-patterns/ are compiled from\n"
        "the hyph-utf8 patterns (https://github.com/hyphenation/tex-hyphen) by\n"
        "kl-hyphenate, as published with Plato 0.9.45. These are their master\n"
        "files; each header names the authors and the licence.\n"
    )

    (LIC / "README.txt").write_text(
        "Licences of everything in this bundle.\n\n"
        "Aristotle-LICENSE.txt  Aristotle: GNU AGPL 3.0 or later\n"
        "plato/                 Plato's document engine (plato-core): GNU AGPL 3.0\n"
        "c/                     C and C++ libraries linked into the binaries\n"
        "rust/                  Rust crates compiled into the binaries\n"
        "fonts/                 Fonts in data/\n"
        "hyphenation/           Hyphenation patterns in data/\n\n"
        "Where to get the corresponding source: ../SOURCE.txt\n"
    )

    # SOURCE.txt (AGPL-3.0 section 6).
    commit = git("rev-parse", "HEAD") or "unknown"
    dirty = " (with uncommitted changes)" if git("status", "--porcelain", "--untracked-files=no") else ""
    plato_rev = git("rev-parse", "HEAD:third_party/plato") or "unknown"
    patches = sorted((REPO / "tools/plato-patches").glob("*.patch"))
    c_sources = []
    in_list = False
    for line in (REPO / "tools/build-plato.sh").read_text().splitlines():
        if line.startswith("done <<'EOF'"):
            in_list = True
        elif in_list and line == "EOF":
            break
        elif in_list:
            name, _, url, sha = line.split()
            if name != "djvulibre" or DJVU == "1":
                c_sources.append(f"  {url}\n    sha256 {sha}")
    (OUT / "SOURCE.txt").write_text(
        "Aristotle is free software under the GNU Affero General Public License,\n"
        "version 3 or later (licenses/Aristotle-LICENSE.txt). The corresponding\n"
        "source of this build:\n\n"
        f"Aristotle: {REPO_URL}\n"
        f"  commit {commit}{dirty}\n\n"
        f"Plato (document engine): https://github.com/baskerville/plato\n"
        f"  commit {plato_rev}, with these patches from Aristotle's tools/plato-patches/:\n"
        + "".join(f"  {p.name}\n" for p in patches)
        + "\nC libraries, built by tools/build-plato.sh with the same patches:\n"
        + "\n".join(c_sources)
        + "\n\nHyphenation patterns: https://github.com/baskerville/plato/releases/tag/0.9.45\n"
        "Rust crates: crates.io, versions in crates/aristotle-render/Cargo.lock\n"
        "Build: tools/build-plato.sh, then tools/build-render.sh device\n"
    )


main()
