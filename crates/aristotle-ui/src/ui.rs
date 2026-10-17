//! The UI driver: turns tree changes into minimal damage with waveform
//! hints, and touch events into actions.
//!
//! # Damage
//!
//! Each [`Ui::render`] works in three steps:
//!
//! 1. **Candidates.** Every node that changed (set_text, set_progress, a
//!    custom widget invalidated, a press) contributes its rectangle, and
//!    every painting node whose rectangle moved, appeared or disappeared
//!    during layout contributes both its old and new rectangle.
//! 2. **Repaint.** Each candidate region is repainted from the tree into
//!    the canvas, clipped to the region.
//! 3. **Tighten.** The repainted region is compared with what the panel
//!    already shows (`front`). Only the bounding box of pixels that really
//!    differ becomes damage; an identical repaint produces none.
//!
//! The waveform of each damage rectangle follows from why it changed:
//! A2 for transient press feedback, DU when the new pixels are pure black
//! and white, GL16 when they contain grays (anti-aliased text), and a
//! full-screen GC16 for new screens, night-mode switches and every n-th page
//! turn, to clear ghosting.
//!
//! # Device hand-off
//!
//! ```text
//! for d in ui.render() {
//!     ui.screen().blit_into(d.rect, fb_pixels, fb_stride);
//!     display.damage(hal_rect(d.rect), Waveform(d.waveform.mode()), &[]);
//! }
//! ```
//!
//! `Display` coalesces whatever arrives while the panel is busy, so the UI
//! never waits for the panel.

use crate::canvas::{Canvas, Painter};
use crate::geom::{Point, Rect};
use crate::input::{Direction, Gesture, GestureRecognizer, TouchEvent};
use crate::text::TextEngine;
use crate::tree::{ActionId, Feedback, Hit, NodeId, Refresh, Tree};
use crate::waveform::Waveform;

/// A rectangle to send to the panel, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Damage {
    pub rect: Rect,
    pub waveform: Waveform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiEvent {
    /// A node with an action was tapped.
    Action {
        id: ActionId,
        node: NodeId,
        at: Point,
    },
    /// A node with an action was held.
    LongAction {
        id: ActionId,
        node: NodeId,
        at: Point,
    },
    /// A tap on nothing actionable; the screen decides (reader tap zones).
    Tap(Point),
    LongPress(Point),
    Swipe(Direction),
}

pub struct Ui {
    pub tree: Tree,
    pub text: TextEngine,
    /// What the panel should show: the tree painted, inverted in night mode.
    canvas: Canvas,
    /// What the panel shows now, as far as we know.
    front: Canvas,
    screen: Rect,
    night: bool,
    /// Every n-th page turn is a full GC16 refresh. Six follows KOReader's
    /// default; the PW4's REAGL-era panel ghosts little, so it can go up.
    pub full_refresh_every: u32,
    pages_since_full: u32,
    full_pending: bool,
    gestures: GestureRecognizer,
    pressed: Option<NodeId>,
    snapshot: Vec<Rect>,
}

impl Ui {
    pub fn new(tree: Tree, text: TextEngine, width: u32, height: u32) -> Self {
        Self {
            tree,
            text,
            canvas: Canvas::new(width, height),
            front: Canvas::new(width, height),
            screen: Rect::new(0, 0, width as i32, height as i32),
            night: false,
            full_refresh_every: 6,
            pages_since_full: 0,
            full_pending: true,
            gestures: GestureRecognizer::default(),
            pressed: None,
            snapshot: Vec::new(),
        }
    }

    /// The pixels the panel should show after the last render.
    pub fn screen(&self) -> &Canvas {
        &self.front
    }

    pub fn bounds(&self) -> Rect {
        self.screen
    }

    /// Replaces the whole screen, e.g. going from the library to a book.
    /// A context switch gets a flashing GC16, which also clears ghosting.
    pub fn set_tree(&mut self, tree: Tree) -> Tree {
        self.pressed = None;
        self.full_pending = true;
        self.snapshot.clear();
        std::mem::replace(&mut self.tree, tree)
    }

    pub fn night(&self) -> bool {
        self.night
    }

    /// Night mode inverts the final pixels rather than restyling widgets,
    /// so every screen supports it and anti-aliasing stays correct.
    pub fn set_night(&mut self, on: bool) {
        if self.night != on {
            self.night = on;
            self.full_pending = true;
        }
    }

    pub fn request_full_refresh(&mut self) {
        self.full_pending = true;
    }

    fn ensure_layout(&mut self) {
        if self.tree.needs_layout {
            self.tree.layout(self.screen, &mut self.text);
        }
    }

    /// Repaints what changed and returns the damage, smallest first.
    pub fn render(&mut self) -> Vec<Damage> {
        self.ensure_layout();
        let now = self.tree.snapshot();
        let mut regions: Vec<(Rect, Refresh)> = Vec::new();
        if now.len() == self.snapshot.len() {
            for (old, new) in self.snapshot.iter().zip(&now) {
                if old != new {
                    regions.push((*old, Refresh::Auto));
                    regions.push((*new, Refresh::Auto));
                }
            }
        } else {
            self.full_pending = true;
        }
        self.snapshot = now;
        let changes = std::mem::take(&mut self.tree.changes);
        let mut rects = Vec::new();
        for ch in changes {
            rects.clear();
            self.tree.rects_of(ch.node, &mut rects);
            regions.extend(rects.iter().map(|r| (*r, ch.refresh)));
            if ch.refresh == Refresh::Page {
                self.pages_since_full += 1;
                if self.pages_since_full >= self.full_refresh_every {
                    self.full_pending = true;
                }
            }
        }
        if self.full_pending {
            self.full_pending = false;
            self.pages_since_full = 0;
            self.repaint(self.screen);
            self.front.copy_from(&self.canvas, self.screen);
            return vec![Damage {
                rect: self.screen,
                waveform: Waveform::GC16,
            }];
        }

        let mut damage = Vec::new();
        for (rect, refresh) in regions {
            let Some(r) = rect.intersect(&self.screen) else {
                continue;
            };
            self.repaint(r);
            let tight = match refresh {
                Refresh::Full => Some(r),
                _ => self.canvas.diff_bounds(&self.front, r),
            };
            let Some(t) = tight else { continue };
            let waveform = match refresh {
                Refresh::Transient => Waveform::A2,
                Refresh::Full => Waveform::GC16,
                Refresh::Auto | Refresh::Page => {
                    if self.canvas.is_bilevel(t) {
                        Waveform::DU
                    } else {
                        Waveform::GL16
                    }
                }
            };
            self.front.copy_from(&self.canvas, t);
            damage.push(Damage { rect: t, waveform });
        }
        let mut merged = merge(damage);
        merged.sort_by_key(|d| d.rect.area());
        merged
    }

    fn repaint(&mut self, r: Rect) {
        let mut p = Painter::new(&mut self.canvas, r);
        self.tree.paint(&mut p, &mut self.text);
        if self.night {
            p.invert_rect(r);
        }
    }

    /// Microseconds until [`Ui::tick`] has work (a pending long press).
    pub fn next_deadline(&self, now_us: u64) -> Option<u64> {
        self.gestures.next_deadline(now_us)
    }

    pub fn tick(&mut self, now_us: u64) -> Vec<UiEvent> {
        let mut out = Vec::new();
        if let Some(g) = self.gestures.tick(now_us) {
            self.on_gesture(g, &mut out);
        }
        out
    }

    pub fn handle_touch(&mut self, ev: TouchEvent) -> Vec<UiEvent> {
        let mut out = Vec::new();
        for g in self.gestures.feed(ev) {
            self.on_gesture(g, &mut out);
        }
        out
    }

    fn release(&mut self) -> Option<NodeId> {
        let id = self.pressed.take()?;
        if self.tree.node(id).pressed {
            self.tree.set_pressed(id, false);
            self.tree.invalidate(id, Refresh::Auto);
        }
        Some(id)
    }

    fn on_gesture(&mut self, g: Gesture, out: &mut Vec<UiEvent>) {
        self.ensure_layout();
        match g {
            Gesture::Press(p) => {
                if let Hit::Target(id) = self.tree.hit(p) {
                    self.pressed = Some(id);
                    if self.tree.feedback(id) != Feedback::None {
                        self.tree.set_pressed(id, true);
                        self.tree.invalidate(id, Refresh::Transient);
                    }
                }
            }
            Gesture::Cancel => {
                self.release();
            }
            Gesture::Tap(p) => match self.release() {
                Some(node) => {
                    if let Some(id) = self.tree.action(node) {
                        out.push(UiEvent::Action { id, node, at: p });
                    }
                }
                None => {
                    if self.tree.hit(p) == Hit::Nothing {
                        out.push(UiEvent::Tap(p));
                    }
                }
            },
            Gesture::LongPress(p) => match self.release() {
                Some(node) => {
                    if let Some(id) = self.tree.action(node) {
                        out.push(UiEvent::LongAction { id, node, at: p });
                    }
                }
                None => {
                    if self.tree.hit(p) == Hit::Nothing {
                        out.push(UiEvent::LongPress(p));
                    }
                }
            },
            Gesture::Swipe { dir, .. } => out.push(UiEvent::Swipe(dir)),
        }
    }
}

/// Merges damage rectangles with the same waveform when they overlap or
/// when their union wastes little area (at most 25 % more than the two
/// parts). Fewer, slightly larger updates beat many small ones because the
/// driver runs one update at a time (`docs/eink-rendering.md`). Rectangles
/// with different waveforms stay apart, so a fast A2 highlight is never
/// slowed down by a GL16 neighbour.
pub fn merge(mut damage: Vec<Damage>) -> Vec<Damage> {
    loop {
        let mut merged = false;
        'outer: for i in 0..damage.len() {
            for j in i + 1..damage.len() {
                let (a, b) = (damage[i], damage[j]);
                if a.waveform != b.waveform {
                    continue;
                }
                let u = a.rect.union(&b.rect);
                let cheap = u.area() * 4 <= (a.rect.area() + b.rect.area()) * 5;
                if a.rect.intersects(&b.rect) || cheap {
                    damage[i].rect = u;
                    damage.swap_remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            return damage;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Insets;
    use crate::input::Phase;
    use crate::text::{FamilyId, Fonts, TextStyle};
    use crate::tree::{progress, Kind, Node};

    fn ui(build: impl FnOnce(&mut Tree)) -> Ui {
        let mut t = Tree::new();
        build(&mut t);
        let mut ui = Ui::new(t, TextEngine::new(Fonts::embedded()), 600, 800);
        let first = ui.render();
        assert_eq!(
            first,
            vec![Damage {
                rect: Rect::new(0, 0, 600, 800),
                waveform: Waveform::GC16
            }]
        );
        ui
    }

    fn style() -> TextStyle {
        TextStyle::new(FamilyId(0), 40.0)
    }

    #[test]
    fn nothing_changed_means_no_damage() {
        let mut label = NodeId(0);
        let mut u = ui(|t| {
            label = t.label("Hello", style());
            let root = t.pad(Insets::all(50), label);
            t.set_root(root);
        });
        assert!(u.render().is_empty());
        u.tree.set_text(label, "Hello");
        assert!(u.render().is_empty());
    }

    #[test]
    fn text_change_damages_only_the_text_with_gl16() {
        let mut label = NodeId(0);
        let mut u = ui(|t| {
            label = t.label("Page 12", style());
            let other = t.label("Static", style());
            let col = t.add(Node::column(20), [other, label]);
            let root = t.pad(Insets::all(50), col);
            t.set_root(root);
        });
        let before = u.tree.rect(label);
        u.tree.set_text(label, "Page 13");
        let d = u.render();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].waveform, Waveform::GL16);
        // Only the changed digit, within the label.
        assert!(before.intersect(&d[0].rect) == Some(d[0].rect));
        assert!(
            d[0].rect.w < before.w / 3,
            "{:?} vs {:?}",
            d[0].rect,
            before
        );
    }

    #[test]
    fn progress_step_is_a_thin_du_strip() {
        let mut bar = NodeId(0);
        let mut u = ui(|t| {
            bar = t.add(progress(0.40, 12), []);
            let root = t.pad(Insets::all(50), bar);
            t.set_root(root);
        });
        u.tree.set_progress(bar, 0.41);
        let d = u.render();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].waveform, Waveform::DU);
        assert!(d[0].rect.w <= 8, "{:?}", d[0].rect);
        assert!(d[0].rect.h <= 6);
    }

    #[test]
    fn press_is_a2_and_release_restores_with_an_action() {
        let mut btn = NodeId(0);
        let mut u = ui(|t| {
            let l = t.label("Open", style());
            btn = t.button(Node::new(Kind::Spacer).border(3, 0, 12), l, 42);
            let root = t.align(0.5, 0.5, btn);
            t.set_root(root);
        });
        let c = u.tree.rect(btn).center();
        let touch = |phase, ms: u64| TouchEvent {
            phase,
            id: 0,
            x: c.x,
            y: c.y,
            time_us: ms * 1000,
        };
        assert!(u.handle_touch(touch(Phase::Down, 0)).is_empty());
        let d = u.render();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].waveform, Waveform::A2);
        assert!(u.tree.rect(btn).intersect(&d[0].rect) == Some(d[0].rect));
        let ev = u.handle_touch(touch(Phase::Up, 80));
        assert!(matches!(ev[..], [UiEvent::Action { id: 42, .. }]));
        let d = u.render();
        assert_eq!(d.len(), 1);
        // The label has gray anti-aliased edges, so the restore is GL16.
        assert_eq!(d[0].waveform, Waveform::GL16);
    }

    #[test]
    fn tap_outside_targets_is_reported_for_zones() {
        let mut u = ui(|t| {
            let l = t.label("Text", style());
            t.set_root(l);
        });
        let ev = u.handle_touch(TouchEvent {
            phase: Phase::Down,
            id: 0,
            x: 500,
            y: 400,
            time_us: 0,
        });
        assert!(ev.is_empty());
        let ev = u.handle_touch(TouchEvent {
            phase: Phase::Up,
            id: 0,
            x: 500,
            y: 400,
            time_us: 50_000,
        });
        assert_eq!(ev, vec![UiEvent::Tap(Point { x: 500, y: 400 })]);
        assert!(u.render().is_empty(), "no feedback for zone taps");
    }

    #[test]
    fn every_nth_page_turn_is_full_gc16() {
        let mut body = NodeId(0);
        let mut u = ui(|t| {
            body = t.label("1", style());
            let root = t.pad(Insets::all(50), body);
            t.set_root(root);
        });
        u.full_refresh_every = 3;
        let mut kinds = Vec::new();
        for page in 2..=7 {
            u.tree.set_text(body, &page.to_string());
            u.tree.invalidate(body, Refresh::Page);
            let d = u.render();
            kinds.push(d.iter().map(|d| d.waveform).collect::<Vec<_>>());
        }
        assert_eq!(kinds[0], vec![Waveform::GL16]);
        assert_eq!(kinds[1], vec![Waveform::GL16]);
        assert_eq!(kinds[2], vec![Waveform::GC16]);
        assert_eq!(kinds[5], vec![Waveform::GC16]);
    }

    #[test]
    fn showing_an_overlay_damages_only_its_bars() {
        let mut overlay = NodeId(0);
        let mut u = ui(|t| {
            let page = t.label("Page text", style());
            let top = t.add(
                Node::new(Kind::Fixed {
                    w: None,
                    h: Some(100),
                })
                .bg(255)
                .border(2, 0, 0),
                [],
            );
            let gap = t.spacer(1);
            let bottom = t.add(
                Node::new(Kind::Fixed {
                    w: None,
                    h: Some(200),
                })
                .bg(255)
                .border(2, 0, 0),
                [],
            );
            overlay = t.add(Node::column(0).hidden(true), [top, gap, bottom]);
            let root = t.add(Node::new(Kind::Stack), [page, overlay]);
            t.set_root(root);
        });
        u.tree.set_hidden(overlay, false);
        let d = u.render();
        let total: i64 = d.iter().map(|d| d.rect.area()).sum();
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(total <= 600 * 300, "middle of the screen untouched: {d:?}");
        // Pure black-and-white bars go out with DU.
        assert!(d.iter().all(|d| d.waveform == Waveform::DU));
        u.tree.set_hidden(overlay, true);
        let d = u.render();
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn night_mode_inverts_with_a_full_refresh() {
        let mut u = ui(|t| {
            let l = t.label("Text", style());
            t.set_root(l);
        });
        assert_eq!(u.screen().get(599, 799), 255);
        u.set_night(true);
        let d = u.render();
        assert_eq!(d[0].waveform, Waveform::GC16);
        assert_eq!(u.screen().get(599, 799), 0);
    }

    #[test]
    fn merge_rules() {
        let d = |x, y, w, h, wf| Damage {
            rect: Rect::new(x, y, w, h),
            waveform: wf,
        };
        // Adjacent same-waveform rects merge.
        let m = merge(vec![
            d(0, 0, 100, 50, Waveform::GL16),
            d(0, 52, 100, 50, Waveform::GL16),
        ]);
        assert_eq!(m, vec![d(0, 0, 100, 102, Waveform::GL16)]);
        // Far apart: the union would waste area, so they stay separate.
        let m = merge(vec![
            d(0, 0, 50, 50, Waveform::DU),
            d(500, 500, 50, 50, Waveform::DU),
        ]);
        assert_eq!(m.len(), 2);
        // Different waveforms never merge, even when overlapping.
        let m = merge(vec![
            d(0, 0, 100, 100, Waveform::A2),
            d(50, 50, 100, 100, Waveform::GL16),
        ]);
        assert_eq!(m.len(), 2);
    }
}
