# Contributing

Small fixes can go straight to a pull request. For anything larger, open an
issue first, so others know what you are working on and the approach can be
agreed before the work. Report bugs in the
[issue tracker](https://github.com/piotrkowski/aristotle/issues) with the
template's device, firmware and log fields.

[AGENTS.md](../AGENTS.md) is the short list of device facts that break naive
code (the framebuffer stride, the waveform numbering, one update at a time);
read it before you touch the display or input code.

## One workspace, two parts

The repository is one Cargo workspace with one `Cargo.lock`:

- **The default members** are the pure-Rust crates: the hardware layer, the UI
  toolkit, the library, the measurement tools. A plain `cargo build`,
  `cargo test` or `cargo clippy` at the root covers exactly these; they build
  and test on any host, and CI checks them.
- **The native members**, `crates/aristotle-render` and the reader app
  `crates/aristotle-reader`, link Plato's document engine and its C libraries
  (MuPDF, FreeType, HarfBuzz and others). Those are built once by
  `tools/build-plato.sh` with Zig, for the device and for the host, and
  `tools/build-render.sh` builds the two crates against them. CI does not
  build them; run their checks locally.

Plato's engine comes from our fork in the `third_party/plato` submodule
(branch `aristotle`, see [docs/reader.md](../docs/reader.md), "Plato fork").
Cargo reads it even for the default members, so clone with
`--recurse-submodules`. In an existing clone, run
`git submodule sync && git submodule update --init` after the submodule's URL
or commit changes. Changes to Plato are commits on the fork's `aristotle`
branch, followed by a submodule bump here.

## Building

You need Rust through [rustup](https://rustup.rs) with the
`armv7-unknown-linux-musleabihf` and `x86_64-unknown-linux-musl` targets, and
for the C libraries curl, tar, xz, openssl, xxd, make, cmake, bash and a host
C compiler.
`tools/build-plato.sh` downloads a pinned Zig. Builds and downloads go to the
cache directory, `$ARISTOTLE_CACHE` (default
`${XDG_CACHE_HOME:-$HOME/.cache}/aristotle`), which `devloop/env.sh` defines;
the scripts source it. For your own cargo runs, source it too and set
`CARGO_TARGET_DIR` there: `/tmp` is a small RAM disk on many hosts.

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

Everything that runs on the Kindle is a **static** `armv7-unknown-linux-musleabihf`
binary. The stock userspace uses the soft-float ABI, so nothing may link its
libraries dynamically.

## Trying changes

1. **On the host.** The UI toolkit and the reader render the same pixels on the
   host: `aristotle-ui-preview` for the screens, `aristotle --preview` for the
   reading interactions (`docs/reader.md`). Both report every panel update with
   its waveform and rectangle (the reader in `updates.txt`); compare them before
   and after.
2. **In the emulator.** `devloop/emulator/setup.sh` sets up a QEMU PW4 with the
   stock firmware. Anything that could hang boot, touch the stock framework or
   load kernel code goes there first.
3. **On the device.** Use the dev loop in `devloop/` (SSH over the USB cable).
   Never write to the root filesystem, the boot partitions, `/mnt/wfm` or
   `/var/local` on someone's device without their explicit consent: no recovery
   path is verified yet. Device experiments are reversible: copies in
   `/var/tmp`, bind mounts, stopped services, so a reboot restores the stock
   system.

Changes to what reaches the panel (page turns, menus, popups, the library) also
run the device scenarios in `devloop/kindle/scenarios/` with `aristotle-observe
run SCRIPT`. Each scenario reports, per interaction, the time to the first and
last update, the number of updates, flashes and the waveforms. A change should
keep the same updates and waveforms, unless changing them is the point, and
stay within 10 % on latency. Numbers in the docs say where they come from:
`[dev]` for the device, `[host]` for the PC.

## Checks

CI runs the first three on every pull request:

```sh
cargo fmt --check        # every member, native ones included; not --all, which
                         # would also check Plato's crates
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
# The native members, locally:
CARGO_CMD=clippy tools/build-render.sh host
CARGO_CMD=test tools/build-render.sh host
```

`tools/build-render.sh` builds, lints or tests the native members with
`cargo <command> -p aristotle-render -p aristotle-reader` against the C
libraries of a `tools/build-plato.sh` run.

README screenshots are composed by `tools/screenshots/compose.sh` from raw
panel captures listed in its manifest.

## Text and test data

Everything is in English: code, comments, docs and commits. Tests, docs and
screenshots use public-domain books (Prus, Mickiewicz, Plato in Witwicki's
translation, Austen, Melville) or text written for the test, never copyrighted
books.

## AI-assisted contributions

You may use AI tools, but:

- Write issue and pull request descriptions yourself.
- Understand and review all code you submit. You are responsible for it.
- Keep the change to the issue, without unrelated generated changes.

## Licence

Contributions are licensed under the [GNU AGPL 3.0 or later](../LICENSE).
