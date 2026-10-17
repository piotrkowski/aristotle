//! Custom widgets the generic node kinds do not cover.
//!
//! All of them draw in solid black and white wherever possible, so their
//! state changes qualify for the fast DU waveform, and keep grays for text
//! anti-aliasing and the odd quiet track.

use crate::canvas::{Painter, BLACK, WHITE};
use crate::geom::{Point, Rect, Size};
use crate::text::{
    layout_paragraph, Align, FamilyId, NoHyphenation, Paragraph, ParagraphLayout, ParagraphStyle,
    Span, TextEngine, TextStyle,
};
use crate::theme::{INK, RULE};
use crate::tree::{Constraints, PaintCx, Widget};

/// A typographic stand-in for a missing cover.
///
/// Most sideloaded EPUBs have poor or no covers, and e-ink renders photo
/// covers as muddy gray. A set-in-type cover is crisp, instantly legible,
/// and the three tonal variants (picked from the title) keep a shelf from
/// looking uniform. Large covers show title and author; thumbnails show a
/// monogram, since the title is printed next to them anyway.
pub struct Cover {
    pub title: String,
    pub author: String,
    pub family: FamilyId,
    size: Size,
    title_layout: Option<ParagraphLayout>,
    author_layout: Option<ParagraphLayout>,
}

impl Cover {
    pub fn new(title: &str, author: &str, family: FamilyId) -> Self {
        Self {
            title: title.to_string(),
            author: author.to_string(),
            family,
            size: Size::default(),
            title_layout: None,
            author_layout: None,
        }
    }

    /// Background and ink of the cover's variant.
    fn palette(&self) -> (u8, u8) {
        let h = self
            .title
            .bytes()
            .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
        match h % 3 {
            0 => (WHITE, BLACK),
            1 => (0x26, WHITE),
            _ => (0xd2, BLACK),
        }
    }

    fn large(&self) -> bool {
        self.size.w >= 150
    }
}

impl Widget for Cover {
    fn measure(&mut self, c: Constraints, text: &mut TextEngine) -> Size {
        let s = c.constrain(c.max);
        if s != self.size {
            self.size = s;
            let (_, ink) = self.palette();
            let w = s.w as f32;
            if self.large() {
                let inner = w * 0.76;
                let title = TextStyle::new(self.family, w * 0.15)
                    .bold()
                    .line_height(1.15)
                    .gray(ink);
                let mut p = Paragraph::plain(&self.title, title, Align::Center);
                p.style.max_lines = Some(4);
                self.title_layout = Some(layout_paragraph(text, &p, inner, &NoHyphenation));
                let author = TextStyle::new(self.family, w * 0.075)
                    .small_caps()
                    .tracking(w * 0.004)
                    .gray(ink);
                let mut p = Paragraph::plain(&self.author, author, Align::Center);
                p.style.max_lines = Some(2);
                self.author_layout = Some(layout_paragraph(text, &p, inner, &NoHyphenation));
            } else {
                let initial: String = self.title.chars().take(1).collect();
                let style = TextStyle::new(self.family, w * 0.62)
                    .italic()
                    .line_height(1.0)
                    .gray(ink);
                let p = Paragraph::new(
                    vec![Span::new(initial, style)],
                    ParagraphStyle {
                        align: Align::Center,
                        ..Default::default()
                    },
                );
                self.title_layout = Some(layout_paragraph(text, &p, w, &NoHyphenation));
                self.author_layout = None;
            }
        }
        s
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let (bg, ink) = self.palette();
        let p = &mut *cx.painter;
        p.fill_rect(r, bg);
        if bg == WHITE {
            p.stroke_rect(r, 2, INK);
        }
        let inset = (r.w as f32 * 0.06).round() as i32;
        p.stroke_rect(r.outset(-inset), 1.max(r.w / 120), ink);
        let w = r.w as f32;
        if let Some(t) = &self.title_layout {
            if self.large() {
                let x = r.x as f32 + (w - w * 0.76) / 2.0;
                let y = r.y as f32 + r.h as f32 * 0.24;
                t.draw(cx.text, cx.painter, x, y);
                let rule_y = (y + t.height + r.h as f32 * 0.05) as i32;
                let rw = (w * 0.18) as i32;
                cx.painter
                    .fill_rect(Rect::new(r.center().x - rw / 2, rule_y, rw, 2), ink);
                if let Some(a) = &self.author_layout {
                    let ay = r.bottom() as f32 - r.h as f32 * 0.12 - a.height;
                    a.draw(cx.text, cx.painter, x, ay);
                }
            } else {
                let y = r.y as f32 + (r.h as f32 - t.height) / 2.0;
                t.draw(cx.text, cx.painter, r.x as f32, y);
            }
        }
    }
}

/// Battery outline with a proportional black fill.
pub struct Battery {
    pub level: f32,
}

impl Widget for Battery {
    fn measure(&mut self, c: Constraints, _: &mut TextEngine) -> Size {
        c.constrain(Size::new(50, 26))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let p = &mut *cx.painter;
        let body = Rect::new(r.x, r.y, r.w - 5, r.h);
        p.stroke_round_rect(body, 5.0, 3.0, INK);
        p.fill_rect(Rect::new(body.right(), r.y + r.h / 3, 4, r.h / 3), INK);
        let inner = body.outset(-6);
        let w = (inner.w as f32 * self.level.clamp(0.0, 1.0)).round() as i32;
        p.fill_rect(Rect::new(inner.x, inner.y, w, inner.h), INK);
    }
}

/// An on/off switch. Pure black and white in both states, so flipping it
/// is a single DU update.
pub struct Toggle {
    pub on: bool,
}

impl Widget for Toggle {
    fn measure(&mut self, c: Constraints, _: &mut TextEngine) -> Size {
        c.constrain(Size::new(104, 58))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let p = &mut *cx.painter;
        let radius = r.h as f32 / 2.0;
        let knob_r = r.h as f32 / 2.0 - 9.0;
        let cy = r.y + r.h / 2;
        if self.on {
            p.fill_round_rect(r, radius, INK);
            let cx = r.right() - r.h / 2;
            p.fill_circle(Point { x: cx, y: cy }, knob_r, WHITE);
        } else {
            p.stroke_round_rect(r, radius, 4.0, INK);
            let cx = r.x + r.h / 2;
            p.fill_circle(Point { x: cx, y: cy }, knob_r, INK);
        }
    }
}

/// A horizontal slider, e.g. for the frontlight. The app maps a tap's x
/// position to a value with [`Slider::value_at`]; there is no dragging,
/// because a drag on e-ink would need a stream of A2 updates for little
/// gain over tapping where you want it.
pub struct Slider {
    pub value: f32,
}

impl Slider {
    pub fn value_at(rect: Rect, x: i32) -> f32 {
        let pad = rect.h / 2;
        ((x - rect.x - pad) as f32 / (rect.w - 2 * pad).max(1) as f32).clamp(0.0, 1.0)
    }
}

impl Widget for Slider {
    fn measure(&mut self, c: Constraints, _: &mut TextEngine) -> Size {
        c.constrain(Size::new(c.max.w, 56))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let p = &mut *cx.painter;
        let pad = r.h / 2;
        let cy = r.y + r.h / 2;
        let x0 = r.x + pad;
        let w = r.w - 2 * pad;
        let kx = x0 + (w as f32 * self.value.clamp(0.0, 1.0)).round() as i32;
        p.fill_rect(Rect::new(x0, cy - 1, w, 3), RULE);
        p.fill_rect(Rect::new(x0, cy - 3, kx - x0, 7), INK);
        p.fill_circle(Point { x: kx, y: cy }, 20.0, INK);
        p.fill_circle(Point { x: kx, y: cy }, 11.0, WHITE);
    }
}

/// A row of dots growing in size, filled up to the current step. Used for
/// the font size: it shows both where you are and how many steps exist.
pub struct Steps {
    pub count: usize,
    pub value: usize,
}

impl Widget for Steps {
    fn measure(&mut self, c: Constraints, _: &mut TextEngine) -> Size {
        c.constrain(Size::new(c.max.w, 56))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let p = &mut *cx.painter;
        let n = self.count.max(1) as i32;
        let step = r.w / n;
        for i in 0..n {
            let d = 10.0 + 14.0 * i as f32 / (n - 1).max(1) as f32;
            let c = Point {
                x: r.x + step * i + step / 2,
                y: r.y + r.h / 2,
            };
            if i as usize <= self.value {
                p.fill_circle(c, d / 2.0, INK);
            } else {
                p.fill_circle(c, d / 2.0, INK);
                p.fill_circle(c, d / 2.0 - 2.5, WHITE);
            }
        }
    }
}

/// Draws a dashed outline, for debug overlays (damage, tap zones).
pub fn dashed_rect(p: &mut Painter, r: Rect, width: i32, dash: i32, gray: u8) {
    let mut x = r.x;
    while x < r.right() {
        let l = dash.min(r.right() - x);
        p.fill_rect(Rect::new(x, r.y, l, width), gray);
        p.fill_rect(Rect::new(x, r.bottom() - width, l, width), gray);
        x += 2 * dash;
    }
    let mut y = r.y;
    while y < r.bottom() {
        let l = dash.min(r.bottom() - y);
        p.fill_rect(Rect::new(r.x, y, width, l), gray);
        p.fill_rect(Rect::new(r.right() - width, y, width, l), gray);
        y += 2 * dash;
    }
}
