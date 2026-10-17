# Stock Kindle vs Aristotle

This document records the stock UI's baseline, how both UIs are measured, Aristotle's targets and its results so far. Numbers come from `aristotle-observe` on the test PW4 (firmware 5.13.6) unless marked otherwise.

## Method

`aristotle-observe` measures the stock UI and ours the same way.

- It turns on the EPDC driver's debug log (`mxc_epdc_debug` in sysfs), which records each panel update with its waveform, region, LUT, start and end. The driver log is the ground truth for when pixels change.
- It reads `/dev/kmsg` and the touchscreen without grabbing it, and pairs each touch with the updates that follow until the screen settles (no update for 1.5 s).
- It injects touches as evdev events into the real touchscreen node, so one script drives either UI.

Scenarios live in `devloop/kindle/scenarios/` (`tap`, `hold`, `swipe`, `wait`, `label`, `go-home`, `snap`). A scenario starts from `go-home` and checks its steps with `snap` screenshots, because unchecked numbers were wrong more than once.

| Metric | Meaning |
| --- | --- |
| first✓ | release to the first update *finished*: the earliest moment the user sees the result |
| settled | release to the last update finished: the screen is calm again |
| upd | number of panel updates the interaction caused |
| flash | full GC16/GCK16 updates (visible black flash) |
| area% | total updated area relative to the screen |

The Goodix touchscreen declares only multi-touch axes, and the input core drops anything else. A synthetic contact needs a slot, a tracking id from 0 to 255, a position, pressure and touch or width major, and no `BTN_TOUCH`, `ABS_X` or `ABS_Y`. The stock UI ignores ids above 255. `aristotle-observe caps` prints the declared axes.

## Stock baseline (2026-10-17)

| Interaction | first✓ | settled | Updates | Notes |
| --- | --- | --- | --- | --- |
| Home: open the ⋮ menu | 425 ms | 1433 ms | 3 | `auto` then `gck16` |
| Home: close the menu | 522 ms | 785 ms | 2 | |
| Home: open a book | 275–314 ms | 772–879 ms | 2 | the book appears in the second update |
| Reader: next page | **564–588 ms** (steady state) | **826–890 ms** | 2 | full page (104 % area) in `glkw16`, then a small `auto` update, presumably the footer |
| Reader: first page turn after opening | 1027 ms | 1315 ms | 2 | |
| Reader: toolbar | 239–264 ms | 748–780 ms | 2 | |

- In one session (a book, 8 page turns forward and some back, the toolbar), `pmond` fired `processTooLarge` for the Java framework at its 49 MB heap cap (`alloc=51642368` of `max=51642368`). The framework restarted to the boot splash, and some page turns were lost.
- The stock GUI stack uses about 200 MB of 490 MB, `cvm` alone 125 MB RSS.
- The stock reader does not hyphenate Polish, so justified text has wide gaps and some unjustified lines. The page shows no header or progress.
- Home shows a store button, a "Find your next great read" store card and a reading-list promotion.

## Page-turn strategy experiment

`aristotle-bench --bin pageturn` alternates two full-screen pages with the stock GUI stopped: a Polish novel page as the stock reader rendered it, and the same page shifted by 37 rows. Times are the median of 6 turns, and two runs agreed within 5 ms.

| Strategy | Visible | Settled |
| --- | --- | --- |
| Stock reader (for reference) | 565 ms | 830 ms |
| GL16, one update | 479 ms | 479 ms |
| GLR16 (REAGL), one update | 495 ms | 495 ms |
| DU, black and white text only | 291 ms | 291 ms |
| DU, then GL16 | 291 ms | 771 ms |
| **A2, then GL16** | **150 ms** | **630 ms** |

"Visible" is the end of the first update that shows the new page. Two-step strategies show a black-and-white page first. Blitting a page takes well under a millisecond (`Framebuffer::blit`). A slow demo of all five (`devloop/kindle/pageturn-demo.sh`) looked fine, but its two pages differed only by the shift, so little residue built up.

### What real reading showed

A whole novel read with A2 then GL16 ghosted heavily. Both updates were partial, so pixels already white after A2 kept the previous page's ghost. A2 also leaves residue that GL16 does not clear, even as a full update.

The driver log showed that the stock reader turns a page with one full-mode update of the whole screen in mode 9 (`glkw16`), and flashes with mode 8 (`gck16`). FBInk calls mode 9 a night-mode waveform, and stock uses it in day mode. Both modes trigger the driver's "Algorithm processing" and "ACD data" steps, Lab126's ghost reduction. Our code now sends the identical request, and the ghosting is gone. The next page renders while the panel refreshes (`Screen::submit`/`finish`). Same novel, median of 6 turns:

| Single-step turn | Tap to page shown | Settled |
| --- | --- | --- |
| Stock reader | 565 ms | 830 ms |
| **Aristotle, mode 9 full (stock-identical update)** | **503 ms** | **503 ms** |
| Aristotle, DU4 (mode 7) full | 317 ms | 317 ms |

Rendering the next page takes about 106 ms on the device, inside the panel time.

Page turns use the mode-9 update with a prerendered page. That is 11 % faster than stock to the page and 40 % faster to a calm screen, since stock sends a second update for its footer. DU4 remains a setting. It is about 1.8 times faster than stock and has fewer gray levels. A2 previews are off.

## Targets for Aristotle

Each target is to be checked with the same scenarios on the device.

| Interaction | Stock | Target | How |
| --- | --- | --- | --- |
| Page turn, visible | 565 ms | **≤ 510 ms** (503 ms measured; 317 ms with DU4); about 440 ms from the release | One full mode-9 update like stock, next page prerendered, turn on the tap's release (a hold selects text and must not turn) |
| Page turn, settled | 830 ms | ≤ 510 ms (503 ms measured) | The footer is drawn in the same frame, so there is no second update |
| Touch feedback | none before the result | ≤ 150 ms | A2 highlight on touch-down (`aristotle-ui`) |
| Open a book | 0.8 s settled | ≤ 0.6 s | The page left last time shown from a snapshot with one mode-8 flash; layout on a worker thread; page numbers cached per book (`reader.md`, "Opening a book") |
| Menu or toolbar | 0.75–1.4 s settled | ≤ 0.5 s | DU-only UI chrome, no gray gradients |
| Flashes during reading | 0 in 8 turns | 0 by default, GC16 on demand or every N pages | Measured ghosting decides N |
| Stability | framework OOM restart observed | none | No JVM; static Rust; memory measured |
| RAM | about 200 MB | ≤ 40 MB | |
| Typesetting | no Polish hyphenation | Liang hyphenation for Polish, Knuth–Plass or a good greedy justification | |

A long MOBI novel took 2.9 s to open before snapshots [dev]. With a snapshot the host submits 1 ms after the release instead of 278 ms, about 0.5 s with the 480 ms flash. The device measured 557–570 ms (below).

## Transitions in the Aristotle app

Device, 2026-10-17, `devloop/kindle/scenarios/aristotle-transitions.txt`. The app grabs the touchscreen, so `aristotle-observe` counts its own injected taps. Times run from the release.

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

In this run, turns fired on touch-down and showed 0 ms to the first update. They now fire on release, because a long press on a word turned the page before the definition appeared. From the release, a turn costs the submit plus the panel's 440 ms. The target stays about 440 ms and is to be measured again.

- Each action ends in one update of its final state. Library controls are acknowledged within about 60 ms.
- The menu shows nothing for half a second. Its top bar and bottom panel coalesce into one box of 95 % of the screen.
- A book takes 1.3 s to open against stock's 0.8 s, mostly chapter layout.
- The shelf flash fires on a count, so it can feel random.

## Device measurements of the app

Device, 2026-10-17, from the app's `[perf]` lines in `/var/tmp/aristotle.log` and `/proc/<pid>/status`.

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
