//! Reading gestures: page turns on touch-down, menu on a tap at the top.
//!
//! The owner wants a turn to start the moment the finger lands, not when it
//! lifts: the panel needs about half a second for a clean full-screen
//! update, and waiting for the release adds the whole tap (often 80 to
//! 150 ms) on top. So a touch-down in a side zone turns at once: the left
//! 30 % goes back, the rest forward (the zones of
//! [`aristotle_ui::screens::reader::zones`]).
//!
//! Horizontal swipes also turn, and they say what the reader meant better
//! than where the finger landed. The rule is that a gesture ends one page
//! away from where it started, in the swipe's direction: a swipe left that
//! began in the forward zone has already turned and needs nothing more; a
//! swipe right that began there (back, after an accidental forward turn)
//! needs two pages back. The top strip opens the menu on a tap (a release),
//! since the menu is not worth an accidental open.

use aristotle_ui::geom::{Point, Rect};
use aristotle_ui::input::{Direction, Gesture, GestureRecognizer, TouchEvent};
use aristotle_ui::screens::reader::{zone_at, Zone};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    Forward,
    Back,
}

/// What the reader should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Turn one page. `at_us` is the kernel timestamp of the touch event that
    /// caused it, for latency logging.
    Turn {
        dir: Turn,
        at_us: u64,
    },
    OpenMenu,
}

/// Turns raw touch events into [`Intent`]s while the page is showing.
pub struct TurnGestures {
    screen: Rect,
    recognizer: GestureRecognizer,
    /// The turn already made on touch-down for the current contact.
    down_turn: Option<Turn>,
}

impl TurnGestures {
    pub fn new(screen: Rect) -> Self {
        Self {
            screen,
            recognizer: GestureRecognizer::default(),
            down_turn: None,
        }
    }

    /// Forget the contact in progress (after the menu took over input).
    pub fn reset(&mut self) {
        self.recognizer = GestureRecognizer::new(self.recognizer.config);
        self.down_turn = None;
    }

    pub fn feed(&mut self, ev: TouchEvent) -> Vec<Intent> {
        let mut out = Vec::new();
        for g in self.recognizer.feed(ev) {
            self.gesture(g, ev.time_us, &mut out);
        }
        out
    }

    fn gesture(&mut self, g: Gesture, at_us: u64, out: &mut Vec<Intent>) {
        match g {
            Gesture::Press(p) => {
                self.down_turn = match zone_at(self.screen, p) {
                    Zone::Prev => Some(Turn::Back),
                    Zone::Next => Some(Turn::Forward),
                    Zone::Menu => None,
                };
                if let Some(dir) = self.down_turn {
                    out.push(Intent::Turn { dir, at_us });
                }
            }
            Gesture::Tap(p) => {
                if self.down_turn.is_none() && self.in_menu_zone(p) {
                    out.push(Intent::OpenMenu);
                }
                self.down_turn = None;
            }
            Gesture::Swipe { dir, .. } => {
                let wanted = match dir {
                    Direction::Left => Some(Turn::Forward),
                    Direction::Right => Some(Turn::Back),
                    Direction::Down if self.down_turn.is_none() => {
                        out.push(Intent::OpenMenu);
                        None
                    }
                    _ => None,
                };
                if let Some(wanted) = wanted {
                    let turns = match self.down_turn {
                        None => 1,
                        Some(done) if done == wanted => 0,
                        // Undo the touch-down turn, then go the wanted way.
                        Some(_) => 2,
                    };
                    for _ in 0..turns {
                        out.push(Intent::Turn { dir: wanted, at_us });
                    }
                }
                self.down_turn = None;
            }
            Gesture::LongPress(_) | Gesture::Cancel => {}
        }
    }

    fn in_menu_zone(&self, p: Point) -> bool {
        zone_at(self.screen, p) == Zone::Menu
    }
}

/// The page a sequence of turns leaves the reader on, relative to the start
/// (for tests and reasoning: +1 is one page forward).
#[cfg(test)]
pub fn net_pages(intents: &[Intent]) -> i32 {
    intents
        .iter()
        .map(|i| match i {
            Intent::Turn {
                dir: Turn::Forward, ..
            } => 1,
            Intent::Turn {
                dir: Turn::Back, ..
            } => -1,
            Intent::OpenMenu => 0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aristotle_ui::geom::mm;
    use aristotle_ui::input::Phase;

    fn ev(phase: Phase, x: i32, y: i32, t_ms: u64) -> TouchEvent {
        TouchEvent {
            phase,
            id: 0,
            x,
            y,
            time_us: t_ms * 1000,
        }
    }

    fn run(events: &[TouchEvent]) -> Vec<Intent> {
        let mut g = TurnGestures::new(Rect::screen());
        events.iter().flat_map(|e| g.feed(*e)).collect()
    }

    #[test]
    fn touch_down_turns_before_the_release() {
        let mut g = TurnGestures::new(Rect::screen());
        // Right of the 30 % line: forward, on the down event itself.
        let out = g.feed(ev(Phase::Down, 900, 800, 10));
        assert_eq!(
            out,
            vec![Intent::Turn {
                dir: Turn::Forward,
                at_us: 10_000
            }]
        );
        assert!(g.feed(ev(Phase::Up, 900, 800, 90)).is_empty());
        // Left 30 %: back.
        let out = g.feed(ev(Phase::Down, 150, 800, 200));
        assert_eq!(net_pages(&out), -1);
        assert!(g.feed(ev(Phase::Up, 150, 800, 260)).is_empty());
    }

    #[test]
    fn zones_split_at_thirty_percent() {
        let x30 = Rect::screen().w * 30 / 100;
        assert_eq!(net_pages(&run(&[ev(Phase::Down, x30 - 2, 700, 0)])), -1);
        assert_eq!(net_pages(&run(&[ev(Phase::Down, x30 + 2, 700, 0)])), 1);
    }

    #[test]
    fn top_strip_opens_the_menu_on_release_only() {
        let mut g = TurnGestures::new(Rect::screen());
        assert!(g.feed(ev(Phase::Down, 500, 60, 0)).is_empty());
        assert_eq!(g.feed(ev(Phase::Up, 500, 60, 80)), vec![Intent::OpenMenu]);
    }

    #[test]
    fn swipes_end_one_page_away_in_their_direction() {
        let travel = mm(20.0);
        // Swipe left starting in the forward zone: the down already turned.
        let out = run(&[
            ev(Phase::Down, 900, 700, 0),
            ev(Phase::Move, 900 - travel, 700, 60),
            ev(Phase::Up, 900 - travel, 700, 90),
        ]);
        assert_eq!(net_pages(&out), 1);
        // Swipe right starting in the forward zone: back to one before start.
        let out = run(&[
            ev(Phase::Down, 400, 700, 0),
            ev(Phase::Move, 400 + travel, 700, 60),
            ev(Phase::Up, 400 + travel, 700, 90),
        ]);
        assert_eq!(net_pages(&out), -1);
        assert_eq!(out.len(), 3, "{out:?}");
        // Swipe left starting in the back zone: forward one from the start.
        let out = run(&[
            ev(Phase::Down, 250, 700, 0),
            ev(Phase::Move, 250 - travel.min(240), 700, 60),
            ev(Phase::Up, 250 - travel.min(240), 700, 90),
        ]);
        assert_eq!(net_pages(&out), 1);
        // A swipe from the top strip turns once.
        let out = run(&[
            ev(Phase::Down, 300, 80, 0),
            ev(Phase::Move, 300 + travel, 80, 60),
            ev(Phase::Up, 300 + travel, 80, 90),
        ]);
        assert_eq!(net_pages(&out), -1);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn swipe_down_from_the_top_opens_the_menu() {
        let travel = mm(20.0);
        let out = run(&[
            ev(Phase::Down, 500, 40, 0),
            ev(Phase::Move, 500, 40 + travel, 60),
            ev(Phase::Up, 500, 40 + travel, 90),
        ]);
        assert_eq!(out, vec![Intent::OpenMenu]);
    }

    #[test]
    fn second_finger_does_not_turn_twice() {
        let mut g = TurnGestures::new(Rect::screen());
        let mut out = g.feed(ev(Phase::Down, 900, 700, 0));
        out.extend(g.feed(TouchEvent {
            id: 1,
            ..ev(Phase::Down, 200, 700, 20)
        }));
        out.extend(g.feed(ev(Phase::Up, 900, 700, 60)));
        out.extend(g.feed(TouchEvent {
            id: 1,
            ..ev(Phase::Up, 200, 700, 70)
        }));
        assert_eq!(net_pages(&out), 1);
    }
}
