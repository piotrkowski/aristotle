# The reader: rendering book pages

`crates/aristotle-render` is Aristotle's first e-book renderer. It uses the document engine of [Plato](https://github.com/baskerville/plato) (`plato-core`) and none of Plato's Kobo UI, and turns books into full-screen 8-bit pages for the PW4 panel. Its CLI, `aristotle-render`, renders pages to files on any host, shows them on the device with the page-turn strategy from [`native-vs-aristotle.md`](native-vs-aristotle.md), and measures CPU and panel time.

Status: renders EPUB, MOBI (DRM-free MOBI 7) and PDF; the CLI runs on the device. The reader app (`aristotle`, see [The reader app](#the-reader-app)) passed a smoke test in the emulator and awaits its first run on the panel. Numbers below are host numbers unless marked [dev].

## Architecture

```text
aristotle-render (CLI: info, render, show, bench, turn)
  └─ Book ─────────── plato-core
  │    EPUB, HTML ──── Plato's HTML engine (our font, Knuth–Plass, Liang hyphenation)
  │    MOBI ────────── mobi.rs → cached EPUB → Plato's HTML engine (MuPDF as fallback)
  │    PDF, TXT ────── MuPDF through Plato's PdfOpener (PDF scaled to fit)
  ├─ Page, Chrome ─── 1072 × 1448 buffer, stride 1088; header and footer drawn in the same frame
  └─ Screen ───────── aristotle-hal Framebuffer: blit the page, one full mode-9 update of the whole screen
```

| Module | Role |
| --- | --- |
| `style.rs` | Geometry and typesetting defaults (`Style::pw4`) |
| `book.rs` | Opening, engine choice, lazy pagination, chapter titles, rendering and composition |
| `lang.rs` | Hyphenation language for EPUBs that declare several |
| `mobi.rs` | MOBI 7 decoder and EPUB writer |
| `page.rs` | Page buffers, the black and white frame, header and footer |
| `screen.rs` | The page turn on the panel |
| `position.rs` | Saved positions: EPUB spine coordinates, MOBI position maps |

## How Plato is consumed

`plato-core` is not on crates.io, needs a few changes, and links about fifteen C libraries. The setup:

- **Source.** `tools/prepare-plato-src.sh` writes `git archive` of the commit the `third_party/plato` submodule pins, plus `tools/plato-patches/plato-*.patch`, to `third_party/plato-patched/` (gitignored). The submodule is never modified. The script is idempotent: a stamp holds the commit and a hash of the patches.
- **Patches** (each small, with a description at the top):
  - `0001`: static musl linking (from the Plato cross-build, `tools/build-plato.sh`);
  - `0002`: `PLATO_LIB_DIRS` lets another workspace point `build.rs` at the static archives, and the x86_64 musl host target is accepted;
  - `0003`: MuPDF's embedded-font symbol names on every musl target (Zig's MuPDF build uses the same names on x86_64 as on ARM);
  - `0004`: `EpubDocument::set_language`, see "Hyphenation";
  - `0005`: links against a MuPDF without its Noto and CJK fonts (no glyph fallback then), and relinks when the archives change;
  - `0006`: DjVu is compiled out when `libdjvulibre.a` is absent;
  - `0007`: `zip` with deflate only;
  - `mupdf-0001` (applied to MuPDF, not Plato): `KEEP_NOTO_SERIF`.

  `0005` to `mupdf-0001` exist for the size budget, see [Size budget](#size-budget).
- **C libraries.** `tools/build-plato.sh` builds them with Zig as static archives: for the device as before, and with `PLATO_TARGET=x86_64-unknown-linux-musl PLATO_LIBS_ONLY=1` for the host (about 6 minutes on 2 cores, once). `MUPDF_FONTS`, `MUPDF_FORMATS` and `PLATO_DJVU` choose what goes in; the defaults are the owner's needs, see [Size budget](#size-budget).
- **Workspace.** `crates/aristotle-render` is its own Cargo workspace and the root workspace excludes it, so `cargo build` at the root keeps working on hosts that never ran the Plato build. It depends on `../../third_party/plato-patched/crates/core` by path and on `aristotle-hal`. Its `Cargo.lock` started from Plato's, so dependency versions match the tested Plato build.
- **Build.** `tools/build-render.sh device|host` prepares the source, sets `PLATO_LIB_DIRS` and the Zig C compiler (for crates that build C), builds, and assembles the bundle in `~/.cache/aristotle/render/<device|host>/`. `CARGO_CMD=clippy|test` runs those instead. `PLATO_BUILD` selects a `build-plato.sh` output directory (default `plato-build/main`, or `host-x86_64`).

Why not a `[patch]` section in the root workspace: it would make every root build depend on the prepared source and the C libraries. Why not build against the copy inside a `build-plato.sh` run: that copy is patched once when the run starts and goes stale when patches change, and its location differs per run.

### Host previews

The host build is the same code linked against the same library versions built for x86_64 musl, so previews match the device apart from floating-point rounding. Render and convert:

```sh
~/.cache/aristotle/render/host/aristotle-render render BOOK.epub 10 page.raw
~/.cache/aristotle/venv/bin/python -I devloop/host/fb2png.py page.raw page.png
```

### Runtime data

Plato opens fonts, stylesheets and hyphenation patterns by paths relative to the current directory, so the CLI changes into its data directory (`data/` next to the binary, or `--data`). `tools/build-render.sh` assembles it:

| Path | Contents | Source |
| --- | --- | --- |
| `fonts/` | The 12 fonts Plato's engine loads unconditionally (Libertinus Serif as a stub, Noto Sans cut to Latin and Greek, Source Code, Parisienne, Delius); the glyph fallback: DejaVu Sans cut to symbols, Noto Sans Coptic | Plato source tree; DejaVu 2.37 release, pinned by SHA-256; MuPDF's source tree |
| `serif/` | Literata, the body, chrome and app UI font, cut to Latin and Greek | `crates/aristotle-render/assets/fonts/` (OFL, committed with its licence) |
| `css/` | `epub.css`, `html.css` | Plato source tree |
| `hyphenation-patterns/` | `pl`, `en-us`, `en-gb`, `grc`, `la` (`HYPH_LANGS`) | Plato 0.9.45 release archive, pinned by SHA-256 |

Fonts are subset by `tools/build-render.sh` (`FONT_SUBSET`), see [Size budget](#size-budget).

Converted MOBI books go to `cache/` next to the binary (or `--cache`).

## Typesetting defaults

| Setting | Value | Why |
| --- | --- | --- |
| Font | Literata 10 pt | Designed for e-reading; sturdy serifs and a large x-height that hold up in 16 grays; full Polish coverage (ą ć ę ł ń ó ś ź ż, „ ” – …); OFL. Its x-height makes 10 pt read like 11 pt of Libertinus. |
| Leading | 1.3 em | Literata is dark and tall; Plato's 1.2 felt cramped. |
| Alignment | Justified | With Polish hyphenation the gaps stay small. |
| Side margins | 5 mm (59 px) | Plato takes whole millimetres. |
| Text block | 104 px from the top, 100 px from the bottom | Leaves bands for the header and footer. Plato's margins are uniform, so the layout area is shifted up and down to put the text block there; the chrome is drawn over Plato's white margins. |
| Header | Chapter title (book title if unknown), Literata Italic 7 pt, gray 0x40, centred | 0x40 is below the A2 threshold, so the header is legible in the black and white frame too. |
| Footer | `x / y` centred, percent at the right margin, 7 pt | Drawn in the same frame as the text: no separate footer update, unlike stock. |

At 10 pt a line holds about 42 to 45 characters on the 81 mm text width. That is narrow for justified text; the occasional loose line remains even with hyphenation. 9 pt (`--font-size 9`) gives about 50 characters. The owner asked for 10 to 11 pt, so 10 is the default.

Plato's own paragraph defaults apply (Knuth–Plass `total_fit`, hyphen penalty 50, stretch tolerance 1.26). The book's CSS is honoured.

## Hyphenation

- Plato's patterns (kl-hyphenate, from hyph-utf8) **include Polish**: `pl.standard.bincode`, minima 2 and 2. Checked word by word: zo-ba-czy-my, po-dob-ne, prze-ci-wień-stwem, rze-czy-wi-stość, Szcze-cin.
- Plato picks patterns per element from `lang`/`xml:lang` and otherwise from the book's **first** `dc:language` (`pl`, `pl-PL` and `PL` all map to Polish). Without a language it uses English.
- Problem found in the owner's library: 2 of 44 Polish EPUBs declare several languages, and one lists `de-DE` first. It was hyphenated with German patterns ("zobac-zymy", "pod-obne"). Patch `0004` adds a language override, and `lang.rs` decides it for books that declare more than one: it samples about 190 KB of the text and picks the declared language whose marker letters (ąćęłńśźż for Polish, äöüß for German, ...) are most frequent. `--lang TAG` forces a language.
- `aristotle-render info BOOK` prints the language and whether its patterns are loaded.
- Plato loads every pattern file present in `hyphenation-patterns/` on first use (9 ms on the host for five languages), which is why the bundle ships only the owner's: Polish, English (US and GB), Ancient Greek and Latin.
- Unverified in Plato's code: in a test book (Polish, with `<p lang="grc" xml:lang="grc">` paragraphs) the Greek paragraphs did not get the `grc` patterns; with `--lang grc` they did. Books declared as `grc` or `la` use their patterns.

## MOBI

MuPDF reads MOBI, but its HTML layout has no Polish patterns, uses its own fonts and justifies poorly. So `mobi.rs` converts DRM-free MOBI 7 books once to a stored (uncompressed) EPUB in the cache and Plato's engine lays that out like any EPUB, with the language from EXTH 524 (or the header's locale). It handles PalmDOC and uncompressed text, UTF-8 and CP1252, trailing record entries, images, `<mbp:pagebreak/>` chapter splits, MOBI's paragraph attributes, and the HTML that Plato's XML parser can't take (void elements, unquoted attributes, unclosed paragraphs). DRM, HUFF/CDIC compression and KF8-only files fall back to MuPDF; joint MOBI/KF8 files use their MOBI 7 part. `--no-convert` forces MuPDF.

The host has no MOBI files; `devloop/host/make-test-mobi.py` makes a MOBI 7 test book from an EPUB.

## Pages and progress

For Plato's engine a location is a byte offset into the book's text. Pages are found by walking `Location::Next`, which lays out one spine item (chapter file) at a time and caches its pages. `x / y` needs the whole book laid out: `render`, `show` and `turn` paginate first unless `--percent` is given. Progress is `x / y` once paginated, else the byte offset over the book's size. For MuPDF documents locations are page indices and the count is known.

A reader cannot afford to lay out the whole book before showing a page, so `Book` also navigates by location: `page_start` (the page containing a location), `next_page`, `previous_page`, `chapter_page`, and `render_location`, whose footer leaves out the page number until one is known. `set_pagination` adopts page starts computed elsewhere; a test (`tests/navigation.rs`) checks that walking from page to page, forwards and backwards across chapters, finds exactly the pages of a full pagination.

Saved positions (`position.rs`) outlive a layout:

- **EPUB:** spine item and byte offset inside it. Plato keeps its spine private, so `Spine::read` repeats how `EpubDocument::new` builds it, and `Book` uses it only when the total agrees with Plato's.
- **MOBI:** the stock Kindle position, a byte offset into the uncompressed MOBI text (what `.mbs` sidecars store, see `docs/library.md`). The converted EPUB has different markup, so the converter records an anchor (MOBI offset, EPUB location) at every run of text it copies, at least 200 bytes apart, and stores them as `<book>.posmap` next to the cached EPUB (converter version 2). Positions in between are interpolated. A test converts a generated book in UTF-8 and CP1252 and checks that marked words land on themselves. With a cached conversion only the MOBI headers are parsed on open.
- **Anything else** (KOReader XPointers, plain-text offsets): the stored fraction.

## Page turn on the device

`show` and `turn` turn pages like the stock reader: one full-mode update of the whole screen with waveform mode 9 (`--turn-mode 7` for DU4, `--a2-preview` for the old two-step turn, which left ghosting the owner rejected). `turn` starts from a clean mode-8 page, renders the next page while the panel refreshes, and reports tap to shown and prerender times. Both refuse to run while the stock framework (`cvm`) is up.

## The reader app

`crates/aristotle-reader` (binary `aristotle`) is the first real reader: the library from `aristotle-library`, pages from this crate, the menu from `aristotle-ui`, on the panel through `aristotle-hal`. It links Plato, so it is a member of the `aristotle-render` workspace (`members = [".", "../aristotle-reader"]`, `package.workspace` pointing back), with one lock file and one build of the C-backed dependencies; the root workspace excludes both. The renderer itself does not depend on the UI or the library.

| Module | Role |
| --- | --- |
| `main.rs` | Arguments, startup, the event loop, library, settings, screensaver and reader screens, menu actions |
| `panel.rs` | Pages and UI damage onto the panel (`plan`: the transition policy), night mode, the menu overlay sequence, completion tracking (host-tested) |
| `session.rs` | One open book: current page start, rendered pages around it, restyle, labels |
| `turns.rs` | Touch-down turns, swipes, menu gesture (host-tested) |
| `shelf.rs` | Library index to the library screens: views, folders mirroring `documents/`, series, book to continue, sort, covers (host-tested) |
| `device.rs` | Clock, battery, storage, USB mode and versions for the status line and settings |
| `progress.rs` | Locator mapping, write debouncing, the index writer thread (host-tested) |
| `pagecache.rs` | Page numbers on disk and the background pagination (host-tested) |
| `snapshot.rs` | Page snapshots: the page a book was left at, on disk (host-tested) |
| `prewarm.rs` | Snapshots made ahead of time while the library is idle |
| `open.rs` | Opening from a snapshot with the layout on a worker thread, `--bench-open` (host-tested) |
| `prefs.rs` | Font size, spacing, margins, turn waveform, night mode, covers, screensaver delay, library view and sort, `reader.json` (host-tested) |

### Behaviour

- **Library.** On start the index (`/mnt/us/.aristotle/library.json`) is shown at once, then `/mnt/us/documents` is scanned incrementally (an unchanged library costs a `stat` per file); if anything changed, the screen is rebuilt with only the changed pixels sent. "Czytaj dalej" shows the most recently opened book, or before any book was opened here, the one whose progress (stock-imported included) changed last. Three views: Książki with the shelves Wszystkie, Czytane, Nowe and Przeczytane; Foldery, which mirrors `documents/` (a book's folder is the directory of its path in the index; inside a folder its subfolders come first, the back arrow goes up a level); and Serie, a shelf per series in series order. The sort button cycles Ostatnie, Tytuł, Autor; view and sort are kept in `reader.json`. Lists page ("Strona x z y"). Covers are off by default; "Karty" draws typographic cards, "Zdjęcia" the thumbnail cache's covers. The gear opens the settings. **Holding the "Biblioteka" title for a second quits.** Layout and design: `docs/ui-framework.md`.
- **Settings.** Frontlight in 24 steps equal in CIE L* over the LM3695's 0–2047 range (`aristotle_hal::power::light_raw`), presets, night mode, covers, screensaver delay, battery, free space, date and time, the USB mode (through `/var/local/aristotle/aristotlectl mode net` or `mode storage 10`), versions. Each change is saved and shown at once.
- **Night mode.** From the menu's "Noc" row or the settings; white on black everywhere, one mode-8 flash to switch. Software inversion by default (UI canvas and page copy inverted), hardware inversion with `--night-method hardware`; see `docs/ui-framework.md`, "Night mode".
- **Screensaver.** After the chosen time without a touch (15 min by default) the app saves, turns the light off and shows the screensaver (the book being read as a card and its progress); a touch brings back the light and the screen, and does nothing else. The app holds `powerd`'s `preventScreenSaver` while it runs and restores it on exit; it does not suspend the CPU.
- **Reader.** A book opens at its saved position with one mode-8 flash (a screen switch), after the tapped card inverts on touch-down; see [Opening a book](#opening-a-book). A touch-down in the left 30 % turns back, anywhere else below the top 12 % forward, without waiting for the release. A horizontal swipe ends one page from where it started in its direction (a swipe right that started in the forward zone goes two back, undoing the touch-down turn). A tap or a swipe down in the top strip opens the menu.
- **Page turns.** Exactly the stock reader's: one full-mode update of the whole screen with mode 9 (`--turn-mode 7` for DU4, kept in `reader.json`). The previous page update must have finished before the next page is written to the framebuffer; that is the only place the input loop waits for the panel. After the submit, the pages on both sides are rendered, so the next turn costs a memcpy and a submit.
- **Menu.** It appears as one A2 update over its top bar and bottom sheet, then a full-mode gray update of each part (`Panel::present_overlay`); its edges sit in the white between two lines of the page. Size (8 to 15 pt), spacing (1.2, 1.3, 1.5 em) and margins (3, 5, 8 mm) lay the book out again at the same place; the new page goes out as a full mode-9 update. Chapter arrows jump through the table of contents. The light slider and its − and + move the frontlight in the settings' 24 steps. "Noc" switches night mode. "Biblioteka" goes back. Closing the menu (a tap on the page) redraws the page with a full mode-9 update, which also clears what the menu left. The font row of the prototype is left out: the renderer ships Literata only.
- **Progress.** Every turn updates the record in memory (a spine or Kindle locator plus the fraction; the last page marks the book finished). The index goes to disk 3 s after the last turn, or at the latest 20 s after the first unsaved one, on a writer thread; also when going back to the library and on exit (SIGTERM and SIGINT included).
- **Page numbers.** `x / y` is cached per book and style in `/mnt/us/.aristotle/pages/`. On a miss a thread opens its own copy of the book and paginates it at `SCHED_IDLE` (musl stubs `sched_setscheduler`, so the system call is made directly), so on the single-core PW4 it only runs while the reader waits for input. When it finishes, the numbers appear from the next page on.

Files in the library data directory besides the index and covers: `reader.json` (preferences), `pages/<book id>-<key>.txt` (page starts, one per line; the key hashes the style, the file's size and mtime and a layout version), `snapshots/<book id>.snap` (page snapshots, below), `epub-cache/` (converted MOBI books and their position maps).

### Opening a book

Plato finds page breaks only by laying out the chapter that holds a position, from the chapter's start. Placing the saved position therefore costs the layout of its whole chapter, and a MOBI without page breaks converts to one chapter: "Pan Tadeusz" took 1989 ms of 2454 ms to the first submit on the device [dev], with the page numbers cached or not. The page the reader left is known, though, so the app keeps it:

1. **Snapshot.** Whenever progress is saved (3 s after the last turn, back to the library, quitting) the current page and the prerendered next one go to `snapshots/<book id>.snap`: the 8-bit page buffers, zstd level 1 (about 63 KB per text page instead of 1.5 MB), with each page's location, progress locator and menu labels. The key is the book id, the book file's size and mtime (stat, not the index), a hash of every style field, the layout version, the app build and the runtime data (fonts, stylesheets, patterns), and the progress locator. The snapshot is used only if all of them match now, so another font size, a replaced file, a position imported from the stock reader, a new build or new data each make it stale (it is overwritten at the next save; snapshots of books gone from the library are deleted at the next scan). Each page carries zstd's content checksum. Copying the two pages (2 × 1.5 MB) happens on the main thread; compressing and writing (with an fsync on the FAT userstore: the 79 to 379 ms seen on the device) happen on the writer thread at nice 10. An open that showed the snapshot unchanged does not write it back. The page chrome (chapter title, `x / y`, percentage) is a function of the location and has no live fields such as a clock or battery, so a snapshot cannot show a stale one.
2. **Tap to page.** On a tap the card inverts (A2) on touch-down; on release the snapshot is read, decoded and drawn into the reader screen, which goes out as the open's one mode-8 flash. Nothing has been laid out yet.
3. **Worker.** A thread opens the book, places it at the snapshot's own page start (not where the locator maps back to), renders the page and both neighbours, and returns them (`Session::prepare`; the result holds no `Rc` pages or FreeType fonts, so it can cross threads; the session loads its own chrome fonts). Meanwhile a forward turn shows the snapshot's next page and a back turn returns from it (mode-9 turns), the menu opens with the stored labels, and "Biblioteka" goes back at once: the worker is abandoned (it stops between open, place and render, its thread drops to `SCHED_IDLE`, and its result is discarded) and the page shown is kept as the progress. Background opens take turns (`OPEN_GATE`), so an abandoned one drops its book before the next one lays out. Anything that needs the book (a turn beyond the stored pages, a menu setting) waits for the worker; if the book's page differs from the snapshot, the action's own full update replaces it, or one full mode-9 update follows when the action draws none (a turn at the first or last page, a light change).
4. **Adoption.** The main loop polls the worker every 10 ms while it runs. The real session must place the position on the page shown, and its rendered page must equal the snapshot's pixels; then nothing is sent. Otherwise (another location, or a different header or footer, for example page numbers that were unknown when the snapshot was taken) the real page replaces it with one full mode-9 update, never a partial fix-up.

Without a snapshot there is nothing to show early: the tapped card stays inverted, the book opens on the worker, and the library keeps taking input; tapping anything else gives that book up. When it is ready, its page goes out with the one mode-8 flash. That case is made rare by **snapshots made ahead of time** (`prewarm.rs`): after 2 s without input on the library screen, one `SCHED_IDLE` thread at a time opens the five most recent books with a saved position and no valid snapshot (after a font size change: all of them), renders the saved page and the next, and stores them. Any touch cancels it between steps; each book is tried once per run, position and style, so the work stops by itself. It never paginates whole books. On the host it costs 48 to 248 ms of CPU per book (Książę to the "Pan Tadeusz" stand-in), so up to about 2 s per book on the device, once.

**Kindle positions.** A converted MOBI stores the stock Kindle position of the page. `PosMap::position` rounded down, and where the EPUB has more bytes than the MOBI text the saved position mapped back to before the page start: 32 of 40 pages reopened one page early in the new test (`tests/navigation.rs`), and "Pan Tadeusz" did on the device (the snapshot's page then disagreed with the book's). Positions are now the smallest that map back onto the page. Books saved with an older build land one page early once more. Going back to the library submits the library first; copying the snapshot's pages, serialising the index and freeing the closed book (the display lists of its laid-out chapters) wait until after the submit. Other cuts on the open path: a cached MOBI conversion reads only the PalmDB header and record 0, not the whole file, and Plato's hyphenation patterns (all files in `hyphenation-patterns/`, 5 to 9 ms on the host) are loaded at idle priority while the library is on screen.

`aristotle --library DIR [--root DIR] --bench-open TITLE` runs the open path without panel or touchscreen and prints both paths (it writes progress, page numbers and a snapshot to `DIR` like the app). Host numbers for a stand-in of "Pan Tadeusz" (420 KB of Polish prose in one spine item, converted from a generated MOBI, 379 pages; one CPU, `taskset`, page numbers cached, a loaded host, 6 runs):

| Step | Old path | New path |
| --- | --- | --- |
| Tap to submit (book, position, first page, UI frame) | 250–400 ms, median 278 (position 244–392) | **1.0–1.2 ms** (read 0.1, decode 0.6, UI frame 0.3) |
| Real page ready | the same | 257–372 ms after the tap, then nothing sent (snapshot `Same`) |
| Snapshot | | 126 KB for two pages; encode 1.5 ms, encode and write 6.6 ms |

On the device the layout is about 8 times slower than on the host ("Pan Tadeusz": 1989 ms against about 250 ms), so the submit should land at about 10 to 20 ms after the release, and the page should be on the panel after the 480 ms of the mode-8 flash: about 0.5 s from tap to page, against 2.9 s before and 0.8 s for the stock reader. These are host numbers until verified on the device; the scenario `devloop/kindle/scenarios/aristotle-open.txt` (`aristotle-observe run`) exercises opening, turning, the menu and going back while a book opens, and opening another book after that.

**Splitting huge chapters instead.** The converter could cut a one-chapter MOBI into spine items of, say, 50 KB, so placing a position lays out only its piece (about a tenth of the time). But a spine item boundary is a forced page break: every cut would end a page early, leaving a part-empty page every 20 pages or so, at places that differ from both the stock reader and the book. Cutting only at scene breaks would hide some of that, at the cost of a typographic rule the book does not have. The snapshot makes reopening fast without touching page breaks, so the converter keeps chapters whole; the remaining cost is the first open of a book (no snapshot yet) and a size change in the menu, which lay out the chapter on the main thread.

### Running it on the device

```sh
PLATO_BUILD=~/.cache/aristotle/plato-build/size PLATO_LIBS_ONLY=1 tools/build-plato.sh   # once
OUT=~/.cache/aristotle/render/device PLATO_BUILD=~/.cache/aristotle/plato-build/size tools/build-render.sh device
ssh -F devloop/host/ssh_config kindle 'tar -xzf - -C /mnt/us' < ~/.cache/aristotle/render/aristotle-app-device.tar.gz
ssh -F devloop/host/ssh_config kindle sh /mnt/us/aristotle/run-aristotle.sh            # mode 9
ssh -F devloop/host/ssh_config kindle sh /mnt/us/aristotle/run-aristotle.sh --turn-mode 7
```

The app bundle (`aristotle/` with the binary, `data/`, `run-aristotle.sh` and `takeover.sh`) goes to the userstore: the binary was 29.5 MB when this was decided (8.4 MB now, see [Size budget](#size-budget)), and the 64 MB `/var/tmp` tmpfs is better left to the renderer CLI and logs. The app maps its UI font from `data/serif/`, so it needs the data directory next to it. `run-aristotle.sh` stops the stock GUI with `takeover.sh start`, runs the app, and restarts the GUI with `takeover.sh stop` (30 to 60 s of boot splash). `pause`/`resume` (SIGSTOP) is not used: on our firmware the frozen framework was killed by Amazon's watchdog and the GUI did not come back without a reboot. The script ignores hangups, logs stderr to `/var/tmp/aristotle.log` and prints it on exit. Other options: `--library DIR`, `--root DIR`, `--data DIR`, `--no-scan`, `--night-method software|hardware`.

### Timing log

Every step that matters for speed writes a `[perf <ms since start>]` line to stderr:

| Line | Meaning |
| --- | --- |
| `startup: first frame submitted` | Process start to the library's first frame handed to the panel |
| `scan: ...` | Incremental scan time and what changed |
| `open: "<title>": book, position, first page, submitted after ...; no snapshot` | Opening a book without a snapshot: Plato open (MOBI conversion on first open), placing the saved position, rendering the first page, total to submit |
| `open: "<title>": snapshot submitted after N ms (read, decode)` | Opening from a snapshot: tap release to the flash submitted |
| `open: "<title>": book ready N ms after the tap (worker: ...); snapshot Same\|Chrome\|Page` | The worker's open adopted; anything but `Same` sent one full mode-9 update (`corrected page`) |
| `open: waited N ms for the book`, `open: ... back to the library ... before the book was ready` | Input that needed the book, or abandoned it |
| `snapshot: N page(s) saved` | Snapshot writes (on their own thread) |
| `prewarm: snapshot of "<title>" made in N ms` | A snapshot made ahead of time (or `cancelled`) |
| `open: "<title>": no snapshot, opening in the background`, `... given up` | An open without a snapshot, and one given up for another tap |
| `startup: UI fonts and battery ready`, `shelf and covers ready`, `library screen built` | Steps before the first frame |
| `warm-up: ...` | Hyphenation patterns loaded at startup |
| `turn <dir>: tap->submit` | Kernel timestamp of the touch-down to the update submitted, whether the page was prerendered, the wait for the previous update, then the prerender times |
| `panel: <what> on the panel N ms after submit` | When the driver reported the update complete |
| `paginate: N pages in T ms` | The background pagination |
| `index: saved` | Index writes (on the writer thread) |

### Smoke test in the emulator

In the PW4 emulator, with the stock GUI left running, three test books (two EPUBs and a MOBI) went through: scan and covers, opening the MOBI and an EPUB, touch-down turns, background pagination (297 and 187 pages, adopted while reading), the menu with a size change at the same place, back to the library, "Czytaj dalej" reopening at the saved page with page numbers from the cache, and quitting by holding the title. Emulator times are not device times and are not reported.

## Measurements

Host (x86_64, release build), Literata 10 pt:

| Book | Open | First page | Render per page (median / p90) | Paginate all | Max RSS |
| --- | --- | --- | --- | --- | --- |
| Platon, *Fedon* (EPUB, 341 pages) | 1.1 ms | 0.6 ms | 2.1 / 2.4 ms | 211 ms | 32 MiB |
| Arystoteles, *Etyka nikomachejska* (EPUB, 269 pages) | 10 ms | 1.9 ms | 4.6 / 5.2 ms | 241 ms | 26 MiB |

"Render" is Plato rasterising the page; composing the chrome adds about 0.3 ms and the black and white frame 0.1 to 0.2 ms. Laying out a new chapter happens when a page turn crosses into it (25 to 70 ms on the host), and dominates the worst case.

Device numbers: pending the first `bench` and `turn` runs [dev].

## Size budget

The device build only has to read what the owner reads: Polish and English, some Ancient Greek and Latin, as EPUB, MOBI, PDF and TXT (their library: 25 EPUB, 4 MOBI, 1 PDF, 1 TXT; no DjVu, no CJK). Everything else was cut, one measured step at a time. Sizes are stripped ARM binaries in MB (10^6 bytes), built with `tools/build-plato.sh` and `tools/build-render.sh device`:

| Step | Reader app `aristotle` | Renderer CLI | What changed |
| --- | ---: | ---: | --- |
| Before | 29.52 | 26.04 | MuPDF with Plato's Kobo font set, DjVuLibre, `zip` defaults, opt-level 3 |
| MuPDF fonts: base 14 only | 13.79 | 10.32 | `-DTOFU -DTOFU_CJK -DNO_CJK`: no Noto (13 MB), Droid Sans Fallback (3.6 MB, CJK), CJK CMaps (0.8 MB), Charis SIL; patch `0005` |
| MuPDF formats | 13.03 | 9.55 | `FZ_ENABLE_*=0` for XPS, SVG, CBZ, FB2, Office, HTML, EPUB; no MuPDF hyphenation (0.6 MB of patterns) |
| No DjVu | 11.75 | 8.27 | DjVuLibre not built or linked; patch `0006` |
| C function sections | 11.21 | 7.73 | `-ffunction-sections -fdata-sections` for HarfBuzz, FreeType and the other C libraries, so the link drops what nothing calls |
| `zip` deflate only | 10.12 | 6.65 | No bzip2, LZMA, XZ, PPMd, Zstandard (0.5 MB of C), Deflate64, AES; patch `0007` |
| UI font from `data/` | 8.85 | 6.65 | The app maps `data/serif/Literata-*.ttf` instead of compiling the same 1.3 MB in |
| Noto Serif back in MuPDF | 9.14 | 6.94 | +0.29 MB, see below; patch `mupdf-0001` |
| opt-level "s", 3 for the page path | 8.47 | 6.77 | `opt-level = "s"`, but 3 for `plato-core`, `aristotle-render`, `paragraph-breaker` and `kl-hyphenate` |
| FreeType modules | **8.36** | **6.60** | No PFR, Type 42, Windows FNT, PCF, BDF, LZW/bzip2 streams, OT-SVG, SDF |
| Merged with `main` (night mode, settings, series, icons, screensaver, page snapshots) | 9.15 | 6.63 | New features; the snapshots' `zstd` brings the Zstandard C library back (about 0.5 MB) |

The data directory went from 5.04 to 2.68 MB: `fonts/` 2.91 to 1.32 (0.14 of it the glyph fallback and licences), `serif/` 1.28 to 0.90, `hyphenation-patterns/` 0.85 to 0.46. The app tarball went from 19.4 to 6.0 MB.

What was removed, and why that is safe for this owner:

- **MuPDF's fonts.** Plato's engine renders EPUB (and converted MOBI) with Literata, which has Latin, Latin Extended and Greek with polytonic, so the Noto fallback fonts only ever drew symbols and CJK. PDFs embed their fonts; those that don't get the 14 standard fonts, which MuPDF keeps (URW Nimbus, Latin, monotonic Greek and Cyrillic; checked with a PDF using non-embedded Times, Helvetica and Courier with Polish and Greek glyph names, unchanged). Noto Serif (Latin, polytonic Greek, Cyrillic) stays because MuPDF's reflowed text (TXT, MOBI the converter can't take) needs it for polytonic Greek: without it the Greek accented letters of a TXT page vanished. Choices: `MUPDF_FONTS=latin-greek` (default), `base14` (without Noto Serif), `full`.
- **Glyph fallback in Plato's engine.** Patch `0005` replaces MuPDF's Noto and Droid fonts with two small files from `data/fonts/`: Noto Sans Coptic for Coptic, and DejaVu Sans cut to arrows, mathematical operators up to ⋅, box drawing, geometric shapes, miscellaneous symbols up to the music signs, dingbats up to the checkmarks, crosses and stars, and U+2B00 to U+2B2F (106 KB unhinted). Other characters Literata lacks, CJK included, show a `.notdef` box. In the 68 EPUBs on the host (the 23 Platon dialogues and 45 other Polish books), Literata lacks only CJK characters (a few in the bibliographies of two books), U+2B1C ⬜ (22 times in one book), and spaces and invisible marks that HarfBuzz handles without the font.
- **MuPDF formats.** EPUB and HTML go through Plato's engine. MuPDF keeps PDF, images (Plato decodes EPUB pictures through it; the owner's books have JPEG and PNG only), MOBI (fallback) and TXT. CBZ would no longer open; the library indexes it, but the owner has none. Choice: `MUPDF_FORMATS=minimal` (default) or `full`.
- **DjVu.** None in the library. `PLATO_DJVU=1` brings it back.
- **Fonts in `data/`.** `FONT_SUBSET=latin-greek` (default) cuts Literata and Noto Sans to Latin (all of it), Greek and Coptic, Greek Extended, combining marks, general punctuation, super- and subscripts, currency, letterlike symbols, number forms, arrows, mathematical operators, geometric shapes and the f-ligatures, with every OpenType feature and Literata's hinting (the app UI hints). Cyrillic goes. Libertinus Serif, which Plato opens but never draws because `Book` replaces the serif family with Literata, ships as a stub with only the space. Source Code Pro and Parisienne are unmodified: their licences reserve their names for the original fonts. `FONT_SUBSET=none` ships everything.
- **Hyphenation.** `pl`, `en-us`, `en-gb` (some books mark English passages `en-GB`), `grc` and `la`; German and French went.

**Rendering check.** The host build before and after rendered every page of Platon's *Fedon* (EPUB, 341 pages), Arystoteles' *Etyka nikomachejska* (Polish EPUB with some Greek, 269), the MOBI made from *Fedon* by `make-test-mobi.py` (converted, 297; and through MuPDF, first 400 pages), a TXT with Polish and polytonic Greek (93), two PDFs (zlib's manual, non-embedded fonts, 2; LittleCMS tutorial, 60), a PDF with non-embedded base-14 fonts and Polish and Greek glyphs (1), and a synthetic EPUB with polytonic Greek, Latin, Polish and symbols (8). The raw frames are byte-identical except:

- The symbols page of the synthetic EPUB: ★ ✓ ⬜ ♪ ☞ ∞ now come from DejaVu Sans instead of Noto, so they look slightly different, and 中文 shows `.notdef` boxes (it came from Droid Sans Fallback). Accepted: the owner reads no CJK.
- The MOBI read through MuPDF (every rendered page): MuPDF's HTML layout uses Times (URW Nimbus Roman) instead of Charis SIL, so the book reflows (469 to 428 pages). This path only serves MOBI files the converter can't take; the text stays complete, Polish included.

The app's UI uses the same subset Literata (mapped from `data/serif/`): `aristotle-ui-preview` built with the subset files in place of the full ones gives the same 48 PNGs, day, night and icons, pixel for pixel. The snapshot key hashes the sizes and mtimes of `data/fonts`, `serif`, `css` and `hyphenation-patterns`, so the new data invalidates old snapshots once; `build-render.sh` keeps the source files' mtimes, so rebuilding the same data does not.

Host times (best of five, Fedon and Arystoteles): render median 2.1 and 2.8 ms before, 2.1 and 2.7 ms after; pagination unchanged within noise. Maximum RSS for Arystoteles fell from 25.4 to 23.5 MiB. Profile measurements: opt-level "s" everywhere cost about 5 % per page on the host, "z" 75 %; the split above stays within noise of opt-level 3.

**Not done:**

- `-C target-cpu=cortex-a9` for Rust. Safe on this device: `/proc/cpuinfo` lists `neon vfpv3 vfpd32` (a Cortex-A9, part 0xc09), and the C libraries are already built with `-mcpu=cortex-a9 -mfpu=neon`. Size is neutral (app +0.04 MB, renderer -0.06 MB), and its speed can only be judged on the device, so it is off until a `bench` run there shows a gain [dev, pending].
- What is left, per the symbol sizes of the app: MuPDF 1.7 MB (0.9 MB of it fonts), HarfBuzz 0.56, FreeType 0.46, MuPDF's HTML parser Gumbo 0.29 (needed for TXT and the MOBI fallback; reading TXT with Plato's engine would drop it and MuPDF's HTML layout), `regex` 0.38 (Plato), the UI's rasteriser stack (zeno, skrifa, swash) 0.7.

How to build the small set (both defaults):

```sh
PLATO_BUILD=~/.cache/aristotle/plato-build/size PLATO_LIBS_ONLY=1 tools/build-plato.sh
PLATO_BUILD=~/.cache/aristotle/plato-build/size tools/build-render.sh device
```

`MUPDF_FONTS=full MUPDF_FORMATS=full PLATO_DJVU=1` and `FONT_SUBSET=none HYPH_LANGS="..."` bring the removed parts back. MuPDF gets one output directory per font and format choice and `PLATO_DJVU` only changes what is collected, so those switch in place; a change to the other C libraries (flags, FreeType modules) needs a new `PLATO_BUILD` directory.

## Open issues

- **Device timings unknown.** Glyphs are rasterised by FreeType on every render (Plato has no glyph cache) and blended pixel by pixel; chapter layout runs on the turn that enters the chapter. If either is too slow on the Cortex-A9: render the next page while the current one settles, lay out the next chapter ahead, add a glyph cache.
- **Pagination cost.** The CLI still lays out the whole book for `x / y`; the reader app caches it per book and style and computes misses in the background. Until then the footer shows only the percentage.
- **Reader app on the device.** Not run on the real panel yet: tap-to-submit, prerender and open times, memory with two copies of a book during background pagination, and ghosting after menu use need the device. The library clock relies on musl reading `/etc/localtime` (a link to `/var/local/system/tz` on the Kindle) when `TZ` is unset [inference].
- **Memory.** Plato keeps the display lists of every laid-out chapter. Host RSS is 26 to 32 MiB after full pagination; check on the device against the 40 MB target.
- **Polish typographic rules.** Single-letter words (a, i, o, u, w, z) can end a line; Polish typesetting keeps them with the next word. Needs a non-breaking space pass before layout.
- **MOBI.** No table of contents yet (NCX index records), so the header shows the book title; `filepos` links are dropped; HUFF/CDIC books are not converted.
- **Binary size.** Cut from 29.5 to 8.4 MB, see [Size budget](#size-budget), which also lists what is left to cut.
- **Images.** Book CSS decides placement; `margin: auto` centring is not honoured by Plato's engine.
