// SPDX-License-Identifier: AGPL-3.0-or-later
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
use std::rc::Rc;
use zeno::PathBuilder;

/// An 8-bit gray picture (0 black, 255 white), shared cheaply between the
/// screen model and the tree. Cover thumbnails arrive this way from the
/// library's cover cache, already dithered to the panel's 16 levels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrayImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Rc<Vec<u8>>,
}

impl GrayImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        assert_eq!(pixels.len(), width as usize * height as usize);
        Self {
            width,
            height,
            pixels: Rc::new(pixels),
        }
    }

    /// The largest size with this aspect ratio that fits in `w` × `h`.
    pub fn fit(&self, w: i32, h: i32) -> Size {
        if self.width == 0 || self.height == 0 || w <= 0 || h <= 0 {
            return Size::default();
        }
        let sx = w as f32 / self.width as f32;
        let sy = h as f32 / self.height as f32;
        let s = sx.min(sy);
        Size::new(
            ((self.width as f32 * s).round() as i32).clamp(1, w),
            ((self.height as f32 * s).round() as i32).clamp(1, h),
        )
    }

    /// Scaled to `w` × `h` by area averaging: every source pixel counts in
    /// proportion to its overlap, so thin type on a cover does not alias
    /// away the way nearest-neighbour sampling would make it.
    pub fn resized(&self, w: u32, h: u32) -> GrayImage {
        if (w, h) == (self.width, self.height) || self.width == 0 || self.height == 0 {
            return self.clone();
        }
        let (sw, sh) = (self.width as f32, self.height as f32);
        let fx = sw / w as f32;
        let fy = sh / h as f32;
        let mut out = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h {
            let y0 = y as f32 * fy;
            let y1 = (y0 + fy).min(sh);
            for x in 0..w {
                let x0 = x as f32 * fx;
                let x1 = (x0 + fx).min(sw);
                let (mut sum, mut area) = (0.0f32, 0.0f32);
                let mut yy = y0.floor();
                while yy < y1 {
                    let wy = (yy + 1.0).min(y1) - yy.max(y0);
                    let row = (yy as usize).min(self.height as usize - 1) * self.width as usize;
                    let mut xx = x0.floor();
                    while xx < x1 {
                        let wx = (xx + 1.0).min(x1) - xx.max(x0);
                        let px = self.pixels[row + (xx as usize).min(self.width as usize - 1)];
                        sum += px as f32 * wx * wy;
                        area += wx * wy;
                        xx += 1.0;
                    }
                    yy += 1.0;
                }
                out.push((sum / area.max(f32::EPSILON)).round().clamp(0.0, 255.0) as u8);
            }
        }
        GrayImage::new(w, h, out)
    }
}

/// What a typographic cover shows besides title and author.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoverMark {
    /// The shelf the book belongs to (its series or folder). Books of one
    /// shelf share a binding, so an author's folder reads as a set.
    pub group: Option<String>,
    /// The book's number in its series, printed small on the card.
    pub series_index: Option<f32>,
}

/// The four bindings of typographic cards: a background and an ink, all
/// panel gray levels. Two light and two dark, so neighbouring shelves
/// differ in day and in night mode (where all four invert).
pub const BINDINGS: [(u8, u8); 4] = [(WHITE, BLACK), (0x22, WHITE), (0xdd, BLACK), (0x55, WHITE)];

/// FNV-1a, a stable hash for picking a binding.
fn fnv(s: &str) -> u32 {
    s.bytes()
        .fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619))
}

/// "3", "2.5" ("2,5" in Polish): a series number without a pointless ".0".
pub fn series_number(i: f32) -> String {
    if i.fract() == 0.0 {
        format!("{i:.0}")
    } else {
        let s = format!("{i}");
        match aristotle_i18n::locale().decimal {
            "." => s,
            sep => s.replacen('.', sep, 1),
        }
    }
}

/// A book cover: a typographic card, or the real cover when one is given.
///
/// Most sideloaded books have poor or no covers, and e-ink renders photo
/// covers as muddy gray. A card set in type is crisp and legible in 16
/// grays and inverted, needs no image decoding, and its binding (one of
/// [`BINDINGS`], picked by shelf) groups an author's or a series' books at a
/// glance. Large cards (150 px wide and up) carry the title, a short rule
/// and the author in small capitals, with "Volume 3" above the title for a
/// series; small ones a monogram, since the title is printed next to them,
/// and the series number in a corner.
pub struct Cover {
    pub title: String,
    pub author: String,
    pub family: FamilyId,
    pub mark: CoverMark,
    /// The real cover, scaled to the widget once per size.
    pub image: Option<GrayImage>,
    size: Size,
    scaled: Option<GrayImage>,
    title_layout: Option<ParagraphLayout>,
    author_layout: Option<ParagraphLayout>,
    mark_layout: Option<ParagraphLayout>,
}

impl Cover {
    pub fn new(title: &str, author: &str, family: FamilyId) -> Self {
        Self {
            title: title.to_string(),
            author: author.to_string(),
            family,
            mark: CoverMark::default(),
            image: None,
            size: Size::default(),
            scaled: None,
            title_layout: None,
            author_layout: None,
            mark_layout: None,
        }
    }

    pub fn with_image(mut self, image: Option<GrayImage>) -> Self {
        self.image = image;
        self
    }

    pub fn with_mark(mut self, mark: CoverMark) -> Self {
        self.mark = mark;
        self
    }

    /// Background and ink of the card's binding.
    pub fn binding(&self) -> (u8, u8) {
        let key = self.mark.group.as_deref().unwrap_or(&self.title);
        BINDINGS[(fnv(key) % BINDINGS.len() as u32) as usize]
    }

    fn large(&self, density: crate::geom::Density) -> bool {
        self.size.w >= density.px(150)
    }
}

impl Widget for Cover {
    fn measure(&mut self, c: Constraints, text: &mut TextEngine) -> Size {
        let s = c.constrain(c.max);
        if s != self.size {
            self.size = s;
            if let Some(img) = &self.image {
                let fit = img.fit(s.w, s.h);
                self.scaled = Some(img.resized(fit.w as u32, fit.h as u32));
                return s;
            }
            let (_, ink) = self.binding();
            let w = s.w as f32;
            let index = self.mark.series_index.map(series_number);
            if self.large(text.density()) {
                let inner = w * 0.72;
                let mut title = TextStyle::new(self.family, w * 0.13)
                    .bold()
                    .line_height(1.12)
                    .gray(ink);
                // Cards are not hyphenated: shrink the type until the
                // longest word fits ("przeznaczenia" on a narrow card).
                let longest = self
                    .title
                    .split_whitespace()
                    .map(|word| text.measure(word, &title))
                    .fold(0.0f32, f32::max);
                if longest > inner {
                    title.size *= inner / longest;
                }
                let inner = w * 0.76;
                let mut p = Paragraph::plain(&self.title, title, Align::Center);
                p.style.max_lines = Some(4);
                p.style.balance = true;
                self.title_layout = Some(layout_paragraph(text, &p, inner, &NoHyphenation));
                let small = TextStyle::new(self.family, w * 0.068)
                    .small_caps()
                    .tracking(w * 0.006)
                    .gray(ink);
                let mut p = Paragraph::plain(&self.author, small.clone(), Align::Center);
                p.style.max_lines = Some(2);
                self.author_layout = Some(layout_paragraph(text, &p, inner, &NoHyphenation));
                self.mark_layout = index.map(|i| {
                    let label = aristotle_i18n::tf!(aristotle_i18n::Msg::LibraryVolume, n = i);
                    let p = Paragraph::plain(label, small, Align::Center);
                    layout_paragraph(text, &p, inner, &NoHyphenation)
                });
            } else {
                let initial: String = self.title.chars().take(1).collect();
                let style = TextStyle::new(self.family, w * 0.58)
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
                self.mark_layout = index.map(|i| {
                    let style = TextStyle::new(self.family, w * 0.2).bold().gray(ink);
                    let p = Paragraph::plain(i, style, Align::Right);
                    layout_paragraph(text, &p, w * 0.84, &NoHyphenation)
                });
            }
        }
        s
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let density = cx.text.density();
        if let Some(img) = &self.scaled {
            // Bottom-aligned and centred, like a book standing on a shelf;
            // a hairline keeps white covers from dissolving into the page.
            let x = r.x + (r.w - img.width as i32) / 2;
            let y = r.bottom() - img.height as i32;
            let p = &mut *cx.painter;
            p.blit_gray(x, y, img.width, img.height, img.width as usize, &img.pixels);
            p.stroke_rect(
                Rect::new(x, y, img.width as i32, img.height as i32),
                1,
                RULE,
            );
            return;
        }
        let (bg, ink) = self.binding();
        let radius = (r.w as f32 * 0.03).max(density.scale(3.0));
        let p = &mut *cx.painter;
        p.fill_round_rect(r, radius, bg);
        if bg == WHITE {
            // A white card needs an edge on white paper.
            p.stroke_round_rect(r, radius, density.scale(2.0), INK);
        }
        let w = r.w as f32;
        let h = r.h as f32;
        let x = r.x as f32 + w * 0.12;
        if self.large(density) {
            // An inner frame, like a border blocked on a cloth binding.
            let inset = (w * 0.06).round() as i32;
            p.stroke_rect(r.outset(-inset), density.px(2), ink);
            if let Some(m) = &self.mark_layout {
                m.draw(cx.text, cx.painter, x, r.y as f32 + h * 0.11);
            }
            if let Some(t) = &self.title_layout {
                // Short titles sit a little lower than long ones.
                let y = r.y as f32 + h * 0.2 + (h * 0.3 - t.height).max(0.0) * 0.5;
                t.draw(cx.text, cx.painter, x, y);
                let rule_y = (y + t.height + h * 0.05) as i32;
                let rw = (w * 0.16) as i32;
                cx.painter.fill_rect(
                    Rect::new(r.center().x - rw / 2, rule_y, rw, density.px(3)),
                    ink,
                );
            }
            if let Some(a) = &self.author_layout {
                let ay = r.bottom() as f32 - h * 0.13 - a.height;
                a.draw(cx.text, cx.painter, x, ay);
            }
        } else {
            if let Some(t) = &self.title_layout {
                let y = r.y as f32 + (h - t.height) / 2.0;
                t.draw(cx.text, cx.painter, r.x as f32, y);
            }
            if let Some(m) = &self.mark_layout {
                let y = r.bottom() as f32 - m.height - h * 0.03;
                m.draw(cx.text, cx.painter, r.x as f32 + w * 0.08, y);
            }
        }
    }
}

/// Battery: the icon family's outline with a proportional fill, or a bolt
/// while charging (the percentage next to it still says how full).
pub struct Battery {
    pub level: f32,
    pub charging: bool,
}

impl Widget for Battery {
    fn measure(&mut self, c: Constraints, text: &mut TextEngine) -> Size {
        c.constrain(Size::new(text.density().px(46), text.density().px(26)))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let d = cx.text.density();
        let p = &mut *cx.painter;
        let body = Rect::new(r.x, r.y, r.w - d.px(6), r.h);
        p.stroke_round_rect(body, d.scale(4.0), d.scale(3.0), INK);
        p.fill_rect(
            Rect::new(body.right() + d.px(2), r.y + r.h / 3, d.px(4), r.h / 3),
            INK,
        );
        let inner = body.outset(-d.px(6));
        if self.charging {
            let s = inner.h as f32 / 7.0;
            let (x, y) = (inner.center().x as f32, inner.y as f32);
            let mut bolt: Vec<zeno::Command> = Vec::new();
            bolt.move_to([x + 0.8 * s, y])
                .line_to([x - 2.4 * s, y + 4.0 * s])
                .line_to([x - 0.2 * s, y + 4.0 * s])
                .line_to([x - 0.8 * s, y + 7.0 * s])
                .line_to([x + 2.4 * s, y + 3.0 * s])
                .line_to([x + 0.2 * s, y + 3.0 * s])
                .close();
            p.fill_path(&bolt, None, INK);
            return;
        }
        let w = (inner.w as f32 * self.level.clamp(0.0, 1.0)).round() as i32;
        p.fill_rect(Rect::new(inner.x, inner.y, w, inner.h), INK);
    }
}

/// The busy mark: a black tab with three white dots, square-cornered and
/// pixel-aligned so it is pure black and white and goes out with DU. Shown
/// in the status line when an action will take longer than about 300 ms.
pub struct BusyMark;

impl Widget for BusyMark {
    fn measure(&mut self, c: Constraints, text: &mut TextEngine) -> Size {
        c.constrain(Size::new(text.density().px(72), text.density().px(28)))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let p = &mut *cx.painter;
        p.fill_rect(r, BLACK);
        let d = cx.text.density().px(8);
        let gap = (r.w - 3 * d) / 4;
        for i in 0..3 {
            let x = r.x + gap + i * (d + gap);
            p.fill_rect(Rect::new(x, r.y + (r.h - d) / 2, d, d), WHITE);
        }
    }
}

/// An on/off switch. Pure black and white in both states, so flipping it
/// is a single DU update.
pub struct Toggle {
    pub on: bool,
}

impl Widget for Toggle {
    fn measure(&mut self, c: Constraints, text: &mut TextEngine) -> Size {
        c.constrain(Size::new(text.density().px(104), text.density().px(58)))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let d = cx.text.density();
        let p = &mut *cx.painter;
        let radius = r.h as f32 / 2.0;
        let knob_r = r.h as f32 / 2.0 - d.scale(9.0);
        let cy = r.y + r.h / 2;
        if self.on {
            p.fill_round_rect(r, radius, INK);
            let cx = r.right() - r.h / 2;
            p.fill_circle(Point { x: cx, y: cy }, knob_r, WHITE);
        } else {
            p.stroke_round_rect(r, radius, d.scale(4.0), INK);
            let cx = r.x + r.h / 2;
            p.fill_circle(Point { x: cx, y: cy }, knob_r, INK);
        }
    }
}

/// A horizontal slider, e.g. for the frontlight. The app maps a tap's x
/// position to a value with [`Slider::value_at`] or [`Slider::step_at`];
/// there is no dragging, because a drag on e-ink would need a stream of A2
/// updates for little gain over tapping where you want it.
pub struct Slider {
    pub value: f32,
}

impl Slider {
    pub fn value_at(rect: Rect, x: i32) -> f32 {
        let pad = rect.h / 2;
        ((x - rect.x - pad) as f32 / (rect.w - 2 * pad).max(1) as f32).clamp(0.0, 1.0)
    }

    /// The nearest of the positions 0 to `steps` to a tap at `x`.
    pub fn step_at(rect: Rect, x: i32, steps: u32) -> u32 {
        (Self::value_at(rect, x) * steps as f32).round() as u32
    }
}

impl Widget for Slider {
    fn measure(&mut self, c: Constraints, text: &mut TextEngine) -> Size {
        c.constrain(Size::new(c.max.w, text.density().px(56)))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let d = cx.text.density();
        let p = &mut *cx.painter;
        let pad = r.h / 2;
        let cy = r.y + r.h / 2;
        let x0 = r.x + pad;
        let w = r.w - 2 * pad;
        let kx = x0 + (w as f32 * self.value.clamp(0.0, 1.0)).round() as i32;
        p.fill_rect(Rect::new(x0, cy - d.px(1), w, d.px(3)), RULE);
        p.fill_rect(Rect::new(x0, cy - d.px(3), kx - x0, d.px(7)), INK);
        p.fill_circle(Point { x: kx, y: cy }, d.scale(20.0), INK);
        p.fill_circle(Point { x: kx, y: cy }, d.scale(11.0), WHITE);
    }
}

/// A row of dots growing in size, filled up to the current step. Used for
/// the font size: it shows both where you are and how many steps exist.
pub struct Steps {
    pub count: usize,
    pub value: usize,
}

impl Widget for Steps {
    fn measure(&mut self, c: Constraints, text: &mut TextEngine) -> Size {
        c.constrain(Size::new(c.max.w, text.density().px(56)))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let density = cx.text.density();
        let p = &mut *cx.painter;
        let n = self.count.max(1) as i32;
        let step = r.w / n;
        for i in 0..n {
            let d = density.scale(10.0 + 14.0 * i as f32 / (n - 1).max(1) as f32);
            let c = Point {
                x: r.x + step * i + step / 2,
                y: r.y + r.h / 2,
            };
            if i as usize <= self.value {
                p.fill_circle(c, d / 2.0, INK);
            } else {
                p.fill_circle(c, d / 2.0, INK);
                p.fill_circle(c, d / 2.0 - density.scale(2.5), WHITE);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_are_panel_levels_with_contrast() {
        for (bg, ink) in BINDINGS {
            assert_eq!(bg % 17, 0);
            assert!(
                (bg as i32 - ink as i32).abs() >= 0x99,
                "{bg:#x} on {ink:#x}"
            );
        }
    }

    #[test]
    fn a_shelf_shares_one_binding() {
        let fam = FamilyId(0);
        let mark = CoverMark {
            group: Some("Mickiewicz, Adam".into()),
            series_index: None,
        };
        let a = Cover::new("Pan Tadeusz", "Adam Mickiewicz", fam).with_mark(mark.clone());
        let b = Cover::new("Dziady", "Adam Mickiewicz", fam).with_mark(mark);
        assert_eq!(a.binding(), b.binding());
    }

    #[test]
    fn series_numbers() {
        assert_eq!(series_number(3.0), "3");
        assert_eq!(series_number(2.5), "2.5");
    }
}
