# The reader

`crates/aristotle-render` turns books into 8-bit gray pages through [Plato](https://github.com/baskerville/plato) and MuPDF. It uses none of Plato's Kobo UI. Its CLI renders files, shows pages on the device, and measures layout and panel time.

`crates/aristotle` combines the renderer, library, UI, and `aristotle-hal` hardware access into the app. It opens EPUB, DRM-free MOBI 7, PDF, and TXT. Measurements are from the host unless marked [dev]. See [Benchmarks](benchmarks.md#device-measurements-of-the-app) for results.

## Architecture

| Code | Role |
| --- | --- |
| `crates/aristotle-render/src/book.rs` | Opens books, chooses an engine, lays out pages, and reads words and links |
| `style.rs`, `page.rs` | Page geometry, pixels, header, and footer |
| `mobi.rs`, `position.rs` | MOBI conversion and saved positions |
| `notes.rs`, `richtext.rs` | Footnotes and styled popup text |
| `crates/aristotle/src/main.rs` | Startup, event loop, screens, and actions |
| `panel.rs` | Hardware updates, night mode, and menu overlays |
| `session.rs`, `turns.rs` | Open book, neighboring pages, restyle, and gestures |
| `open.rs`, `snapshot.rs`, `prewarm.rs`, `pagecache.rs` | Background opening, saved pages, and pagination |
| `shelf.rs`, `progress.rs`, `prefs.rs` | Library views, saved progress, and preferences |
| `reading.rs`, `overlay.rs`, `selection.rs`, `marks.rs` | Reading interactions and annotations |
| `dict/`, `speed.rs` | Dictionaries and reading-speed estimate |
| `preview.rs` | Host preview on an in-memory panel |

Short paths in each group are relative to the preceding crate's `src/` directory.

### Threads

The main thread handles input and draws frames. Workers keep other work off that path.

| Thread | Work |
| --- | --- |
| `open` | Opens a book and renders its page and neighbors |
| `paginate`, `prewarm`, `warm-up` | Page numbers, recent-book snapshots, and hyphenation patterns at idle priority |
| `index-saver` | Writes `library.json` |
| `snapshot-writer` | Compresses and writes saved pages at nice 10 |
| `dict`, `dict-index` | Dictionary lookup and idle-priority index builds |
| `page-completions`, `panel-sequence` | Page completion tracking and menu update sequences |

Idle priority means `SCHED_IDLE` plus nice 19. `pagecache::lower_priority` uses a direct system call because musl stubs `sched_setscheduler`.

Plato's font and page objects cannot cross threads. `Session::prepare` returns data without them. `OPEN_GATE` serializes background opens so an abandoned book is dropped before the next layout starts. Partial UI updates use the HAL display thread ([UI model](ui-framework.md#model)).

## How Plato is consumed

The renderer uses `plato-core` from the `third_party/plato` submodule. Both reader binaries change into their data directory because Plato uses relative paths.

### Plato fork

The fork is [piotrkowski/plato](https://github.com/piotrkowski/plato), branch `aristotle`. Its separate commits add static musl linking, library-path overrides, optional DjVu, smaller font and ZIP support, language selection, lazy EPUB layout, and page-break fixes.

Keep one described commit per change in the fork, then commit the submodule update here. Cargo uses the submodule's working tree. Changes to `thirdparty/` or `mupdf_wrapper/` need a fresh `PLATO_BUILD` and a C-library rebuild. Rust-only changes do not.

Before rebasing the fork, tag its current tip so older Aristotle commits remain buildable. Compare tests and rendered pages after the rebase.

### Runtime data

The data directory is `data/` next to the binary, or the path given by `--data`.

| Path | Contents |
| --- | --- |
| `fonts/` | Plato's required fonts and fallback glyphs |
| `serif/` | Literata from `crates/aristotle-render/assets/fonts/` |
| `css/` | `epub.css` and `html.css` |
| `hyphenation-patterns/` | Polish, US and UK English, Ancient Greek, and Latin patterns |

Downloads are pinned by SHA-256. The CLI stores converted MOBI books in `cache/` next to the binary, or `--cache`. `tools/font-koronis.py` substitutes the apostrophe glyph for Greek U+1FBD in Literata and Noto Sans to avoid an oversized elision mark.

## Building and testing

The host build uses the device's library versions, compiled for x86_64 musl.

```sh
. tools/env.sh
PLATO_TARGET=x86_64-unknown-linux-musl PLATO_LIBS_ONLY=1 tools/build-plato.sh
CARGO_CMD=test tools/build-render.sh host
tools/build-render.sh host
"$ARISTOTLE_CACHE/render/host/aristotle-render" render BOOK.epub 10 "$ARISTOTLE_CACHE/page.raw"
"$ARISTOTLE_CACHE/venv/bin/python" -I tools/host/fb2png.py "$ARISTOTLE_CACHE/page.raw" "$ARISTOTLE_CACHE/page.png"
```

`tests/navigation.rs` checks navigation, MOBI positions, and lazy layout against full pagination. `aristotle-render pages BOOK 20` prints page starts and placements for layout comparisons. `aristotle --library DIR --bench-open TITLE` measures opening without hardware.

### Preview and device scenario

```sh
. tools/env.sh
tools/host/make-reading-test-epub.py "$ARISTOTLE_CACHE/proba-czytania.epub"
"$ARISTOTLE_CACHE/render/host/aristotle" \
	--data "$ARISTOTLE_CACHE/render/host/data" \
	--library "$ARISTOTLE_CACHE/reading-library" \
	--preview "$ARISTOTLE_CACHE/ui-preview/reading" \
	"$ARISTOTLE_CACHE/proba-czytania.epub"
```

The test book contains original Polish text, Greek and Latin words, and footnotes. The preview writes step PNGs, `updates.txt`, test clippings, and a device scenario in `reading.txt`. Regenerate scenario coordinates after typography or language changes. `tools/scenarios/aristotle-open.txt` covers opening, turning, the menu, and cancellation.

### Running it on the device

```sh
. tools/env.sh
PLATO_LIBS_ONLY=1 tools/build-plato.sh
tools/build-render.sh device
ssh -F tools/host/ssh_config kindle 'tar -xzf - -C /mnt/us' < "$ARISTOTLE_CACHE/render/aristotle-device.tar.gz"
ssh -F tools/host/ssh_config kindle sh /mnt/us/aristotle/run-aristotle.sh
```

The bundle lives in `/mnt/us/aristotle/`; `/var/tmp` is a 64 MB tmpfs [dev]. The launcher stops the stock GUI and restores it on exit, with 30 to 60 seconds of boot splash [dev]. It accepts only a PW4 on 5.13.x unless `ARISTOTLE_FORCE=1`.

Hold the **Library** title for one second to quit, or run `killall aristotle`. Logs go to `/var/tmp/aristotle.log`. The render CLI's `show` and `turn` commands refuse to run while `cvm`, the stock framework, is active.

## Typesetting

Defaults come from `Style::pw4`.

| Setting | Value |
| --- | --- |
| Font | Literata, 10 pt |
| Line height | 1.3 em |
| Alignment | Justified |
| Side margins | 5 mm, 59 px |
| Text block | 104 px from the top, 100 px from the bottom |
| Header | Chapter or book title, 7 pt italic, gray `0x40` |
| Footer | Chapter progress, page count when known, and percentage, 7 pt |

The book's CSS applies. The menu offers seven sizes from 8 to 15 pt, line heights of 1.2, 1.3, or 1.5 em, and margins of 3, 5, or 8 mm. Constants are `FONT_PT`, `LINE_HEIGHT`, and `MARGIN_MM` in `prefs.rs`.

### Hyphenation

Plato uses language patterns, including Polish with minima of 2 letters on either side of a break. The book language comes from `dc:language`, then the first HTML element's `xml:lang`, with English as fallback. Elements below `body` can override it with `lang` or `xml:lang`; `lang` on `html` or `body` is ignored. Greek words in a non-Greek language use the Ancient Greek patterns. Other text without matching patterns is not hyphenated.

For books declaring several languages, `lang.rs` samples 192 KiB of text and compares marker letters. `aristotle-render --lang TAG` overrides the result. `aristotle-render info BOOK` reports the language and pattern availability.

## Formats and positions

`mobi.rs` converts DRM-free MOBI 7 to cached EPUB for Plato's layout. It handles PalmDOC, UTF-8, CP1252, images, page breaks, and `filepos` links. HUFF/CDIC and KF8-only files fall back to MuPDF. `--no-convert` forces MuPDF. Encrypted books are not readable.

Plato locations are byte offsets in book text. MuPDF locations are page indices. Saved EPUB positions use a spine item and offset. Converted MOBI uses Kindle byte positions and a `.posmap` beside the cached EPUB. Save page starts with `PosMap::position_at_or_after` to avoid rounding back a page.

Page numbers require full pagination. Until `set_pagination` supplies them, the footer omits them. Changes that move page breaks must bump `LAYOUT_VERSION` in `pagecache.rs`, invalidating page caches and snapshots.

## Opening a book

### Snapshots

At each progress save, the current and next pages go to `snapshots/<book id>.snap`, compressed with zstd. A snapshot includes positions and menu labels. Its key checks the book, file size and mtime, style, layout version, app build, runtime data, UI language, and progress. A snapshot saved before a language change is not shown.

The main thread copies pixels. `snapshot-writer` compresses and writes them, keeping slow filesystem syncs off the input path. Scans remove snapshots for missing books.

### The open path

1. Touch-down highlights the card with A2. On release, a valid snapshot appears with one mode-8 flash.
2. The `open` worker opens the book at that position and renders neighboring pages.
3. While it works, the snapshot supports page turns and the menu. Returning to the library cancels the open. Word selection and links require the book.
4. The app keeps matching snapshot pixels. A header-only difference waits until the next turn. A page difference causes one full mode-9 correction.

Without a snapshot, the library stays responsive while the worker opens the book. After 2 seconds of library inactivity, `prewarm` makes missing snapshots for the five most recent books. Touch input cancels that work between steps.

### Lazy layout

The Plato fork stops chapter layout after two complete pages beyond the requested position. It resumes eight pages ahead when needed, without changing page breaks. Going back to a previous spine item lays that item out in full.

For changes that must preserve layout, compare `aristotle-render pages BOOK 20` and rendered pages byte for byte. Include books with nested blocks, lists, tables, and a single long chapter.

## Page turns and waveforms

A page turn is one full-screen, full-mode update in mode 9, matching the stock reader [dev]. Partial updates and A2 followed by GL16 left visible ghosts during real reading ([Experiment](benchmarks.md#page-turn-strategy-experiment)). `--turn-mode 7` selects faster DU4 with fewer grays. The CLI retains `--a2-preview` for experiments.

Taps turn on release. The top 12 % opens the menu, the left 30 % goes back, and the rest goes forward. Horizontal swipes turn pages. A hold selects text without turning.

After submission, the app renders neighboring pages while the panel refreshes. Before replacing framebuffer pixels, it waits for the previous page and display workers to finish. This page path bypasses the partial-update display thread.

| Change | Update |
| --- | --- |
| Page turn, menu close, restyle | Full mode 9 |
| Book open, screen switch | Full mode 8 flash |
| Black-and-white UI | Partial DU |
| Library and settings grays | Partial GL16 |
| Reader menu open | A2, then full-mode gray update per bar |

Night mode keeps page-turn and flash modes. See [Night mode](ui-framework.md#night-mode).

## The app

Startup shows the saved library index, then scans `/mnt/us/documents`. The library has Books, Folders, Series, and a continue-reading card. Settings control the frontlight, night mode, covers, screensaver delay, and USB mode.

The default screensaver saves progress, turns off the light, and shows a book card. A touch wakes it without taking another action. The app holds `powerd`'s `preventScreenSaver`, so the CPU does not suspend and the power button has no effect.

`ARISTOTLE_POWERD=1` enables the implemented `powerd` event path in `powerd.rs`. It releases `preventScreenSaver`, handles sleep and wake events, and restores stock screensaver behavior on exit. This path has host tests only; device suspend, wake light levels, and battery use are unverified.

Progress saves 3 seconds after the last turn, or at most 20 seconds after the first unsaved turn. It also saves on return to the library and exit, including SIGTERM and SIGINT. Background pagination writes page numbers under `/mnt/us/.aristotle/pages/`.

## Reading interactions

`reading.rs` handles these actions in both the app and host preview.

| Gesture | Result |
| --- | --- |
| Hold a word for 500 ms | Select the word and show its definition |
| Hold, then drag | Extend the selection; offer Highlight, Dictionary, Translate, and Cancel on release |
| Hold a highlight | Show its definition and removal action |
| Tap a footnote marker | Show the note in a popup |
| Tap another link | Jump to its target |
| Tap the top-right 10 mm | Toggle a bookmark |
| Swipe on a popup or list | Change its page |

Link and bookmark targets do not turn pages. A selection stops at the current page's end.

### Updates on the panel

Transient selection feedback uses A2. Completed popups with grays use a full-screen mode-9 update. Closing a popup restores the page with a full refresh. See `reading.rs` and [Transitions](ui-framework.md#transitions).

### Dictionary

StarDict dictionaries are read one folder deep under `/mnt/us/dictionaries`, `/mnt/us/koreader/data/dict`, or `--dict DIR`. File names such as `grc-pl.ifo` identify language pairs.

`dict/fold.rs` builds indexes that ignore combining marks and case and normalize final sigma. Exact spelling wins over folded matches. Greek text selects Ancient Greek dictionaries; other text uses the book language. The selected dictionary is saved per language in `dictionaries.json`.

Lookups try listed forms, related headwords, guessed suffix rules, and nearby entries. Guesses are labeled. **Translate** looks up each selected word. Popups show senses first and expand with **More**.

Device index builds reached 66 MB [dev]. Build large indexes on the host with `aristotle --index-dicts --dict DIR --library OUT`, then copy them to `/mnt/us/.aristotle/dict-index/`.

### Footnotes

`Book::footnote` recognizes EPUB semantics, common note IDs and classes, and numbered or symbolic markers. It removes back links and limits note text to 2,400 characters.

### Highlights, bookmarks and export

Annotations in `library.json` have source `aristotle`. They store positions that survive font-size changes, plus chapter and time.

New highlights append to `/mnt/us/documents/My Clippings.txt` in the stock en_GB format. The **Save highlights to My Clippings.txt** setting disables that output. Markdown notes in `/mnt/us/aristotle-notes/` are rewritten per book. Deleting a highlight does not remove its clippings entry.

Tests and scenarios must use `--test-clippings PATH`, never the user's clippings file. The app has no keyboard for adding annotation notes.

### Progress line and navigation

The footer shows the chapter and estimated minutes left. `speed.rs` averages forward turns lasting 5 seconds to 10 minutes, starting at 75 seconds per page. **Go to** offers a slider and percentage steps. Reading speed is saved in `reading.json`.

## Settings and files

`reader.json` stores global preferences. Missing or unknown fields use defaults.

| Field | Default |
| --- | --- |
| `size`, `spacing`, `margins` | `2`, `1`, `1` |
| `turn_mode` | `9` |
| `night`, `night_method` | `false`, `software` |
| `library_view`, `sort` | `0`, `0` |
| `language` | `en` |
| `clippings` | `true` |

The frontlight level is read from the device, not saved. See `prefs.rs` for all fields.

The library data directory is `/mnt/us/.aristotle`, overridden by `--library DIR` or `$ARISTOTLE_DATA`. It contains the index, covers, preferences, dictionary indexes, page starts, snapshots, and converted MOBI cache. `--data` selects renderer assets, not library state. `aristotle --help` lists options.

## Timing log

The app writes `[perf <ms since start>]` lines to stderr. The launcher sends them to `/var/tmp/aristotle.log`.

| Prefix | Measures |
| --- | --- |
| `startup`, `scan` | First frame and library scan |
| `open` | Snapshot submission and background open |
| `turn`, `panel` | Tap-to-submit and update completion |
| `paginate`, `snapshot`, `prewarm`, `index`, `dict` | Worker activity |
| `hold`, `link`, `highlight`, `export` | Reading actions |

## Size budget

The default build includes Latin and Greek fonts, five hyphenation languages, and EPUB, MOBI, PDF, and TXT support. Build sizes are recorded in [Benchmarks](benchmarks.md#binary-size).

| Switch | Default | Other choices |
| --- | --- | --- |
| `MUPDF_FONTS` | `latin-greek` | `base14` drops Noto Serif; `full` restores Plato's fonts |
| `MUPDF_FORMATS` | `minimal` | `full` adds formats including XPS, SVG, CBZ, FB2, Office, HTML, and EPUB |
| `PLATO_DJVU` | `0` | `1` links DjVuLibre |
| `FONT_SUBSET` | `latin-greek` | `none` keeps whole fonts |
| `HYPH_LANGS` | `pl en-us en-gb grc la` | Pattern list |

The first three belong to `build-plato.sh`, the last two to `build-render.sh`. Other C-library changes need a fresh `PLATO_BUILD`.

MuPDF keeps Noto Serif for Greek in reflowed text. Plato uses smaller fallback fonts. Missing glyphs, including unsupported CJK characters, appear as boxes.

## Known limitations

- MOBI has no table of contents, and the converter does not support HUFF/CDIC.
- Single-letter Polish words can end a book-text line.
- Plato ignores `margin: auto` for image centering.
- Memory use during background pagination has not been measured.
- Selection legibility and popup ghosting have not been checked on the panel.
