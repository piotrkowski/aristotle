# Stock Kindle vs Aristotle

Aristotle has to be measurably better than the stock UI, not just different. This document holds the stock baseline, the method that produces both sides' numbers, and the targets. Every number comes from `aristotle-observe` on the test PW4 (firmware 5.13.6) unless marked otherwise.

## Method

`aristotle-observe` (crate `aristotle-observe`) measures any UI, stock or ours, the same way:

- It turns on the EPDC driver's own debug log (`mxc_epdc_debug` in sysfs). That logs every panel update with waveform, region, LUT and start and end times. The driver is the ground truth for when pixels change, whoever asked for them.
- It reads `/dev/kmsg` and the touchscreen side by side, without grabbing the touchscreen, and pairs every touch interaction with the updates that follow it until the screen settles (no update for 1.5 s).
- It injects touches by writing evdev events into the real touchscreen node, so the same script drives the stock UI and ours.

Scripts live in `devloop/kindle/scenarios/` (`tap`, `hold`, `swipe`, `wait`, `label`, `go-home`, `snap`). Each scenario should start from a known state (`go-home`) and verify its steps with `snap` screenshots. Numbers without visual verification turned out to be wrong more than once.

Metrics per interaction:

| Metric | Meaning |
| --- | --- |
| first✓ | release to the first update *finished*: the earliest moment the user sees the result |
| settled | release to the last update finished: the screen is calm again |
| upd | number of panel updates the interaction caused |
| flash | full GC16/GCK16 updates (visible black flash) |
| area% | total updated area relative to the screen |

### Injection pitfalls (learned the hard way)

The PW4's Goodix touchscreen declares only multi-touch axes, and the input core silently drops anything else. A synthetic contact needs:

- slot, tracking id **within 0–255**, position, pressure and touch or width major;
- no `BTN_TOUCH` and no `ABS_X`/`ABS_Y`, which the device does not have.

Without pressure and size, taps only worked when slot 0 still held values from the last real finger. Ids above 255 are ignored by the stock UI. `aristotle-observe caps` prints the declared axes and ranges.

## Stock baseline (2026-10-17)

| Interaction | first✓ | settled | Updates | Notes |
| --- | --- | --- | --- | --- |
| Home: open the ⋮ menu | 425 ms | 1433 ms | 3 | `auto` then `gck16` |
| Home: close the menu | 522 ms | 785 ms | 2 | |
| Home: open a book | 275–314 ms | 772–879 ms | 2 | the book appears in the second update |
| Reader: next page | **564–588 ms** (steady state) | **826–890 ms** | 2 | full page (104 % area) in `glkw16`, then a small `auto` update, presumably the footer |
| Reader: first page turn after opening | 1027 ms | 1315 ms | 2 | |
| Reader: toolbar | 239–264 ms | 748–780 ms | 2 | |

Other stock observations:

- **Stability.** During one scripted session (opening a book, 8 page turns forward and some back, toolbar), Amazon's process monitor (`pmond`) fired `processTooLarge` for the Java framework, whose heap sat at its 49 MB cap (`alloc=51642368` of `max=51642368`), and the framework restarted to the boot splash. Some page turns in that run were lost.
- **Memory.** The stock GUI stack uses about 200 MB of 490 MB (`cvm` alone 125 MB RSS); see `pw4-reference.md`.
- **Typesetting.** The stock reader does not hyphenate Polish, which leaves very wide word gaps in justified text and some lines visibly unjustified. It shows no header or progress on the page itself.
- **Clutter.** Home shows a store button, a "Find your next great read" store card and a reading-list promotion.

## Page-turn strategy experiment

`aristotle-bench --bin pageturn` alternates two full-screen pages of real text: a dump of a page of a Polish novel as the stock reader rendered it, and the same page shifted by 37 rows so every glyph changes. It runs with the stock GUI stopped and reports the median of 6 turns (two runs agreed within 5 ms):

| Strategy | Visible | Settled |
| --- | --- | --- |
| Stock reader (for reference) | 565 ms | 830 ms |
| GL16, one update | 479 ms | 479 ms |
| GLR16 (REAGL), one update | 495 ms | 495 ms |
| DU, black and white text only | 291 ms | 291 ms |
| DU, then GL16 | 291 ms | 771 ms |
| **A2, then GL16** | **150 ms** | **630 ms** |

"Visible" means the first update showing the new page has finished. In the two-step strategies the page is first thresholded to pure black and white, then upgraded to the anti-aliased page. Blitting a page into the framebuffer takes well under a millisecond (`Framebuffer::blit`).

In a slow demo of all five (`devloop/kindle/pageturn-demo.sh`), all of them looked fast, with no objectionable ghosting or flicker. That demo was misleading: its two pages differ only by a 37-row shift, so little residue could build up.

### What real reading showed (superseding the first decision)

Reading a whole novel (`aristotle-app`, then `aristotle-render`) showed heavy ghosting and "shadowing" with A2 then GL16. Two mistakes compounded:

- Both updates were **partial**. A partial update drives only pixels whose value changes, so pixels the A2 pass had already set to white kept the previous page's ghost.
- **A2 leaves residue** that GL16 does not clear, even as a full update. A2 preview followed by a full mode-9 pass still looked like "drawing twice", so it was rejected.

The driver log of the stock reader then showed what it does on every page turn: **one full-mode update of the whole screen with waveform mode 9** (`glkw16`, which FBInk calls a night-mode waveform, but stock uses it in day mode), and **mode 8** (`gck16`) for its flashing full refresh. Both trigger the driver's extra "Algorithm processing" and "ACD data" steps, Lab126's ghost reduction. Sending the identical request from our code makes ours indistinguishable in the log, and the ghosting was gone on the panel.

With the next page rendered while the panel refreshes (`Screen::submit`/`finish`), a turn costs only the panel. Measured on the same novel, median of 6 turns:

| Single-step turn | Tap to page shown | Settled |
| --- | --- | --- |
| Stock reader | 565 ms | 830 ms |
| **Aristotle, mode 9 full (stock-identical update)** | **503 ms** | **503 ms** |
| Aristotle, DU4 (mode 7) full | 317 ms | 317 ms |

Rendering the next page takes about 106 ms on the device and fits inside the panel time.

**Decision:** page turns use one full mode-9 update of the whole screen, the stock reader's own request, with the next page prerendered. That is as clean as stock and 11 % faster to the page and 40 % faster to a calm screen, since stock sends a second update for its footer. DU4 stays a setting for anyone who prefers speed (about 1.8 times faster than stock), at some cost in gray levels. Two-step turns with an A2 preview are off.

## Targets for Aristotle

| Interaction | Stock | Target | How |
| --- | --- | --- | --- |
| Page turn, visible | 565 ms | **≤ 510 ms** (503 ms measured; 317 ms with DU4); about 440 ms from the release | One full mode-9 update like stock, next page prerendered, turn triggered on the tap's release (a hold selects text and must not turn) |
| Page turn, settled | 830 ms | ≤ 510 ms (503 ms measured) | The footer is drawn in the same frame, so there is no second update |
| Touch feedback | none before the result | ≤ 150 ms | A2 highlight on touch-down (`aristotle-ui`) |
| Open a book | 0.8 s settled | ≤ 0.6 s (2.9 s measured for a long MOBI novel before snapshots [dev]; host: submit 1 ms after the release instead of 278 ms, so about 0.5 s with the 480 ms flash; device check pending) | The page left last time shown from a snapshot with one mode-8 flash; layout on a worker thread; page numbers cached per book (see `reader.md`, "Opening a book") |
| Menu or toolbar | 0.75–1.4 s settled | ≤ 0.5 s | DU-only UI chrome, no gray gradients |
| Flashes during reading | 0 in 8 turns | 0 by default, GC16 on demand or every N pages | Measured ghosting decides N |
| Stability | framework OOM restart observed | none | No JVM; static Rust; memory measured |
| RAM | about 200 MB | ≤ 40 MB | |
| Typesetting | no Polish hyphenation | Liang hyphenation for Polish, Knuth–Plass or a good greedy justification | |

These are commitments to measure, not claims. Each will be checked with the same scenarios once the Aristotle reader runs on the device.

## Transitions in the Aristotle app

Measured on the device on 2026-10-17 with `devloop/kindle/scenarios/aristotle-transitions.txt` (injected taps; `aristotle-observe` counts the taps it injects, since the app grabs the touchscreen). Times run from the finger lifting; page turns then fired on touch-down, so they showed 0 ms to the first update. Since turns fire on release (a long press on a word had turned the page first and flashed the next page before the definition), a turn's time from the release is the submit (the next page is prerendered) plus the panel's 440 ms; the target stays about 440 ms from the release, to be measured again with the same scenario.

| Interaction | Acknowledged | Settled | Updates | Flash | Waveforms |
| --- | --- | --- | --- | --- | --- |
| Page turn (forward or back) | 440 ms (the page) | 440 ms | 1 | no | glkw16 full |
| Reader menu open | 513–577 ms (the menu) | 513–577 ms | 1 | no | gl16, 95 % of the screen |
| Reader menu close | 497 ms | 497 ms | 1 | no | glkw16 full |
| Book → library | 52 ms | 765 ms | 2 | yes | a2 tap mark, gck16 full |
| Library filter tab | 60–64 ms | 552–741 ms | 2 | no | a2, gl16 |
| Shelf page | 51–53 ms | 516–535 ms | 2 | every 5th | a2, gl16 (gck16 when flashing) |
| Sort | 59–61 ms | 743–749 ms | 2 | no | a2, gl16 |
| Open the "Continue reading" book | 67 ms | 1282 ms | 2 | yes | a2, gck16 full |

What this shows, to fix next:

- Every action ends in exactly one update of its final state, and library controls are acknowledged within about 60 ms. No intermediate states were seen.
- The menu opens with no acknowledgement: nothing changes for half a second. Its top bar and bottom panel are coalesced into one bounding box of 95 % of the screen, so the unchanged text between them is redrawn too.
- Opening a book takes 1.3 s to settle (stock: 0.8 s); laying out the chapter dominates.
- The periodic shelf flash fires on a count, not on content, so it can feel random.

## Device measurements of the app

Measured on the device on 2026-10-17 with the app's own timing log (`[perf]` lines in `/var/tmp/aristotle.log`) and `/proc/<pid>/status`.

| What | Result | Stock |
| --- | --- | --- |
| Open a book from its snapshot, tap to settled | 557–570 ms | 0.8 s |
| Back to the library, settled | 552 ms | not measured |
| Wake from the screensaver, settled | 441 ms | not measured |
| Dictionary lookup, Polish (pl-pl, 1.37 million forms) | 89 ms; popup 239 ms after the long press | not measured |
| Dictionary lookup, Ancient Greek (grc-pl) | 10.5 ms; popup 71 ms after the long press | not measured |
| Footnote popup | 2 ms to resolve the link | not measured |
| Memory while reading (`VmRSS`) | 14.5 MB | about 200 MB for the stock GUI stack |
| Peak memory (`VmHWM`) | 66 MB, during the one-time indexing of five dictionaries | |
| One-time dictionary indexing | pl-pl 22.9 s, grc-en 37.2 s, the others under 3.1 s each | |
