# Changelog

All notable changes to Aristotle are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Each release is a
tag `vX.Y.Z` whose version matches `Cargo.toml`, and its GitHub release notes
are its section below.

## [Unreleased]

### Security

- Update `rustls-webpki` to 0.103.16 and `webpki-roots` to 1.0.10 to address RUSTSEC-2026-0382 and RUSTSEC-2026-0381.

### Fixed

- Stop idle polling outside the library, use worker notifications, and honor relative hold deadlines.
- Keep idle clocks from refreshing the panel and avoid forced flashes when waking Last screen.
- Save fewer reading checkpoints and make release timing logs opt-in with `ARISTOTLE_PERF=1`.
- Use a reversible `ondemand` governor during takeover and reduce the agent's USB maintenance work.
- Ignore touch during managed sleep and fail startup if managed sleep cannot be acquired.

### Changed

- New preferences use a 10-minute sleep delay. Existing preferences stay unchanged.

## [0.1.0] - 2026-10-17

The first release, for the Kindle Paperwhite 4 (2018) on firmware 5.13.x.
It is alpha software, tested on one device.

### Added

- A library and reader that run in place of the stock interface. While it
  runs, the stock interface, the store and the cloud and telemetry services
  are stopped; quitting or a reboot brings them back.
- The library lists `documents/` with *Books*, *Folders* and *Series* views,
  shelves and a *Continue reading* card. It reads the reading position the
  stock reader saved for each book.
- The reader opens EPUB, DRM-free MOBI and AZW3, PDF and TXT, using Plato's
  document engine. Pages turn with one full-screen update, as in the stock
  reader.
- Text is set in Literata with Knuth-Plass justification, with hyphenation
  for English, Polish, Ancient Greek and Latin.
- Your own reading fonts: TTF or OTF files in `fonts/` on the USB drive show
  up in the reader's menu.
- Hold a word to look it up in offline StarDict dictionaries, inflected forms
  included. Footnotes open in a popup.
- Highlights and bookmarks. Highlights are added to `My Clippings.txt` in the
  stock format and written as one Markdown note per book.
- *Settings* for the frontlight, night mode, covers, the screensaver delay, the
  USB mode and the language (English or Polish), and an *About* section with
  the version.
- *Start Aristotle* and *Return to the Kindle interface* in KUAL, and,
  with the agent, *Start Aristotle at boot*, which starts it instead of the
  stock interface and falls back to stock after three failed launches.
- Reading-position sync with KOReader through a KOSync server, and downloads
  from OPDS catalogs such as calibre's content server. Passwords stay in a
  root-only folder on the device.
- A KUAL extension with an optional agent that keeps a USB network and SSH
  up for development, switches the USB drive mode on request, and restarts
  the stock interface if it stays down. A file named `aristotle-disable` at
  the root of the USB drive keeps the agent from starting. While installed,
  the agent keeps Amazon's update daemons from running.
- An optional Wi-Fi policy that lets the device reach only the hosts you
  list.
- The scripts that change the device refuse to run on anything but a
  Paperwhite 4 on firmware 5.13.x.
- Release downloads: one zip to unpack onto the Kindle, a source tarball and
  SHA-256 sums.

[Unreleased]: https://github.com/piotrkowski/aristotle/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/piotrkowski/aristotle/releases/tag/v0.1.0
