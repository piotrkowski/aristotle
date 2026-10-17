# aristotle

**Aristotle** — a from-scratch e-reader system for the Kindle Paperwhite 4, borrowing from [Plato](https://github.com/baskerville/plato).

## Goal

Full liberation of the device. Replace Amazon's entire userspace (Java framework, X11 and awesome, the Pillow WebKit chrome, roughly 150 upstart services, store, recommendations, telemetry, OTA) with a small, fast system we own end to end:

- **Own UI framework** in Rust, drawing straight to the e-ink controller.
- **Own reader**, modernised in Rust, keeping what the native reader does well (typography, dictionaries, highlights) and dropping everything else.
- **Own sync**, replacing Send to Kindle and Whispersync with something that doesn't talk to Amazon.
- **Own kernel** later, once we have a recovery path.

Out of scope: the Kindle store, Goodreads, recommendations, Audible, ads, Amazon accounts and DRM content.

## Status

Early bring-up. Our Rust code draws on the panel and reads touch on the real device over an SSH-over-USB dev loop; the stock system still runs underneath.

## Context

- **Hardware:** one refurbished Kindle Paperwhite 4 (2018), bought used. It is a test device and may be bricked.
- **Use:** personal, non-commercial research project.
- **Library:** DRM-free EPUB, MOBI, PDF and TXT only.

## Layout

| Path | What |
| --- | --- |
| `AGENTS.md` | Working rules and non-obvious device facts for contributors and coding agents (`CLAUDE.md` links here) |
| `docs/architecture.md` | Crates, extension points, languages, and the migration plan |
| `docs/pw4-reference.md` | Detailed hardware, kernel and stock-userspace reference |
| `docs/eink-rendering.md` | How the EPDC driver behaves, the display-thread design, and how stroke latency is measured |
| `docs/library.md` | Book library: formats, metadata precedence, index, cover pipeline, stock Kindle sidecars and clippings |
| `docs/ui-framework.md` | The UI toolkit: architecture, e-ink UX rules, font engine choice, damage and waveform hints, preview |
| `docs/prior-art.md` | Survey of existing Kindle, e-ink and Plato-related projects |
| `docs/reader.md` | The book renderer: how Plato's engine is built in, typesetting and hyphenation for Polish, MOBI, measurements |
| `crates/aristotle-hal` | Hardware layer: e-ink framebuffer (Rex update ABI, pipelined `Presenter`), touch input |
| `crates/aristotle-ui` | Retained UI toolkit for e-ink: shaped and justified text, widgets, damage with waveform hints, gestures, desktop PNG preview |
| `crates/aristotle-pen` | Stroke rendering plus record/replay latency tooling |
| `crates/aristotle-library` | Book library: format detection, metadata, e-ink cover thumbnails, JSON index, Kindle/KOReader import, `aristotle-library-cli` |
| `crates/aristotle-hello`, `aristotle-touch`, `aristotle-bench` | On-device tests and the EPDC waveform benchmark |
| `crates/aristotle-render` | E-book pages (EPUB, MOBI, PDF) with Plato's document engine; separate workspace, built by `tools/build-render.sh` |
| `crates/aristotle-reader` | The reader app `aristotle`: library, Plato pages, menu, progress, page-number cache; member of the aristotle-render workspace |
| `devloop/` | SSH-over-USB dev loop (`host/run.sh`), the KUAL network extension, and PW4 emulator setup |
| `probe/host/` | Host-side tools that take apart the stock firmware |
| `probe/device/` | KUAL extensions that run read-only probes on the Kindle |
| `tools/` | Fetch scripts for pinned reverse-engineering tools; the Plato cross-build and patches; `build-render.sh` |
| `third_party/plato` | Upstream Plato (git submodule), used as a read-only reference |

## License

Aristotle will reuse Plato code, so the project is licensed under the GNU AGPL v3, the same as Plato. See `LICENSE`.
