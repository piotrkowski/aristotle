# The reader: rendering book pages

`crates/aristotle-render` is Aristotle's first e-book renderer. It uses the document engine of [Plato](https://github.com/baskerville/plato) (`plato-core`) and none of Plato's Kobo UI, and turns books into full-screen 8-bit pages for the PW4 panel. Its CLI, `aristotle-render`, renders pages to files on any host, shows them on the device with the page-turn strategy from [`native-vs-aristotle.md`](native-vs-aristotle.md), and measures CPU and panel time.

Status: renders EPUB, MOBI (DRM-free MOBI 7) and PDF; the CLI runs on the device. The reader app (`aristotle`, see [The reader app](#the-reader-app)) passed a smoke test in the emulator and awaits its first run on the panel. Numbers below are host numbers unless marked [dev].

## Architecture

```text
aristotle-render (CLI: info, render, show, bench, turn)
  └─ Book ─────────── plato-core
  │    EPUB, HTML ──── Plato's HTML engine (our font, Knuth–Plass, Liang hyphenation)
  │    MOBI ────────── mobi.rs → cached EPUB → Plato's HTML engine (MuPDF as fallback)
  │    PDF, CBZ, ... ─ MuPDF through Plato's PdfOpener, scaled to fit
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
  - `0008`: lazy layout of spine items, see "Lazy layout".
- **C libraries.** `tools/build-plato.sh` builds them with Zig as static archives: for the device as before, and with `PLATO_TARGET=x86_64-unknown-linux-musl PLATO_LIBS_ONLY=1` for the host (about 6 minutes on 2 cores, once).
- **Workspace.** `crates/aristotle-render` is its own Cargo workspace and the root workspace excludes it, so `cargo build` at the root keeps working on hosts that never ran the Plato build. It depends on `../../third_party/plato-patched/crates/core` by path and on `aristotle-hal`. Its `Cargo.lock` started from Plato's, so dependency versions match the tested Plato build.
- **Build.** `tools/build-render.sh device|host` prepares the source, sets `PLATO_LIB_DIRS` and the Zig C compiler (for `zstd-sys`), builds, and assembles the bundle in `~/.cache/aristotle/render/<device|host>/`. `CARGO_CMD=clippy|test` runs those instead. `PLATO_BUILD` selects a `build-plato.sh` output directory (default `plato-build/main`, or `host-x86_64`).

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
| `fonts/` | The 12 fonts Plato's engine loads unconditionally (Libertinus Serif, Noto Sans, Source Code, Parisienne, Delius) | Plato source tree |
| `serif/` | Literata, the body and chrome font | `crates/aristotle-render/assets/fonts/` (OFL, committed with its licence) |
| `css/` | `epub.css`, `html.css` | Plato source tree |
| `hyphenation-patterns/` | `pl`, `en-us`, `en-gb`, `de-1996`, `fr` (`HYPH_LANGS`) | Plato 0.9.45 release archive, pinned by SHA-256 |

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
- Plato loads every pattern file present in `hyphenation-patterns/` on first use (9 ms on the host for five languages), which is why the bundle ships only a few.

## MOBI

MuPDF reads MOBI, but its HTML layout has no Polish patterns, uses its own fonts and justifies poorly. So `mobi.rs` converts DRM-free MOBI 7 books once to a stored (uncompressed) EPUB in the cache and Plato's engine lays that out like any EPUB, with the language from EXTH 524 (or the header's locale). It handles PalmDOC and uncompressed text, UTF-8 and CP1252, trailing record entries, images, `<mbp:pagebreak/>` chapter splits, MOBI's paragraph attributes, and the HTML that Plato's XML parser can't take (void elements, unquoted attributes, unclosed paragraphs). DRM, HUFF/CDIC compression and KF8-only files fall back to MuPDF; joint MOBI/KF8 files use their MOBI 7 part. `--no-convert` forces MuPDF.

The host has no MOBI files; `devloop/host/make-test-mobi.py` makes a MOBI 7 test book from an EPUB.

## Pages and progress

For Plato's engine a location is a byte offset into the book's text. Pages are found by walking `Location::Next`, which lays out a spine item (chapter file) as far as needed and caches its pages (see "Lazy layout" below). `x / y` needs the whole book laid out: `render`, `show` and `turn` paginate first unless `--percent` is given. Progress is `x / y` once paginated, else the byte offset over the book's size. For MuPDF documents locations are page indices and the count is known.

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

**Lazy layout.** Plato laid out a whole spine item before it could place a position in it or draw any of its pages, so "Pan Tadeusz" cost 2.2 to 2.9 s on the device for every open without a snapshot, every placement on the worker and every font, spacing or margin change, wherever the reader was in it. Patch `0008` makes the engine's block loop stop between two children of a block (never inside a paragraph or a table) once two complete pages follow the location asked for, keeping one frame per nesting level (next child, loop context, rectangles); `EpubDocument` keeps the parsed chapter and the layout state, and resumes when a later page is asked for, 8 pages ahead. Resuming replays exactly what a full layout does, so the cost of placing a position is now proportional to how far into its chapter it is, and page breaks do not move: `aristotle-render pages BOOK 20` (every page start plus 20 placements in fresh copies) gives identical output for 39 books (the "Pan Tadeusz" stand-in, Lalka and 13 other converted MOBIs, the 23 Platon EPUBs, a test book with nested blocks, lists, quotes and tables) against an unpatched build, and rendered pages match byte for byte. Going back into a previous spine item still lays it out whole. Host, the "Pan Tadeusz" stand-in on one CPU (`aristotle-render place`, before → after):

| Position in the chapter | Place | Restyle (+1 pt) and place | Next page |
| --- | --- | --- | --- |
| 0 % | 261 → 9 ms | 225 → 5 ms | 3 → 3 ms |
| 2 % | 230 → 7 ms | 224 → 9 ms | 3 → 8 ms |
| 25 % | 229 → 62 ms | 229 → 59 ms | 3 → 11 ms |
| 50 % | 234 → 121 ms | 233 → 129 ms | 2 → 7 ms |
| 95 % | 235 → 226 ms | 228 → 224 ms | 3 → 7 ms |

A whole pagination costs the same as before (225 → 234 ms for the stand-in, 1357 → 1307 ms for Lalka). On the device, where "Pan Tadeusz" takes 2.5 s whole, the place step should scale the same way (host numbers until verified). A restyle whose chapter text before the page exceeds 48 KB (about 300 ms on the device) shows the busy mark at the right of the menu's top bar with the button's release, one DU update, before the new page's full mode-9 update.

**Splitting huge chapters instead** (not done). The converter could cut a one-chapter MOBI into spine items of, say, 50 KB. But a spine item boundary is a forced page break: every cut would end a page early, leaving a part-empty page every 20 pages or so, at places that differ from both the stock reader and the book; cutting only at scene breaks would hide some of that. Lazy layout gives the same gain near the start of a chapter without moving any page break, and less towards its end.

### Running it on the device

```sh
OUT=~/.cache/aristotle/render/device PLATO_BUILD=~/.cache/aristotle/plato-build/run2 tools/build-render.sh device
ssh -F devloop/host/ssh_config kindle 'tar -xzf - -C /mnt/us' < ~/.cache/aristotle/render/aristotle-app-device.tar.gz
ssh -F devloop/host/ssh_config kindle sh /mnt/us/aristotle/run-aristotle.sh            # mode 9
ssh -F devloop/host/ssh_config kindle sh /mnt/us/aristotle/run-aristotle.sh --turn-mode 7
```

The app bundle (`aristotle/` with the binary, `data/`, `run-aristotle.sh` and `takeover.sh`) goes to the userstore: the binary is 29.5 MB, and with the renderer CLI's bundle in `/var/tmp` the 64 MB tmpfs would overflow. `run-aristotle.sh` stops the stock GUI with `takeover.sh start`, runs the app, and restarts the GUI with `takeover.sh stop` (30 to 60 s of boot splash). `pause`/`resume` (SIGSTOP) is not used: on our firmware the frozen framework was killed by Amazon's watchdog and the GUI did not come back without a reboot. The script ignores hangups, logs stderr to `/var/tmp/aristotle.log` and prints it on exit. Other options: `--library DIR`, `--root DIR`, `--data DIR`, `--no-scan`, `--night-method software|hardware`.

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

## Open issues

- **Device timings unknown.** Glyphs are rasterised by FreeType on every render (Plato has no glyph cache) and blended pixel by pixel; chapter layout runs on the turn that enters the chapter. If either is too slow on the Cortex-A9: render the next page while the current one settles, lay out the next chapter ahead, add a glyph cache.
- **Pagination cost.** The CLI still lays out the whole book for `x / y`; the reader app caches it per book and style and computes misses in the background. Until then the footer shows only the percentage.
- **Reader app on the device.** Not run on the real panel yet: tap-to-submit, prerender and open times, memory with two copies of a book during background pagination, and ghosting after menu use need the device. The library clock relies on musl reading `/etc/localtime` (a link to `/var/local/system/tz` on the Kindle) when `TZ` is unset [inference].
- **Memory.** Plato keeps the display lists of every laid-out chapter. Host RSS is 26 to 32 MiB after full pagination; check on the device against the 40 MB target.
- **Polish typographic rules.** Single-letter words (a, i, o, u, w, z) can end a line; Polish typesetting keeps them with the next word. Needs a non-breaking space pass before layout.
- **MOBI.** No table of contents yet (NCX index records), so the header shows the book title; `filepos` links are dropped; HUFF/CDIC books are not converted.
- **Binary size.** 25 MB for ARM, mostly MuPDF's embedded fonts. MuPDF build flags (`TOFU_*`, no CJK) can cut it.
- **Images.** Book CSS decides placement; `margin: auto` centring is not honoured by Plato's engine.
