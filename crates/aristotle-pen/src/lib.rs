// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stroke rendering for touch input on e-ink, plus the record/replay format
//! used to tune it.
//!
//! Events are drawn into the framebuffer as soon as they arrive. Getting the
//! pixels onto the panel is left to one of two backends:
//!
//! - [`Backend::Display`] (default): damage goes to the HAL's display thread,
//!   which coalesces everything drawn while the panel is busy into the next
//!   update. The input loop never blocks on the driver.
//! - [`Backend::Pipeline`]: one update per frame submitted from the input loop
//!   through a bounded [`Presenter`]. Kept as the measured baseline.
//!
//! Why the display thread wins, with numbers: `docs/eink-rendering.md`.

use std::io::{self, BufRead, Write};

use aristotle_hal::display::Display;
use aristotle_hal::framebuffer::{Framebuffer, Presenter, Rect, Waveform};
use aristotle_hal::input::{Phase, TouchEvent};

const MAX_FINGERS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Pipeline,
    Display,
}

#[derive(Clone, Copy, Debug)]
pub struct PenParams {
    pub backend: Backend,
    /// Pipeline backend only: updates allowed in flight before the oldest is
    /// awaited.
    pub in_flight: usize,
    /// Extra time to gather events after the first one of a frame.
    pub window_us: u64,
    /// Waveform for strokes while drawing.
    pub stroke: Waveform,
    /// Pipeline backend only: waveform to redraw a finished stroke with.
    /// Measured to double tail latency while drawing continues, so off by
    /// default.
    pub cleanup: Option<Waveform>,
    /// Half-width of finger 0's stroke in pixels; other fingers use half.
    pub radius: u32,
}

impl Default for PenParams {
    fn default() -> Self {
        Self {
            backend: Backend::Display,
            in_flight: 4,
            window_us: 0,
            stroke: Waveform::A2,
            cleanup: None,
            radius: 4,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Stroke {
    last: Option<(u32, u32)>,
    bounds: Option<Rect>,
}

/// One submitted update and the input it carried (pipeline backend), for
/// latency accounting by an external waiter.
#[derive(Debug)]
pub struct Submitted {
    pub marker: u32,
    /// Kernel timestamps of the events whose pixels this update shows.
    pub event_times_us: Vec<u64>,
    pub cleanup: bool,
}

enum Engine {
    Pipeline(Presenter),
    Display {
        display: Display,
        /// Event -> on-panel latencies observed so far, in microseconds.
        latencies: Vec<u64>,
    },
}

pub struct Pen {
    params: PenParams,
    engine: Engine,
    strokes: [Stroke; MAX_FINGERS],
}

impl Pen {
    pub fn new(fb: &Framebuffer, params: PenParams) -> io::Result<Self> {
        let engine = match params.backend {
            Backend::Pipeline => Engine::Pipeline(Presenter::new(params.in_flight)),
            Backend::Display => Engine::Display {
                display: Display::new(fb)?,
                latencies: Vec::new(),
            },
        };
        Ok(Self {
            params,
            engine,
            strokes: [Stroke::default(); MAX_FINGERS],
        })
    }

    pub fn params(&self) -> PenParams {
        self.params
    }

    /// Draw one frame's worth of events and hand the damage on. The pipeline
    /// backend returns its submissions for external accounting; the display
    /// backend accounts latency itself (see [`Pen::latencies`]).
    pub fn frame(
        &mut self,
        fb: &mut Framebuffer,
        batch: &[TouchEvent],
    ) -> io::Result<Vec<Submitted>> {
        let mut damage: Option<Rect> = None;
        let mut cleanups = Vec::new();
        for ev in batch {
            let id = ev.id.min(MAX_FINGERS - 1);
            let r = if id == 0 {
                self.params.radius
            } else {
                (self.params.radius / 2).max(1)
            };
            let s = &mut self.strokes[id];
            let from = match ev.phase {
                Phase::Down => (ev.x, ev.y),
                _ => s.last.unwrap_or((ev.x, ev.y)),
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
            }
        }
        let Some(rect) = damage else {
            return Ok(Vec::new());
        };
        let times: Vec<u64> = batch.iter().map(|e| e.time_us).collect();

        let mut out = Vec::new();
        match &mut self.engine {
            Engine::Pipeline(presenter) => {
                if let Some(marker) = presenter.present(fb, rect, self.params.stroke)? {
                    out.push(Submitted {
                        marker,
                        event_times_us: times,
                        cleanup: false,
                    });
                }
                if let Some(wf) = self.params.cleanup {
                    for rect in cleanups {
                        if let Some(marker) = presenter.present(fb, rect, wf)? {
                            out.push(Submitted {
                                marker,
                                event_times_us: Vec::new(),
                                cleanup: true,
                            });
                        }
                    }
                }
            }
            Engine::Display { display, .. } => {
                display.damage(rect, self.params.stroke, &times);
                self.tick();
            }
        }
        Ok(out)
    }

    /// Collect latency samples for updates that reached the panel. Cheap; call
    /// it whenever convenient (the display thread works without it).
    pub fn tick(&mut self) {
        if let Engine::Display { display, latencies } = &mut self.engine {
            for (shown_at, times) in display.drain_shown() {
                latencies.extend(times.into_iter().map(|t| shown_at.saturating_sub(t)));
            }
        }
    }

    /// True while drawn pixels still have to reach the panel.
    pub fn pending(&self) -> bool {
        match &self.engine {
            Engine::Pipeline(_) => false,
            Engine::Display { display, .. } => display.busy(),
        }
    }

    /// Event -> on-panel latencies measured by the display backend
    /// (microseconds).
    pub fn latencies(&self) -> &[u64] {
        match &self.engine {
            Engine::Display { latencies, .. } => latencies,
            Engine::Pipeline(_) => &[],
        }
    }

    /// Wait until everything drawn is on the panel.
    pub fn flush(&mut self, fb: &mut Framebuffer) -> io::Result<()> {
        if let Engine::Pipeline(presenter) = &mut self.engine {
            return presenter.flush(fb);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.pending() && std::time::Instant::now() < deadline {
            self.tick();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        self.tick();
        Ok(())
    }
}

/// Text format for recorded input: `time_us phase id x y` per line.
pub fn write_event(out: &mut impl Write, ev: &TouchEvent) -> io::Result<()> {
    let phase = match ev.phase {
        Phase::Down => 'D',
        Phase::Move => 'M',
        Phase::Up => 'U',
    };
    writeln!(out, "{} {} {} {} {}", ev.time_us, phase, ev.id, ev.x, ev.y)
}

pub fn read_events(input: impl BufRead) -> io::Result<Vec<TouchEvent>> {
    let mut events = Vec::new();
    for line in input.lines() {
        let line = line?;
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 5 {
            continue;
        }
        let phase = match f[1] {
            "D" => Phase::Down,
            "M" => Phase::Move,
            "U" => Phase::Up,
            _ => continue,
        };
        let num = |s: &str| {
            s.parse::<u64>()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        };
        let (x, y) = (num(f[3])? as u32, num(f[4])? as u32);
        events.push(TouchEvent {
            phase,
            id: num(f[2])? as usize,
            x,
            y,
            raw_x: x as i32,
            raw_y: y as i32,
            time_us: num(f[0])?,
        });
    }
    Ok(events)
}

/// Parse `key=value` arguments into [`PenParams`]; anything else is returned.
/// Keys: `backend=display|pipeline`, `inflight=4`, `window=0` (us),
/// `stroke=6` (waveform number), `cleanup=1|none`, `radius=4`.
pub fn parse_params(args: impl Iterator<Item = String>) -> (PenParams, Vec<String>) {
    let mut p = PenParams::default();
    let mut rest = Vec::new();
    for arg in args {
        match arg.split_once('=') {
            Some(("backend", "pipeline")) => p.backend = Backend::Pipeline,
            Some(("backend", "display")) => p.backend = Backend::Display,
            Some(("inflight", v)) => p.in_flight = v.parse().unwrap_or(p.in_flight),
            Some(("window", v)) => p.window_us = v.parse().unwrap_or(p.window_us),
            Some(("stroke", v)) => p.stroke = Waveform(v.parse().unwrap_or(p.stroke.0)),
            Some(("cleanup", "none")) => p.cleanup = None,
            Some(("cleanup", v)) => p.cleanup = v.parse().ok().map(Waveform),
            Some(("radius", v)) => p.radius = v.parse().unwrap_or(p.radius),
            _ => rest.push(arg),
        }
    }
    (p, rest)
}
