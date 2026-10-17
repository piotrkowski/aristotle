# Third-party software

Copyright (C) 2026 Aristotle contributors.

Aristotle is licensed under the GNU AGPL 3.0 or later ([LICENSE](LICENSE)). It
includes or links the following work by others.

## In this repository

| Path | What | Licence |
| --- | --- | --- |
| `third_party/plato` | [Plato](https://github.com/baskerville/plato), a submodule pointing at our fork [piotrkowski/plato](https://github.com/piotrkowski/plato), branch `aristotle`. The MuPDF it pins is built with the patches in `tools/mupdf-patches/`. | AGPL-3.0 |
| `crates/aristotle-render/assets/fonts/`, `crates/aristotle-ui/assets/fonts/` | [Literata](https://github.com/googlefonts/literata) (The Literata Project Authors, designed by TypeTogether), [Charis](https://software.sil.org/charis/) and [Gentium Book](https://software.sil.org/gentium/) (SIL Global) | SIL OFL 1.1, text next to each font |
| `devloop/emulator/eink-emulator-pw4.patch` | Our patch to [eink-emulator](https://github.com/katadelos/eink-emulator), which has no licence. The patch keeps one line of upstream context per change. | Ours |
| `crates/aristotle-library/src/id.rs` | The partial-MD5 document hash follows KOReader's `util.partialMD5`, so both compute the same id. | KOReader: AGPL-3.0 |

## In the device bundle

`tools/build-render.sh` adds a `licenses/` folder and a `SOURCE.txt` to the
bundle. `tools/bundle-licenses.py` generates them from what the build used.
In short:

- The document engine is Plato's `plato-core` (AGPL-3.0) with MuPDF (AGPL-3.0),
  jbig2dec (AGPL-3.0), FreeType (FTL or GPL-2.0), HarfBuzz (MIT), libjpeg (IJG),
  libpng, OpenJPEG (BSD-2-Clause), zlib, bzip2, Gumbo (Apache-2.0) and Little
  CMS (MIT).
- The runtime is musl (MIT), LLVM's libc++, libc++abi and libunwind (Apache-2.0
  with LLVM exception), the Rust standard library (MIT or Apache-2.0) and
  about a hundred Rust crates, listed with their licences in
  `licenses/rust/CRATES.txt`. Page snapshots use Zstandard (BSD) through
  `zstd-sys`.
- The fonts are Literata, Noto Sans, Noto Sans Coptic, Libertinus Serif,
  Source Code Pro, Parisienne and Delius (SIL OFL 1.1), DejaVu Sans (Bitstream
  Vera licence), and inside MuPDF the URW base 14 fonts and Noto Serif.
- Hyphenation patterns for Polish, English (US and GB), Ancient Greek and
  Latin from [hyph-utf8](https://github.com/hyphenation/tex-hyphen), compiled
  by [kl-hyphenate](https://github.com/baskerville/kl-hyphenate). Each is
  under the licence named in its master file, which the bundle includes.

As AGPL-3.0 section 6 requires, `SOURCE.txt` names the Aristotle commit, the
Plato revision and patches, and the exact C library sources.

## Not included

Dictionaries, books and the stock Kindle software are not part of Aristotle.
The notes on the stock reader's file formats (`docs/library.md`,
`crates/aristotle-library/src/sidecar.rs`) come from interoperability analysis
and contain no Amazon code.
