# E-ink rendering on the PW4

How Aristotle gets pixels onto the Kindle Paperwhite 4 panel quickly, what the stock EPDC driver really does, and how we measured it. Every number here was measured on our device unless marked otherwise. The tools that produced them are in the repo, so any claim can be re-checked.

## Model

Drawing and displaying are separate steps:

1. **Draw**: write 8-bit gray pixels into the memory-mapped framebuffer (`Framebuffer::fill`, `Framebuffer::line`). Nothing changes on the panel.
2. **Update**: ask the EPDC to move a rectangle of the panel to the framebuffer's content with a chosen waveform (`MXCFB_SEND_UPDATE`, Lab126's Rex struct).
3. **Wait** (optional): block until that update has finished (`MXCFB_WAIT_FOR_UPDATE_COMPLETE`).

The waveform decides speed and quality: how many gray levels, whether the panel flashes, and how much ghosting is left.

## Waveforms

The PW4 driver uses the **Zelda numbering** from FBInk's `mxcfb-kindle.h`. The older PW2 numbering is wrong here: its "A2" (4) is REAGL on this device and is almost four times slower.

Latency of one update on an idle panel, 120 × 120 region, black and white toggles, median of 6 (`aristotle-bench`):

| Mode | Name | Use | Time |
| --- | --- | --- | --- |
| 6 | A2 | strokes, animation; black and white only | 120–144 ms |
| 1 | DU | fast black and white, cleaner than A2 | 309 ms |
| 7 | DU4 | 4 gray levels | 344 ms |
| 3 | GL16 | 16 grays, no flash; text and UI | 544 ms |
| 4 | GLR16 | REAGL, ghost-compensated text | 545 ms |
| 2 | GC16 | 16 grays with a flash; full clean refresh | 544 ms |
| 257 | AUTO | driver picks; chose DU for black and white content | 309 ms |

## The driver runs one update at a time

`aristotle-bench --bin stream` submits 24 small updates at a fixed interval and times each one from submit to completion. It also times how long the submit ioctl itself blocks.

| Updates | Interval | Latency p50 | Submit blocks |
| --- | --- | --- | --- |
| A2, disjoint regions | 10–80 ms | 222–240 ms | about 120 ms |
| A2, overlapping regions | 10–80 ms | 232–240 ms | about 120 ms |
| A2, either | 160 ms | 120 ms | 0.6 ms |
| DU, disjoint | 80 ms | 522 ms | 261 ms |
| GL16, disjoint | 80 ms | 899 ms | 450 ms |

Conclusions:

- A new update cannot start while another is on the panel. **`MXCFB_SEND_UPDATE` blocks** until the earlier update finishes, so a streamed update costs about two waveform durations.
- **Overlap does not matter.** Disjoint and overlapping regions behave the same, so this is not EPDC pixel collision but serialisation in the driver.
- Several updates submitted back to back *before* the first one starts are merged and finish together. That burst result is what first made it look as if the driver ran updates in parallel. It does not.

## Design: a display thread with latest-wins damage

`aristotle_hal::display::Display`:

- The input loop draws pixels and calls `display.damage(rect, waveform)`, which only unions `rect` into a pending rectangle and never blocks.
- A dedicated thread loops: take the pending rectangle, submit it, repeat. It is the only thread that blocks, inside the ioctl, while the panel finishes the previous update.
- Everything drawn while the panel was busy goes out together in the next update. A point therefore waits for at most the rest of the current update plus its own: about 120 to 240 ms for A2.
- When merged damage asks for different waveforms, the most faithful one wins (A2 < DU < GL16 family < GC16).
- A small completion thread (`Completions`) waits on each update marker and timestamps it, for latency measurement. Only one thread may wait on a given marker, because the driver forgets a marker once it has signalled it.

## Measuring stroke latency: record and replay

Feel is hard to judge by eye and easy to fool yourself about, so stroke latency is measured from real input:

1. **Record**: `aristotle-pen-live record=/var/tmp/pen.rec` draws as usual and writes every touch event with its kernel timestamp. The evdev clock is switched to `CLOCK_MONOTONIC` with `EVIOCSCLOCKID`, so timestamps compare with `monotonic_us()`.
2. **Replay**: `aristotle-pen-bench /var/tmp/pen.rec [params]` replays the recording at its original pace through the same `Pen` code. Idle gaps between strokes are shortened to 300 ms; timing within strokes is unchanged.
3. **Measure**: for each event, the latency is the time from its (replayed) timestamp until the driver reports the update showing it as complete. That is what the eye actually waits for, minus the panel's own optical settling.

`devloop/kindle/pen-sweep.sh` runs a set of variants in one go. The reference recording (`~/.cache/aristotle/pen-session-1.rec`) has 604 events and 52 strokes: fast and slow lines, handwriting, two fingers.

## Results on the reference recording

Event-to-panel latency, in milliseconds:

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

What we learned along the way:

- **Waiting on each update is the worst option.** The first touch test waited per event and built up tens of seconds of backlog.
- **Cleanup refreshes hurt.** Redrawing a finished stroke with DU while drawing continues doubles the tail (p99 664 ms against 375 ms), because it occupies the one-at-a-time driver for 309 ms.
- **Queue depth is irrelevant**, as expected once you know submits serialise in the driver.
- **Tiles were a wrong hypothesis.** The idea was to avoid overlapping rectangles so the driver could run them concurrently. Every tile is another blocking submit, so smaller tiles made it worse.
- **A display thread is the fix**: p50 down a third, input never blocked (one frame per event instead of one per 3.7 events).

## Where the remaining time goes

The floor is one A2 update, about 120 ms. At p50 199 ms we sit about 80 ms above it: on average a point arrives halfway through the current update and waits for it. Ideas not yet tried, roughly in order of promise:

- **Stroke prediction**: extrapolate the finger a few tens of milliseconds ahead and draw the prediction, correcting it in the next update. This is how pen tablets hide latency.
- **Earlier first update**: start a stroke's first update on touch-down, before any movement.
- **Driver-level changes**: a kernel module or out-of-tree EPDC driver that keeps several LUTs busy in parallel. Unverified: i.MX EPDC hardware has multiple LUTs, so it is designed for concurrent updates, but whether the stock Lab126 driver serialises by policy or because of a hardware limit on this panel is not known yet.

## Code map

| Piece | Where |
| --- | --- |
| Framebuffer, update ioctl, waveform numbers | `crates/aristotle-hal/src/framebuffer.rs` |
| Display thread, completion tracking | `crates/aristotle-hal/src/display.rs` |
| Touch input with kernel timestamps | `crates/aristotle-hal/src/input.rs` |
| Stroke rendering, record/replay format | `crates/aristotle-pen/src/lib.rs` |
| Live drawing and recording | `crates/aristotle-pen/src/bin/live.rs` |
| Replay benchmark | `crates/aristotle-pen/src/bin/bench.rs` |
| Waveform and streaming benchmarks | `crates/aristotle-bench/` |
