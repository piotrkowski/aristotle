//! Full-screen 8-bit page buffers, and the header and footer drawn on them.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use plato_core::anyhow::{format_err, Error};
use plato_core::color::Color;
use plato_core::font::{Font, FontOpener};
use plato_core::framebuffer::{Framebuffer, Pixmap, UpdateMode};
use plato_core::geom::{Point, Rectangle};

use crate::style::Style;

/// Gray level below which a pixel becomes black in the black and white
/// frame of a page turn. Same value as the page-turn experiment
/// (`aristotle-bench --bin pageturn`), which the owner judged legible.
pub const BW_THRESHOLD: u8 = 160;

/// A full-screen page: `height` rows of `stride` bytes, the first `width`
/// of each visible. 0 is black, 255 white, like the PW4 framebuffer.
#[derive(Clone)]
pub struct Page {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub data: Vec<u8>,
}

impl Page {
    /// A white page with the style's geometry.
    pub fn white(style: &Style) -> Page {
        Page {
            width: style.width,
            height: style.height,
            stride: style.stride,
            data: vec![0xff; style.stride * style.height as usize],
        }
    }

    /// Copy a grayscale pixmap (1 sample per pixel) with its top-left
    /// corner at (`x`, `y`), clipped to the page. One memcpy per row.
    pub fn blit_pixmap(&mut self, pixmap: &Pixmap, x: i32, y: i32) {
        debug_assert_eq!(pixmap.samples, 1);
        let src_w = pixmap.width as i32;
        let x0 = x.max(0);
        let x1 = (x + src_w).min(self.width as i32);
        if x0 >= x1 {
            return;
        }
        for row in 0..pixmap.height as i32 {
            let dy = y + row;
            if dy < 0 || dy >= self.height as i32 {
                continue;
            }
            let src = (row * src_w + (x0 - x)) as usize;
            let dst = dy as usize * self.stride + x0 as usize;
            let len = (x1 - x0) as usize;
            self.data[dst..dst + len].copy_from_slice(&pixmap.data[src..src + len]);
        }
    }

    /// The black and white version shown first on a page turn: A2 can only
    /// drive pure black and white, and anything in between would be
    /// rounded unpredictably by the waveform.
    pub fn threshold(&self) -> Page {
        Page {
            data: self
                .data
                .iter()
                .map(|&p| if p < BW_THRESHOLD { 0x00 } else { 0xff })
                .collect(),
            ..*self
        }
    }

    /// Write the raw buffer (stride included), readable by
    /// `devloop/host/fb2png.py` like a framebuffer dump.
    pub fn write_raw(&self, path: &Path) -> io::Result<()> {
        File::create(path)?.write_all(&self.data)
    }
}

// Plato's fonts draw into anything that implements its `Framebuffer` trait.
// Only the pixel setters matter here; the display methods are never called
// on an off-screen page.
impl Framebuffer for Page {
    fn set_pixel(&mut self, x: u32, y: u32, color: Color) {
        if x < self.width && y < self.height {
            self.data[y as usize * self.stride + x as usize] = color.gray();
        }
    }

    fn set_blended_pixel(&mut self, x: u32, y: u32, color: Color, alpha: f32) {
        if x < self.width && y < self.height {
            let p = &mut self.data[y as usize * self.stride + x as usize];
            let (a, b) = (*p as f32, color.gray() as f32);
            *p = (a + (b - a) * alpha.clamp(0.0, 1.0)) as u8;
        }
    }

    fn invert_region(&mut self, _rect: &Rectangle) {}
    fn shift_region(&mut self, _rect: &Rectangle, _drift: u8) {}
    fn update(&mut self, _rect: &Rectangle, _mode: UpdateMode) -> Result<u32, Error> {
        Err(format_err!("an off-screen page has no display"))
    }
    fn wait(&self, _token: u32) -> Result<i32, Error> {
        Err(format_err!("an off-screen page has no display"))
    }
    fn save(&self, path: &str) -> Result<(), Error> {
        Ok(self.write_raw(Path::new(path))?)
    }
    fn set_rotation(&mut self, _n: i8) -> Result<(u32, u32), Error> {
        Err(format_err!("an off-screen page has no rotation"))
    }
    fn set_monochrome(&mut self, _enable: bool) {}
    fn set_dithered(&mut self, _enable: bool) {}
    fn set_inverted(&mut self, _enable: bool) {}
    fn monochrome(&self) -> bool {
        false
    }
    fn dithered(&self) -> bool {
        false
    }
    fn inverted(&self) -> bool {
        false
    }
    fn width(&self) -> u32 {
        self.width
    }
    fn height(&self) -> u32 {
        self.height
    }
}

/// What the header and footer say about the current page.
#[derive(Clone, Debug, Default)]
pub struct Folio {
    /// Chapter title, or the book title when the chapter is unknown.
    pub header: String,
    /// 1-based page number.
    pub page: usize,
    /// Total pages, when the book has been paginated.
    pub pages: Option<usize>,
    /// Position in the book, 0.0 to 1.0.
    pub progress: f32,
}

/// The slim header and footer ("chrome") drawn by Aristotle itself rather
/// than by the book: chapter title on top, page number and progress below.
/// They are drawn in the same frame as the text, so a page turn needs no
/// separate footer update (the stock reader sends one, see
/// `docs/native-vs-aristotle.md`).
pub struct Chrome {
    regular: Font,
    italic: Font,
    size: u32,
    dpi: u16,
    gray: u8,
    margin: i32,
    header_baseline: i32,
    footer_baseline: i32,
}

impl Chrome {
    /// Load the chrome fonts from `<font_dir>/<family>-{Regular,Italic}.ttf`
    /// (relative to the data directory).
    pub fn load(style: &Style) -> Result<Chrome, Error> {
        let opener = FontOpener::new()?;
        let path = |face: &str| format!("{}/{}-{}.ttf", style.font_dir, style.font_family, face);
        let mut regular = opener.open(path("Regular"))?;
        let mut italic = opener.open(path("Italic"))?;
        let size = style.chrome_size_26_6();
        regular.set_size(size, style.dpi);
        italic.set_size(size, style.dpi);
        // Centre the chrome lines in the bands above and below the text
        // block, with their x-height optically centred.
        let x_height = regular.x_heights.0 as i32;
        let header_baseline = (style.text_top as i32 + x_height) / 2;
        let footer_baseline = style.height as i32 - (style.text_bottom as i32 - x_height) / 2;
        Ok(Chrome {
            regular,
            italic,
            size,
            dpi: style.dpi,
            gray: style.chrome_gray,
            margin: style.margin_px(),
            header_baseline,
            footer_baseline,
        })
    }

    /// Draw the header and footer for `folio` onto `page`.
    pub fn draw(&mut self, page: &mut Page, folio: &Folio) {
        let color = Color::Gray(self.gray);
        let width = page.width as i32;
        let max_width = width - 2 * self.margin;

        self.italic.set_size(self.size, self.dpi);
        let mut plan = self.italic.plan(&folio.header, None, None);
        self.italic.crop_right(&mut plan, max_width);
        let x = (width - plan.width) / 2;
        self.italic
            .render(page, color, &plan, Point::new(x, self.header_baseline));

        self.regular.set_size(self.size, self.dpi);
        let number = match folio.pages {
            Some(total) => format!("{} / {}", folio.page, total),
            None => folio.page.to_string(),
        };
        let plan = self.regular.plan(&number, None, None);
        let x = (width - plan.width) / 2;
        self.regular
            .render(page, color, &plan, Point::new(x, self.footer_baseline));

        let percent = format!("{}%", (folio.progress * 100.0).floor() as u32);
        let plan = self.regular.plan(&percent, None, None);
        let x = width - self.margin - plan.width;
        self.regular
            .render(page, color, &plan, Point::new(x, self.footer_baseline));
    }
}
