# Benchmarks: stock Kindle vs Aristotle

Device results use a PW4 on firmware 5.13.6 unless marked otherwise. These numbers also appear in the README. [Host results](#host-measurements) are separate. See [E-ink rendering](ui-framework.md#e-ink-rendering) for panel timings.

The tables below are historical measurements, not a benchmark of the current
commit. Their original observer included touch-down acknowledgements in
`first✓`. Follow [Compare reader latency](benchmark-latency.md) for new
release-driven measurements with replayable traces.

## Method

`aristotle-observe` measures both interfaces with the same method:

- It enables [EPDC](glossary.md#epdc) debug logging through `mxc_epdc_debug` in sysfs. The log records update regions, [waveforms](glossary.md#waveform), and completion times.
- It reads `/dev/kmsg` and touch events, pairing touches with updates until none arrive for 1.5 s or the next touch starts.
- It injects evdev touches into the real touchscreen node.

`tools/scenarios/` contains `tap`, `hold`, `swipe`, `wait`, `label`, `go-home`, and `snap` commands. Start from `go-home` and verify steps with screenshots.

Full flashes use [GC16](glossary.md#gc16) or [GCK16](glossary.md#gck16) waveforms, while stock page turns use [GLKW16](glossary.md#glkw16).

| Metric | Meaning |
| --- | --- |
| `first>` | release to the first update that starts after release |
| `first✓` | release to the earliest completion among updates that start after release |
| `page✓` | release to the earliest full-screen completion at or after release, including updates started during the touch |
| `settled` | release to the last completed update assigned to this touch |
| `upd` | number of panel updates the interaction caused |
| `pre` | updates that start before release, including touch-down highlights |
| `open` | updates without a completion record |
| `flash` | full GC16/GCK16 updates (visible black flash) |
| `area%` | total updated area relative to the screen |

`first✓` can measure a small acknowledgement, not the book page. `page✓`
checks the update rectangle, not its contents or waveform. Verify the page
and the single full-mode mode-9 update in the screenshot and driver log.
Touch-down updates contribute to update counts and `settled`, but not to
`first✓`. A full-screen update that spans release contributes to `page✓`
without clamping its start to zero. A small touch-down-only highlight has
no release sample.

Missing completion records produce `n/a`, not a fabricated completion at
submit time. A kernel log overrun invalidates all completion metrics in the
capture. Summary lines show the valid sample count for each metric. For an
even sample count, the median averages the middle pair. Compare the same
interaction labels, not the combined median of unrelated controls.

The Goodix touchscreen accepts multi-touch axes only. Inject a slot, tracking ID from 0 to 255, position, pressure, and touch or width major. Do not send `BTN_TOUCH`, `ABS_X`, or `ABS_Y`. Stock ignores IDs above 255. `aristotle-observe caps` lists supported axes.

## Stock baseline (2026-10-17)

| Interaction | `first✓` | `settled` | Updates | Notes |
| --- | --- | --- | --- | --- |
| Home: open the ⋮ menu | 425 ms | 1433 ms | 3 | `auto` then `gck16` |
| Home: close the menu | 522 ms | 785 ms | 2 | |
| Home: open a book | 275 to 314 ms | 772 to 879 ms | 2 | the book appears in the second update |
| Reader: next page | 564 to 588 ms (steady state) | 826 to 890 ms | 2 | full page (104 % area) in `glkw16`, then a small `auto` update, presumably the footer |
| Reader: first page turn after opening | 1027 ms | 1315 ms | 2 | |
| Reader: toolbar | 239 to 264 ms | 748 to 780 ms | 2 | |

- After 8 forward turns, some back turns, and toolbar use, `pmond` reported `processTooLarge` at the 49 MB heap cap (`alloc=51642368` of `max=51642368`). The framework restarted and lost some turns.
- The stock GUI stack uses about 200 MB of 490 MB, `cvm` alone 125 MB RSS.
- Stock has no Polish hyphenation, page header, or progress display.
- Home includes store and reading-list promotions.

## Page-turn strategy experiment

With the stock GUI stopped, `aristotle-observe bench pageturn` alternates a stock-rendered Polish page and a copy shifted by 37 rows. Results are medians of 6 turns, and two runs agreed within 5 ms.

The alternatives were [GL16](glossary.md#gl16), [REAGL](glossary.md#reagl), [DU](glossary.md#du), and [A2](glossary.md#a2), alone or in sequence.

| Strategy | Visible | Settled |
| --- | --- | --- |
| Stock reader (for reference) | 565 ms | 830 ms |
| GL16, one update | 479 ms | 479 ms |
| GLR16 (REAGL), one update | 495 ms | 495 ms |
| DU, black and white text only | 291 ms | 291 ms |
| DU, then GL16 | 291 ms | 771 ms |
| A2, then GL16 | 150 ms | 630 ms |

"Visible" ends when the first new-page update finishes. Two-step methods show black and white first. A page copy takes under a millisecond. The slow demo (`aristotle-observe bench pageturn PAGE.raw 4 3000`) built up little residue because the pages differed only by a shift.

### What real reading showed

A2 then GL16 left heavy [ghosting](glossary.md#ghosting) over a whole novel because partial updates skipped pixels already white after A2. Full GL16 did not clear all A2 residue either.

The driver log showed that the stock reader turns a page with one full-mode update of the whole screen in mode 9 (`glkw16`), and flashes with mode 8 (`gck16`). [FBInk](glossary.md#fbink) calls mode 9 (GLKW16) a night-mode waveform, though stock uses it in day mode. Both modes trigger the driver's "Algorithm processing" and "ACD data" steps, [Lab126](glossary.md#lab126)'s ghost reduction. Our code now sends the identical request, which removed ghosting in this experiment. The next page renders while the panel refreshes (`Screen::submit`/`Screen::finish`). The comparison also includes the four-gray-level [DU4](glossary.md#du4) mode. Same novel, median of 6 turns:

| Single-step turn | Tap to page shown | Settled |
| --- | --- | --- |
| Stock reader | 565 ms | 830 ms |
| Aristotle, mode 9 full (stock-identical update) | 503 ms | 503 ms |
| Aristotle, DU4 (mode 7) full | 317 ms | 317 ms |

Rendering the next page takes about 106 ms on the device, inside the panel time.

Page turns use the mode-9 update with a prerendered page. That is 11 % faster than stock to the page and 40 % faster to a calm screen, since stock sends a second update for its footer. DU4 remains a setting, about 1.8 times faster than stock but with fewer gray levels. A2 previews are off.

A long [MOBI](glossary.md#mobi) took 2.9 s to open before snapshots.[^1] A snapshot reduced host tap-to-submit from 278 ms to 1 ms. With the 480 ms flash, that is about 0.5 s. Device results were 557 to 570 ms.

## Transitions in the Aristotle app

Device, 2026-10-17, `tools/scenarios/aristotle-transitions.txt`. The app grabs the touchscreen, so `aristotle-observe` counts its own injected taps. Times run from the release.

| Interaction | Acknowledged | Settled | Updates | Flash | Waveforms |
| --- | --- | --- | --- | --- | --- |
| Page turn (forward or back) | 440 ms (the page) | 440 ms | 1 | no | `glkw16` full |
| Reader menu open | 513 to 577 ms (the menu) | 513 to 577 ms | 1 | no | `gl16`, 95 % of the screen |
| Reader menu close | 497 ms | 497 ms | 1 | no | `glkw16` full |
| Book → library | 52 ms | 765 ms | 2 | yes | `a2` tap mark, `gck16` full |
| Library filter tab | 60 to 64 ms | 552 to 741 ms | 2 | no | `a2`, `gl16` |
| Shelf page | 51 to 53 ms | 516 to 535 ms | 2 | every 5th | `a2`, `gl16` (`gck16` when flashing) |
| Sort | 59 to 61 ms | 743 to 749 ms | 2 | no | `a2`, `gl16` |
| Open the *Continue reading* book | 67 ms | 1282 ms | 2 | yes | `a2`, `gck16` full |

This historical run turned on touch-down, with 0 ms to update start and 440 ms panel time. Current code turns on release so holds can select words. This run does not measure the current release-to-panel path.

In this run:

- Each action ends in one update of its final state. Library controls are acknowledged within about 60 ms.
- The menu shows nothing for half a second. Its top bar and bottom panel coalesce into one box of 95 % of the screen.
- A book takes 1.3 s to open against stock's 0.8 s, mostly chapter layout.
- The shelf flash fires on a count, so it can feel random.

## Current release-driven baseline (2026-10-18)

Measured on a PW4 with firmware 5.13.6, using the original TXT fixture and
[`aristotle-latency.txt`](../tools/scenarios/aristotle-latency.txt).
The reader came from the CI bundle for commit `4afc98b`, with Plato
`6a4fc45`; the observer was built from `35a82e7`. The device was charging
at 80 %, in day mode with light level 2 and page-turn mode 9. Accounts,
library state, dictionaries, fonts, and clippings were isolated in
`/var/tmp`; no personal books were indexed or opened.

| Interaction | Samples | Median release to completion | Range |
| --- | ---: | ---: | ---: |
| Page forward | 6 | 514.1 ms | 506.0 to 525.4 ms |
| Page back | 6 | 513.6 ms | 501.7 to 524.8 ms |
| Menu open | 1 | 617.2 ms | one sample |
| Menu close | 1 | 512.4 ms | one sample |

Warm-up turns are excluded from the table. Every measured interaction
produced one full-screen, full-mode `glkw16` update. The trace has no lost
kernel records or missing completions; host replay reports `valid: true`.
The app log confirms forward/back dispatch and menu closure.

The [raw trace](measurements/pw4-release-2026-10-18.json) contains timings
and update metadata, not framebuffer contents. Replay it with:

```sh
cargo run --locked -p aristotle-observe -- replay \
    docs/measurements/pw4-release-2026-10-18.json --json
```

This is a current-path baseline for one TXT fixture, not a speedup or a
paired stock comparison. Menu timings need more samples. It does not
verify long-session ghosting, suspend, wake, or battery consumption. The
test app exited, its child processes ended, stock was restored, and the
previous light, screensaver-prevention, and EPDC debug values were restored.

## Device measurements of the app

Device, 2026-10-17, from the app's `[perf]` lines in `/var/tmp/aristotle.log` and `/proc/<pid>/status`.

| What | Result | Stock |
| --- | --- | --- |
| Open a book from its snapshot, tap to settled | 557 to 570 ms | 0.8 s |
| Back to the library, settled | 552 ms | not measured |
| Wake from the screensaver, settled | 441 ms | not measured |
| Dictionary lookup, Polish (`pl-pl`, 1.37 million forms) | 89 ms; popup 239 ms after the long press | not measured |
| Dictionary lookup, Ancient Greek (`grc-pl`) | 10.5 ms; popup 71 ms after the long press | not measured |
| Footnote popup | 2 ms to resolve the link | not measured |
| Memory while reading (`VmRSS`) | 14.5 MB | about 200 MB for the stock GUI stack |
| Free RAM while reading (`MemFree` + `Buffers` + `Cached`, of 490 MB) | 379 to 382 MB | 236 to 240 MB with the stock GUI running |
| Peak memory (`VmHWM`) | 66 MB, during the one-time indexing of five dictionaries | |
| One-time dictionary indexing | `pl-pl` 22.9 s, `grc-en` 37.2 s, the others under 3.1 s each | |

[`takeover.sh start`](glossary.md#takeover) alone, before the app starts, takes available RAM from 236 MB to 392 MB.[^2]

## Binary size

Stripped ARM binaries in MB (10⁶ bytes), built with `tools/build-plato.sh` and `tools/build-render.sh device`. Font and format cuts were checked against rendered test pages. See [Build options](reader.md#size-budget).

| Build | `aristotle` | `aristotle-render` | `data/` | App tarball |
| --- | --- | --- | --- | --- |
| Before the cuts (2026-10-17) | 29.52 MB | 26.04 MB | 5.04 MB | 19.4 MB |
| After the cuts (2026-10-17) | 8.36 MB | 6.60 MB | 2.68 MB | Not recorded |
| With night mode, settings, series, screensaver and page snapshots (2026-10-17) | 9.15 MB | 6.63 MB | 2.68 MB | 6.0 MB |
| Recorded default build | 9.64 MB | 6.66 MB | 2.68 MB | 6.4 MB |

## Host measurements

On a PC (`x86_64`, release build) with the same code and library versions as the device build.[^3] Layout on the device is about 8 times slower.

### Opening a book from a snapshot

`aristotle --bench-open`, 6 runs on one CPU, with a 379-page stand-in novel in a single spine item and page numbers cached.

| Step | Old path | New path |
| --- | --- | --- |
| Tap to submit | 250 to 400 ms, median 278 | 1.0 to 1.2 ms |
| Real page ready | the same | 257 to 372 ms after the tap, with no panel update |
| Snapshot | Not recorded | 126 KB for two pages, 6.6 ms to encode and write |

### Typesetting

`aristotle-render bench` with [Plato](glossary.md#plato) rendering EPUB books in Literata at 10 pt.

- Plato, *Phaedo*, EPUB, 341 pages: open 1.1 ms, first page 0.6 ms, render per page 2.1 / 2.4 ms (median / p90), paginate all 211 ms, max RSS 32 MiB.
- Polish non-fiction, EPUB, 269 pages: open 10 ms, first page 1.9 ms, render per page 4.6 / 5.2 ms (median / p90), paginate all 241 ms, max RSS 26 MiB.

### Dictionary indexes

The [fold index](reader.md#dictionary) is built once per dictionary. Warm lookups take 0.03 to 1.3 ms.

| Dictionary | Words | Forms | Index | Build |
| --- | --- | --- | --- | --- |
| `grc-pl` | 2,108 | 5,472 | 81 KB | 13 ms |
| `la-pl` | 15,814 | 83,969 | 0.9 MB | 44 ms |
| `en-pl` | 92,605 | 222,731 | 2.0 MB | 88 ms |
| `pl-pl` | 123,535 | 1,369,720 | 21 MB | 846 ms |
| `grc-en` | 21,819 | 1,789,814 | 23.6 MB | 1462 ms |

[^1]: Measured on the device.
[^2]: Measured on the device (`docs/pw4-reference.md`, "Taking over the screen").
[^3]: Host measurement.
