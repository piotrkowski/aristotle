# Architecture

Aristotle is one Rust workspace that builds static binaries for the PW4, plus shell scripts that run on the device. This document maps the crates, the processes and threads on the device, and the places meant for extension. The last section lists planned changes. Where this document and the code disagree, the code is right.

## Principles

- One static binary per device. Static musl binaries cannot `dlopen`, and Rust has no stable ABI, so there is no runtime plugin system. Extensions are Rust code chosen at compile time. Code from outside comes in as data, such as books and dictionaries, or as a separate program.
- Pure-Rust crates build and test on any host. Only `aristotle-render` and the reader app link C code.
- Offline first. Network features are optional and off by default. Wi-Fi stays with the stock system or is limited to an allowlist of hosts ([Network](../device/README.md#network)).
- English is the source language. Other languages are catalogs checked at compile time.
- Refresh rules come from measurements on the panel and are tested on the host.

## Repository

```
Cargo.toml          the workspace; default-members leave out the two crates that link C
.cargo/config.toml  static musl targets: armv7 for the device, x86_64 for host previews
crates/             the Rust crates, flat; the directory name is the package name
device/             what runs on the Kindle: the KUAL extension and agent, the app's launch
                    scripts, the read-only probe, and host-side tests of the scripts
tools/              what runs on the host: env.sh, the C cross-build, the dev loop (host/),
                    the emulator setup, device scenarios, firmware unpackers (probe/),
                    licence bundling, packaging (package.sh), screenshots, RE tools
third_party/plato   our Plato fork (git submodule, branch aristotle)
docs/, site/        developer documents and the project site
```

## Crates

| Crate | Role |
| --- | --- |
| `aristotle-hal` | Framebuffer and EPDC updates, the display thread, touch, battery and frontlight, system facts, the agent's USB mode |
| `aristotle-ui` | The e-ink toolkit and the screens, with the `aristotle-ui-preview` binary ([UI framework](ui-framework.md)) |
| `aristotle-i18n` | Message catalogs, plurals, dates and numbers |
| `aristotle-library` | Finds, indexes and describes EPUB, MOBI/AZW3, PDF, TXT and CBZ files. Covers, clippings, export, and `aristotle-library-cli` ([Library](library.md)) |
| `aristotle-render` | Plato's document engine and MuPDF: opening, pagination, page rendering, footnotes ([Reader](reader.md)) |
| `aristotle` | The app, binary `aristotle`: event loop, screens, reading session, snapshots, dictionaries |
| `aristotle-observe` | Pairs touches with the EPDC updates they cause, in any UI, and injects scripted touches. `bench` measures EPDC latency per waveform, streamed updates and page-turn strategies |

`aristotle-hal` and `aristotle-i18n` sit at the bottom. `aristotle-ui` and `aristotle-library` depend on `aristotle-i18n` only, so the toolkit and the screens run on any host. `aristotle-render` uses `aristotle-hal` and `plato-core` from the submodule. The app, `aristotle`, depends on all of them but `aristotle-observe`, and takes `aristotle-ui` without its embedded fonts.

The workspace has one `Cargo.lock` and one release profile: `opt-level = "s"`, LTO, one codegen unit, `panic = "abort"`, stripped. `plato-core`, `aristotle-render`, `paragraph-breaker` and `kl-hyphenate` use `opt-level = 3`, because they lay out pages inside the page-turn budget (`docs/reader.md`, "Size budget"). A plain `cargo build`, `cargo test` or `cargo clippy` skips `aristotle-render` and `aristotle`, the two crates that link C. Cargo still reads `plato-core`'s manifest, so the submodule must be checked out. Build the C libraries with `tools/build-plato.sh`, then the native crates and their bundle with `tools/build-render.sh [device|host]`. `tools/package.sh zip` turns the device bundle and the KUAL extension into the release zip.

## On the device

### Processes

The app bundle unpacks into `/mnt/us/aristotle/`: the `aristotle` binary, `data/` with fonts, stylesheets and hyphenation patterns, and the launch scripts. `run-aristotle.sh` stops the stock GUI with `takeover.sh start`, runs the app, and restarts the stock GUI with `takeover.sh stop` when the app exits. The script does not freeze the stock GUI with SIGSTOP. On our firmware the watchdog killed the frozen framework, and the GUI came back only after a reboot.

`powerd`, `volumd`, `wifid` and `lipcd` keep running. The app sets `preventScreenSaver` so `powerd` does not draw the stock screensaver, and draws its own.

The device agent is a separate upstart job (`aristotle-agent.conf`), installed from `device/extension/agent/`. It brings up the USB network and SSH at boot ([device/README.md](../device/README.md)). `aristotlectl` switches between network and drive mode, and the settings screen calls it through `aristotle_hal::system::set_usb_mode`.

Books are read from `/mnt/us/documents`. The app keeps its index, covers, page cache, snapshots and `reader.json` in `/mnt/us/.aristotle`.

### Threads

| Thread | Work |
| --- | --- |
| main | Polls touch with a timeout from the nearest deadline, runs gestures and the UI, renders, submits page turns, adopts results from the workers |
| `epdc-display` | The HAL display thread: merges UI damage and submits it |
| `epdc-completions`, `page-completions` | Wait for update markers, for latency logs and the next page blit |
| `panel-sequence` | The second step of the menu overlay |
| `open` | Opens a book while its snapshot is on screen |
| `paginate`, `prewarm`, `warm-up`, `dict-index` | Low-priority work: page numbers, snapshots ahead of time, hyphenation patterns, dictionary indexes |
| `dict` | Dictionary lookups |
| `snapshot-writer`, `index-saver` | Disk writes of snapshots and the library index |

The main thread waits for the panel in one place. The driver reads the framebuffer when it processes an update, so a new book page is blitted only after the previous page update has finished.

## Extension points

Today these are code paths in the workspace. None is a stable API yet.

| Point | Where |
| --- | --- |
| Screens | A builder in `crates/aristotle-ui/src/screens/` and a `Screen` variant in the reader ([Adding a screen](ui-framework.md#adding-a-screen)) |
| Widgets | `aristotle_ui::tree::Widget` ([Adding a widget](ui-framework.md#adding-a-widget)) |
| Languages | A JSON file in `crates/aristotle-i18n/locales/` ([Adding a language](ui-framework.md#adding-a-language)) |
| Hyphenation | `aristotle_ui::text::Hyphenator` for the UI. Liang patterns in `data/hyphenation-patterns/` for books, chosen by `HYPH_LANGS` in `tools/build-render.sh` |
| Book formats | `aristotle_library::format` detects a format by content. EPUB goes through Plato's HTML engine. MOBI 7 is converted to EPUB first (`aristotle-render/src/mobi.rs`). PDF and the rest go through MuPDF |
| Dictionaries | StarDict `*.ifo` files in `/mnt/us/dictionaries` or KOReader's folder. `crates/aristotle/src/dict/` picks them by script and the book's language |
| Content | Folders under `/mnt/us/documents`. Any tool that copies files over USB or SSH is a source (the site's [ideas](https://piotrkowski.github.io/aristotle/guides/ideas.html)) |
| Refresh policy | The waveform hints in `aristotle_ui::ui` and `panel::plan` in the reader ([Transitions](ui-framework.md#transitions)) |

`aristotle-hal` knows only the PW4. There is no device profile yet.

### Why not a web framework or Slint

Leptos and Topcoat render HTML for a browser. On the device that means a browser engine of tens of megabytes with no control over waveforms, regions or the driver's one update at a time. Slint is the serious alternative. It has a software renderer, a PW4 backend under MIT or Apache, and GPLv3 terms compatible with the AGPL. Its renderer reports dirty rectangles without the reason for the change, so our refresh policy would have to guess from rectangles. Its text wraps greedily, without hyphenation or justification. It adds a compiler and a runtime to a binary we cut from 29.5 to 8.4 MB. Its royalty-free licence excludes embedded devices, so a closed third-party frontend would need a commercial licence. iced, egui and Xilem are GPU-first or redraw each frame. Third-party authors who prefer Slint can still ship a Slint program as an external app.

## Related projects

Check these before building something that may already exist. What the device bundle ships, with licences, is in [THIRD_PARTY.md](../THIRD_PARTY.md).

- Built on: [Plato](https://github.com/baskerville/plato) (AGPL-3.0) renders the pages, through `plato-core` from our fork ([piotrkowski/plato](https://github.com/piotrkowski/plato), branch `aristotle`; [reader.md](reader.md)), with none of its Kobo interface. [KOReader](https://github.com/koreader/koreader) (AGPL-3.0) gave the PW4 sysfs paths (battery, frontlight, hall sensor), the `refresh_rex` waveform policy in [koreader-base](https://github.com/koreader/koreader-base) and the way to stop the stock framework, and the dev loop uses its `dropbear`. [FBInk](https://github.com/NiLuJe/FBInk)'s `eink/mxcfb-kindle.h` (GPL-3.0) is our reference for the Kindle EPDC ABI; no FBInk code is linked. [eink-emulator](https://github.com/katadelos/eink-emulator) and its [QEMU fork](https://github.com/katadelos/qemu/tree/eink-emulator) boot the stock PW4 (`imx6sll-rex`) firmware; `tools/emulator/` sets them up.
- Device knowledge: [slint-kindle-backend](https://github.com/sverrejb/slint-kindle-backend) (MIT or Apache-2.0), a Rust Kindle backend tested on the PW4 (Rex update struct, ioctl probing, evdev touch). [yb-reader](https://github.com/albu/yb-reader) (AGPL-3.0), a Rust reader for newer Kindles that takes over through Amazon's `DONT_START_FRAMEWORK` flag. [kindlemodding.org](https://kindlemodding.org) lists the LIPC services, their properties and the upstart jobs. [openlipc](https://github.com/arkq/openlipc) and [liblipc-go](https://github.com/clintharrison/liblipc-go) show that LIPC is D-Bus underneath and works without `liblipc.so`. Amazon's GPL source release for 5.13.6 (`Kindle_src_5.13.6_3731980038.tar.gz`) has the 4.1.15-lab126 kernel tree and configuration that kernel modules need.
- Jailbreak and tools: [KindleModding](https://github.com/KindleModding) publishes the jailbreaks (LanguageBreak covers firmware up to 5.16.2.1.1), Hotfix and the documentation the install relies on. KUAL and MRPI come from NiLuJe's snapshots on MobileRead. [KindleTool](https://github.com/NiLuJe/KindleTool) unpacks update packages. [koxtoolchain](https://github.com/koreader/koxtoolchain)'s `kindlepw2` matches the PW4 below 5.16.3; we build static musl binaries with Zig and Rust instead.
- A kernel of our own would start through kexec from the stock one, since the PW4 is secure-boot locked ([pw4-reference.md](pw4-reference.md)). [kindle-kexec](https://github.com/katadelos/eink-emulator/tree/HEAD/kindle-kexec) is a kexec module with a PW4 profile, tested only in the emulator. [akemnade/linux](https://github.com/akemnade/linux) has an out-of-tree mainline DRM driver for the i.MX6 EPDC, with device trees for Kobo readers on the same SoC.
- Other readers and systems: [cadmus](https://github.com/OGKevin/cadmus), a maintained Plato fork for Kobo. [kindle-dash](https://github.com/H4rryp0tt3r/kindle-dash), a replacement OS for the PW2, which is not locked. [Quill OS](https://github.com/Quill-OS/quill), a replacement system for Kobo readers and the Kindle Touch.

## Planned

None of this is in the code yet. Each step should land on its own and pass the same checks. The previews stay identical unless the step changes visuals on purpose. The device scenarios keep the same updates, modes and flashes, with latency within 10 %. The binary grows by at most 50 KB per step, and peak RAM is checked.

### xtask

Host tooling moves to `cargo xtask`, one Rust binary with std and `xshell`. Its commands are `check`, `build [--device]`, `dist`, `run PKG[:BIN]`, `screenshot`, `scenario NAME`, `preview` and `install`. Shell stays for the C cross-build, the QEMU setup and the scripts in `device/` that run on the Kindle's busybox.

### Structure

The steps, in order:

1. `aristotle-core` for shared types: geometry, waveform kinds, formats, locators.
2. A hal `Device` trait with a `WaveTable`. The rest of the code then speaks of update kinds such as page turn, flash and gray chrome, and the device profile maps them to mode numbers.
3. `aristotle-dict` out of the reader, `aristotle-testkit` for sample books and a headless device, and `aristotle-screens` out of `aristotle-ui`.
4. A `Screen` trait with typed action enums and a `Nav` result, a settings registry that renders the settings screen and the reader menu, and `Engine`, `Layout` and `Converter` traits in `aristotle-render`.

Shortcuts to remove along the way:

- The app changes its working directory so Plato finds its data.
- Waveforms exist in four forms: a hal newtype, a ui enum, integer constants and `FLASH`.
- Screen geometry and device paths are literals in several crates.
- Typography steps are defined twice, in points and in pixels.
- Screens use integer action ranges.
- Rejected page-turn strategies (`render::screen`, `ARISTOTLE_TURN`) are still code. They belong in `aristotle-observe bench`.
- Test books come from Python scripts in `tools/host`.
- Quitting needs a one-second hold on the library title.

### Dashboards and the SDK

Dashboards will put your own data on the screen without code on the device. The simplest is an image dashboard: a PNG or JPEG made anywhere, scaled to the screen and converted to 16 grays. A document dashboard is a small JSON file that describes one screen from the toolkit's nodes (`column`, `row`, `stack`, `text`, `image`, `icon`, `progress`, `rule`, `spacer`) with placeholders such as `{date}` and `{battery}`. The device lays it out with its own fonts and refresh policy. `aristotle-dash` will render documents, and `aristotle-dash preview` will produce the device's pixels on the host. Documents will arrive over USB or SSH, or by HTTP from one host behind a `net` feature. Between updates the device will be able to suspend and wake on an RTC alarm. A web app on a server can produce the JSON document or a captured PNG, so no browser runs on the device.

External apps will live in `/mnt/us/apps/<id>/app.json`. Aristotle will stop its display thread, release the touchscreen, run the program and redraw with a flash when it exits.

`aristotle-sdk` will re-export the stable parts for native apps: the `Screen` trait, the tree builders and widgets, waveform kinds, settings and the i18n catalog. It follows once the `Screen` trait has gone a release without changes.

Scripting is not planned. Lua, Luau, Rhai and WASM were considered. Dashboards cover the "show my data" case, and the hot paths need native code. If on-device automation ever needs scripts, the candidate is Lua 5.4 through `mlua` behind a `lua` feature.
