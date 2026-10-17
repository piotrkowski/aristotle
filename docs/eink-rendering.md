# E-ink rendering on the PW4

How the stock EPDC driver on the Kindle Paperwhite 4 behaves, and how Aristotle gets pixels onto the panel quickly. All numbers were measured on our device with the tools in this repo.

Drawing into the memory-mapped framebuffer does not change the panel. An update (`MXCFB_SEND_UPDATE`, Lab126's Rex struct) asks the EPDC to show a rectangle of the framebuffer with a chosen waveform. `MXCFB_WAIT_FOR_UPDATE_COMPLETE` optionally waits for it to finish. The waveform sets the speed, the gray levels, whether the panel flashes and how much ghosting remains.

## Waveforms

The PW4 driver uses the Zelda numbering from FBInk's `mxcfb-kindle.h`. In the older PW2 numbering, "A2" is 4, which on this device is REAGL and almost four times slower.

One update on an idle panel, 120 × 120 region, black and white toggles, median of 6 (`aristotle-bench`):

| Mode | Name | Use | Time |
| --- | --- | --- | --- |
| 6 | A2 | strokes, animation; black and white only | 120–144 ms |
| 1 | DU | fast black and white, cleaner than A2 | 309 ms |
| 7 | DU4 | 4 gray levels | 344 ms |
| 3 | GL16 | 16 grays, no flash; text and UI | 544 ms |
| 4 | GLR16 | REAGL, ghost-compensated text | 545 ms |
| 2 | GC16 | 16 grays with a flash; full clean refresh | 544 ms |
| 257 | AUTO | driver picks; chose DU for black and white content | 309 ms |

## One update at a time

`aristotle-bench --bin stream` submits 24 small updates at a fixed interval and records each one's latency and how long its submit blocks.

| Updates | Interval | Latency p50 | Submit blocks |
| --- | --- | --- | --- |
| A2, disjoint regions | 10–80 ms | 222–240 ms | about 120 ms |
| A2, overlapping regions | 10–80 ms | 232–240 ms | about 120 ms |
| A2, either | 160 ms | 120 ms | 0.6 ms |
| DU, disjoint | 80 ms | 522 ms | 261 ms |
| GL16, disjoint | 80 ms | 899 ms | 450 ms |

`MXCFB_SEND_UPDATE` blocks until the update already on the panel has finished, so an update submitted while another runs costs about two waveform durations. Disjoint and overlapping regions behave the same, so the driver serialises all updates. Updates submitted back to back before the first one starts are merged and finish together, which can make the driver look parallel.

## The display thread

In `aristotle_hal::display::Display`, the input loop draws and calls `display.damage(rect, waveform)`, which adds `rect` to a pending rectangle and never blocks. A dedicated thread submits the pending rectangle and repeats, and only this thread blocks in the ioctl.

- Everything drawn while the panel is busy goes out in the next update. A point waits at most for the rest of the current update plus its own, about 120 to 240 ms with A2.
- When merged damage asks for different waveforms, the most faithful one wins (A2 < DU < GL16 family < GC16).
- A completion thread (`Completions`) waits on each update marker and records when it finished. Only one thread may wait on a marker, because the driver forgets a marker once it has signalled it.

## Measuring stroke latency

The display thread came out of a finger-drawing experiment, the most demanding stream of updates the panel gets. The stroke renderer and its tools were removed later, since the PW4 has no pen; they remain in the history as the `aristotle-pen` crate.

1. A live drawing tool wrote each touch event with its kernel timestamp. `EVIOCSCLOCKID` switches the evdev clock to `CLOCK_MONOTONIC`, so the timestamps compare with `monotonic_us()`.
2. A replay benchmark played the recording back at its original pace through the same stroke code. Gaps between strokes were shortened to 300 ms.
3. An event's latency runs from its timestamp until the update that shows it completes. The panel's optical settling comes on top.

The reference recording had 604 events in 52 strokes, including fast and slow lines, handwriting and two fingers.

## Results

Event-to-panel latency on the reference recording, in milliseconds:

| Variant | p50 | p90 | p99 |
| --- | --- | --- | --- |
| First version: per-event updates, waited on, A2 numbered wrong | tens of seconds of backlog | | |
| Pipeline, A2, DU cleanup after each stroke | 334 | 508 | 664 |
| Pipeline, A2, no cleanup, any queue depth 1–32 | 293–296 | 347–351 | 359–375 |
| Pipeline, 40 ms coalescing window | 254 | 307 | 331 |
| Tiles, 128 px | 417 | 719 | 962 |
| Tiles, 64 px | 564 | 1433 | 2488 |
| Tiles, 32 px | 1245 | 3684 | 5442 |
| Tiles, 16 px | 6397 | 8030 | 9673 |
| **Display thread, A2** (default) | **199** | **252** | **354** |
| Display thread, A2, thin stroke | 197 | 250 | 336 |
| Display thread, DU | 419 | 532 | 730 |

- Waiting on each update built up tens of seconds of backlog.
- A DU cleanup of each finished stroke raised p99 from 375 to 664 ms, because it holds the driver for 309 ms while drawing continues.
- Queue depth made no difference, since the driver serialises submits.
- Tiles were meant to let the driver run non-overlapping updates in parallel. Each tile is one more blocking submit, so smaller tiles were slower.
- The display thread cut p50 by a third. Input never blocks, so the loop handles one frame per event instead of one per 3.7 events.

## Remaining latency

The floor is one A2 update, about 120 ms. The p50 of 199 ms is about 80 ms above it, because on average a point arrives halfway through the current update. Ideas not yet tried, most promising first:

- Predict the stroke a few tens of milliseconds ahead and correct the prediction in the next update, as pen tablets do.
- Start a stroke's first update on touch-down, before any movement.
- A kernel module or out-of-tree EPDC driver that keeps several LUTs busy at once. i.MX EPDC hardware has several LUTs, but we do not know whether the Lab126 driver serialises by policy or because of a limit of this panel.

## Code map

| Piece | Where |
| --- | --- |
| Framebuffer, update ioctl, waveform numbers | `crates/aristotle-hal/src/framebuffer.rs` |
| Display thread, completion tracking | `crates/aristotle-hal/src/display.rs` |
| Touch input with kernel timestamps | `crates/aristotle-hal/src/input.rs` |
| Waveform and streaming benchmarks | `crates/aristotle-bench/` |
