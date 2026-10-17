# Architecture

Aristotle is a Rust workspace with static PW4 binaries and shell scripts for the device. The UI and library run on the host too. The reader uses Plato's document engine, not its Kobo interface.

## Principles

- Features are compiled in. There is no runtime plugin system.
- Books and dictionaries are files. Network access is optional and limited to configured hosts ([Network](../device/README.md#network)).
- English is the source language. The build checks translation catalogs.
- Panel measurements set the refresh rules. Host tests check those rules.

## Repository

| Path | Contents |
| --- | --- |
| `Cargo.toml`, `.cargo/config.toml` | Workspace, release settings, and static musl targets |
| `crates/` | Rust crates |
| `device/` | KUAL extension, agent, launch scripts, read-only probe, and script tests |
| `tools/` | Builds, packaging, host tools, emulator setup, scenarios, and firmware probes |
| `third_party/plato` | Plato fork, submodule on branch `aristotle` |
| `docs/`, `site/` | Documentation and project site |

## Crates

| Crate | Role |
| --- | --- |
| `aristotle-hal` | Framebuffer, panel updates, display thread, touch, battery, frontlight, and USB mode |
| `aristotle-ui` | Screen toolkit and `aristotle-ui-preview` ([UI framework](ui-framework.md)) |
| `aristotle-i18n` | Translations, plurals, dates, and numbers |
| `aristotle-library` | Book index, metadata, covers, reading state, clippings, and CLI ([Library](library.md)) |
| `aristotle-net` | HTTP, authentication, progress sync, and catalogs ([Network](library.md#network-progress-sync-and-catalogs)) |
| `aristotle-render` | Plato and MuPDF, pagination, page rendering, and footnotes ([Reader](reader.md)) |
| `aristotle` | App event loop, screens, reading sessions, snapshots, and dictionaries |
| `aristotle-observe` | Touch injection, panel logs, and latency benchmarks |

`aristotle-ui` and `aristotle-library` depend on `aristotle-i18n`, not the HAL. The renderer uses the HAL and `plato-core`. The app combines these crates; `aristotle-observe` is a separate tool.

Plain `cargo build`, `cargo test`, and `cargo clippy` omit `aristotle-render` and `aristotle`, which link C libraries. Cargo still needs the Plato submodule checked out. The network crate's optional `tls` feature also builds C through ring.

`tools/build-plato.sh` builds the C libraries. `tools/build-render.sh [device|host]` builds the reader bundle. `tools/package.sh zip` adds the KUAL extension to the release zip. See [Building](../.github/CONTRIBUTING.md#building).

The release profile uses `opt-level = "s"`, LTO, one codegen unit, stripped symbols, and `panic = "abort"`. The layout crates use level 3 ([Size budget](reader.md#size-budget)).

## On the device

### Processes

The bundle lives in `/mnt/us/aristotle/`. `run-aristotle.sh` stops the stock GUI through `takeover.sh`, runs the app, and restores the GUI on exit. Logs go to `/var/tmp/aristotle.log`.

The optional agent provides USB networking, drive mode, and boot startup. Its files and jobs are listed in [device/README.md](../device/README.md). Installation is covered in [Installing Aristotle](install.md).

### Threads

The app's main thread handles input, screens, and page rendering. Workers open books, paginate, save the index and snapshots, and look up dictionary words. See [Reader threads](reader.md#threads).

Partial UI updates go through the HAL's `epdc-display` thread. Book pages use a separate full-screen path. Menu overlays use `panel-sequence`. [The UI model](ui-framework.md#model) shows these paths.

## Extension points

These are source-code changes, not runtime plugins.

| Change | Location |
| --- | --- |
| Screen | `crates/aristotle-ui/src/screens/` and the app's `Screen` enum ([Adding a screen](ui-framework.md#adding-a-screen)) |
| Widget | `aristotle_ui::tree::Widget` ([Adding a widget](ui-framework.md#adding-a-widget)) |
| Language | `crates/aristotle-i18n/locales/` ([Adding a language](ui-framework.md#adding-a-language)) |
| Format detection | `aristotle_library::format` |
| MOBI conversion | `crates/aristotle-render/src/mobi.rs` |
| Dictionaries | `crates/aristotle/src/dict/`, StarDict files on the device |
| Refresh policy | `aristotle_ui::ui` and the app's `panel::plan` ([Transitions](ui-framework.md#transitions)) |

The HAL supports only the PW4.
