# Contributing

Small fixes can go straight to a pull request. For anything larger, open an
[issue](https://github.com/piotrkowski/aristotle/issues) first to agree on the
approach. Before you touch the display or input code, read
[AGENTS.md](../AGENTS.md).

## Workspace

The repository is one Cargo workspace. A plain `cargo build`, `cargo test` or
`cargo clippy` covers the pure-Rust default members, which CI checks. The two
native members, `crates/aristotle-render` and the reader app
`crates/aristotle-reader`, link Plato's engine and its C libraries. CI does not
build them, so check them locally.

Plato comes from our fork in the `third_party/plato` submodule
([docs/reader.md](../docs/reader.md), "Plato fork"). Cargo needs it even for
the default members, so clone with `--recurse-submodules`, and run
`git submodule sync && git submodule update --init` when the submodule
changes. Changes to Plato are commits on the fork's `aristotle` branch,
followed by a submodule bump here.

## Building

You need Rust from [rustup](https://rustup.rs) with the
`armv7-unknown-linux-musleabihf` and `x86_64-unknown-linux-musl` targets. The
C libraries also need curl, tar, xz, openssl, xxd, make, cmake, bash and a host
C compiler. `tools/build-plato.sh` downloads a pinned Zig.

Builds go to `$ARISTOTLE_CACHE` (default `~/.cache/aristotle`), since `/tmp`
is a small RAM disk on many hosts. Do the same for your own cargo runs:

```sh
. devloop/env.sh   # ARISTOTLE_CACHE, and TMPDIR inside it
export CARGO_TARGET_DIR="$ARISTOTLE_CACHE/target"
```

```sh
git clone --recurse-submodules https://github.com/piotrkowski/aristotle && cd aristotle
cargo test                                                   # default members
cargo run --release -p aristotle-ui --bin aristotle-ui-preview   # every screen as PNG

# Native members on the host (C libraries once, about 6 minutes on 2 cores)
PLATO_TARGET=x86_64-unknown-linux-musl PLATO_LIBS_ONLY=1 tools/build-plato.sh
CARGO_CMD=test tools/build-render.sh host

# The device bundle
PLATO_LIBS_ONLY=1 tools/build-plato.sh
tools/build-render.sh device
```

Everything that runs on the Kindle is a static
`armv7-unknown-linux-musleabihf` binary.

## Trying changes

1. On the host, `aristotle-ui-preview` renders the screens and
   `aristotle --preview` the reading interactions (`docs/reader.md`). Both log
   each panel update with its waveform and rectangle. Compare the logs before
   and after your change.
2. Test anything that could hang boot, touch the stock framework or load
   kernel code in the emulator (`devloop/emulator/setup.sh`).
3. On the device, use the dev loop in `devloop/`. No recovery path is verified
   yet, so never write to the root filesystem, the boot partitions, `/mnt/wfm`
   or `/var/local` on someone's device without their consent.

If your change affects what reaches the panel, run the device scenarios in
`devloop/kindle/scenarios/` with `aristotle-observe run SCRIPT`. Unless the
change is meant to alter them, the updates and waveforms should stay the same
and latency should stay within 10 %. Numbers in the docs are tagged `[dev]` for
the device and `[host]` for the PC.

## Checks

CI runs the first five on every pull request:

```sh
cargo fmt --check        # every member, native ones included; not --all, which
                         # would also check Plato's crates
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo build --release --locked --target armv7-unknown-linux-musleabihf
cargo deny check         # deny.toml: licences, advisories, bans, sources
# The native members, locally:
CARGO_CMD=clippy tools/build-render.sh host
CARGO_CMD=test tools/build-render.sh host
```

`tools/screenshots/compose.sh` makes the screenshots for the README, the docs
and the site from the device captures in its manifest
(`tools/screenshots/README.md`).

## Text and test data

Code, comments, docs and commits are in English. Tests, docs and screenshots
use public-domain books (Prus, Mickiewicz, Plato in Witwicki's translation,
Aristotle in Chase's translation, Austen, Melville) or text written for the
test. Never use copyrighted books.

## What this project will never include

Allowed:

- Our own userspace, UI, reader and sync, with no Amazon services.
- A kernel built from Lab126's published GPL sources.
- E-ink waveforms and Wi-Fi or touch firmware read from the user's device at
  install or run time and never shipped, as postmarketOS's
  `kobo-epdc-extractor` does.
- Descriptions of file formats and ABIs in our own words.
- StarDict or Wiktionary dictionaries that the user supplies.

Never:

- A jailbreak, exploits, keys or OTA blockers, shipped, downloaded or
  scripted.
- Anything that touches DRM: keys, KFX, books from the Kindle Store.
- Firmware, waveforms (`/mnt/wfm`), Amazon binaries or libraries, Amazon fonts
  (Bookerly, Ember) or Kindle Store dictionaries in the repository.
- Decompiled code, Ghidra or JADX output, or copied Amazon configuration files
  and upstart jobs.
- Amazon cloud APIs: Whispersync, accounts, device tokens.
- Writes to the boot partitions, U-Boot, IDME or diags before a recovery path
  is verified, or anything that breaks secure boot.
- Amazon logos or trade dress, or "Kindle-X" names.

## AI-assisted contributions

You may use AI tools, but:

- Write issue and pull request descriptions yourself.
- Understand and review the code you submit. You are responsible for it.
- Leave out unrelated generated changes.

## Licence

Contributions are licensed under the [GNU AGPL 3.0 or later](../LICENSE).
