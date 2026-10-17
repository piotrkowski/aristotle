# Stock Kindle vs Aristotle

Aristotle has to be measurably better than the stock UI, not just different. This document holds the stock baseline, the method that produces both sides' numbers, and the targets. Every number comes from `aristotle-observe` on our PW4 (firmware 5.13.6) unless marked otherwise.

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

`aristotle-bench --bin pageturn` alternates two full-screen pages of real text: a dump of the stock reader's own page of "Pan Tadeusz", and the same page shifted by 37 rows so every glyph changes. It runs with the stock GUI stopped and reports the median of 6 turns (two runs agreed within 5 ms):

| Strategy | Visible | Settled |
| --- | --- | --- |
| Stock reader (for reference) | 565 ms | 830 ms |
| GL16, one update | 479 ms | 479 ms |
| GLR16 (REAGL), one update | 495 ms | 495 ms |
| DU, black and white text only | 291 ms | 291 ms |
| DU, then GL16 | 291 ms | 771 ms |
| **A2, then GL16** | **150 ms** | **630 ms** |

"Visible" means the first update showing the new page has finished. In the two-step strategies the page is first thresholded to pure black and white, then upgraded to the anti-aliased page. Blitting a page into the framebuffer takes well under a millisecond (`Framebuffer::blit`).

In a slow demo of all five (`devloop/kindle/pageturn-demo.sh`), the owner judged them all fast and saw no objectionable ghosting or flicker.

**Decision:** the default page turn is A2, then GL16, so the page is readable about 3.8 times sooner than on stock. GL16 alone and REAGL stay available as settings. A full GC16 refresh every N pages is a setting too; N will be tuned by watching ghost build-up over long reading sessions.

## Targets for Aristotle

| Interaction | Stock | Target | How |
| --- | --- | --- | --- |
| Page turn, visible | 565 ms | **≤ 200 ms** (150 ms measured on real text) | A2 black and white page first, then a GL16 anti-aliasing upgrade |
| Page turn, settled | 830 ms | ≤ 700 ms (630 ms measured) | One GL16 upgrade; no separate footer update, because the footer is drawn in the same frame |
| Touch feedback | none before the result | ≤ 150 ms | A2 highlight on touch-down (`aristotle-ui`) |
| Open a book | 0.8 s settled | ≤ 0.6 s | Layout cached per book; first page rendered from cache |
| Menu or toolbar | 0.75–1.4 s settled | ≤ 0.5 s | DU-only UI chrome, no gray gradients |
| Flashes during reading | 0 in 8 turns | 0 by default, GC16 on demand or every N pages | Measured ghosting decides N |
| Stability | framework OOM restart observed | none | No JVM; static Rust; memory measured |
| RAM | about 200 MB | ≤ 40 MB | |
| Typesetting | no Polish hyphenation | Liang hyphenation for Polish, Knuth–Plass or a good greedy justification | |

These are commitments to measure, not claims. Each will be checked with the same scenarios once the Aristotle reader runs on the device.
