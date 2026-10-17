# Contributing

Small fixes can go straight to a pull request. For anything larger, open an
[issue](https://github.com/piotrkowski/aristotle/issues) first to agree on the
approach. Before you touch the display or input code, read
[`AGENTS.md`](../AGENTS.md).

The [documentation index](../docs/README.md) separates user guides from
implementation references. Keep behavior and limitations consistent across
both. For bug reports, include the details in
[Report a bug](../docs/troubleshooting.md#report-a-bug).

The [glossary](../docs/glossary.md) explains device and book-format terms.

## Workspace

The repository is one Cargo workspace, a group of Rust crates built together.
A plain `cargo build`, `cargo test` or
`cargo clippy` covers the pure-Rust default members, which CI checks. The two
native members, `crates/aristotle-render` and the reader app
`crates/aristotle`, link [Plato](../docs/glossary.md#plato)'s engine and its C libraries, which
`tools/build-plato.sh` builds. CI checks those members too, with the libraries cached.

Plato comes from our fork in the `third_party/plato`
submodule. See the
[fork workflow](../docs/reader.md#plato-fork). Cargo needs it even for
the default members, so clone with `--recurse-submodules`, and run
`git submodule sync && git submodule update --init` when the submodule
changes. Changes to Plato are commits on the fork's `aristotle` branch,
followed by a submodule bump here.

## Building

You need Rust from [rustup](https://rustup.rs) with the
`armv7-unknown-linux-musleabihf` and `x86_64-unknown-linux-musl` targets. The
C libraries also need:

- `curl`
- `tar`
- `xz`
- `openssl`
- `xxd`
- `make`
- `cmake`
- `bash`
- A host C compiler.

`tools/build-plato.sh` downloads a pinned Zig toolchain.

Builds go to `$ARISTOTLE_CACHE` (default `~/.cache/aristotle`), since `/tmp`
is a small RAM disk on many hosts. Do the same for your own `cargo` runs:

```sh
git clone --recurse-submodules https://github.com/piotrkowski/aristotle
cd aristotle
. tools/env.sh   # ARISTOTLE_CACHE, and TMPDIR inside it
export CARGO_TARGET_DIR="$ARISTOTLE_CACHE/target"

cargo test                                                   # default members
cargo run --release -p aristotle-ui --bin aristotle-ui-preview   # every screen as PNG

# Native members on the host (C libraries once, about 6 minutes on 2 cores)
PLATO_TARGET=x86_64-unknown-linux-musl PLATO_LIBS_ONLY=1 tools/build-plato.sh
tools/build-render.sh host            # fonts and patterns the tests read; again after changing them
CARGO_CMD=test tools/build-render.sh host

# The device bundle, and the zip a release ships
PLATO_LIBS_ONLY=1 tools/build-plato.sh
tools/build-render.sh device
tools/package.sh zip     # $ARISTOTLE_CACHE/dist/aristotle-VERSION.zip
```

Each compiled Aristotle program for the Kindle is a
static binary for
`armv7-unknown-linux-musleabihf`, with the musl
C library linked in. `tools/package.sh` defines what goes
on the Kindle's USB drive: `aristotle/` (the app bundle with its licences and
`SOURCE.txt`) and `extensions/aristotle/` (the [KUAL](../docs/glossary.md#kual) launcher extension). Unzip the zip
to the root of the drive, as users do. For development access, follow
[Host setup](../device/README.md#host-setup) for the dedicated key,
NetworkManager profile, and SSH connection. After creating the key, you can
also use `tools/host/install-extensions.sh` with the Kindle mounted. It
copies the same tree plus the read-only probe and your dev-loop public key,
then ejects the Kindle.

## Trying changes

1. On the host, run `aristotle-ui-preview` for screens and
   `aristotle --preview` for [reading interactions](../docs/reader.md#preview-and-device-scenario). Both log
   each panel update with its [waveform](../docs/glossary.md#waveform) and rectangle. Compare the logs before
   and after your change.
2. Test anything that could hang boot, touch the stock framework or load
   kernel code in the emulator (`tools/emulator/setup.sh`).
3. On the device, use the dev loop in `tools/host/` (`device/README.md`). No recovery path is verified
   yet, so never write to the [root filesystem](../docs/glossary.md#rootfs), the boot partitions, `/mnt/wfm`
   or `/var/local` on someone's device without their consent.

If your change affects what reaches the panel, run the device scenarios in
`tools/scenarios/` with `aristotle-observe run SCRIPT`. Unless the
change is meant to alter them, the updates and waveforms should stay the same
and latency should stay within 10 %. Footnotes mark results from the device
or the PC.

## Checks

CI runs these checks on every pull request:

```sh
cargo fmt --check        # every member, native ones included; not --all, which
                         # would also check Plato's crates
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo build --release --locked --target armv7-unknown-linux-musleabihf
cargo deny check         # .cargo/deny.toml: licences, advisories, bans, sources
gitleaks git --gitleaks-ignore-path .github/.gitleaksignore .   # secrets in the history
# The native members (clippy with --all-targets and -D warnings):
CARGO_CMD=clippy tools/build-render.sh host
CARGO_CMD=test tools/build-render.sh host
```

CI also builds the device bundle and keeps the zip from `tools/package.sh`
as a workflow artifact for 14 days.

`tools/screenshots/compose.sh` makes the screenshots for the README, the docs
and the site from the device captures in its manifest
(`tools/screenshots/README.md`).

## Releases

Aristotle has one version, `workspace.package.version` in `Cargo.toml`. The
app shows it with the short commit in *Settings › About*, and `SOURCE.txt`
names the version, commit and tag. To release:

1. Set the version in `Cargo.toml` and move the "Unreleased" entries of
   `CHANGELOG.md` into a section for it, with the date.
2. Tag the commit on `main` as `vX.Y.Z` and push the tag.

The release workflow (`.github/workflows/release.yml`) builds everything from
source without caches, checks that the tag matches the version, and creates a
draft release with:

- The zip.
- A source tarball from `tools/package.sh source`.[^1]
- `SHA256SUMS`.
- Build provenance attestations.
- The changelog section as notes.

Read the draft, then publish it.

[^1]: Aristotle and Plato at the tagged commits, with the URLs and SHA-256 sums of the C library sources.

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
