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

<p align="center"><a href="docs/screenshots.md">More screenshots</a></p>

Aristotle replaces the Kindle's stock Java interface. While it runs, the store, the web browser and 12 background services for Amazon's cloud and telemetry are stopped. Quitting Aristotle brings the stock interface back. Aristotle connects only to the sync server and catalogs you add, and books can also be copied over USB. An older Paperwhite still has a good screen and battery, and this keeps it useful instead of buying a new e-reader. Less runs in the background, much less memory is used, and the Kindle feels like it is actually yours.

The Paperwhite 4 has 512 MB of RAM, of which 490 MB is usable.

| Memory | Stock | Aristotle |
|---|--:|--:|
| RAM used by the interface | about 200 MB | **14.5 MB** |
| RAM free while reading | about 240 MB | **about 380 MB** |

## Features

- Pages turn with one full-screen update and the stock reader's [waveform](docs/glossary.md#waveform), so they don't ghost.
- The library shows `documents/` with shelves, folders, series and the current book. It opens [EPUB](docs/glossary.md#epub), DRM-free [MOBI](docs/glossary.md#mobi), PDF and TXT.
- Text is set in Literata with [Knuth-Plass](docs/glossary.md#knuth-plass) justification. Hyphenation covers English, Polish, Ancient Greek and Latin.
- Add your own fonts: copy TTF or OTF files into `fonts/` on the Kindle and pick one in the reader's menu ([how](docs/install.md#your-own-fonts)).
- Hold a word to look it up in offline [StarDict](docs/glossary.md#stardict) dictionaries. Inflected forms work too.
- On the first scan, it imports stock MOBI reading positions [without changing stock files](docs/library.md#stock-files-sidecars-and-my-clippingstxt). New highlights go to [`My Clippings.txt`](docs/glossary.md#my-clippingstxt) and Markdown. Footnotes open in a popup.
- Night mode, and an English or Polish interface.
- Reading positions sync with [KOReader](docs/glossary.md#koreader) through a [KOSync](docs/glossary.md#kosync) server (EPUB chapters, PDF pages).
- Books download from [OPDS](docs/glossary.md#opds) catalogs, such as calibre's content server, into the library.

| Speed | Stock | Aristotle |
|---|--:|--:|
| Page turn, from release | 0.57 s | 0.44 s |
| Open a book (Aristotle: from its saved page) | 0.8 s | 0.56 s |

The speed, size and memory numbers here come from [docs/benchmarks.md](docs/benchmarks.md), which describes the method.

## Install

I'd recommend installing it with an AI agent like Claude Code or Codex. Just point it at [AGENTS.md](AGENTS.md). And if you run into bugs, ghosting or flickering along the way, I'm happy to get an issue or a PR ;)

> [!WARNING]
> Changing a Kindle's software can **brick** it, and no recovery method has been verified for the Paperwhite 4. Aristotle is alpha software.
>
> The default screensaver does not suspend the CPU, and the power button has no effect while Aristotle runs. Battery use is unverified. Quit to the stock interface when you finish reading.

You need:

- a Kindle Paperwhite 4 (2018) on firmware 5.13.x, jailbroken;
- [KUAL](docs/glossary.md#kual), the launcher that holds Aristotle's menu;
- a computer and a USB cable.

Then:

1. Download `aristotle-X.Y.Z.zip` from the latest [release](https://github.com/piotrkowski/aristotle/releases).
2. Unzip it to the root of the Kindle's USB drive and eject the Kindle.
3. On the Kindle, open KUAL › Aristotle › Start Aristotle.

[Install](docs/install.md) covers the optional agent and starting at boot. Then follow [Your first book](docs/first-book.md) to read and export a highlight. To build Aristotle yourself, see [CONTRIBUTING.md](.github/CONTRIBUTING.md#building).

## Documentation

| I want to... | Read |
| --- | --- |
| Start reading | [Your first book](docs/first-book.md) |
| Add word lookups | [Install dictionaries](docs/dictionaries.md) |
| Sync with KOReader or download from calibre | [Set up network features](docs/network.md) |
| Preserve my data or install a new version | [Back up and update](docs/maintenance.md) |
| Fix a problem or collect logs | [Troubleshooting](docs/troubleshooting.md) |
| Check whether a book will work | [Format support](docs/formats.md) |

The [documentation index](docs/README.md) also links the architecture, reader, library, UI framework, and device references. The [glossary](docs/glossary.md) explains technical terms. [Benchmarks](docs/benchmarks.md) describe the measurements behind the numbers above. [Screenshots](docs/screenshots.md) and the [interactive demo](https://piotrkowski.github.io/aristotle/) show the interface.

## Contributing

Report bugs as [issues](https://github.com/piotrkowski/aristotle/issues) with the details in [Report a bug](docs/troubleshooting.md#report-a-bug). [CONTRIBUTING.md](.github/CONTRIBUTING.md) explains the build and the checks. Report vulnerabilities privately through the [security policy](.github/SECURITY.md).

## Licence

GNU AGPL 3.0 or later, like Plato. See [LICENSE](LICENSE). [THIRD_PARTY.md](THIRD_PARTY.md) lists the third-party work it includes.

## Acknowledgements

- [Plato](https://github.com/baskerville/plato) by Bastien Dejean, with [MuPDF](https://mupdf.com), [FreeType](https://freetype.org) and [HarfBuzz](https://harfbuzz.github.io)
- [KOReader](https://github.com/koreader/koreader), for Kindle know-how and the SSH server
- [FBInk](https://github.com/NiLuJe/FBInk), for the e-ink display interface
- katadelos' [eink-emulator](https://github.com/katadelos/eink-emulator), the Paperwhite 4 in [QEMU](docs/glossary.md#qemu)
- [KindleModding](https://kindlemodding.org) and [MobileRead](https://www.mobileread.com), for KUAL, [MRPI](docs/glossary.md#mrpi) and the Kindle guides
- [Literata](https://www.type-together.com/literata-font) by TypeTogether
- [hyph-utf8](https://github.com/hyphenation/tex-hyphen) hyphenation patterns
- Wiktionary dictionaries via [wiktionary_stardict](https://github.com/xxyzz/wiktionary_stardict) ([CC BY-SA](https://creativecommons.org/licenses/by-sa/4.0/))
- Sample books from [Project Gutenberg](https://www.gutenberg.org)

Aristotle contains no exploit or DRM-removal code, and it opens only DRM-free books. It uses the e-ink waveforms and firmware already on the device and never ships them or copies them off it.

Aristotle is not affiliated with, endorsed or sponsored by Amazon. Kindle, Paperwhite and related marks are trademarks of Amazon.com, Inc. or its affiliates.
