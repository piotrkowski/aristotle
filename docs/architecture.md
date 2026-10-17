# Architecture

This is the target structure of Aristotle and the plan for getting there. Parts of it already exist; the rest lands in small steps (see [Migration](#migration)). Where the code and this document disagree, the code is the current truth and this document the direction.

## Principles

- **One static binary per device.** Extensions are Rust traits with compile-time registries (`const` slices), selected by Cargo features. There is no runtime plugin system: static musl binaries can't `dlopen`, and Rust has no stable ABI.
- **Pure-Rust crates build and test on any host.** Only `aristotle-render` (Plato's engine and its C libraries) and the app link C code.
- **Device facts live in one place, the device profile.** Everything else speaks in semantic update kinds ("page turn", "flash", "gray chrome"), never in raw waveform numbers.
- **English is the source language.** Other languages are catalogs checked at compile time.

## Crates

```
                              aristotle  (the app: shell, screen controllers, services)
      ┌────────────────┬───────────────┼───────────────┬────────────────┬───────────────┐
aristotle-screens  aristotle-render  aristotle-dict  aristotle-library  aristotle-hal
   │         │       │ plato-core + C       │                │               │
aristotle-ui  aristotle-i18n                │                │               │
   └─────────────────┴──────────┬───────────┴────────────────┴───────────────┘
                          aristotle-core
dev and test: aristotle-observe, aristotle-pen, aristotle-devtools      host: tools/press (Python)
```

| Crate | Responsibility |
| --- | --- |
| `aristotle-core` | Shared types without dependencies: geometry, the semantic update kind (`Wave`), formats, locators, rich text, `Lang`, the `perf!` timing macro |
| `aristotle-hal` | The `Device` trait and its implementations: `pw4` (default feature) and `headless`, which records updates for host tests. Display thread, touch, frontlight, battery, system facts, the device agent's client |
| `aristotle-ui` | The toolkit: canvas, text, layout tree, widgets, gestures, icons, theme, refresh policy |
| `aristotle-i18n` | Message catalogs, plural rules, date, number, size and duration formatting |
| `aristotle-screens` | View builders: library, reader menu, settings, sleep screen, dictionary and footnote popups; the preview binary |
| `aristotle-library` | Scanning, the index, metadata, covers, importers (Kindle sidecars, clippings), export, inbox folders |
| `aristotle-dict` | The `Dictionary` trait and the per-language router; StarDict now, Kindle dictionaries later |
| `aristotle-render` | The `Engine`, `Layout` and `Converter` traits; Plato reflow, MuPDF reflow and fixed layout; pages and their header and footer |
| `aristotle` | The app: event loop, navigation stack, controllers, background jobs, snapshots, progress, the settings store |
| `aristotle-devtools` | Small binaries for bring-up and measurement: hello, touch, e-ink benchmarks |

**Two workspaces, on purpose.** The root workspace holds the pure-Rust crates, so `cargo test` works on a fresh clone. A second workspace holds `aristotle-render` and the app, because they need Plato's patched sources and C libraries, built by `tools/build-plato.sh`. `tools/check.sh` runs fmt, clippy and the tests in both and checks that shared dependencies agree.

**Size.** Cargo features trim the binary: formats (`render/djvu` is off), MuPDF's font set, dictionary formats, device profiles, and app modules (`dict`, `notes`, `newspaper`). Night mode and languages aren't features: they cost a few kilobytes.

## Extension points

| Point | Shape |
| --- | --- |
| **Document engines** | `Engine { id, accepts(Format), open(path, &Style, &Cx) -> Box<dyn Layout> }`. `Layout` locates a position, steps pages, renders a page, and reports the table of contents, position and capabilities (lazy or eager pagination, word boxes, links). A `Converter` turns one format into another, e.g. MOBI into EPUB. `render::ENGINES` lists the compiled-in engines in order of preference. |
| **Dictionaries** | `Dictionary { info() -> {id, name, from, to}, lookup(word) -> Vec<Entry> }`, plus `DictFormat { discover(dir) }`. The router orders results by the book's language. |
| **Content sources** | The device is offline, so a source is a file protocol, not code. A producer on the computer (the newspaper, a future `aristotle push`, Calibre's folder driver) writes `documents/<Name>/.aristotle-source.json` with `{kind: periodical, series or inbox; title; keep}`. The library shows that folder as a collection, with retention for periodicals. The newspaper already writes `documents/Gazeta/.aristotle-source.json` (`{"kind": "periodical", "title", "keep", "producer", "version": 1}`, see [press.md](press.md)); the library does not read it yet. Importers bring in metadata from Kindle sidecars, clippings and Calibre. |
| **Screens and apps** | `Screen { build(&mut Cx) -> Tree, on_event(UiEvent) -> Nav, on_touch_raw(...) -> Option<Nav>, tick, transition() }`. `Nav` is `Stay`, `Push`, `Pop`, `Replace` or `Quit`. An `APPS` registry (id, title, icon, constructor) builds the apps menu. Each screen has a typed action enum. |
| **Settings** | `Setting<T> { key, label, section, default, kind, scope, effect }`, where `kind` is a toggle, choice, steps or slider; `scope` is global or per book; and `effect` says whether the change is live, needs a re-layout, or a restart. The settings screen and the reader menu are rendered from the `SETTINGS` registry. Values are stored as a JSON map. |
| **Themes** | Data, not a trait: `Theme { palette (day or night), fonts, scale }`. Night mode is a palette plus an inversion method (software, or the EPDC's hardware inversion). |
| **Device profiles** | `Device { info() -> {model, size, stride, dpi, rotation}, panel, touch, frontlight, battery, waves() -> &WaveTable, paths() -> {books, state, dictionaries}, agent() }`. `hal::detect()` picks the profile from the board id. |
| **Refresh policy** | `ui::refresh::plan(Transition, Content, night, &mut Counters) -> Update { wave, region, full }` for each transition: page turn, restyle, screen switch, menu open and close, chrome, press, shelf page, sleep, wake. The device's `WaveTable` maps each kind to a waveform mode; on the PW4, page turns use mode 9 and flashes mode 8 (see [the e-ink notes](eink-rendering.md)). All refresh rules live here and are tested on the host as a table. |

**Scripting.** Lua, Luau, Rhai and WASM were weighed and left out. Every extension point is either on a hot path and native (engines, dictionaries, refresh) or would need the whole UI bound into scripts (screens). A scripting layer would add an API to keep stable and save nothing in size: Lua 5.4 costs about 0.5 MB, Luau 1.5–2.5 MB. If user automations (tap zones, gesture maps, newspaper filters) ever need it, Lua 5.4 through `mlua` behind a `lua` feature is the candidate, with scripts calling commands, never widgets.

## Languages

- A `catalog!` macro defines one struct with a field per message; each language is a struct literal, so a missing or misspelt key fails to compile. No build script, no runtime parser.
- Plurals: `Plural { one, few, many, other }`. English uses `one` for 1 and `other` otherwise. Polish uses `one` for 1, `few` when n mod 10 is 2–4 and n mod 100 isn't 12–14 (2, 22 książki), and `many` otherwise (5, 12, 25 książek).
- The catalog also holds month and weekday names, the decimal separator, and date, size and duration formatting.
- Tests check that every plural form is filled and that placeholders match across languages. CI fails on string literals with non-ASCII letters in screen code outside the catalogs.
- The language is a setting (Automatic, English, Polski). The UI's hyphenation follows the UI language; a book's hyphenation follows the book.

## Migration

The steps land one at a time, each verified the same way: `tools/check.sh`, identical preview images unless the step changes visuals on purpose, the device scenarios in `devloop/kindle/scenarios` (same updates, modes and flashes as before, latency within 10 %), the binary size (budget: +50 KB per step) and peak RAM.

1. **Merge the feature work in flight**, one branch at a time, then record the baselines.
2. **Rename** the project from its working name, aristotle, to Aristotle: host scripts and docs, the cache directory, the KUAL extension, and the device agent. The agent's upstart job lives on the root filesystem, so its migration is a script with an automatic rollback, rehearsed in the emulator first.
3. **Structure**, in order: `aristotle-core`; then, in parallel, the hal `Device` trait and `WaveTable`, the i18n crate, `aristotle-dict`, inbox folders, and the native workspace root; then the refresh policy, `aristotle-screens` with typed actions and translated strings, the app shell with the `Screen` trait, the settings registry, and finally the render traits.

Known shortcuts to remove along the way:

- The app changes its working directory so Plato finds its data; Plato should take an absolute data root.
- Waveforms exist in four forms (a hal newtype, a ui enum, integer constants, `FLASH`); they become `core::Wave` and the `WaveTable`.
- Screen geometry and device paths are literals in several crates; they come from the device profile.
- Typography steps are defined twice, in points and in pixels.
- Screens use integer action ranges instead of typed actions.
- Rejected page-turn strategies (an A2 preview before the page) are still code; they move to the devtools as benchmarks.
- Quitting needs a one-second hold on the library title; it becomes a menu item.
