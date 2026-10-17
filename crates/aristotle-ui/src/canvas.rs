// SPDX-License-Identifier: AGPL-3.0-or-later
//! 8-bit grayscale canvas and clipped drawing primitives.
//!
//! The canvas mirrors the PW4 framebuffer format (8 bpp, 0 black, 255
//! white, rows `stride` bytes apart) so a damaged rectangle can be copied to
//! the device with one `memcpy` per row and no conversion. The stride may be
//! wider than the width, as on the device (1072 visible, 1088 per line).
//!
//! Vector shapes (rounded rectangles, icons) are rasterized with `zeno`, the
//! anti-aliasing rasterizer `swash` already uses for glyphs, so they cost no
//! extra code size.

use crate::geom::{Point, Rect};
use zeno::{Command, Fill, Mask, PathBuilder, PathData, Stroke, Style, Transform};

pub const BLACK: u8 = 0;
pub const WHITE: u8 = 255;

/// A grayscale pixel buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    /// Bytes between the starts of two rows; at least `width`.
    pub stride: u32,
    pub pixels: Vec<u8>,
}

impl Canvas {
    /// A white canvas with a tight stride.
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_stride(width, height, width)
    }

    /// A white canvas with an explicit row stride, for matching a
    /// framebuffer's line length.
    pub fn with_stride(width: u32, height: u32, stride: u32) -> Self {
        assert!(stride >= width, "stride must cover the width");
        Self {
            width,
            height,
            stride,
            pixels: vec![WHITE; stride as usize * height as usize],
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }

    fn index(&self, x: i32, y: i32) -> usize {
        y as usize * self.stride as usize + x as usize
    }

    /// The pixel at `(x, y)`; panics outside the canvas.
    pub fn get(&self, x: i32, y: i32) -> u8 {
        self.pixels[self.index(x, y)]
    }

    /// One row of the rectangle, which must lie inside the canvas.
    fn row(&self, r: &Rect, y: i32) -> &[u8] {
        let i = self.index(r.x, y);
        &self.pixels[i..i + r.w as usize]
    }

    fn row_mut(&mut self, r: &Rect, y: i32) -> &mut [u8] {
        let i = self.index(r.x, y);
        &mut self.pixels[i..i + r.w as usize]
    }

    pub fn fill(&mut self, rect: Rect, gray: u8) {
        let Some(r) = rect.intersect(&self.bounds()) else {
            return;
        };
        for y in r.y..r.bottom() {
            self.row_mut(&r, y).fill(gray);
        }
    }

    /// Copies a rectangle from another canvas of the same size.
    pub fn copy_from(&mut self, src: &Canvas, rect: Rect) {
        let Some(r) = rect.intersect(&self.bounds()) else {
            return;
        };
        for y in r.y..r.bottom() {
            let s = src.index(r.x, y);
            let d = self.index(r.x, y);
            let w = r.w as usize;
            self.pixels[d..d + w].copy_from_slice(&src.pixels[s..s + w]);
        }
    }

    /// Bounding box of the pixels inside `rect` that differ from `other`.
    ///
    /// This is what makes damage minimal: the widget tree only knows which
    /// boxes *may* have changed, while this finds the pixels that did. A
    /// progress bar going from 41 % to 42 % repaints its whole box but
    /// reports a few pixels wide strip, and a repaint that produces the same
    /// pixels reports nothing at all.
    pub fn diff_bounds(&self, other: &Canvas, rect: Rect) -> Option<Rect> {
        let r = rect.intersect(&self.bounds())?;
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for y in r.y..r.bottom() {
            let a = self.row(&r, y);
            let b = other.row(&r, y);
            if a == b {
                continue;
            }
            let first = a.iter().zip(b).position(|(p, q)| p != q).unwrap() as i32;
            let last = a.iter().zip(b).rposition(|(p, q)| p != q).unwrap() as i32;
            x0 = x0.min(r.x + first);
            x1 = x1.max(r.x + last);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        (x1 >= x0).then(|| Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }

    /// True if every pixel in `rect` is pure black or pure white. Such
    /// content can go out with DU, which is faster than GL16 and settles to
    /// clean black and white.
    pub fn is_bilevel(&self, rect: Rect) -> bool {
        let Some(r) = rect.intersect(&self.bounds()) else {
            return true;
        };
        (r.y..r.bottom()).all(|y| self.row(&r, y).iter().all(|&p| p == 0 || p == 255))
    }

    /// Writes `src` into `dst` inverted (night mode) or as is.
    pub fn copy_mapped(&mut self, src: &Canvas, rect: Rect, invert: bool) {
        if !invert {
            return self.copy_from(src, rect);
        }
        let Some(r) = rect.intersect(&self.bounds()) else {
            return;
        };
        for y in r.y..r.bottom() {
            let s = src.index(r.x, y);
            let d = self.index(r.x, y);
            for i in 0..r.w as usize {
                self.pixels[d + i] = 255 - src.pixels[s + i];
            }
        }
    }

    /// Copies a rectangle into a foreign buffer such as the memory-mapped
    /// framebuffer, honouring both strides.
    pub fn blit_into(&self, rect: Rect, dst: &mut [u8], dst_stride: usize) {
        let Some(r) = rect.intersect(&self.bounds()) else {
            return;
        };
        for y in r.y..r.bottom() {
            let d = y as usize * dst_stride + r.x as usize;
            dst[d..d + r.w as usize].copy_from_slice(self.row(&r, y));
        }
    }

    /// Reduces every pixel to the 16 levels the panel can show, by
    /// replicating the top nibble. The preview applies this so screenshots
    /// show what GL16 will really display.
    pub fn quantize16(&mut self) {
        for p in &mut self.pixels {
            *p = (*p & 0xF0) | (*p >> 4);
        }
    }

    /// Tightly packed copy of the visible pixels, for encoders.
    pub fn packed(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity((self.width * self.height) as usize);
        let b = self.bounds();
        for y in 0..self.height as i32 {
            out.extend_from_slice(self.row(&b, y));
        }
        out
    }
}

/// Drawing onto a canvas, restricted to a clip rectangle.
///
/// Every primitive clips, which is what lets the UI repaint only the damaged
/// part of the screen by painting the whole tree with a small clip.
pub struct Painter<'a> {
    pub canvas: &'a mut Canvas,
    pub clip: Rect,
}

impl<'a> Painter<'a> {
    pub fn new(canvas: &'a mut Canvas, clip: Rect) -> Self {
        let clip = clip.intersect(&canvas.bounds()).unwrap_or_default();
        Self { canvas, clip }
    }

    pub fn fill_rect(&mut self, rect: Rect, gray: u8) {
        if let Some(r) = rect.intersect(&self.clip) {
            self.canvas.fill(r, gray);
        }
    }

    /// A rectangle outline drawn inside `rect`.
    pub fn stroke_rect(&mut self, rect: Rect, width: i32, gray: u8) {
        let w = width.min(rect.w / 2).min(rect.h / 2).max(0);
        self.fill_rect(Rect::new(rect.x, rect.y, rect.w, w), gray);
        self.fill_rect(Rect::new(rect.x, rect.bottom() - w, rect.w, w), gray);
        self.fill_rect(Rect::new(rect.x, rect.y + w, w, rect.h - 2 * w), gray);
        self.fill_rect(
            Rect::new(rect.right() - w, rect.y + w, w, rect.h - 2 * w),
            gray,
        );
    }

    pub fn hline(&mut self, x: i32, y: i32, w: i32, thickness: i32, gray: u8) {
        self.fill_rect(Rect::new(x, y, w, thickness), gray);
    }

    pub fn vline(&mut self, x: i32, y: i32, h: i32, thickness: i32, gray: u8) {
        self.fill_rect(Rect::new(x, y, thickness, h), gray);
    }

    /// Copies 8-bit gray pixels (rows `stride` bytes apart) with their
    /// top-left corner at `(x, y)`, clipped. For pictures: cover thumbnails
    /// and book pages rendered by another engine.
    pub fn blit_gray(&mut self, x: i32, y: i32, w: u32, h: u32, stride: usize, pixels: &[u8]) {
        let area = Rect::new(x, y, w as i32, h as i32);
        let Some(r) = area.intersect(&self.clip) else {
            return;
        };
        for row in r.y..r.bottom() {
            let s = (row - y) as usize * stride + (r.x - x) as usize;
            let d = self.canvas.index(r.x, row);
            let n = r.w as usize;
            self.canvas.pixels[d..d + n].copy_from_slice(&pixels[s..s + n]);
        }
    }

    /// Blends an 8-bit coverage mask whose top-left corner is at `(x, y)`
    /// towards `gray`. Used for glyphs and rasterized paths.
    pub fn blend_mask(&mut self, x: i32, y: i32, w: u32, h: u32, mask: &[u8], gray: u8) {
        let area = Rect::new(x, y, w as i32, h as i32);
        let Some(r) = area.intersect(&self.clip) else {
            return;
        };
        let g = gray as i32;
        for row in r.y..r.bottom() {
            let m0 = (row - y) as usize * w as usize + (r.x - x) as usize;
            let d0 = self.canvas.index(r.x, row);
            let m = &mask[m0..m0 + r.w as usize];
            let d = &mut self.canvas.pixels[d0..d0 + r.w as usize];
            for (p, &a) in d.iter_mut().zip(m) {
                if a == 0 {
                    continue;
                }
                let v = *p as i32;
                // Rounded integer lerp: v + (g - v) * a / 255.
                let t = (g - v) * a as i32;
                *p = (v + (t + 127 * t.signum()) / 255) as u8;
            }
        }
    }

    /// Inverts pixels under a coverage mask; full coverage maps p to 255 - p.
    fn invert_mask(&mut self, x: i32, y: i32, w: u32, h: u32, mask: &[u8]) {
        let area = Rect::new(x, y, w as i32, h as i32);
        let Some(r) = area.intersect(&self.clip) else {
            return;
        };
        for row in r.y..r.bottom() {
            let m0 = (row - y) as usize * w as usize + (r.x - x) as usize;
            let d0 = self.canvas.index(r.x, row);
            for i in 0..r.w as usize {
                let a = mask[m0 + i] as i32;
                if a == 0 {
                    continue;
                }
                let v = self.canvas.pixels[d0 + i] as i32;
                self.canvas.pixels[d0 + i] = (v + (255 - 2 * v) * a / 255) as u8;
            }
        }
    }

    /// Snaps every pixel of `rect` to black or white at `level` (darker
    /// becomes black). For controls that must stay pure black and white,
    /// such as keyboard keys, so A2 shows them exactly and DU restores
    /// them; see `keyboard`.
    pub fn threshold(&mut self, rect: Rect, level: u8) {
        if let Some(r) = rect.intersect(&self.clip) {
            for y in r.y..r.bottom() {
                for p in self.canvas.row_mut(&r, y) {
                    *p = if *p < level { 0 } else { 255 };
                }
            }
        }
    }

    /// Inverts a rectangle: the pressed-state feedback.
    pub fn invert_rect(&mut self, rect: Rect) {
        if let Some(r) = rect.intersect(&self.clip) {
            for y in r.y..r.bottom() {
                for p in self.canvas.row_mut(&r, y) {
                    *p = 255 - *p;
                }
            }
        }
    }

    fn render_mask(
        data: impl PathData,
        style: Style,
        transform: Option<Transform>,
    ) -> (Vec<u8>, zeno::Placement) {
        Mask::new(data).style(style).transform(transform).render()
    }

    /// Fills any path (SVG path data or zeno commands) after `transform`.
    pub fn fill_path(&mut self, data: impl PathData, transform: Option<Transform>, gray: u8) {
        let (m, p) = Self::render_mask(data, Style::Fill(Fill::NonZero), transform);
        self.blend_mask(p.left, p.top, p.width, p.height, &m, gray);
    }

    /// Strokes a path with round joins and caps, which survive e-ink's
    /// 16-level quantization better than sharp miters.
    pub fn stroke_path(
        &mut self,
        data: impl PathData,
        width: f32,
        transform: Option<Transform>,
        gray: u8,
    ) {
        let mut s = Stroke::new(width);
        s.join(zeno::Join::Round).cap(zeno::Cap::Round);
        let (m, p) = Self::render_mask(data, Style::Stroke(s), transform);
        self.blend_mask(p.left, p.top, p.width, p.height, &m, gray);
    }

    pub fn fill_round_rect(&mut self, rect: Rect, radius: f32, gray: u8) {
        if radius <= 0.0 {
            return self.fill_rect(rect, gray);
        }
        let path = round_rect_path(rect, radius);
        self.fill_path(&path, None, gray);
    }

    /// A rounded outline drawn inside `rect`.
    pub fn stroke_round_rect(&mut self, rect: Rect, radius: f32, width: f32, gray: u8) {
        if radius <= 0.0 {
            return self.stroke_rect(rect, width.round() as i32, gray);
        }
        let mut path = round_rect_path(rect, radius);
        // Even-odd fill of outer and inner shapes gives an outline with
        // exact pixel-aligned edges, unlike a centred stroke of odd width.
        let inner = Rect::new(
            rect.x + width as i32,
            rect.y + width as i32,
            rect.w - 2 * width as i32,
            rect.h - 2 * width as i32,
        );
        if !inner.is_empty() {
            path.extend(round_rect_path(inner, (radius - width).max(0.5)));
        }
        let (m, p) = Self::render_mask(&path, Style::Fill(Fill::EvenOdd), None);
        self.blend_mask(p.left, p.top, p.width, p.height, &m, gray);
    }

    /// Inverts the inside of a rounded rectangle.
    pub fn invert_round_rect(&mut self, rect: Rect, radius: f32) {
        if radius <= 0.0 {
            return self.invert_rect(rect);
        }
        let path = round_rect_path(rect, radius);
        let (m, p) = Self::render_mask(&path, Style::Fill(Fill::NonZero), None);
        self.invert_mask(p.left, p.top, p.width, p.height, &m);
    }

    pub fn fill_circle(&mut self, c: Point, r: f32, gray: u8) {
        let mut path: Vec<Command> = Vec::new();
        path.add_circle([c.x as f32, c.y as f32], r);
        self.fill_path(&path, None, gray);
    }
}

fn round_rect_path(r: Rect, radius: f32) -> Vec<Command> {
    let mut path: Vec<Command> = Vec::new();
    let radius = radius.min(r.w as f32 / 2.0).min(r.h as f32 / 2.0);
    path.add_round_rect(
        [r.x as f32, r.y as f32],
        r.w as f32,
        r.h as f32,
        radius,
        radius,
    );
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_respects_stride_and_clip() {
        let mut c = Canvas::with_stride(10, 4, 16);
        let mut p = Painter::new(&mut c, Rect::new(0, 0, 5, 4));
        p.fill_rect(Rect::new(-3, 1, 100, 2), 0);
        assert_eq!(c.get(4, 1), 0);
        assert_eq!(c.get(5, 1), 255);
        assert_eq!(c.get(0, 0), 255);
        // Padding bytes beyond the width are never touched.
        assert!(c.pixels[10..16].iter().all(|&p| p == 255));
    }

    #[test]
    fn diff_bounds_is_tight() {
        let a = Canvas::new(20, 20);
        let mut b = a.clone();
        assert_eq!(a.diff_bounds(&b, a.bounds()), None);
        b.fill(Rect::new(3, 4, 2, 1), 0);
        b.fill(Rect::new(9, 7, 1, 1), 128);
        assert_eq!(a.diff_bounds(&b, a.bounds()), Some(Rect::new(3, 4, 7, 4)));
        // Restricted to a region that excludes one change.
        assert_eq!(
            a.diff_bounds(&b, Rect::new(0, 0, 8, 20)),
            Some(Rect::new(3, 4, 2, 1))
        );
    }

    #[test]
    fn bilevel_detection() {
        let mut c = Canvas::new(8, 8);
        c.fill(Rect::new(0, 0, 4, 8), 0);
        assert!(c.is_bilevel(c.bounds()));
        c.fill(Rect::new(6, 6, 1, 1), 100);
        assert!(!c.is_bilevel(c.bounds()));
        assert!(c.is_bilevel(Rect::new(0, 0, 6, 6)));
    }

    #[test]
    fn blend_full_and_half_coverage() {
        let mut c = Canvas::new(2, 1);
        let mut p = Painter::new(&mut c, Rect::new(0, 0, 2, 1));
        p.blend_mask(0, 0, 2, 1, &[255, 128], 0);
        assert_eq!(c.get(0, 0), 0);
        assert_eq!(c.get(1, 0), 127);
    }

    #[test]
    fn invert_twice_is_identity() {
        let mut c = Canvas::new(30, 30);
        c.fill(Rect::new(5, 5, 10, 10), 40);
        let before = c.clone();
        let mut p = Painter::new(&mut c, Rect::new(0, 0, 30, 30));
        p.invert_rect(Rect::new(2, 2, 20, 20));
        p.invert_rect(Rect::new(2, 2, 20, 20));
        assert_eq!(c, before);
    }
}
