//! Reading gestures: page turns on touch-down, menu on a tap at the top,
//! and the text gestures: hold a word, drag to extend, tap a footnote or
//! the bookmark corner.
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
//!
//! Text gestures:
//!
//! - **Hold** (500 ms without moving) selects the word under the finger.
//!   The touch-down has already turned the page by then, so the intent
//!   carries that turn for the reader to undo ([`Intent::Hold`]). While
//!   the finger stays down every move is an [`Intent::Drag`] and the lift
//!   an [`Intent::Release`]; nothing turns.
//! - **Quiet zones** (footnote markers and other links, the bookmark
//!   corner) do not turn on touch-down; a tap there is an [`Intent::Tap`]
//!   for the reader to resolve. A link is a few millimetres wide, so this
//!   costs page turns almost nothing.

use aristotle_ui::geom::{Point, Rect};
use aristotle_ui::input::{Direction, Gesture, GestureRecognizer, Phase, TouchEvent};
use aristotle_ui::screens::reader::{zone_at, Zone};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    Forward,
    Back,
}

impl Turn {
    pub fn opposite(self) -> Turn {
        match self {
            Turn::Forward => Turn::Back,
            Turn::Back => Turn::Forward,
        }
    }
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
    /// A long press: select the word at `at`. `undo` is the turn the
    /// touch-down already made, to be taken back first.
    Hold {
        at: Point,
        undo: Option<Turn>,
    },
    /// The finger moved while holding.
    Drag(Point),
    /// The finger lifted after a hold.
    Release(Point),
    /// A tap in a quiet zone (a link, the bookmark corner).
    Tap(Point),
}

/// Turns raw touch events into [`Intent`]s while the page is showing.
pub struct TurnGestures {
    screen: Rect,
    recognizer: GestureRecognizer,
    /// The turn already made on touch-down for the current contact.
    down_turn: Option<Turn>,
    /// Where touch-down does not turn (links, the bookmark corner).
    quiet: Vec<Rect>,
    /// A hold is in progress: moves are drags until the lift.
    holding: bool,
}

/// The bookmark corner: the top right 10 mm square.
pub fn bookmark_corner(screen: Rect) -> Rect {
    let s = 118;
    Rect::new(screen.x + screen.w - s, screen.y, s, s)
}

impl TurnGestures {
    pub fn new(screen: Rect) -> Self {
        Self {
            screen,
            recognizer: GestureRecognizer::default(),
            down_turn: None,
            quiet: vec![bookmark_corner(screen)],
            holding: false,
        }
    }

    /// Forget the contact in progress (after the menu took over input).
    pub fn reset(&mut self) {
        self.recognizer = GestureRecognizer::new(self.recognizer.config);
        self.down_turn = None;
        self.holding = false;
    }

    /// The page's quiet zones besides the bookmark corner (link targets,
    /// already enlarged to a touch target).
    pub fn set_quiet(&mut self, zones: Vec<Rect>) {
        self.quiet = zones;
        self.quiet.push(bookmark_corner(self.screen));
    }

    /// A hold is in progress (the reader must keep feeding raw events here
    /// even while a popup is open).
    pub fn holding(&self) -> bool {
        self.holding
    }

    fn is_quiet(&self, p: Point) -> bool {
        self.quiet.iter().any(|r| r.contains(p))
    }

    /// Microseconds until [`Self::tick`] may fire a long press.
    pub fn next_deadline(&self, now_us: u64) -> Option<u64> {
        self.recognizer.next_deadline(now_us)
    }

    /// Fire a pending long press; call from the loop's timeout.
    pub fn tick(&mut self, now_us: u64) -> Vec<Intent> {
        let mut out = Vec::new();
        if let Some(g) = self.recognizer.tick(now_us) {
            self.gesture(g, now_us, &mut out);
        }
        out
    }

    pub fn feed(&mut self, ev: TouchEvent) -> Vec<Intent> {
        let mut out = Vec::new();
        let was_holding = self.holding;
        let gestures = self.recognizer.feed(ev);
        if was_holding {
            // The recognizer still tracks the contact (so it is idle after
            // the lift), but its tap or swipe means nothing now.
            let p = Point { x: ev.x, y: ev.y };
            match ev.phase {
                Phase::Move => out.push(Intent::Drag(p)),
                Phase::Up => {
                    self.holding = false;
                    self.down_turn = None;
                    out.push(Intent::Release(p));
                }
                Phase::Down => {}
            }
            return out;
        }
        for g in gestures {
            self.gesture(g, ev.time_us, &mut out);
        }
        // A long press detected late (by this event's timestamp) is
        // followed by this event itself.
        if self.holding && !was_holding {
            let p = Point { x: ev.x, y: ev.y };
            match ev.phase {
                Phase::Move => out.push(Intent::Drag(p)),
                Phase::Up => {
                    self.holding = false;
                    out.push(Intent::Release(p));
                }
                Phase::Down => {}
            }
        }
        out
    }

    fn gesture(&mut self, g: Gesture, at_us: u64, out: &mut Vec<Intent>) {
        match g {
            Gesture::Press(p) => {
                self.down_turn = if self.is_quiet(p) {
                    None
                } else {
                    match zone_at(self.screen, p) {
                        Zone::Prev => Some(Turn::Back),
                        Zone::Next => Some(Turn::Forward),
                        Zone::Menu => None,
                    }
                };
                if let Some(dir) = self.down_turn {
                    out.push(Intent::Turn { dir, at_us });
                }
            }
            Gesture::Tap(p) => {
                if self.down_turn.is_none() {
                    if self.is_quiet(p) {
                        out.push(Intent::Tap(p));
                    } else if self.in_menu_zone(p) {
                        out.push(Intent::OpenMenu);
                    }
                }
                self.down_turn = None;
            }
            Gesture::LongPress(p) => {
                self.holding = true;
                out.push(Intent::Hold {
                    at: p,
                    undo: self.down_turn.take(),
                });
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
            Gesture::Cancel => {}
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
            _ => 0,
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

    #[test]
    fn hold_undoes_the_touch_down_turn_then_drags() {
        let mut g = TurnGestures::new(Rect::screen());
        let mut out = g.feed(ev(Phase::Down, 600, 700, 0));
        assert_eq!(net_pages(&out), 1, "turned on touch-down");
        // The loop's timeout fires the long press while the finger rests.
        assert!(g.next_deadline(100_000).is_some());
        out = g.tick(510_000);
        assert_eq!(
            out,
            vec![Intent::Hold {
                at: Point { x: 600, y: 700 },
                undo: Some(Turn::Forward)
            }]
        );
        assert!(g.holding());
        // Moving, even far, drags; it never turns or swipes.
        out = g.feed(ev(Phase::Move, 640, 702, 600));
        out.extend(g.feed(ev(Phase::Move, 900, 760, 700)));
        assert_eq!(
            out,
            vec![
                Intent::Drag(Point { x: 640, y: 702 }),
                Intent::Drag(Point { x: 900, y: 760 })
            ]
        );
        out = g.feed(ev(Phase::Up, 900, 760, 800));
        assert_eq!(out, vec![Intent::Release(Point { x: 900, y: 760 })]);
        assert!(!g.holding());
        // The next touch turns again.
        assert_eq!(net_pages(&g.feed(ev(Phase::Down, 600, 700, 2000))), 1);
    }

    #[test]
    fn a_late_long_press_is_found_from_the_next_event() {
        let mut g = TurnGestures::new(Rect::screen());
        g.feed(ev(Phase::Down, 150, 700, 0));
        let out = g.feed(ev(Phase::Up, 150, 700, 700));
        assert_eq!(
            out,
            vec![
                Intent::Hold {
                    at: Point { x: 150, y: 700 },
                    undo: Some(Turn::Back)
                },
                Intent::Release(Point { x: 150, y: 700 })
            ]
        );
    }

    #[test]
    fn quiet_zones_wait_for_the_tap() {
        let mut g = TurnGestures::new(Rect::screen());
        g.set_quiet(vec![Rect::new(500, 600, 106, 106)]);
        // A footnote marker: no turn on touch-down, a tap on release.
        assert!(g.feed(ev(Phase::Down, 550, 650, 0)).is_empty());
        assert_eq!(
            g.feed(ev(Phase::Up, 551, 650, 80)),
            vec![Intent::Tap(Point { x: 551, y: 650 })]
        );
        // The bookmark corner is in the menu strip but is its own target.
        let corner = bookmark_corner(Rect::screen());
        let (x, y) = (corner.x + 50, corner.y + 50);
        assert!(g.feed(ev(Phase::Down, x, y, 1000)).is_empty());
        assert_eq!(
            g.feed(ev(Phase::Up, x, y, 1080)),
            vec![Intent::Tap(Point { x, y })]
        );
        // Elsewhere in the menu strip: the menu.
        g.feed(ev(Phase::Down, 300, 60, 2000));
        assert_eq!(g.feed(ev(Phase::Up, 300, 60, 2080)), vec![Intent::OpenMenu]);
        // Holding a footnote marker selects it, with nothing to undo.
        g.feed(ev(Phase::Down, 550, 650, 3000));
        assert_eq!(
            g.tick(3_600_000),
            vec![Intent::Hold {
                at: Point { x: 550, y: 650 },
                undo: None
            }]
        );
    }
}
