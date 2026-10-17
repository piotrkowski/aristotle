//! Live finger drawing. Optionally records every touch event for replay.
//!   aristotle-pen-live [SECONDS] [record=/var/tmp/pen.rec] [backend=display|pipeline stroke=6 radius=4 ...]
//! Tap the top-left target three times in a row to quit.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::{Duration, Instant};

use aristotle_hal::framebuffer::{Framebuffer, Rect, Waveform};
use aristotle_hal::input::{monotonic_us, Phase, TouchScreen, Transform};
use aristotle_pen::{parse_params, write_event, Pen};

fn square(cx: u32, cy: u32, r: u32) -> Rect {
    Rect {
        top: cy.saturating_sub(r),
        left: cx.saturating_sub(r),
        width: 2 * r,
        height: 2 * r,
    }
}

fn main() -> std::io::Result<()> {
    let (params, rest) = parse_params(std::env::args().skip(1));
    let seconds: u64 = rest.iter().find_map(|a| a.parse().ok()).unwrap_or(600);
    let record = rest
        .iter()
        .find_map(|a| a.strip_prefix("record=").map(str::to_string));
    let mut recorder = match &record {
        Some(path) => Some(BufWriter::new(File::create(path)?)),
        None => None,
    };

    let mut fb = Framebuffer::open("/dev/fb0")?;
    let g = fb.geometry();
    let mut touch = TouchScreen::open(g.width, g.height, Transform::default())?;
    touch.grab(true)?;
    println!(
        "touch {} at {}; {params:?}; record={record:?}",
        touch.name(),
        touch.path()
    );

    let screen = fb.full_screen();
    fb.fill(screen, 0xff);
    for (cx, cy) in [
        (60, 60),
        (g.width - 60, 60),
        (60, g.height - 60),
        (g.width - 60, g.height - 60),
    ] {
        fb.fill(square(cx, cy, 30), 0x00);
        fb.fill(square(cx, cy, 26), 0xff);
    }
    let m = fb.update(screen, Waveform::GC16, true)?;
    fb.wait(m)?;

    let quit = square(60, 60, 30);
    let mut quit_taps = 0;
    let mut pen = Pen::new(&fb, params)?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let (mut frames, mut events, mut strokes) = (0u64, 0u64, 0u64);
    let mut input_lag_us = 0u64;
    'outer: while Instant::now() < deadline {
        // Poll briefly while updates are in flight so latency samples are
        // collected promptly; otherwise sleep longer.
        let timeout = if pen.pending() {
            Duration::from_millis(4)
        } else {
            Duration::from_millis(100)
        };
        let mut batch = touch.poll(timeout)?;
        if batch.is_empty() {
            pen.tick();
            continue;
        }
        // Optionally keep gathering for `window_us` to coalesce the frame.
        let frame_end = monotonic_us() + params.window_us;
        loop {
            let now = monotonic_us();
            let wait = Duration::from_micros(frame_end.saturating_sub(now));
            let more = touch.poll(wait)?;
            if more.is_empty() {
                break;
            }
            batch.extend(more);
        }
        for ev in &batch {
            if let Some(r) = recorder.as_mut() {
                write_event(r, ev)?;
            }
            if ev.phase == Phase::Down {
                let inside = ev.x >= quit.left
                    && ev.x < quit.left + quit.width
                    && ev.y >= quit.top
                    && ev.y < quit.top + quit.height;
                quit_taps = if inside { quit_taps + 1 } else { 0 };
                if quit_taps >= 3 {
                    break 'outer;
                }
            }
            strokes += (ev.phase == Phase::Up) as u64;
        }
        let now = monotonic_us();
        input_lag_us += batch
            .iter()
            .map(|e| now.saturating_sub(e.time_us))
            .sum::<u64>();
        events += batch.len() as u64;
        frames += 1;
        pen.frame(&mut fb, &batch)?;
        if frames % 50 == 0 {
            println!("frames {frames} events {events} strokes {strokes} mean events/frame {:.1} mean kernel->draw {} us",
                events as f64 / frames as f64, input_lag_us / events.max(1));
        }
    }
    pen.flush(&mut fb)?;
    let mut lat = pen.latencies().to_vec();
    lat.sort_unstable();
    if !lat.is_empty() {
        println!(
            "event->on-panel ms: p50 {} p90 {} p99 {}",
            lat[lat.len() / 2] / 1000,
            lat[lat.len() * 9 / 10] / 1000,
            lat[(lat.len() * 99 / 100).min(lat.len() - 1)] / 1000
        );
    }
    if let Some(mut r) = recorder {
        r.flush()?;
    }
    println!(
        "done: frames {frames} events {events} strokes {strokes} mean kernel->draw {} us",
        input_lag_us / events.max(1)
    );
    Ok(())
}
