// SPDX-License-Identifier: AGPL-3.0-or-later
//! `aristotle-observe bench pageturn`: page-turn strategies on real text.
//!
//! Alternates between two full-screen text pages (raw 8-bit framebuffer
//! dumps, stride 1088) and measures, per strategy, when the new page is
//! first legible ("visible") and when the panel is done ("settled"). The goal
//! is a turn that beats the stock reader's 565 ms visible and 830 ms settled
//! (`docs/benchmarks.md`).
//!
//! ```text
//! aristotle-observe bench pageturn PAGE.raw [TURNS] [PAUSE_MS] [STRATEGY]
//! ```
//!
//! The second page is the first shifted down by 37 rows, so every glyph
//! changes the way it would on a real turn.
//!
//! Strategies:
//!
//! - `gl16`: anti-aliased page in one GL16 update (closest to stock).
//! - `glr16`: the same with REAGL (ghost compensation).
//! - `du`: page thresholded to pure black and white, DU only.
//! - `du+gl16`: black and white page with DU first (visible), then the
//!   anti-aliased page with GL16 (settled).
//! - `a2+gl16`: as above, but the first pass uses A2.
//!
//! With a `PAUSE_MS` above 0 it is a demo for judging by eye: each strategy
//! starts from a full flash, and the turns are `PAUSE_MS` apart, for example
//! `bench pageturn /var/tmp/page.raw 4 3000`. Run it with the stock GUI
//! stopped (`takeover.sh start`).

use std::fs;
use std::io;
use std::time::Instant;

use aristotle_hal::framebuffer::{Framebuffer, Rect, Waveform};

const STRIDE: usize = 1088;
const W: usize = 1072;
const H: usize = 1448;

fn load(path: &str) -> io::Result<Vec<u8>> {
    let mut page = fs::read(path)?;
    page.resize(STRIDE * H, 0xff);
    Ok(page)
}

fn shifted(page: &[u8], rows: usize) -> Vec<u8> {
    let mut out = vec![0xff; STRIDE * H];
    out[rows * STRIDE..].copy_from_slice(&page[..(H - rows) * STRIDE]);
    out
}

fn threshold(page: &[u8]) -> Vec<u8> {
    page.iter()
        .map(|&p| if p < 160 { 0x00 } else { 0xff })
        .collect()
}

/// Copy a full page into the framebuffer mapping.
fn blit(fb: &mut Framebuffer, page: &[u8]) {
    fb.blit(
        page,
        STRIDE,
        Rect {
            top: 0,
            left: 0,
            width: W as u32,
            height: H as u32,
        },
    );
}

fn median(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

pub fn run(args: &[String]) -> io::Result<()> {
    let path = args
        .first()
        .cloned()
        .unwrap_or_else(|| "/var/tmp/page.raw".into());
    let turns: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(6);
    // Demo mode for judging by eye: a pause between turns, and optionally a
    // single strategy.
    let pause_ms: u64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(0);
    let only = args.get(3).cloned();
    let a = load(&path)?;
    let b = shifted(&a, 37);
    let pages = [(&a, threshold(&a)), (&b, threshold(&b))];

    let mut fb = Framebuffer::open("/dev/fb0")?;
    let screen = fb.full_screen();
    blit(&mut fb, &a);
    let m = fb.update(screen, Waveform::GC16, true)?;
    fb.wait(m)?;

    println!(
        "{:<9} {:>9} {:>9}   (median of {turns} turns, ms)",
        "strategy", "visible", "settled"
    );
    for name in ["gl16", "glr16", "du", "du+gl16", "a2+gl16"] {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        if pause_ms > 0 {
            // Demo: start each strategy from a clean page.
            println!("== {name}");
            blit(&mut fb, &a);
            let m = fb.update(screen, Waveform::GC16, true)?;
            fb.wait(m)?;
        }
        let (mut visible, mut settled) = (Vec::new(), Vec::new());
        for t in 0..turns {
            std::thread::sleep(std::time::Duration::from_millis(pause_ms));
            let (aa, bw) = &pages[(t + 1) % 2];
            let start = Instant::now();
            match name {
                "gl16" | "glr16" => {
                    blit(&mut fb, aa);
                    let wf = if name == "gl16" {
                        Waveform::GL16
                    } else {
                        Waveform::GLR16
                    };
                    let m = fb.update(screen, wf, false)?;
                    fb.wait(m)?;
                    visible.push(start.elapsed().as_millis());
                    settled.push(start.elapsed().as_millis());
                }
                "du" => {
                    blit(&mut fb, bw);
                    let m = fb.update(screen, Waveform::DU, false)?;
                    fb.wait(m)?;
                    visible.push(start.elapsed().as_millis());
                    settled.push(start.elapsed().as_millis());
                }
                _ => {
                    let first = if name == "du+gl16" {
                        Waveform::DU
                    } else {
                        Waveform::A2
                    };
                    blit(&mut fb, bw);
                    let m = fb.update(screen, first, false)?;
                    fb.wait(m)?;
                    visible.push(start.elapsed().as_millis());
                    blit(&mut fb, aa);
                    let m = fb.update(screen, Waveform::GL16, false)?;
                    fb.wait(m)?;
                    settled.push(start.elapsed().as_millis());
                }
            }
        }
        println!("{name:<9} {:>9} {:>9}", median(visible), median(settled));
    }
    // Leave a clean page behind.
    blit(&mut fb, &a);
    let m = fb.update(screen, Waveform::GC16, true)?;
    fb.wait(m)?;
    Ok(())
}
