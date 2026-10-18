// SPDX-License-Identifier: AGPL-3.0-or-later
//! Gesture recognition: tap, long press and swipe from raw touch events.
//!
//! Events have the shape of `aristotle_hal::input::TouchEvent` (phase,
//! slot id, x, y, kernel timestamp in microseconds), so the device glue is
//! a field-by-field copy.
//!
//! Recognition favours *early* feedback. [`Gesture::Press`] is emitted on
//! touch-down, before the recognizer knows whether this will be a tap, a
//! long press or a swipe, so the UI can start the ~130 ms A2 highlight
//! immediately. If the finger then travels, [`Gesture::Cancel`] withdraws
//! the press. On e-ink this ordering matters more than anywhere else: the
//! highlight must already be on its way to the panel by the time the finger
//! lifts, or the user waits a whole waveform for any sign of life.

use crate::geom::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchEvent {
    pub phase: Phase,
    /// Contact slot.
    pub id: usize,
    pub x: i32,
    pub y: i32,
    /// CLOCK_MONOTONIC microseconds.
    pub time_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// A finger went down: show pressed feedback now.
    Press(Point),
    /// The press will not become a tap (moved, second finger): drop the
    /// feedback.
    Cancel,
    Tap(Point),
    /// Held still for [`GestureConfig::long_press_us`]. Fired once, while
    /// the finger is still down.
    LongPress(Point),
    /// A flick or drag, named by the direction the finger moved. A swipe to
    /// the left turns to the next page.
    Swipe {
        dir: Direction,
        from: Point,
        to: Point,
    },
}

/// Thresholds, in device pixels and microseconds.
#[derive(Clone, Copy, Debug)]
pub struct GestureConfig {
    /// Movement still counted as a tap: 4 mm. A fingertip rolls 1 to 3 mm
    /// while pressing; less slop turns firm taps into failed swipes.
    pub tap_slop: i32,
    /// 500 ms, the common platform default.
    pub long_press_us: u64,
    /// Minimum travel for a swipe: 8 mm, well past the tap slop so the two
    /// never compete.
    pub swipe_min: i32,
    /// The dominant axis must exceed the other by this factor, so a sloppy
    /// diagonal is not misread as the wrong page turn.
    pub axis_ratio: f32,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self::for_density(crate::geom::Density::default())
    }
}

impl GestureConfig {
    pub fn for_density(density: crate::geom::Density) -> Self {
        Self {
            tap_slop: density.mm(4.0),
            long_press_us: 500_000,
            swipe_min: density.mm(8.0),
            axis_ratio: 1.5,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum State {
    Idle,
    /// One finger down, still within the tap slop.
    Pressed {
        id: usize,
        start: Point,
        t0: u64,
        long_fired: bool,
    },
    /// One finger down and travelling: only a swipe can come of it.
    Moving {
        id: usize,
        start: Point,
        last: Point,
    },
    /// A second finger arrived; ignore everything until all are up.
    Ignoring,
}

/// Turns touch events into gestures. Feed every event to [`Self::feed`]
/// and call [`Self::tick`] from the event loop's timeout so a long press
/// fires while the finger is still down.
pub struct GestureRecognizer {
    pub config: GestureConfig,
    state: State,
    /// Contacts currently down, to detect multi-touch and the final lift.
    down: Vec<usize>,
}

impl Default for GestureRecognizer {
    fn default() -> Self {
        Self::new(GestureConfig::default())
    }
}

fn dist2(a: Point, b: Point) -> i64 {
    let dx = (a.x - b.x) as i64;
    let dy = (a.y - b.y) as i64;
    dx * dx + dy * dy
}

impl GestureRecognizer {
    pub fn new(config: GestureConfig) -> Self {
        Self {
            config,
            state: State::Idle,
            down: Vec::new(),
        }
    }

    /// Microseconds until [`Self::tick`] may fire a long press, if one is
    /// pending. Use it as the input poll timeout.
    pub fn next_deadline(&self, now_us: u64) -> Option<u64> {
        match self.state {
            State::Pressed {
                t0,
                long_fired: false,
                ..
            } => Some((t0 + self.config.long_press_us).saturating_sub(now_us)),
            _ => None,
        }
    }

    pub fn tick(&mut self, now_us: u64) -> Option<Gesture> {
        if let State::Pressed {
            start,
            t0,
            ref mut long_fired,
            ..
        } = self.state
        {
            if !*long_fired && now_us >= t0 + self.config.long_press_us {
                *long_fired = true;
                return Some(Gesture::LongPress(start));
            }
        }
        None
    }

    pub fn feed(&mut self, ev: TouchEvent) -> Vec<Gesture> {
        let mut out = Vec::new();
        let p = Point { x: ev.x, y: ev.y };
        // A long press is due even if no tick came in between.
        if ev.phase != Phase::Down {
            out.extend(self.tick(ev.time_us));
        }
        match ev.phase {
            Phase::Down => {
                if !self.down.contains(&ev.id) {
                    self.down.push(ev.id);
                }
                match self.state {
                    State::Idle if self.down.len() == 1 => {
                        self.state = State::Pressed {
                            id: ev.id,
                            start: p,
                            t0: ev.time_us,
                            long_fired: false,
                        };
                        out.push(Gesture::Press(p));
                    }
                    State::Pressed { .. } | State::Moving { .. } => {
                        // Two fingers: reserved for future pinch gestures.
                        if matches!(self.state, State::Pressed { .. }) {
                            out.push(Gesture::Cancel);
                        }
                        self.state = State::Ignoring;
                    }
                    _ => {}
                }
            }
            Phase::Move => match self.state {
                State::Pressed {
                    id,
                    start,
                    long_fired,
                    ..
                } if id == ev.id => {
                    let slop = self.config.tap_slop as i64;
                    if dist2(start, p) > slop * slop {
                        if !long_fired {
                            out.push(Gesture::Cancel);
                        }
                        self.state = State::Moving { id, start, last: p };
                    }
                }
                State::Moving {
                    id, ref mut last, ..
                } if id == ev.id => *last = p,
                _ => {}
            },
            Phase::Up => {
                self.down.retain(|&d| d != ev.id);
                match self.state {
                    State::Pressed { id, long_fired, .. } if id == ev.id => {
                        if !long_fired {
                            out.push(Gesture::Tap(p));
                        }
                        self.state = State::Idle;
                    }
                    State::Moving { id, start, .. } if id == ev.id => {
                        out.extend(self.swipe(start, p));
                        self.state = State::Idle;
                    }
                    State::Ignoring if self.down.is_empty() => self.state = State::Idle,
                    _ => {}
                }
            }
        }
        out
    }

    fn swipe(&self, from: Point, to: Point) -> Option<Gesture> {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let (ax, ay) = (dx.abs() as f32, dy.abs() as f32);
        let min = self.config.swipe_min as f32;
        let dir = if ax >= min && ax >= ay * self.config.axis_ratio {
            if dx < 0 {
                Direction::Left
            } else {
                Direction::Right
            }
        } else if ay >= min && ay >= ax * self.config.axis_ratio {
            if dy < 0 {
                Direction::Up
            } else {
                Direction::Down
            }
        } else {
            return None;
        };
        Some(Gesture::Swipe { dir, from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(phase: Phase, id: usize, x: i32, y: i32, ms: u64) -> TouchEvent {
        TouchEvent {
            phase,
            id,
            x,
            y,
            time_us: ms * 1000,
        }
    }

    fn run(events: &[TouchEvent]) -> Vec<Gesture> {
        let mut r = GestureRecognizer::default();
        events.iter().flat_map(|&e| r.feed(e)).collect()
    }

    #[test]
    fn tap_with_small_wobble() {
        let g = run(&[
            ev(Phase::Down, 0, 500, 500, 0),
            ev(Phase::Move, 0, 510, 495, 40),
            ev(Phase::Up, 0, 512, 497, 90),
        ]);
        assert_eq!(
            g,
            vec![
                Gesture::Press(Point { x: 500, y: 500 }),
                Gesture::Tap(Point { x: 512, y: 497 })
            ]
        );
    }

    #[test]
    fn swipe_left_cancels_press() {
        let g = run(&[
            ev(Phase::Down, 0, 800, 700, 0),
            ev(Phase::Move, 0, 700, 705, 30),
            ev(Phase::Move, 0, 400, 720, 80),
            ev(Phase::Up, 0, 300, 725, 120),
        ]);
        assert_eq!(g[0], Gesture::Press(Point { x: 800, y: 700 }));
        assert_eq!(g[1], Gesture::Cancel);
        assert!(matches!(
            g[2],
            Gesture::Swipe {
                dir: Direction::Left,
                ..
            }
        ));
        assert_eq!(g.len(), 3);
    }

    #[test]
    fn diagonal_drag_is_not_a_swipe() {
        let g = run(&[
            ev(Phase::Down, 0, 300, 300, 0),
            ev(Phase::Move, 0, 450, 430, 50),
            ev(Phase::Up, 0, 450, 430, 100),
        ]);
        assert_eq!(
            g,
            vec![Gesture::Press(Point { x: 300, y: 300 }), Gesture::Cancel]
        );
    }

    #[test]
    fn long_press_from_tick_suppresses_tap() {
        let mut r = GestureRecognizer::default();
        assert_eq!(r.feed(ev(Phase::Down, 0, 100, 100, 0)).len(), 1);
        assert_eq!(r.next_deadline(100_000), Some(400_000));
        assert_eq!(r.tick(300_000), None);
        assert_eq!(
            r.tick(520_000),
            Some(Gesture::LongPress(Point { x: 100, y: 100 }))
        );
        assert_eq!(r.tick(900_000), None, "fires once");
        assert_eq!(r.feed(ev(Phase::Up, 0, 102, 101, 1000)), vec![]);
    }

    #[test]
    fn long_press_detected_on_late_event_without_tick() {
        let g = run(&[
            ev(Phase::Down, 0, 100, 100, 0),
            ev(Phase::Up, 0, 100, 100, 700),
        ]);
        assert_eq!(g[1], Gesture::LongPress(Point { x: 100, y: 100 }));
        assert_eq!(g.len(), 2);
    }

    #[test]
    fn second_finger_cancels_and_is_ignored() {
        let g = run(&[
            ev(Phase::Down, 0, 100, 100, 0),
            ev(Phase::Down, 1, 600, 600, 20),
            ev(Phase::Up, 1, 600, 600, 60),
            ev(Phase::Up, 0, 100, 100, 80),
            ev(Phase::Down, 0, 200, 200, 300),
            ev(Phase::Up, 0, 200, 200, 350),
        ]);
        assert_eq!(
            g,
            vec![
                Gesture::Press(Point { x: 100, y: 100 }),
                Gesture::Cancel,
                Gesture::Press(Point { x: 200, y: 200 }),
                Gesture::Tap(Point { x: 200, y: 200 }),
            ]
        );
    }

    #[test]
    fn vertical_swipe() {
        let g = run(&[
            ev(Phase::Down, 0, 500, 1000, 0),
            ev(Phase::Move, 0, 505, 700, 60),
            ev(Phase::Up, 0, 505, 600, 100),
        ]);
        assert!(matches!(
            g.last(),
            Some(Gesture::Swipe {
                dir: Direction::Up,
                ..
            })
        ));
    }
}
