// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integer geometry in device pixels, plus physical-unit helpers.
//!
//! Everything in the toolkit is laid out in whole device pixels. E-ink
//! updates are rectangles of whole pixels and the panel has no compositor, so
//! fractional layout would only add rounding seams between damage rects.
//! Text is the exception: glyph pen positions stay fractional inside a line
//! and are snapped only when a glyph is drawn.

pub use aristotle_screen::{Density, Screen};

/// Panel width of the Kindle Paperwhite 4 in portrait, in pixels.
pub const SCREEN_WIDTH: i32 = 1072;
/// Panel height of the Kindle Paperwhite 4 in portrait, in pixels.
pub const SCREEN_HEIGHT: i32 = 1448;
/// Converts millimetres to reference pixels, at
/// [`aristotle_screen::REFERENCE_DPI`]; the tree applies the active density.
pub fn mm(v: f32) -> i32 {
    Density::default().mm(v)
}

/// Converts points to reference pixels, at
/// [`aristotle_screen::REFERENCE_DPI`]; native text uses the active density.
pub fn pt(v: f32) -> f32 {
    Density::default().pt(v)
}

/// The smallest touch target in reference pixels, 9 mm (106 px at 300 DPI).
/// Fingertip contact patches are 8 to 10 mm wide, and on e-ink a missed tap
/// is expensive: the user waits a full refresh to see that nothing happened.
/// Use [`min_target`] for native pixels.
pub const MIN_TARGET: i32 = 106;

/// [`MIN_TARGET`] in native pixels at `density`: the size the tree gives a
/// button and the area it hit-tests around a small target, so both agree.
pub fn min_target(density: Density) -> i32 {
    density.px(MIN_TARGET)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Size {
    pub w: i32,
    pub h: i32,
}

impl Size {
    pub const fn new(w: i32, h: i32) -> Self {
        Self { w, h }
    }
}

/// An axis-aligned rectangle. Empty when `w` or `h` is not positive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn screen() -> Self {
        Self::new(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT)
    }

    pub fn from_size(size: Size) -> Self {
        Self::new(0, 0, size.w, size.h)
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn size(&self) -> Size {
        Size::new(self.w, self.h)
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn area(&self) -> i64 {
        if self.is_empty() {
            0
        } else {
            self.w as i64 * self.h as i64
        }
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }

    /// The overlapping part, or `None` if the rectangles do not overlap.
    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        (r > x && b > y).then(|| Rect::new(x, y, r - x, b - y))
    }

    pub fn intersects(&self, o: &Rect) -> bool {
        self.intersect(o).is_some()
    }

    /// Smallest rectangle covering both. An empty operand is ignored, so
    /// unions can start from `Rect::default()`.
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let r = self.right().max(o.right());
        let b = self.bottom().max(o.bottom());
        Rect::new(x, y, r - x, b - y)
    }

    /// Shrinks by the insets. The result never has a negative size.
    pub fn inset(&self, i: Insets) -> Rect {
        Rect::new(
            self.x + i.left,
            self.y + i.top,
            (self.w - i.left - i.right).max(0),
            (self.h - i.top - i.bottom).max(0),
        )
    }

    /// Grows by `d` pixels on every side.
    pub fn outset(&self, d: i32) -> Rect {
        Rect::new(self.x - d, self.y - d, self.w + 2 * d, self.h + 2 * d)
    }

    pub fn translate(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    pub fn center(&self) -> Point {
        Point {
            x: self.x + self.w / 2,
            y: self.y + self.h / 2,
        }
    }

    /// Grows the rectangle around its centre until both sides are at least
    /// `min`. Used for hit testing, so a small icon still gets a full-size
    /// touch target without changing how it is drawn.
    pub fn expand_to(&self, min: i32) -> Rect {
        let gw = (min - self.w).max(0);
        let gh = (min - self.h).max(0);
        Rect::new(self.x - gw / 2, self.y - gh / 2, self.w + gw, self.h + gh)
    }
}

/// Space on each side of a box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Insets {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl Insets {
    pub const fn all(v: i32) -> Self {
        Self {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }

    /// Vertical and horizontal, CSS order.
    pub const fn vh(v: i32, h: i32) -> Self {
        Self {
            top: v,
            right: h,
            bottom: v,
            left: h,
        }
    }

    pub const fn new(top: i32, right: i32, bottom: i32, left: i32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    pub fn horizontal(&self) -> i32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> i32 {
        self.top + self.bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units() {
        assert_eq!(mm(9.0), 106);
        assert_eq!(MIN_TARGET, mm(9.0));
        // At every density the native target stays about 9 mm: off by the
        // reference rounding (106 px is 8.97 mm) and half a native pixel.
        for dpi in 72..=1200 {
            let d = Density::new(dpi).unwrap();
            let size_mm = min_target(d) as f32 * 25.4 / dpi as f32;
            assert!((size_mm - 9.0).abs() < 0.2, "{dpi}: {size_mm} mm");
        }
        assert!((pt(72.0) - 300.0).abs() < 1e-3);
    }

    #[test]
    fn union_and_intersect() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert_eq!(a.intersect(&b), Some(Rect::new(5, 5, 5, 5)));
        assert_eq!(a.intersect(&Rect::new(10, 0, 5, 5)), None);
        assert_eq!(Rect::default().union(&b), b);
    }

    #[test]
    fn expand_keeps_center() {
        let r = Rect::new(100, 100, 40, 40).expand_to(MIN_TARGET);
        assert_eq!(r.w, 106);
        assert_eq!(r.h, 106);
        assert_eq!(r.center(), Point { x: 120, y: 120 });
    }
}
