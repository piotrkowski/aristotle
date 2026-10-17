// SPDX-License-Identifier: AGPL-3.0-or-later
//! EPDC latency benchmark. Prints submit-to-complete times per waveform for a
//! small region, then checks whether disjoint regions refresh concurrently and
//! how overlapping ones queue. Results drive the refresh policy.

use std::time::{Duration, Instant};

use aristotle_hal::framebuffer::{Framebuffer, Rect, Waveform};

const RUNS: usize = 6;

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn tile(i: u32) -> Rect {
    Rect {
        top: 200 + (i / 4) * 160,
        left: 60 + (i % 4) * 240,
        width: 120,
        height: 120,
    }
}

fn main() -> std::io::Result<()> {
    let mut fb = Framebuffer::open("/dev/fb0")?;
    let screen = fb.full_screen();
    fb.fill(screen, 0xff);
    let m = fb.update(screen, Waveform::GC16, true)?;
    fb.wait(m)?;

    println!("single 120x120 region, black/white toggles, median of {RUNS}:");
    let r = tile(0);
    let modes = (1..=9).map(Waveform).chain([Waveform::AUTO]);
    for wf in modes {
        let mut times = Vec::new();
        for i in 0..RUNS {
            fb.fill(r, if i % 2 == 0 { 0x00 } else { 0xff });
            let t = Instant::now();
            let m = fb.update(r, wf, false)?;
            fb.wait(m)?;
            times.push(t.elapsed());
        }
        println!("  mode {:<4} {:>8.1?}", wf.0, median(times));
    }

    println!("submit cost only (no wait), DU:");
    let mut submits = Vec::new();
    let mut last = None;
    for i in 0..RUNS {
        let r = tile(1 + i as u32 % 3);
        fb.fill(r, if i % 2 == 0 { 0x00 } else { 0xff });
        let t = Instant::now();
        last = fb.update(r, Waveform::DU, false)?;
        submits.push(t.elapsed());
    }
    fb.wait(last)?;
    println!("  submit {:>8.1?}", median(submits));

    for wf in [Waveform::DU, Waveform::A2] {
        for n in [1u32, 4, 8] {
            let start = Instant::now();
            let mut markers = Vec::new();
            for i in 0..n {
                let r = tile(4 + i);
                fb.fill(r, 0x00);
                markers.push(fb.update(r, wf, false)?);
            }
            for m in markers {
                fb.wait(m)?;
            }
            println!(
                "{:?}: {n} disjoint tiles submitted together, all done in {:?}",
                wf,
                start.elapsed()
            );
            for i in 0..n {
                fb.fill(tile(4 + i), 0xff);
            }
            let m = fb.update(
                Rect {
                    top: 200 + 160,
                    left: 60,
                    width: 960,
                    height: 280,
                },
                Waveform::DU,
                false,
            )?;
            fb.wait(m)?;
        }
    }

    println!("overlapping: 4 updates of the same region submitted back to back (DU):");
    let start = Instant::now();
    let mut markers = Vec::new();
    for i in 0..4 {
        fb.fill(r, if i % 2 == 0 { 0x00 } else { 0xff });
        markers.push(fb.update(r, Waveform::DU, false)?);
    }
    for m in markers {
        fb.wait(m)?;
    }
    println!("  all done in {:?}", start.elapsed());
    Ok(())
}
