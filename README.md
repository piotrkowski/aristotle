<h1 align="center">Aristotle</h1>

<p align="center">
  A new library and reader for the Kindle Paperwhite 4.
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
  <img src="docs/images/readme-light.png#gh-light-mode-only" alt="Aristotle on a Kindle Paperwhite 4: the library, a page of Aristotle's Nicomachean Ethics in English, and the dictionary popup for the Greek word ἀγαθοῦ">
  <img src="docs/images/readme-dark.png#gh-dark-mode-only" alt="Aristotle on a Kindle Paperwhite 4 in night mode: the library, a page of Aristotle's Nicomachean Ethics in English, and the dictionary popup for the Greek word ἀγαθοῦ">
</p>

Aristotle replaces the Kindle's stock Java interface. While it runs, the store, the web browser and 12 background services for Amazon's cloud and telemetry are stopped, and a reboot brings them back. Aristotle makes no network connections, and books are copied over USB. An older Paperwhite still has a good screen and battery, and this keeps it useful instead of buying a new e-reader.

The Paperwhite 4 has 512 MB of RAM, of which 490 MB is usable.

| Size and memory | Stock | Aristotle |
|---|--:|--:|
| Interface | Java framework and WebKit | one Rust binary, 9.6 MB |
| RAM used by the interface | about 200 MB | **14.5 MB** |
| RAM free while reading | about 240 MB | **about 380 MB** |

## Features

- Pages turn with one full-screen update and the stock reader's waveform, so they don't ghost.
- The library shows `documents/` with shelves, folders, series and the current book. It opens EPUB, DRM-free MOBI, PDF and TXT.
- Text is set in Literata with Knuth–Plass justification. Hyphenation covers English, Polish, Ancient Greek and Latin.
- Hold a word to look it up in offline StarDict dictionaries. Inflected forms work too.
- Highlights go to `My Clippings.txt` in the stock format and to Markdown. Footnotes open in a popup.
- Night mode, and an English or Polish interface.

| Speed | Stock | Aristotle |
|---|--:|--:|
| Page turn, from release | 0.57 s | 0.44 s |
| Open a book (Aristotle: from its saved page) | 0.8 s | 0.56 s |

See [benchmarks](https://piotrkowski.github.io/aristotle/benchmarks.html) and [docs/native-vs-aristotle.md](docs/native-vs-aristotle.md) for the method.

## Install

> [!WARNING]
> Aristotle needs a **jailbroken** Kindle. Changing a Kindle's software can **brick** it, and no recovery method has been verified for the Paperwhite 4.

Aristotle contains no jailbreak, exploit or DRM-removal code, and it opens only DRM-free books. It uses the e-ink waveforms and firmware already on the device and never ships them or copies them off the device.

Aristotle is alpha software.

| Device | Firmware | Status |
|---|---|---|
| Kindle Paperwhite 4 (2018, PQ94WIF) | 5.13.x | Tested |
| Other devices | | Not supported. The install scripts refuse to run unless `ARISTOTLE_FORCE=1` is set. |

The Kindle also needs KUAL, MRPI and KOReader (for its SSH server). The [install guide](https://piotrkowski.github.io/aristotle/guides/install.html) covers building on Linux, copying over USB and uninstalling.

The stock interface stays installed and comes back when you quit Aristotle or reboot. Aristotle picks up where you left off in the stock reader, reading its sidecar files, and adds its highlights to `My Clippings.txt`. It does not write the stock sidecars ([docs/library.md](docs/library.md#stock-files-sidecars-and-my-clippingstxt)).

Uninstalling removes everything Aristotle installed, including its SSH key. To disable Aristotle without SSH, choose KUAL › Aristotle › USB drive mode for 10 minutes, create an empty file named `aristotle-disable` at the root of the drive and restart the Kindle. The stock system then starts instead.

## Documentation

- [Guides](https://piotrkowski.github.io/aristotle/guides/) and [benchmarks](https://piotrkowski.github.io/aristotle/benchmarks.html)
- Design notes on the [architecture](docs/architecture.md), [reader](docs/reader.md), [UI framework](docs/ui-framework.md), [library](docs/library.md), [network](docs/network.md) and [e-ink rendering](docs/eink-rendering.md)
- [Paperwhite 4 reference](docs/pw4-reference.md) and [related projects](docs/related-projects.md)

## Contributing

Report bugs as [issues](https://github.com/piotrkowski/aristotle/issues) with the device, firmware version and app log. [CONTRIBUTING.md](.github/CONTRIBUTING.md) explains the build and the checks. Report vulnerabilities privately through the [security policy](.github/SECURITY.md).

## Licence

GNU AGPL 3.0 or later, like Plato. See [LICENSE](LICENSE). [THIRD_PARTY.md](THIRD_PARTY.md) lists the third-party work it includes.

## Acknowledgements

- [Plato](https://github.com/baskerville/plato) by Bastien Dejean, with [MuPDF](https://mupdf.com), [FreeType](https://freetype.org) and [HarfBuzz](https://harfbuzz.github.io)
- [KOReader](https://github.com/koreader/koreader), for Kindle know-how and the SSH server
- [FBInk](https://github.com/NiLuJe/FBInk), for the e-ink display interface
- katadelos' [eink-emulator](https://github.com/katadelos/eink-emulator), the Paperwhite 4 in QEMU
- [KindleModding](https://kindlemodding.org) and [MobileRead](https://www.mobileread.com), for the jailbreak, KUAL and MRPI
- [Literata](https://www.type-together.com/literata-font) by TypeTogether
- [hyph-utf8](https://github.com/hyphenation/tex-hyphen) hyphenation patterns
- Wiktionary dictionaries via [wiktionary_stardict](https://github.com/xxyzz/wiktionary_stardict) ([CC BY-SA](https://creativecommons.org/licenses/by-sa/4.0/))
- Sample books from [Project Gutenberg](https://www.gutenberg.org)

Aristotle is not affiliated with, endorsed or sponsored by Amazon. Kindle, Paperwhite and related marks are trademarks of Amazon.com, Inc. or its affiliates.
