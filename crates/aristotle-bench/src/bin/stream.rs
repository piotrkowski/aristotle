// SPDX-License-Identifier: AGPL-3.0-or-later
//! EPDC streaming experiment: how does A2 latency behave when updates arrive
//! continuously, as they do while a finger draws?
//!
//! Submits `COUNT` small A2 updates at a fixed interval along a horizontal
//! path and measures each one's submit-to-complete time on a waiter thread.
//! Two layouts:
//!
//! - disjoint: consecutive rectangles touch but never share pixels
//! - overlap: each rectangle shares half its width with the previous one
//!
//! Also reports how long the submit ioctl itself blocked, which revealed that
//! the driver runs one update at a time (see `docs/eink-rendering.md`). Pass
//! `waveforms` to compare A2, DU and GL16 instead.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use aristotle_hal::framebuffer::{Framebuffer, Rect, Waveform};
use aristotle_hal::input::monotonic_us;

const COUNT: u32 = 24;
const SIZE: u32 = 40;

fn pct(sorted: &[u64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize] as f64 / 1000.0
}

fn main() -> std::io::Result<()> {
    let mut fb = Framebuffer::open("/dev/fb0")?;
    let screen = fb.full_screen();
    fb.fill(screen, 0xff);
    let m = fb.update(screen, Waveform::GC16, true)?;
    fb.wait(m)?;

    let only_waveforms = std::env::args().any(|a| a == "waveforms");
    let cases: Vec<(bool, u64, Waveform)> = if only_waveforms {
        [Waveform::A2, Waveform::DU, Waveform::GL16]
            .into_iter()
            .flat_map(|w| [20u64, 80].into_iter().map(move |i| (false, i, w)))
            .collect()
    } else {
        [false, true]
            .into_iter()
            .flat_map(|o| {
                [10u64, 20, 40, 80, 160]
                    .into_iter()
                    .map(move |i| (o, i, Waveform::A2))
            })
            .collect()
    };
    for (row, (overlap, interval_ms, waveform)) in (0u32..).zip(cases) {
        {
            let waiter = fb.waiter()?;
            let (tx, rx) = mpsc::channel::<(u32, u64)>();
            let measure = thread::spawn(move || {
                rx.into_iter()
                    .filter_map(|(marker, submitted)| {
                        waiter.wait(marker).ok().map(|_| monotonic_us() - submitted)
                    })
                    .collect::<Vec<u64>>()
            });
            let step = if overlap { SIZE / 2 } else { SIZE };
            let top = 120 + row * (SIZE + 20);
            let mut submit_block = Vec::new();
            let start = monotonic_us();
            for i in 0..COUNT {
                let due = start + i as u64 * interval_ms * 1000;
                let now = monotonic_us();
                if due > now {
                    thread::sleep(Duration::from_micros(due - now));
                }
                let rect = Rect {
                    top,
                    left: 40 + i * step,
                    width: SIZE,
                    height: SIZE,
                };
                fb.fill(rect, 0x00);
                let t = monotonic_us();
                if let Some(marker) = fb.update(rect, waveform, false)? {
                    submit_block.push(monotonic_us() - t);
                    let _ = tx.send((marker, t));
                }
            }
            drop(tx);
            let mut lat = measure.join().expect("measure thread");
            lat.sort_unstable();
            submit_block.sort_unstable();
            println!(
                "{:<8} mode {} every {:>3} ms: latency p50 {:>4.0} p90 {:>4.0} max {:>4.0} ms | submit blocked p50 {:.1} max {:.1} ms",
                if overlap { "overlap" } else { "disjoint" }, waveform.0, interval_ms,
                pct(&lat, 0.5), pct(&lat, 0.9), *lat.last().unwrap() as f64 / 1000.0,
                pct(&submit_block, 0.5), *submit_block.last().unwrap() as f64 / 1000.0,
            );
        }
    }
    Ok(())
}
