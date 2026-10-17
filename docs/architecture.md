# Architecture

This is the target structure of Aristotle and the plan for getting there. Parts of it already exist; the rest lands in small steps (see [Migration](#migration)). Where the code and this document disagree, the code is the current truth and this document the direction.

## Principles

- **One static binary per device.** Extensions are Rust traits with compile-time registries (`const` slices), selected by Cargo features. There is no runtime plugin system: static musl binaries can't `dlopen`, and Rust has no stable ABI. Code from outside the binary comes in as data (dashboard documents, books, dictionaries) or as a separate program (external apps).
- **Pure-Rust crates build and test on any host.** Only `aristotle-render` (Plato's engine and its C libraries) and the app link C code.
- **Device facts live in one place, the device profile.** Everything else speaks in semantic update kinds ("page turn", "flash", "gray chrome"), never in raw waveform numbers.
- **Offline first.** Every network feature is optional, off by default, and switched on per use.
- **English is the source language.** Other languages are catalogs checked at compile time.

## Repository

```
Cargo.toml          root workspace: the pure-Rust crates and xtask; `cargo test` works on a fresh clone
.cargo/config.toml  the `cargo xtask` alias and the armv7 musl target settings
crates/             every Rust crate, flat; the directory name is the package name
native/             second workspace: the C-linked crates, Plato's patches, the C cross-build
xtask/              build, check, run, screenshot, scenario and release commands, in Rust
device/             what runs on the Kindle, as busybox shell: KUAL extensions, the upstart agent, launch scripts
tools/              host-side tools that aren't Rust: emulator setup, firmware unpacking, ssh_config
docs/               developer documents; docs/guide/ is the user guide
third_party/plato   upstream Plato (git submodule), read-only reference
```

```
crates/
  aristotle-core      shared types: geometry, Wave, formats, locators, rich text, Lang, perf!
  aristotle-hal       the Device trait: pw4 and headless; display thread, touch, light, battery, RTC, network
  aristotle-ui        the e-ink toolkit: canvas, text, layout tree, widgets, gestures, icons, theme, refresh policy
  aristotle-i18n      message catalogs, plurals, date, number, size and duration formatting
  aristotle-screens   view builders for every built-in screen; the preview binary
  aristotle-library   scanning, index, metadata, covers, importers, export, inbox folders; aristotle-library-cli
  aristotle-dict      the Dictionary trait, the per-language router, StarDict
  aristotle-dash      dashboard documents: schema, validation, layout into a Tree, image dithering; host preview CLI
  aristotle-devtools  bring-up and measurement binaries: hello, touch, waveform and page-turn benches, pen, observe; device scenarios
  aristotle-testkit   test-only (publish = false): sample books, libraries and dictionaries, headless device, image comparison
  aristotle-render    [native] Engine, Layout and Converter traits over Plato and MuPDF; the render CLI
  aristotle           [native] the app: event loop, navigation, controllers, jobs, snapshots, settings, dashboards
native/
  Cargo.toml          virtual manifest: members ../crates/aristotle-render and ../crates/aristotle; size profile
  patches/            plato-*.patch and mupdf-*.patch, applied in order
  build.sh            cross-builds MuPDF and the other C libraries; called by xtask
  plato/              the patched Plato source, generated, gitignored
device/
  kual/aristotle/     KUAL extension: start Aristotle, USB network on and off
  kual/probe/         KUAL extension: the read-only device probe
  agent/              the upstart job and aristotlectl, installed with an automatic rollback
  bin/                run-aristotle.sh, takeover.sh
tools/
  emulator/           the PW4 QEMU setup and its patch
  re/                 fetch-tools.sh (Ghidra, JADX), firmware unpackers in Python
```

**Two workspaces, on purpose.** Cargo reads every member's path dependencies, so one workspace would fail on a fresh clone without the generated Plato source, and its tests would need the C libraries. The root workspace stays pure Rust; `native/` holds the two crates that link C, with its own lock file and the size profile (`opt-level = 3` for the layout crates). `cargo xtask check` runs fmt, clippy and the tests in both and checks that shared dependencies agree.

**xtask, not shell, for host tooling.** Host commands are one Rust binary with std and `xshell` only: `check`, `build [--device]` (runs `native/build.sh` when the C libraries are stale, assembles `data/`), `dist` (the bundle and the size report against the budget), `run PKG[:BIN]` (copy over USB network, run, screenshot), `screenshot`, `scenario NAME`, `preview` (screens and dashboards to PNG, compared with the last accepted images) and `install` (copy `device/` to a mounted userstore). Shell stays where it is the native tongue: the C cross-build, the QEMU setup, and everything under `device/`, which runs on the Kindle's busybox.

**Examples and benches.** `examples/` belong to the crate they demonstrate and double as templates: `aristotle-ui` (a screen, render timing), `aristotle-dash` (a weather and calendar dashboard). There is no `benches/` and no criterion: host timings come from examples and the `perf!` macro; device timings come from `aristotle-devtools` and the scenarios, because only the panel tells the truth.

**Docs.** `docs/` holds developer documents, flat, one per subsystem. `docs/guide/` is for users: installing, reading, dictionaries, dashboards, building a frontend. `README.md` points at both; `AGENTS.md` stays the short list of device traps.

**Size.** Cargo features trim the binary: formats (`render/djvu` is off), MuPDF's font set, dictionary formats, device profiles, and app modules (`reader`, `dict`, `notes`, `dash`, `net`). A wall-display build is `--no-default-features --features dash`, without Plato. Night mode and languages aren't features: they cost a few kilobytes.

## Extension points

| Point | Shape |
| --- | --- |
| **Document engines** | `Engine { id, accepts(Format), open(path, &Style, &Cx) -> Box<dyn Layout> }`. `Layout` locates a position, steps pages, renders a page, and reports the table of contents, position and capabilities (lazy or eager pagination, word boxes, links). A `Converter` turns one format into another, e.g. MOBI into EPUB. `render::ENGINES` lists the compiled-in engines in order of preference. |
| **Dictionaries** | `Dictionary { info() -> {id, name, from, to}, lookup(word) -> Vec<Entry> }`, plus `DictFormat { discover(dir) }`. The router orders results by the book's language. |
| **Content sources** | The device is offline, so a source is a file protocol, not code. Anything on the computer (Calibre's folder driver, a script, a future `aristotle push`) copies books into `documents/<Name>/` over USB; a folder may carry `.aristotle-source.json` with `{kind: periodical, series or inbox; title; keep}`, which the library will show as a collection with retention. [Fun use cases](fun-use-cases.md) has examples. |
| **Screens and apps** | `Screen { build(&mut Cx) -> Tree, on_event(UiEvent) -> Nav, on_touch_raw(...) -> Option<Nav>, tick, transition() }`. `Nav` is `Stay`, `Push`, `Pop`, `Replace` or `Quit`. An `APPS` registry (id, title, icon, constructor) builds the apps menu. Each screen has a typed action enum. |
| **Dashboards** | Data, not code: a document in `/mnt/us/dashboards/<name>/` or behind a URL, rendered by `aristotle-dash`. See [Frontends](#frontends). |
| **External apps** | `/mnt/us/apps/<id>/app.json` with `{title, icon, exec, args}`: a static binary or script listed in the apps menu. Aristotle stops its display thread, releases the touchscreen grab, runs it, and redraws with a flash when it exits. |
| **Settings** | `Setting<T> { key, label, section, default, kind, scope, effect }`, where `kind` is a toggle, choice, steps or slider; `scope` is global or per book; and `effect` says whether the change is live, needs a re-layout, or a restart. The settings screen and the reader menu are rendered from the `SETTINGS` registry. Values are stored as a JSON map. |
| **Themes** | Data, not a trait: `Theme { palette (day or night), fonts, scale }`. Night mode is a palette plus an inversion method (software, or the EPDC's hardware inversion). |
| **Device profiles** | `Device { info() -> {model, size, stride, dpi, rotation}, panel, touch, frontlight, battery, rtc, network, waves() -> &WaveTable, paths() -> {books, state, dictionaries, dashboards, apps}, agent() }`. `hal::detect()` picks the profile from the board id. |
| **Refresh policy** | `ui::refresh::plan(Transition, Content, night, &mut Counters) -> Update { wave, region, full }` for each transition: page turn, restyle, screen switch, menu open and close, chrome, press, shelf page, dashboard update, sleep, wake. The device's `WaveTable` maps each kind to a waveform mode; on the PW4, page turns use mode 9 and flashes mode 8 (see [the e-ink notes](eink-rendering.md)). All refresh rules live here and are tested on the host as a table. |

## Frontends

Four ways to put something of your own on the screen, cheapest first. Only the first two are planned for v1.

1. **Image dashboards.** Any PNG or JPEG, made anywhere: Home Assistant, a script, a web page captured by a headless browser. Aristotle scales it to the screen, converts it to 16 grays (Floyd–Steinberg for photographs, none for images already in 16 grays) and shows it full screen. SVG is rasterised by whoever makes it; a vector renderer on the device isn't worth its size.
2. **Document dashboards.** A small JSON document describes one screen out of the toolkit's nodes; the device lays it out with its own fonts and refresh policy, so text is as crisp as the reader's. An image dashboard is the same document with an image as its root.
3. **External apps.** A program that takes the framebuffer itself: one built with another toolkit, an FBInk script, another reader. Aristotle only lends it the screen and input.
4. **Native apps.** Rust code against `aristotle-sdk`, a facade that re-exports the stable parts (`Screen`, the tree builders and widgets, `Wave` kinds, `Setting`, the i18n catalog), compiled in through the `APPS` registry and a feature. It comes after the `Screen` trait and the settings registry have settled; until then, an app is a fork with a feature.

A dashboard document, version 1:

```json
{
  "version": 1,
  "refresh": { "every": "30m", "sleep": true, "flash_every": 8 },
  "source": "http://192.168.1.10:8080/kindle.json",
  "root": { "column": { "gap": 2, "children": [
    { "text": "Kraków · {date}", "style": "title" },
    { "row": { "children": [ { "image": "weather.png", "width": "40%" },
                             { "text": "12° / 4°\nrain after 15:00", "style": "large" } ] } },
    { "rule": {} },
    { "text": "09:00  Dentist\n13:30  Call with Ania", "style": "body" },
    { "progress": 0.62, "label": "Week" }
  ] } }
}
```

- **Nodes:** `column`, `row`, `stack`, `text` (styles from the theme's type scale; `align`; hyphenated and justified like the UI), `image` (path relative to the document, or a URL), `icon` (the toolkit's set), `progress`, `rule`, `spacer`. Unknown nodes and fields are errors, reported on the screen and by the host preview.
- **Placeholders** filled on the device at each render: `{date}`, `{time}`, `{battery}`, `{updated}`.
- **Refresh:** each update is a page turn to the refresh policy (one full-screen mode-9 update) with a mode-8 flash every `flash_every`. With `sleep`, the device suspends between updates and wakes on the RTC alarm. A tap refreshes now; the usual gesture leaves.
- **Delivery:** copied over USB or `scp` into the dashboard's folder, or fetched from `source` when the `net` feature is built and Wi-Fi is switched on for that dashboard. The fetch turns Wi-Fi on, gets the document and the images it names, and turns Wi-Fi off; on failure the last good copy stays with a stale mark. v1 speaks plain HTTP on the local network; HTTPS is a later `tls` feature.
- **Host preview:** `aristotle-dash preview dashboard.json out.png` produces the device's pixels, and `aristotle-dash check` validates. Server code in Rust can build documents from `aristotle-dash`'s serde types.
- A dashboard can be the sleep screen, or the start screen of a wall-display build.

Web frameworks belong on the server. A Leptos, Topcoat or any other web app produces either the JSON document from a route (the server-rendered model maps onto it directly) or HTML that a headless browser captures as a PNG. The device never runs a browser.

**v1 for a weather and calendar dashboard in an afternoon:** `aristotle-dash` with the nodes above and its host preview; the dashboards entry in the apps menu; folder delivery over USB and SSH; HTTP fetch and RTC sleep behind `net` and `sleep` features; one example in `crates/aristotle-dash/examples/` that turns a forecast API and an ICS calendar into a document; a page in `docs/guide/`. Buttons with actions (fetch another document, the next page of a dashboard) are v2.

**Scripting.** Lua, Luau, Rhai and WASM were weighed and left out. Every extension point is either on a hot path and native (engines, dictionaries, refresh) or would need the whole UI bound into scripts (screens), and dashboards cover the "show my data" case without code on the device. A scripting layer would add an API to keep stable and save nothing in size: Lua 5.4 costs about 0.5 MB, Luau 1.5–2.5 MB. If user automations (tap zones, gesture maps, newspaper filters, dashboards that compute on the device) ever need it, Lua 5.4 through `mlua` behind a `lua` feature is the candidate, with scripts calling commands, never widgets.

### Why not a web framework or Slint

Leptos and Topcoat render HTML for a browser, and Topcoat's interactivity runs as JavaScript there. On the device that means a browser engine, the WebKit we are removing or another one of tens of megabytes, with no say over waveforms, regions or the driver's one update at a time. Slint is the serious alternative: a software renderer, a PW4 backend under MIT or Apache, and GPLv3 terms that combine with the AGPL (GPLv3 §13). It still loses here. Its renderer reports dirty rectangles but not why they changed, so the refresh policy (mode-9 page turns, A2 press feedback, flash counters, night inversion) would become a backend guessing from rectangles; its text wraps greedily, without hyphenation or justification, where we need Knuth–Plass and the swash rasteriser tuned for 300 dpi grays; it adds a compiler and runtime to a binary we cut from 29.5 to 8.4 MB; and its royalty-free licence excludes embedded devices, so a closed third-party frontend would need a commercial licence. iced, egui and Xilem are GPU-first, RGBA, or redraw every frame, the opposite of what e-ink wants. Slint does offer third-party authors a nicer language and tooling than ours; they can use it as an external app.

## Languages

- A `catalog!` macro defines one struct with a field per message; each language is a struct literal, so a missing or misspelt key fails to compile. No build script, no runtime parser.
- Plurals: `Plural { one, few, many, other }`. English uses `one` for 1 and `other` otherwise. Polish uses `one` for 1, `few` when n mod 10 is 2–4 and n mod 100 isn't 12–14 (2, 22 książki), and `many` otherwise (5, 12, 25 książek).
- The catalog also holds month and weekday names, the decimal separator, and date, size and duration formatting.
- Tests check that every plural form is filled and that placeholders match across languages. CI fails on string literals with non-ASCII letters in screen code outside the catalogs.
- The language is a setting (Automatic, English, Polski). The UI's hyphenation follows the UI language; a book's hyphenation follows the book. A dashboard's text follows the document's `lang`, or the UI language.

## Migration

The steps land one at a time, each verified the same way: `cargo xtask check` (until it exists, fmt, clippy and the tests in both workspaces by hand), identical preview images unless the step changes visuals on purpose, the device scenarios (same updates, modes and flashes as before, latency within 10 %), the binary size (budget: +50 KB per step) and peak RAM.

1. **Merge the feature work in flight**, one branch at a time, then record the baselines.
2. **Rename** the project from its working name, aristotle, to Aristotle: host scripts and docs, the cache directory, the KUAL extension, and the device agent. The agent's upstart job lives on the root filesystem, so its migration is a script with an automatic rollback, rehearsed in the emulator first.
3. **Repository**, mechanical moves with no behaviour change: `xtask` with `check`, `run` and `screenshot` first, then the rest of `devloop/host`; `native/` as the second workspace root (its lock file, profile, patches and C build move out of `crates/aristotle-render` and `tools/`); `devloop/` and `probe/` split into `device/` and `tools/`; `aristotle-hello`, `aristotle-touch`, `aristotle-bench`, `aristotle-pen` and `aristotle-observe` merged into `aristotle-devtools` (one binary each, the pen record format as a module, the scenarios beside them); `aristotle-reader` renamed `aristotle`; `aristotle-app` deleted, since the reader app runs the same screens on the device.
4. **Structure**, in order: `aristotle-core`; then, in parallel, the hal `Device` trait and `WaveTable`, the i18n crate, `aristotle-dict` (out of the app's `dict` module), `aristotle-testkit`, and inbox folders; then the refresh policy, `aristotle-screens` with typed actions and translated strings (out of `aristotle-ui`, with the preview binary), the app shell with the `Screen` trait, the settings registry, and finally the render traits.
5. **Frontends:** `aristotle-dash` and its host preview; the dashboards app with folder delivery; RTC sleep; Wi-Fi and HTTP fetch; external apps. `aristotle-sdk` follows once the `Screen` trait has gone a release without changes.

Known shortcuts to remove along the way:

- The app changes its working directory so Plato finds its data; Plato should take an absolute data root.
- Waveforms exist in four forms (a hal newtype, a ui enum, integer constants, `FLASH`); they become `core::Wave` and the `WaveTable`.
- Screen geometry and device paths are literals in several crates; they come from the device profile.
- Typography steps are defined twice, in points and in pixels.
- Screens use integer action ranges instead of typed actions.
- Rejected page-turn strategies (an A2 preview before the page, `render::screen` and `ARISTOTLE_TURN`) are still code; they move to the devtools as benchmarks.
- Test books are made by Python scripts in `devloop/host`; they become builders in `aristotle-testkit`.
- Quitting needs a one-second hold on the library title; it becomes a menu item.
