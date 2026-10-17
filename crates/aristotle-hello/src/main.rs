// SPDX-License-Identifier: AGPL-3.0-or-later
//! Draws a gray ramp and a moving bar, timing full and partial e-ink updates.
//! Run on the device (or the PW4 emulator) while the stock UI is idle.

use std::time::Instant;

use aristotle_hal::framebuffer::{Framebuffer, Rect, Waveform};

fn main() -> std::io::Result<()> {
    let mut fb = Framebuffer::open("/dev/fb0")?;
    let g = fb.geometry();
    println!("fb id={} {:?}", fb.id(), g);

    // Full-screen flash to a clean white page.
    let screen = fb.full_screen();
    fb.fill(screen, 0xff);
    let started = Instant::now();
    let marker = fb.update(screen, Waveform::GC16, true)?;
    fb.wait(marker)?;
    println!("GC16 full refresh: {:?}", started.elapsed());

    // Sixteen gray bands across the top half, refreshed without a flash.
    let band = g.width / 16;
    for i in 0..16 {
        let gray = (i * 17) as u8;
        fb.fill(
            Rect {
                top: 0,
                left: i * band,
                width: band,
                height: g.height / 2,
            },
            gray,
        );
    }
    let started = Instant::now();
    let top = Rect {
        top: 0,
        left: 0,
        width: g.width,
        height: g.height / 2,
    };
    let marker = fb.update(top, Waveform::GL16, false)?;
    fb.wait(marker)?;
    println!("GL16 partial (half screen): {:?}", started.elapsed());

    // A bar stepping down the lower half with the fast DU waveform.
    let bar_h = 60;
    let mut prev: Option<Rect> = None;
    for step in 0..8 {
        let bar = Rect {
            top: g.height / 2 + step * bar_h * 3 / 2,
            left: 40,
            width: g.width - 80,
            height: bar_h,
        };
        if let Some(p) = prev {
            fb.fill(p, 0xff);
            fb.update(p, Waveform::DU, false)?;
        }
        fb.fill(bar, 0x00);
        let started = Instant::now();
        let marker = fb.update(bar, Waveform::DU, false)?;
        fb.wait(marker)?;
        println!("DU partial bar {step}: {:?}", started.elapsed());
        prev = Some(bar);
    }
    Ok(())
}
