<h1 align="center">Aristotle</h1>

<p align="center">
  A new interface for the Kindle Paperwhite 4: a library and a reader that turn pages cleanly and work offline.
</p>

<p align="center">
  <a href="https://github.com/piotrkowski/aristotle/actions/workflows/ci.yml"><img src="https://github.com/piotrkowski/aristotle/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/licence-AGPL--3.0-blue" alt="Licence: AGPL-3.0"></a>
</p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="https://piotrkowski.github.io/aristotle/guides/">Guides</a> ·
  <a href="https://piotrkowski.github.io/aristotle/benchmarks.html">Benchmarks</a> ·
  <a href="#documentation">Documentation</a> ·
  <a href=".github/CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <img src="assets/screenshots/readme-light.png#gh-light-mode-only" alt="Aristotle on a Kindle Paperwhite 4: the library, a page of Aristotle's Nicomachean Ethics in English, and the dictionary popup for the Greek word ἀγαθοῦ">
  <img src="assets/screenshots/readme-dark.png#gh-dark-mode-only" alt="Aristotle on a Kindle Paperwhite 4 in night mode: the library, a page of Aristotle's Nicomachean Ethics in English, and the dictionary popup for the Greek word ἀγαθοῦ">
</p>

## Features

- **Clean page turns:** one full-screen update with the stock reader's own waveform, so pages do not ghost.
- **Library:** everything in `documents/`, with the book to continue, shelves, your folders and series. EPUB, DRM-free MOBI, PDF and TXT.
- **Typography:** Literata, Knuth–Plass justification and hyphenation for English, Polish, Ancient Greek and Latin.
- **Dictionaries:** hold a word to look it up offline in StarDict dictionaries; inflected forms find their dictionary form.
- **Highlights and footnotes:** highlights go to `My Clippings.txt` in the stock format and to Markdown; footnotes open in a popup.
- **Night mode** on every screen, and the interface in English or Polish.

| Measured on a Paperwhite 4 | Stock | Aristotle |
| --- | ---: | ---: |
| Page turn, from release | 0.57 s | 0.44 s |
| Open a book | 0.8 s | 0.56 s |
| Memory used by the interface | about 200 MB | 14.5 MB |

How these were measured: [benchmarks](https://piotrkowski.github.io/aristotle/benchmarks.html) and [docs/native-vs-aristotle.md](docs/native-vs-aristotle.md).

## Upcycle, don't replace

An older Kindle's screen and battery outlast its software. Aristotle gives a Paperwhite 4 a second life instead of a new e-reader: no store, no account, no telemetry. Books arrive over USB.

> [!WARNING]
> Aristotle needs a **jailbroken** Kindle. Changing a Kindle's software can **brick** it, and no recovery method has been verified for the Paperwhite 4.

Aristotle contains no jailbreak, exploit or DRM-removal code. It runs only on a Kindle you have already jailbroken with the community's tools, and it opens DRM-free books only. It uses the e-ink waveforms and firmware already on your device; it never ships, copies off or uploads them.

## Install

Aristotle is alpha and runs on one device:

| Device | Firmware | Status |
| --- | --- | --- |
| Kindle Paperwhite 4 (2018, PQ94WIF) | 5.13.x | Tested |
| Anything else | | Not supported: the install scripts refuse to run (developers can override with `ARISTOTLE_FORCE=1`) |

The Kindle needs a jailbreak, KUAL, MRPI and KOReader (for its SSH server); you build Aristotle on a Linux computer and copy it over the USB cable.
The [install guide](https://piotrkowski.github.io/aristotle/guides/install.html) has every step, and how to uninstall.
The stock interface stays installed: Aristotle stops it while it runs, and it comes back when you quit or reboot.
Reading positions and highlights carry over between the stock reader and Aristotle, through the stock sidecar files and `My Clippings.txt` ([docs/library.md](docs/library.md)).

Uninstalling removes everything the agent put on the device, including its SSH key.
There is also a way out without SSH: in KUAL choose Aristotle › USB drive mode for 10 minutes, connect the Kindle to a computer, create an empty file named `aristotle-disable` at its root, and restart it. The agent then does not start, and the stock system takes over.

## Documentation

- [Guides](https://piotrkowski.github.io/aristotle/guides/): install, books, dictionaries, languages, development
- [Benchmarks](https://piotrkowski.github.io/aristotle/benchmarks.html): every number and how it was measured
- [Architecture](docs/architecture.md), [the reader](docs/reader.md), [UI framework](docs/ui-framework.md), [library](docs/library.md), [network](docs/network.md), [e-ink rendering](docs/eink-rendering.md)
- [Paperwhite 4 reference](docs/pw4-reference.md) and [related projects](docs/related-projects.md)

## Contributing

Report bugs as [issues](https://github.com/piotrkowski/aristotle/issues), with the device, firmware and app log. See [CONTRIBUTING.md](.github/CONTRIBUTING.md) to build Aristotle and run the checks. Report vulnerabilities privately, as the [security policy](.github/SECURITY.md) describes.

## Licence

GNU AGPL 3.0 or later, the same as Plato. See [LICENSE](LICENSE), and [THIRD_PARTY.md](THIRD_PARTY.md) for the work of others that Aristotle includes.

## Acknowledgements

- [Plato](https://github.com/baskerville/plato) by Bastien Dejean: the document engine that lays out every page.
- [MuPDF](https://mupdf.com), [FreeType](https://freetype.org) and [HarfBuzz](https://harfbuzz.github.io), beneath it.
- [KOReader](https://github.com/koreader/koreader): how to run on a Kindle, and the SSH server.
- [FBInk](https://github.com/NiLuJe/FBInk): the Kindle's e-ink interface.
- katadelos' [eink-emulator](https://github.com/katadelos/eink-emulator): the Paperwhite 4 in QEMU.
- [KindleModding](https://kindlemodding.org) and [MobileRead](https://www.mobileread.com): the jailbreak, KUAL and MRPI.
- [Literata](https://www.type-together.com/literata-font) by TypeTogether: the reading font.
- [hyph-utf8](https://github.com/hyphenation/tex-hyphen): the hyphenation patterns.
- Wiktionary, through [wiktionary_stardict](https://github.com/xxyzz/wiktionary_stardict) ([CC BY-SA](https://creativecommons.org/licenses/by-sa/4.0/)): the dictionaries.
- [Project Gutenberg](https://www.gutenberg.org): the sample books.

Aristotle is an independent project, not affiliated with, endorsed or sponsored by Amazon. Kindle, Paperwhite and all related marks are trademarks of Amazon.com, Inc. or its affiliates.
