# UI framework: aristotle-ui

`crates/aristotle-ui` is Aristotle's own UI toolkit: a small, retained, pure-Rust UI designed for the PW4's e-ink panel. It draws into an 8-bit gray canvas, works out the smallest rectangles that changed and the waveform each one needs, and turns raw touch events into gestures. It does not depend on `aristotle-hal`, so it builds and tests on the host, and a desktop preview renders every screen to PNG.

Status: the toolkit and three prototype screens (library, reader page, reader menu) work on the host and cross-compile for the device. Nothing has run on the panel yet; see [Open issues](#open-issues).

## Architecture

```
touch events ──► input::GestureRecognizer ──► Ui::handle_touch ──► UiEvent (Action, Tap, Swipe…)
                                                   │                      │
                                                   ▼                      ▼
                                           press feedback (A2)     screen code mutates the Tree
                                                                          │
Tree (nodes, layout, paint, hit test) ◄───────────────────────────────────┘
   │
   ▼
Ui::render ── layout ─► candidate rects ─► repaint into Canvas ─► diff against front ─► Damage {rect, Waveform}
                                                                                              │
                              device: Canvas::blit_into(framebuffer) + Display::damage ◄──────┘
                              host:   PNG via aristotle-ui-preview
```

| Module | What |
| --- | --- |
| `canvas` | `Canvas { width, height, stride, pixels }`, 0 black, 255 white, explicit stride like the framebuffer (1088 bytes per 1072-pixel line on the device). Clipped `Painter` primitives, zeno-rasterized paths, pixel diffing, black-and-white detection, 16-level quantization. |
| `geom` | Integer `Rect`, `Size`, `Insets`; `mm()` and `pt()` at 300 DPI; `MIN_TARGET` = 9 mm = 106 px. |
| `waveform` | `Waveform::{A2, DU, GL16, GC16}` with the Zelda numbers of `aristotle_hal::framebuffer::Waveform` (6, 1, 3, 2), without depending on the HAL. |
| `text` | Fonts and families, swash shaping and hinted rasterization, hyphenation, Knuth–Plass paragraph layout. |
| `tree` | The retained node tree: box-constraint layout, painting, hit testing, mutation that records damage. |
| `ui` | `Ui`: owns tree, text engine, canvas and the copy of what the panel shows; computes damage; dispatches gestures. |
| `input` | Tap, long press and swipe recognizer over `(phase, id, x, y, time_us)` events. |
| `icons` | Vector icons on a 24-unit grid. |
| `widgets` | Custom leaves: typographic cover, battery, toggle, slider, size steps. |
| `theme` | Grays, margins and the type scale. |
| `screens` | Prototype library and reader screens with sample data. |

### The tree

Nodes live in an arena (`Vec<Node>`, addressed by `NodeId`). Kinds: `Column`, `Row` (with `gap`, cross-axis alignment and `flex` children), `Stack` (overlays), `Padding`, `Align`, `Fixed`, `MinSize`, `Text`, `Icon`, `Rule`, `Progress`, `Spacer`, `Pages` and `Custom(Box<dyn Widget>)`. Any node can carry a background, a rounded border, an `action` and a press `Feedback` (invert, outline or none).

Layout is one pass of box constraints, as in Flutter: the parent passes minimum and maximum sizes, the child chooses its size, the parent positions it. It is linear in the number of nodes; paragraph layouts are cached per width, so relayout after a text change only reshapes that paragraph.

`Pages` is the list widget. It fills one page with as many children as fit and hides the rest; `set_page` turns it. There is deliberately no scrolling container (see below).

Mutations go through the tree (`set_text`, `set_progress`, `set_hidden`, `set_page`, `set_inverted`, `widget_mut` + `invalidate`) so every change is recorded with a refresh class.

## E-ink UX principles

These rules shaped both the toolkit and the screens.

1. **Pages, never scrolling.** Every scroll step would be a panel update of 120 to 550 ms that leaves ghosting. A page turn is one clean update, and the user always knows where they are ("Page 1 of 3"). Lists use `Pages`; the reader paginates.
2. **Feedback on touch-down, in A2.** The gesture recognizer emits `Press` on the first contact, before it knows the gesture, and `Ui` inverts (or outlines) the target at once with A2, the 120–144 ms waveform. When the finger lifts, the real result is already on its way. If the finger travels, `Cancel` withdraws the highlight.
3. **9 mm targets.** Every action is at least 106 × 106 px, and smaller visual elements are hit-tested as if they were (`Rect::expand_to`). On e-ink a missed tap is expensive: you wait a full refresh to see that nothing happened.
4. **Black, white and few grays.** The panel shows 16 levels and light grays wash out under the frontlight. Hierarchy comes from size, weight and space; the lightest text is `INK_3` (0x6a). Controls are solid black and white, so their state changes qualify for DU. Progress fills are plain rectangles without anti-aliased ends, so a progress step is a DU update a few pixels wide.
5. **The result is the feedback for big gestures.** Reader tap zones and swipes do not highlight anything; the new page is the response.
6. **Calm screens.** Generous 6 mm margins, one title per screen, no badges, no carousels, nothing that exists to sell something.
7. **Night mode by inversion.** `Ui::set_night` inverts the final pixels, so every screen supports it and anti-aliasing stays correct, followed by one GC16.

## Text

### Font engine choice: swash

The candidates were evaluated by what they do for this device; sizes below are from our own armv7 build where marked, otherwise from crate scope (not measured).

| Crate | Shaping | Hinting | Rasterizer | Notes |
| --- | --- | --- | --- | --- |
| **swash** (chosen) | OpenType GSUB and GPOS: kerning pairs and classes, ligatures, marks, `smcp`, `onum`, `locl` | Yes, through skrifa (TrueType interpreter and autohinter) | Yes (zeno) | One crate for shaping, scaling and rendering. Measured on armv7: swash 243 KiB plus skrifa 271 KiB of `.text`. |
| rustybuzz + ttf-parser | Best available (HarfBuzz port) | No | No | Needs a separate rasterizer and still no hinting; two outline parsers in the binary. |
| cosmic-text | Via a HarfBuzz port | Via swash | Via swash | A full desktop text stack (font discovery, bidi, editing). Larger, and its greedy layout leaves no room for our justification. |
| fontdue | No (legacy `kern` table only) | No | Yes, small and fast | Literata, Charis and Gentium kern through GPOS, so fontdue would set them unkerned and without ligatures. |

swash gives the best quality per byte: real OpenType shaping and hinting in one pure-Rust dependency, and zeno, its rasterizer, also draws our icons and rounded shapes at no extra cost.

Hinting mode: swash 0.2 hints through skrifa in "smooth" mode, which snaps vertical features (baseline, x-height, horizontal stems) to the pixel grid and keeps horizontal metrics linear. That suits a 300 DPI panel: crisper x-height in 16 grays, while justified lines keep exact widths and glyphs sit at quarter-pixel horizontal positions (cached per phase).

Darkening: coverage goes through `coverage^0.8` (`TextEngine::set_darkening`). Unadjusted anti-aliased text looks thin and gray on e-ink next to the stock reader.

### Fonts

Bundled in `crates/aristotle-ui/assets/fonts/`, each with its OFL text:

| Family | Styles | Licence | Role |
| --- | --- | --- | --- |
| Literata | Regular, Italic, Bold, Bold Italic | SIL OFL 1.1 | UI and default reading face, compiled in (`embedded-fonts` feature, 1.22 MiB) |
| Charis | Regular, Italic | SIL OFL 1.1 (Reserved Font Names "Charis", "SIL") | Alternative reading face, loaded from disk |
| Gentium Book | Regular, Italic | SIL OFL 1.1 (Reserved Font Names "Gentium", "SIL") | Alternative reading face, loaded from disk |

The OFL allows bundling and redistribution with any software, AGPL included, as long as the fonts are not sold on their own and keep their licence; the reserved names only matter if we modify the fonts. Missing styles are synthesized: a missing bold is emboldened outlines (advances widened to match), a missing italic a 12° slant. A test checks that every bundled face covers `ĄĆĘŁŃÓŚŹŻąćęłńóśźż„”«»–—…`.

### Paragraph layout

`text::layout_paragraph` turns styled spans into lines:

1. Spans are flattened; soft hyphens (U+00AD) are removed and remembered as hyphenation points.
2. Break opportunities come from UAX #14 (`unicode-linebreak`), so dashes, quotes, slashes and non-breaking spaces behave as Unicode specifies.
3. Each segment is shaped whole (kerning inside a word survives hyphenation) and becomes Knuth–Plass items: boxes, stretchable glue for spaces, and penalties for hyphenation points and explicit hyphens. A hyphenation point inside a ligature cluster is skipped.
4. **Justified** text is broken with Knuth–Plass total fit, which chooses all breaks of a paragraph together to minimise uneven spacing, with penalties for hyphens, consecutive hyphens and tight-next-to-loose lines. If no solution fits the tolerance it retries looser, then falls back to first fit. **Ragged** text (UI labels) uses first fit, optionally **balanced** for headings (narrowest width with the same line count, so a title does not end in one lonely word).
5. Polish rules: a one-letter word (a, i, o, u, w, z) and an initial ("S.") never end a line.
6. `max_lines` truncates with an ellipsis, cutting inside a word if needed.

Hyphenation is a hook: `trait Hyphenator { fn hyphenate(&self, word, out) }`. `PolishSyllables` is a rule-based syllable hyphenator (digraphs, softening *i*, onset rules: Kra-kow-skie-go, pań-stwo, sio-stra, przy-szło-ścią); Polish spelling allows splitting consonant clusters almost anywhere, which makes a syllable rule unusually safe. It does not know prefixes (rare errors like *prze-dmie-ście* are possible). A pattern-based (Liang) hyphenator can implement the same trait.

The reader adds book typesetting on top: chapter sinkage, small-caps chapter number, balanced italic title, no indent after a heading, old-style figures, and pagination that keeps headings with the following text and never leaves an orphan or a widow (a page may end one line short instead).

## Damage and waveform hints

`Ui::render()` returns `Vec<Damage { rect, waveform }>`:

1. **Candidates.** Each recorded change contributes its node's rectangle (a target's whole box for press feedback); every painting node whose rectangle moved, appeared or disappeared during layout contributes the old and the new rectangle. Hiding or showing a subtree therefore damages only the nodes inside it that actually paint: opening the reader menu touches the two bars, not the page between them.
2. **Repaint.** Each candidate is repainted from the tree into the canvas with the painter clipped to it. Transient (press) regions are processed first, then larger before smaller.
3. **Tighten.** The repainted region is compared with `front`, the copy of what the panel shows. Only the bounding box of pixels that differ becomes damage; a repaint that produces the same pixels produces nothing. Rects already covered by a more faithful one are dropped. The result is available as `Ui::minimal_damage()`.
4. **Waveform per rect**, from why it changed:

   | Cause | Waveform | Why |
   | --- | --- | --- |
   | Press feedback (`Refresh::Transient`) | A2 (6) | Fastest, black and white only; replaced moments later. |
   | Content change whose new pixels are all 0 or 255 | DU (1) | Clean black and white at 309 ms instead of 544. |
   | Content change with grays (anti-aliased text, images) | GL16 (3) | 16 levels, no flash. Also used to restore after A2. |
   | New screen (`set_tree`), night mode, every 6th page turn | GC16 (2), full screen | Flashing refresh that clears accumulated ghosting. |

5. **Coalesce** (`Ui::coalesce`, default `Coalesce::Single`). The stock driver runs one update at a time and blocks while one is on the panel (`docs/eink-rendering.md`), and an update costs about a whole waveform whatever its size. So by default all non-A2 damage goes out as one rectangle with the most faithful waveform, and A2 feedback as a separate, first update. GL16 leaves unchanged white pixels alone, so the larger rectangle has no visible cost. `Coalesce::Nearby` (merge only overlapping or nearly adjacent rects) and `Coalesce::None` exist for a future driver that runs updates in parallel.

Rebuilding a screen does not need a flash: `Ui::set_tree_with(tree, Refresh::Auto)` repaints everything and sends only the pixels that differ, which is how the reader applies a new font size.

### Device hand-off

```rust
for d in ui.render() {
    ui.screen().blit_into(d.rect, fb_pixels, fb_stride);
    display.damage(hal_rect(d.rect), Waveform(d.waveform.mode()), &[]);
}
```

`aristotle_hal::display::Display` coalesces whatever arrives while the panel is busy, so the input loop never blocks. Touch events convert field by field from `aristotle_hal::input::TouchEvent`; poll with `ui.next_deadline(now)` as the timeout and call `ui.tick(now)` so a long press fires while the finger is still down.

## Input

`GestureRecognizer` thresholds (all in device pixels at 300 DPI): tap slop 4 mm, long press 500 ms, swipe at least 8 mm along a dominant axis (1.5 × the other). A second finger cancels the press and the rest of the touch is ignored (reserved for pinch). A long press is detected either by `tick` or, failing that, by the next event's timestamp.

Reader zones: the top 12 % opens the menu, the left 30 % goes back, the rest goes forward, because most turns are forward and that target should be the largest. Swipe left and right turn pages, swipe down opens the menu. While the menu is open, a tap on the visible page closes it.

## Screens and the stock UI

The screens are real, not only prototypes: the reader app (`crates/aristotle-reader`, see `docs/reader.md`) builds the library with `Library::build_shelf` from a `Shelf` (book to continue, books with status and cover thumbnails as `GrayImage`, filter, sort label, clock, battery) and lays `build_menu` over pages rendered by Plato (`Painter::blit_gray`). `MenuSpec` leaves out rows a caller cannot serve (font faces, night mode). All labels the owner reads are Polish.

Preview PNGs (from `aristotle-ui-preview`, quantized to 16 levels): `library.png`, `library-pressed.png`, `library-page2.png`, `reader.png`, `reader-page2.png`, `reader-zones.png`, `reader-menu.png`, `reader-menu-pressed.png`, `reader-menu-night.png`, `reader-night.png`, `reader-charis.png`, damage overlays `*-damage.png`, and `compare-library.png` (stock home on the left, ours on the right).

Compared with the stock home screen (`~/.cache/aristotle/screens/stock-home.png`, firmware 5.13.x):

- **No store.** Stock spends about a third of the screen on "Find your next great read" with a cart tile, a "store" button in the toolbar and a "Swipe up to see more for you" feed. Ours has none of it.
- **Back into the book in one obvious tap.** Stock shows the current book as a 170 × 235 px thumbnail between a large KUAL tile and an empty "reading lists" panel. Ours puts it first, large, with chapter, progress bar and time left, as one 9 mm-plus target.
- **One hierarchy.** Stock stacks a title bar, a four-button toolbar, a search field and three all-caps section headings. Ours has one title, one small-caps label and one tab row.
- **Pages instead of an endless scroll.** Stock has a scrollbar and a feed; ours shows four books per page with "Page 1 of 3" and 9 mm arrows.
- **Legible covers.** Stock renders photographic covers in muddy gray and labels progress with small corner badges. Ours uses crisp typographic covers in three solid tones and a progress bar with a percentage, or "New" and "Finished".
- **Quieter status line.** Time and battery only; no airplane icon or "Your Kindle".
- **Instant press feedback** in A2 on touch-down for every target.

The stock reader page was not captured in our screenshots, so the reader comparison is limited to what this toolkit does by design: Knuth–Plass justification, Polish hyphenation and orphan rules, widow and orphan control across pages, and a menu that leaves the page in place.

## Running the preview

```sh
export CARGO_TARGET_DIR=~/.cache/aristotle/target-ui TMPDIR=~/.cache/aristotle/tmp
cargo run --release -p aristotle-ui --bin aristotle-ui-preview      # writes ~/.cache/aristotle/ui-preview/*.png
cargo run --release -p aristotle-ui --bin aristotle-ui-preview -- OUT_DIR
cargo run --release -p aristotle-ui --example render-bench           # timings, also runs on the device
cargo test -p aristotle-ui
```

The preview prints, for every simulated step, how many minimal rectangles changed and what would be sent to the panel.

## Device build and size

```sh
cargo build --release --target armv7-unknown-linux-musleabihf -p aristotle-ui
cargo build --release --target armv7-unknown-linux-musleabihf -p aristotle-ui --example render-bench
```

No C dependencies; static musl binaries. Measured with the workspace release profile (`opt-level = "s"`, LTO, stripped):

| Binary | Size |
| --- | --- |
| Empty std program (baseline) | 311 KiB |
| `render-bench` (both screens, all of the toolkit) | 2.66 MiB |
| of which embedded Literata | 1.22 MiB |
| of which code and tables | about 1.1 MiB |

`.text` by crate (cargo-bloat): skrifa 271 KiB, std 257 KiB, aristotle-ui 254 KiB, swash 243 KiB, read-fonts 46 KiB, zeno 13 KiB. Building with `default-features = false` drops the embedded fonts (load them from the userstore with `Fonts::load_family`) and the PNG encoder.

Host timings (`render-bench`, x86-64, release): cold library frame 12 ms including font parsing and glyph rasterization, page turn 3.4 ms, opening the menu 11 ms. Device timings are still to be measured.

## Open issues

- **Not yet on the panel.** The device glue (blit plus `Display::damage`, touch conversion) is a few lines but has not run; ghosting, the GC16 interval and A2 legibility of inverted anti-aliased labels need checking on the real panel.
- **Hyphenation** is a syllable heuristic without prefix knowledge. A Liang pattern hyphenator (with freely licensed Polish patterns) should replace it for books.
- **Charis and Gentium** ship Regular and Italic only; bold is synthesized. Add the real bold files if the size is acceptable.
- **No bidi, no complex scripts tested** (script is fixed to Latin); fine for the current library, not general.
- **Reader content is sample data** (a lightly abridged opening of Prus's *Lalka*, public domain). The book model, EPUB parsing and real chapter navigation are the reader's job; the tap targets for contents, bookmark, search and sort are wired to actions but not implemented.
- **Settings change re-paginates from the chapter start**; keeping the exact reading position needs a text-offset model.
- **Library pager labels** need a `sync` after layout; a layout callback would remove that step.
