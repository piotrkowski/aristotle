# aristotle-render

Aristotle's e-book page renderer for the Kindle Paperwhite 4, built on the document engine of [Plato](https://github.com/baskerville/plato) (`plato-core`) without Plato's Kobo UI. It opens EPUB, MOBI, PDF and the other formats Plato reads, lays pages out for the 1072 × 1448 panel at 300 DPI with Polish hyphenation, and draws its own header and footer. The `aristotle-render` binary renders pages to files, shows them on the panel with the A2-then-GL16 page turn, and benchmarks both.

Design, defaults and measurements: [`docs/reader.md`](../../docs/reader.md).

## Why a separate workspace

`plato-core` is not on crates.io and needs two things a fresh checkout lacks:

1. the Plato source with our patches (`tools/plato-patches/`), which `tools/prepare-plato-src.sh` writes to `third_party/plato-patched/` (gitignored; the submodule stays untouched);
2. Plato's C dependencies (MuPDF, FreeType, HarfBuzz, ...) as static archives, which `tools/build-plato.sh` builds with Zig.

So this crate has its own `[workspace]` and the root workspace excludes it: `cargo build` at the repo root keeps working on any host, and only this crate needs the Plato build.

## Build

```sh
# once per target: the C libraries
tools/build-plato.sh                                             # device (also builds Plato itself)
PLATO_TARGET=x86_64-unknown-linux-musl PLATO_LIBS_ONLY=1 tools/build-plato.sh   # host

# then
tools/build-render.sh device   # -> ~/.cache/aristotle/render/device/{aristotle-render,data/}
tools/build-render.sh host     # -> ~/.cache/aristotle/render/host/...
CARGO_CMD=clippy tools/build-render.sh host
CARGO_CMD=test tools/build-render.sh host
```

`PLATO_BUILD` points at a different `build-plato.sh` output directory if needed.

## Use

```sh
aristotle-render info   BOOK
aristotle-render render BOOK PAGE OUT.raw     # 8-bit, stride 1088; devloop/host/fb2png.py makes a PNG
aristotle-render show   BOOK PAGE             # device, stock GUI stopped
aristotle-render bench  BOOK [PAGE] [COUNT]   # CPU only, no screen
aristotle-render turn   BOOK START COUNT PAUSE_MS
```

The binary expects its runtime data in `data/` next to it (or `--data DIR`).

## Licences

AGPL-3.0-or-later, like Plato, which it links. `assets/fonts/` holds Literata (SIL Open Font License 1.1, `Literata-OFL.txt`). The bundle also carries some of Plato's own fonts (OFL) and its prebuilt hyphenation patterns, which `tools/build-render.sh` copies or downloads; they are not committed here.
