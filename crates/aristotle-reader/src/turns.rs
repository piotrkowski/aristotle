//! Reading gestures: page turns on release, the menu on a tap at the top,
//! and the text gestures: hold a word, drag to extend, tap a footnote or
//! the bookmark corner.
//!
//! A tap turns when the finger lifts: the left 30 % goes back, the rest
//! forward (the zones of [`aristotle_ui::screens::reader::zones`]). The
//! next page is prerendered, so the submit follows the release at once.
//! Turning on touch-down instead saved the tap's own duration (80 to 150
//! ms), but then a long press on a word had always turned the page first,
//! and taking that turn back flashed the next page before the definition,
//! so turns fire on release.
//!
//! Horizontal swipes turn one page in their direction (left forward); a
//! swipe down from the top strip opens the menu.
//!
//! Text gestures:
//!
//! - **Hold** (500 ms without moving) selects the word under the finger
//!   ([`Intent::Hold`]); a hold never turns. While the finger stays down
//!   every move is an [`Intent::Drag`] and the lift an
//!   [`Intent::Release`].
//! - **Quiet zones** (footnote markers and other links, the bookmark
//!   corner): a tap there is an [`Intent::Tap`] for the reader to resolve,
//!   not a turn.

use aristotle_ui::geom::{Point, Rect};
use aristotle_ui::input::{Direction, Gesture, GestureRecognizer, Phase, TouchEvent};
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
    /// caused it (the release), for latency logging.
    Turn {
        dir: Turn,
        at_us: u64,
    },
    OpenMenu,
    /// A long press: select the word at `at`.
    Hold {
        at: Point,
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
    /// Where a tap does not turn (links, the bookmark corner).
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
            quiet: vec![bookmark_corner(screen)],
            holding: false,
        }
    }

    /// Forget the contact in progress (after the menu took over input).
    pub fn reset(&mut self) {
        self.recognizer = GestureRecognizer::new(self.recognizer.config);
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
        if !was_holding {
            for g in gestures {
                self.gesture(g, ev.time_us, &mut out);
            }
        }
        // While holding (a long press found by the loop's tick, or late, by
        // this event's timestamp), the recognizer still tracks the contact
        // so it is idle after the lift, but its tap or swipe means nothing.
        if self.holding {
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
            // Nothing on touch-down: a hold must not have turned the page.
            Gesture::Press(_) | Gesture::Cancel => {}
            Gesture::Tap(p) => {
                if self.is_quiet(p) {
                    out.push(Intent::Tap(p));
                } else {
                    match zone_at(self.screen, p) {
                        Zone::Menu => out.push(Intent::OpenMenu),
                        Zone::Prev => out.push(Intent::Turn {
                            dir: Turn::Back,
                            at_us,
                        }),
                        Zone::Next => out.push(Intent::Turn {
                            dir: Turn::Forward,
                            at_us,
                        }),
                    }
                }
            }
            Gesture::LongPress(p) => {
                self.holding = true;
                out.push(Intent::Hold { at: p });
            }
            Gesture::Swipe { dir, from, .. } => match dir {
                Direction::Left => out.push(Intent::Turn {
                    dir: Turn::Forward,
                    at_us,
                }),
                Direction::Right => out.push(Intent::Turn {
                    dir: Turn::Back,
                    at_us,
                }),
                Direction::Down if zone_at(self.screen, from) == Zone::Menu => {
                    out.push(Intent::OpenMenu)
                }
                _ => {}
            },
        }
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
    fn taps_turn_on_release() {
        let mut g = TurnGestures::new(Rect::screen());
        // Right of the 30 % line: nothing on the down event, forward on the
        // release, timed from the release.
        assert!(g.feed(ev(Phase::Down, 900, 800, 10)).is_empty());
        assert_eq!(
            g.feed(ev(Phase::Up, 900, 800, 90)),
            vec![Intent::Turn {
                dir: Turn::Forward,
                at_us: 90_000
            }]
        );
        // Left 30 %: back.
        assert!(g.feed(ev(Phase::Down, 150, 800, 200)).is_empty());
        assert_eq!(net_pages(&g.feed(ev(Phase::Up, 150, 800, 260))), -1);
    }

    #[test]
    fn zones_split_at_thirty_percent() {
        let x30 = Rect::screen().w * 30 / 100;
        let tap = |x| run(&[ev(Phase::Down, x, 700, 0), ev(Phase::Up, x, 700, 60)]);
        assert_eq!(net_pages(&tap(x30 - 2)), -1);
        assert_eq!(net_pages(&tap(x30 + 2)), 1);
    }

    #[test]
    fn top_strip_opens_the_menu_on_release() {
        let mut g = TurnGestures::new(Rect::screen());
        assert!(g.feed(ev(Phase::Down, 500, 60, 0)).is_empty());
        assert_eq!(g.feed(ev(Phase::Up, 500, 60, 80)), vec![Intent::OpenMenu]);
    }

    #[test]
    fn swipes_turn_once_in_their_direction() {
        let travel = mm(20.0);
        let swipe = |x0: i32, x1: i32, y| {
            run(&[
                ev(Phase::Down, x0, y, 0),
                ev(Phase::Move, x1, y, 60),
                ev(Phase::Up, x1, y, 90),
            ])
        };
        // Wherever they start.
        assert_eq!(net_pages(&swipe(900, 900 - travel, 700)), 1);
        assert_eq!(net_pages(&swipe(400, 400 + travel, 700)), -1);
        assert_eq!(net_pages(&swipe(250, 250 - travel.min(240), 700)), 1);
        let out = swipe(300, 300 + travel, 80);
        assert_eq!(out.len(), 1);
        assert_eq!(net_pages(&out), -1);
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
    fn second_finger_does_not_turn() {
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
        assert_eq!(net_pages(&out), 0);
    }

    #[test]
    fn a_hold_never_turns_and_drags_until_the_lift() {
        let mut g = TurnGestures::new(Rect::screen());
        assert!(g.feed(ev(Phase::Down, 600, 700, 0)).is_empty());
        // The loop's timeout fires the long press while the finger rests.
        assert!(g.next_deadline(100_000).is_some());
        assert_eq!(
            g.tick(510_000),
            vec![Intent::Hold {
                at: Point { x: 600, y: 700 }
            }]
        );
        assert!(g.holding());
        // Moving, even far, drags; it never turns or swipes.
        let mut out = g.feed(ev(Phase::Move, 640, 702, 600));
        out.extend(g.feed(ev(Phase::Move, 900, 760, 700)));
        assert_eq!(
            out,
            vec![
                Intent::Drag(Point { x: 640, y: 702 }),
                Intent::Drag(Point { x: 900, y: 760 })
            ]
        );
        let out = g.feed(ev(Phase::Up, 900, 760, 800));
        assert_eq!(out, vec![Intent::Release(Point { x: 900, y: 760 })]);
        assert!(!g.holding());
        // The next tap turns again.
        g.feed(ev(Phase::Down, 600, 700, 2000));
        assert_eq!(net_pages(&g.feed(ev(Phase::Up, 600, 700, 2060))), 1);
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
                    at: Point { x: 150, y: 700 }
                },
                Intent::Release(Point { x: 150, y: 700 })
            ]
        );
    }

    #[test]
    fn quiet_zones_tap_instead_of_turning() {
        let mut g = TurnGestures::new(Rect::screen());
        g.set_quiet(vec![Rect::new(500, 600, 106, 106)]);
        // A footnote marker.
        assert!(g.feed(ev(Phase::Down, 550, 650, 0)).is_empty());
        assert_eq!(
            g.feed(ev(Phase::Up, 551, 650, 80)),
            vec![Intent::Tap(Point { x: 551, y: 650 })]
        );
        // The bookmark corner is in the menu strip but is its own target.
        let corner = bookmark_corner(Rect::screen());
        let (x, y) = (corner.x + 50, corner.y + 50);
        g.feed(ev(Phase::Down, x, y, 1000));
        assert_eq!(
            g.feed(ev(Phase::Up, x, y, 1080)),
            vec![Intent::Tap(Point { x, y })]
        );
        // Elsewhere in the menu strip: the menu.
        g.feed(ev(Phase::Down, 300, 60, 2000));
        assert_eq!(g.feed(ev(Phase::Up, 300, 60, 2080)), vec![Intent::OpenMenu]);
    }
}
