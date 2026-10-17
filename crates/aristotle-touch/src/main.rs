//! Touch and pen-feel test. Each finger draws a stroke; finger 0 is thick,
//! finger 1 thin. Strokes go out as A2 updates (about 144 ms on the PW4)
//! through a pipelined `Presenter`, so input is never blocked by the panel.
//! When a finger lifts, its stroke is redrawn with DU to clean A2 artifacts.
//! Tap the top-left target three times in a row to quit; otherwise exits after
//! `SECONDS` (default 600).

use std::time::{Duration, Instant};

use aristotle_hal::framebuffer::{Framebuffer, Presenter, Rect, Waveform};
use aristotle_hal::input::{Phase, TouchScreen, Transform};

const MAX_FINGERS: usize = 10;

fn square(cx: u32, cy: u32, r: u32) -> Rect {
    Rect {
        top: cy.saturating_sub(r),
        left: cx.saturating_sub(r),
        width: 2 * r,
        height: 2 * r,
    }
}

fn contains(r: Rect, x: u32, y: u32) -> bool {
    x >= r.left && x < r.left + r.width && y >= r.top && y < r.top + r.height
}

#[derive(Clone, Copy, Default)]
struct Stroke {
    last: Option<(u32, u32)>,
    bounds: Option<Rect>,
}

fn main() -> std::io::Result<()> {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    let mut fb = Framebuffer::open("/dev/fb0")?;
    let g = fb.geometry();
    let mut touch = TouchScreen::open(g.width, g.height, Transform::default())?;
    touch.grab(true)?;
    println!("touch {} at {}", touch.name(), touch.path());

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
    let mut presenter = Presenter::new(4);
    let mut strokes = [Stroke::default(); MAX_FINGERS];
    let mut deadline = Instant::now() + Duration::from_secs(seconds);
    let (mut frames, mut events) = (0u32, 0usize);
    let mut frame_cost = Duration::ZERO;
    while Instant::now() < deadline {
        let batch = touch.poll(Duration::from_millis(100))?;
        if batch.is_empty() {
            continue;
        }
        let started = Instant::now();
        let mut damage: Option<Rect> = None;
        let mut cleanups = Vec::new();
        for ev in &batch {
            if ev.phase == Phase::Down {
                quit_taps = if contains(quit, ev.x, ev.y) {
                    quit_taps + 1
                } else {
                    0
                };
                if quit_taps >= 3 {
                    println!("quit target tapped three times");
                    deadline = Instant::now();
                }
            }
            let id = ev.id.min(MAX_FINGERS - 1);
            let r = if id == 0 { 4 } else { 2 };
            let s = &mut strokes[id];
            let from = if ev.phase == Phase::Down {
                (ev.x, ev.y)
            } else {
                s.last.unwrap_or((ev.x, ev.y))
            };
            let d = fb.line(from, (ev.x, ev.y), r, 0x00);
            s.bounds = Some(s.bounds.map_or(d, |b| b.union(d)));
            s.last = Some((ev.x, ev.y));
            damage = Some(damage.map_or(d, |x| x.union(d)));
            if ev.phase == Phase::Up {
                if let Some(b) = s.bounds.take() {
                    cleanups.push(b);
                }
                s.last = None;
                println!("stroke {id} done");
            }
        }
        if let Some(rect) = damage {
            presenter.present(&mut fb, rect, Waveform::A2)?;
        }
        for rect in cleanups {
            presenter.present(&mut fb, rect, Waveform::DU)?;
        }
        frames += 1;
        events += batch.len();
        frame_cost += started.elapsed();
    }
    presenter.flush(&fb)?;
    println!(
        "frames {frames}, events {events}, mean CPU+submit per frame {:?}, raw kernel events {}",
        frame_cost / frames.max(1),
        touch.raw_events()
    );
    Ok(())
}
