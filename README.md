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
  <a href="https://piotrkowski.github.io/aristotle/">Demo</a> ·
  <a href="docs/benchmarks.md">Benchmarks</a> ·
  <a href="#documentation">Documentation</a> ·
  <a href=".github/CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <img src="docs/images/readme-light.png#gh-light-mode-only" alt="Aristotle on a Kindle Paperwhite 4: the library, a page of Aristotle's Nicomachean Ethics in English, and the dictionary popup for the Greek word ἀγαθοῦ">
  <img src="docs/images/readme-dark.png#gh-dark-mode-only" alt="Aristotle on a Kindle Paperwhite 4 in night mode: the library, a page of Aristotle's Nicomachean Ethics in English, and the dictionary popup for the Greek word ἀγαθοῦ">
</p>

Aristotle replaces the Kindle's stock Java interface. While it runs, the store, the web browser and 12 background services for Amazon's cloud and telemetry are stopped. Quitting Aristotle brings the stock interface back. Aristotle connects only to the sync server and catalogs you add, and books can also be copied over USB. An older Paperwhite still has a good screen and battery, and this keeps it useful instead of buying a new e-reader. Less runs in the background, much less memory is used, and the Kindle feels like it is actually yours.

The Paperwhite 4 has 512 MB of RAM, of which 490 MB is usable.

| Memory | Stock | Aristotle |
|---|--:|--:|
| RAM used by the interface | about 200 MB | **14.5 MB** |
| RAM free while reading | about 240 MB | **about 380 MB** |

## Features

- Pages turn with one full-screen update and the stock reader's waveform, so they don't ghost.
- The library shows `documents/` with shelves, folders, series and the current book. It opens EPUB, DRM-free MOBI, PDF and TXT.
- Text is set in Literata with Knuth–Plass justification. Hyphenation covers English, Polish, Ancient Greek and Latin.
- Hold a word to look it up in offline StarDict dictionaries. Inflected forms work too.
- It continues where the stock reader left off ([without changing its files](docs/library.md#stock-files-sidecars-and-my-clippingstxt)) and adds highlights to `My Clippings.txt` and Markdown. Footnotes open in a popup.
- Night mode, and an English or Polish interface.
- Reading positions sync with KOReader through a KOSync server (EPUB chapters, PDF pages).
- Books download from OPDS catalogs, such as calibre's content server, into the library.

| Speed | Stock | Aristotle |
|---|--:|--:|
| Page turn, from release | 0.57 s | 0.44 s |
| Open a book (Aristotle: from its saved page) | 0.8 s | 0.56 s |

The speed, size and memory numbers here come from [docs/benchmarks.md](docs/benchmarks.md), which describes the method.

## Install

> [!WARNING]
> Changing a Kindle's software can **brick** it, and no recovery method has been verified for the Paperwhite 4. Aristotle is alpha software.

You need:

- a Kindle Paperwhite 4 (2018) on firmware 5.13.x, jailbroken;
- KUAL, the launcher that holds Aristotle's menu;
- a computer and a USB cable.

Then:

1. Download `aristotle-X.Y.Z.zip` from the latest [release](https://github.com/piotrkowski/aristotle/releases).
2. Unzip it to the root of the Kindle's USB drive and eject the Kindle.
3. On the Kindle, open KUAL › Aristotle › Start Aristotle.

**[docs/install.md](docs/install.md)** covers starting at boot, the optional agent, updates, uninstalling and what to do if something goes wrong. To build Aristotle yourself, see [CONTRIBUTING.md](.github/CONTRIBUTING.md#building).

## Documentation

- [Project post and interactive demo](https://piotrkowski.github.io/aristotle/)
- Design notes on the [architecture](docs/architecture.md) (with related projects), [reader](docs/reader.md), [UI framework](docs/ui-framework.md) (with e-ink rendering) and [library](docs/library.md)
- [Benchmarks](docs/benchmarks.md), the source of every number in this README
- [Installing](docs/install.md)
- [Device-side code](device/README.md): the KUAL extension, the agent, the network policy, the kill switch and uninstalling
- [Paperwhite 4 reference](docs/pw4-reference.md)

## Contributing

Report bugs as [issues](https://github.com/piotrkowski/aristotle/issues) with the device, firmware version and app log. [CONTRIBUTING.md](.github/CONTRIBUTING.md) explains the build and the checks. Report vulnerabilities privately through the [security policy](.github/SECURITY.md).

## Licence

GNU AGPL 3.0 or later, like Plato. See [LICENSE](LICENSE). [THIRD_PARTY.md](THIRD_PARTY.md) lists the third-party work it includes.

## Acknowledgements

- [Plato](https://github.com/baskerville/plato) by Bastien Dejean, with [MuPDF](https://mupdf.com), [FreeType](https://freetype.org) and [HarfBuzz](https://harfbuzz.github.io)
- [KOReader](https://github.com/koreader/koreader), for Kindle know-how and the SSH server
- [FBInk](https://github.com/NiLuJe/FBInk), for the e-ink display interface
- katadelos' [eink-emulator](https://github.com/katadelos/eink-emulator), the Paperwhite 4 in QEMU
- [KindleModding](https://kindlemodding.org) and [MobileRead](https://www.mobileread.com), for KUAL, MRPI and the Kindle guides
- [Literata](https://www.type-together.com/literata-font) by TypeTogether
- [hyph-utf8](https://github.com/hyphenation/tex-hyphen) hyphenation patterns
- Wiktionary dictionaries via [wiktionary_stardict](https://github.com/xxyzz/wiktionary_stardict) ([CC BY-SA](https://creativecommons.org/licenses/by-sa/4.0/))
- Sample books from [Project Gutenberg](https://www.gutenberg.org)

Aristotle contains no exploit or DRM-removal code, and it opens only DRM-free books. It uses the e-ink waveforms and firmware already on the device and never ships them or copies them off it.

Aristotle is not affiliated with, endorsed or sponsored by Amazon. Kindle, Paperwhite and related marks are trademarks of Amazon.com, Inc. or its affiliates.
