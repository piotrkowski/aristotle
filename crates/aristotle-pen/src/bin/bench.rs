// SPDX-License-Identifier: AGPL-3.0-or-later
//! Replays a recorded touch session at its original timing through [`Pen`] and
//! measures, for every event, the time from its (replayed) kernel timestamp to
//! the moment the EPDC reports the update showing it as complete.
//!
//!   aristotle-pen-bench [/var/tmp/pen.rec] [maxgap=300000] [backend=display|pipeline ...]
//!
//! Idle gaps between strokes are shortened to `maxgap` microseconds so a long
//! session replays quickly; timing within a stroke is kept exactly. Prints one
//! line: the parameters, then latency percentiles in milliseconds.

use std::fs::File;
use std::io::BufReader;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use aristotle_hal::framebuffer::{Framebuffer, Waveform};
use aristotle_hal::input::monotonic_us;
use aristotle_pen::{parse_params, read_events, Backend, Pen};

/// How often latency samples are collected between input events.
const TICK_US: u64 = 4_000;

fn pct(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize] as f64 / 1000.0
}

fn sleep_until(t: u64) {
    let now = monotonic_us();
    if t > now {
        thread::sleep(Duration::from_micros(t - now));
    }
}

fn main() -> std::io::Result<()> {
    let (params, rest) = parse_params(std::env::args().skip(1));
    let path = rest
        .iter()
        .find(|a| !a.contains('='))
        .cloned()
        .unwrap_or_else(|| "/var/tmp/pen.rec".into());
    let max_gap: u64 = rest
        .iter()
        .find_map(|a| a.strip_prefix("maxgap=")?.parse().ok())
        .unwrap_or(300_000);
    let recorded = read_events(BufReader::new(File::open(&path)?))?;
    if recorded.is_empty() {
        eprintln!("no events in {path}");
        return Ok(());
    }

    let mut fb = Framebuffer::open("/dev/fb0")?;
    let screen = fb.full_screen();
    fb.fill(screen, 0xff);
    let m = fb.update(screen, Waveform::GC16, true)?;
    fb.wait(m)?;

    // Pipeline backend: a separate thread waits on each marker and timestamps
    // it. The display backend measures itself through its completion thread.
    let (tx, rx) = mpsc::channel::<(u32, Vec<u64>)>();
    let pipeline_waiter = fb.waiter()?;
    let measure = thread::spawn(move || {
        let mut latencies = Vec::new();
        for (marker, times) in rx {
            if pipeline_waiter.wait(marker).is_ok() {
                let done = monotonic_us();
                latencies.extend(times.iter().map(|t| done.saturating_sub(*t)));
            }
        }
        latencies
    });

    // Re-base timestamps onto now, shortening idle gaps.
    let base = monotonic_us() + 200_000;
    let mut events = Vec::with_capacity(recorded.len());
    let mut t = base;
    for (k, e) in recorded.iter().enumerate() {
        if k > 0 {
            t += (e.time_us - recorded[k - 1].time_us).min(max_gap);
        }
        let mut e = *e;
        e.time_us = t;
        events.push(e);
    }

    let mut pen = Pen::new(&fb, params)?;
    let mut i = 0;
    let mut frames = 0u64;
    while i < events.len() {
        // Until the next event is due, keep collecting latency samples.
        while monotonic_us() + TICK_US < events[i].time_us {
            if pen.pending() {
                pen.tick();
            }
            sleep_until((monotonic_us() + TICK_US).min(events[i].time_us));
        }
        sleep_until(events[i].time_us);
        if params.window_us > 0 {
            sleep_until(monotonic_us() + params.window_us);
        }
        // Everything already due forms one frame.
        let now = monotonic_us();
        let mut j = i;
        while j < events.len() && events[j].time_us <= now {
            j += 1;
        }
        for s in pen.frame(&mut fb, &events[i..j])? {
            if !s.cleanup {
                let _ = tx.send((s.marker, s.event_times_us));
            }
        }
        frames += 1;
        i = j;
    }
    pen.flush(&mut fb)?;
    drop(tx);
    let mut lat = match params.backend {
        Backend::Pipeline => measure.join().expect("measure thread"),
        Backend::Display => {
            let _ = measure.join();
            pen.latencies().to_vec()
        }
    };
    lat.sort_unstable();
    let span_ms = (events.last().unwrap().time_us - base) / 1000;
    println!(
        "{:?}: events {} measured {} frames {} over {} ms | p50 {:.0} p90 {:.0} p99 {:.0} max {:.0}",
        params, events.len(), lat.len(), frames, span_ms,
        pct(&lat, 0.5), pct(&lat, 0.9), pct(&lat, 0.99), lat.last().copied().unwrap_or(0) as f64 / 1000.0,
    );
    Ok(())
}
