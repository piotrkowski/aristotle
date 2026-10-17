<h1 align="center">Aristotle</h1>

<p align="center">
  A replacement interface for the Kindle Paperwhite 4: a library and a reader that turn pages cleanly and work offline.
</p>

<p align="center">
  <a href="https://github.com/piotrkowski/aristotle/actions/workflows/ci.yml"><img src="https://github.com/piotrkowski/aristotle/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/licence-AGPL--3.0-blue" alt="Licence: AGPL-3.0"></a>
</p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#documentation">Documentation</a> ·
  <a href="docs/architecture.md">Architecture</a> ·
  <a href=".github/CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <img src="assets/screenshots/readme-light.png#gh-light-mode-only" alt="Aristotle on the Kindle: the library, a page of Bolesław Prus's Lalka, and a dictionary popup">
  <img src="assets/screenshots/readme-dark.png#gh-dark-mode-only" alt="Aristotle on the Kindle in night mode: the library, a page of Bolesław Prus's Lalka, and a dictionary popup">
</p>

Aristotle replaces the whole interface of a jailbroken Kindle Paperwhite 4: the
home screen, the reader, the settings. It is written in Rust with its own UI
toolkit for e-ink, and lays out books with the document engine of
[Plato](https://github.com/baskerville/plato). There is no store, no account
and no network code in the reader; books arrive over USB.

**Status: alpha.** It runs on one device, the Paperwhite 4 (2018) on firmware
5.13.x. The stock system stays installed underneath: Aristotle stops the stock
interface while it runs, and the stock interface comes back when you quit or
reboot. The interface is in English, or Polish (Settings, Language); adding a
language is one JSON file (`docs/ui-framework.md`, "Adding a language").

## Features

- **Clean page turns.** Every turn is one full-screen update with the stock
  reader's own waveform, so pages do not ghost. The next page is rendered while
  the panel refreshes, so a turn costs only the panel's 0.44 s, against 0.57 s for
  the stock reader, which then sends a second update for its footer
  ([measurements](docs/native-vs-aristotle.md)).
- **Fast opening.** A book you have read before opens on the page you left,
  from a snapshot of that page, in one flash; the layout finishes in the
  background.
- **Library.** Everything in `documents/`, with the book to continue at the
  top, shelves for reading, new and finished books, your folders as they are on
  the device, and series in order. Progress from the stock reader and KOReader
  is picked up. EPUB, DRM-free MOBI, PDF and TXT.
- **Typography.** EPUB and MOBI books are set in Literata, justified with
  Knuth–Plass line breaking and Liang hyphenation, including Polish, Ancient
  Greek and Latin. Books that declare
  several languages are hyphenated in the one their text is written in.
- **Dictionaries.** Hold a word to look it up in StarDict dictionaries, for
  example the Wiktionary exports. Greek lookups ignore accents and breathings,
  inflected Greek, Latin and Polish forms find their dictionary form, and a
  selection can be glossed word by word.
- **Footnotes and highlights.** Footnotes open in a popup. Highlights and
  bookmarks are kept per book and exported to `My Clippings.txt` in the stock
  format, so existing tools keep working, and to one Markdown file per book.
- **Night mode.** White on black on every screen, switched with one flash.
- **Settings.** The frontlight on 24 steps that look even, the screensaver
  delay, covers or a plain list, battery, storage, and the USB mode.
- **Offline.** No store, recommendations, accounts or telemetry. With the stock
  interface stopped, available memory goes from 236 MB to 392 MB.

## Before you start

> [!WARNING]
> Aristotle needs a **jailbroken** Kindle with **KUAL** and **MRPI**, and
> **KOReader** installed for its SSH server. Changing a Kindle's software can
> brick it, and **no recovery method has been verified** for the Paperwhite 4,
> which is secure-boot locked. Try it only on a device you can afford to lose.
> Aristotle reads **DRM-free** books only.

Installing the device agent writes one file to the root filesystem
(`/etc/upstart/aristotle-agent.conf`); everything else lives in the user storage and in
`/var/local`. KUAL removes the agent again.

## Install

You build Aristotle on a Linux computer and copy it over the USB cable.

1. **Jailbreak the Kindle** and install KUAL, MRPI and KOReader, following
   [kindlemodding.org](https://kindlemodding.org).
2. **Build.** You need Rust through [rustup](https://rustup.rs) with the
   `armv7-unknown-linux-musleabihf` target, and curl, tar, xz, openssl, xxd,
   make, cmake and a C compiler. The first script fetches a pinned Zig and builds
   Plato's C libraries once.

   ```sh
   git clone --recurse-submodules https://github.com/piotrkowski/aristotle && cd aristotle
   PLATO_LIBS_ONLY=1 tools/build-plato.sh
   tools/build-render.sh device      # $ARISTOTLE_CACHE/render/aristotle-app-device.tar.gz
   ```

3. **Copy it over.** Create the key the device will accept, set up the USB
   network on the computer (NetworkManager), then plug in the Kindle as a USB
   drive and install:

   ```sh
   . devloop/env.sh                     # sets ARISTOTLE_CACHE, the cache directory
   mkdir -p "$ARISTOTLE_CACHE/ssh"
   ssh-keygen -t ed25519 -f "$ARISTOTLE_CACHE/ssh/kindle_ed25519"
   devloop/host/setup-host-net.sh
   devloop/host/install-extensions.sh   # KINDLE_MOUNT if not /run/media/$USER/Kindle
   ```

   This puts the app in `aristotle/` on the Kindle with its licences and
   `SOURCE.txt`, adds two KUAL extensions, and ejects the Kindle.
4. **Start the agent.** On the Kindle, open KUAL › Aristotle › Install agent. From
   then on the Kindle is reachable as `ssh -F devloop/host/ssh_config kindle`
   whenever the cable is in. (Start USB network once does the same until the
   next reboot, without the agent.)
5. **Run Aristotle:**

   ```sh
   ssh -F devloop/host/ssh_config kindle sh /mnt/us/aristotle/run-aristotle.sh
   ```

   To quit, hold the "Library" title for a second. The stock interface comes
   back after its boot splash.

**Uninstall:** KUAL › Aristotle › Remove agent, then delete `aristotle/`,
`.aristotle/` (Aristotle's index, page cache and snapshots) and
`extensions/aristotle-*` from the Kindle's storage. Your books, `My Clippings.txt`
and the exported notes stay.

[Fun use cases](docs/fun-use-cases.md) has ideas for what to do with an open
Kindle: a daily newspaper, web pages, highlights in your notes.

## Development

The pure-Rust crates build and test with `cargo test` on any host, and every
screen renders to PNG without a Kindle:

```sh
cargo run --release -p aristotle-ui --bin aristotle-ui-preview
```

`devloop/emulator/setup.sh` sets up a QEMU Paperwhite 4 running the stock
firmware, for anything that could hang the device. On the device,
`aristotle-observe` runs touch scenarios from `devloop/kindle/scenarios/` against
the stock interface or Aristotle and reports each interaction's panel updates,
waveforms and latency. [CONTRIBUTING.md](.github/CONTRIBUTING.md) has the build
and the checks, and [docs/architecture.md](docs/architecture.md) the crates and
the plan.

## Documentation

- [Architecture](docs/architecture.md): crates, extension points and the plan
- [The reader](docs/reader.md): how Plato's engine is built in, typesetting, opening books, dictionaries, highlights
- [UI framework](docs/ui-framework.md): the toolkit, e-ink rules, night mode
- [Library](docs/library.md): formats, metadata, covers, stock Kindle sidecars and clippings
- [Network](docs/network.md): offline by default, and the scoped Wi-Fi policy
- [E-ink rendering](docs/eink-rendering.md): how the panel driver behaves and how latency is measured
- [Stock Kindle vs Aristotle](docs/native-vs-aristotle.md): the measurements
- [Paperwhite 4 reference](docs/pw4-reference.md): hardware, kernel and stock userspace
- [Related projects](docs/related-projects.md)

## Contributing

Report bugs as [issues](https://github.com/piotrkowski/aristotle/issues), with
the device, firmware and app log. See [CONTRIBUTING.md](.github/CONTRIBUTING.md)
to build Aristotle and run the checks. Report vulnerabilities privately, as the
[security policy](.github/SECURITY.md) describes.

## Licence

GNU AGPL 3.0 or later, the same as Plato. See [LICENSE](LICENSE), and
[THIRD_PARTY.md](THIRD_PARTY.md) for the work of others that Aristotle includes.

## Acknowledgements

Aristotle stands on [Plato](https://github.com/baskerville/plato) by Bastien
Dejean, whose document engine lays out every page, and on MuPDF, FreeType and
HarfBuzz beneath it. [KOReader](https://github.com/koreader/koreader) showed how
to run on a Kindle and provides the SSH server;
[FBInk](https://github.com/NiLuJe/FBInk) documents the Kindle's e-ink interface;
katadelos' [eink-emulator](https://github.com/katadelos/eink-emulator) runs the
Paperwhite 4 in QEMU. The jailbreak, KUAL and MRPI come from the KindleModding
and MobileRead communities. Literata is by TypeTogether, the hyphenation
patterns by the authors collected in [hyph-utf8](https://github.com/hyphenation/tex-hyphen),
and the dictionary in the screenshots is a Wiktionary export
([CC BY-SA](https://creativecommons.org/licenses/by-sa/4.0/)) by
[wiktionary_stardict](https://github.com/xxyzz/wiktionary_stardict).
