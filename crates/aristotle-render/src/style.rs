// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page geometry and typesetting defaults for the Kindle Paperwhite 4.

/// The header and footer font, the UI's Literata, and its directory in the
/// data directory (`<dir>/<family>-{Regular,Italic}.ttf`).
pub const CHROME_FAMILY: &str = "Literata";
pub const CHROME_DIR: &str = "serif";

/// Everything that decides how a page looks. [`Style::pw4`] holds the
/// defaults; the CLI can override the typographic ones.
#[derive(Clone, Debug)]
pub struct Style {
    /// Visible screen width in pixels (1072 on the PW4).
    pub width: u32,
    /// Visible screen height in pixels (1448 on the PW4).
    pub height: u32,
    /// Bytes per row of the page buffer. 1088 on the PW4, matching the
    /// framebuffer's line length, so a page can be copied with one memcpy
    /// per row and dumped files read like framebuffer dumps.
    pub stride: usize,
    /// Panel resolution; Plato converts points and millimetres with it.
    pub dpi: u16,
    /// Font family for body text, looked up by name in [`Style::font_dir`].
    pub font_family: String,
    /// Directory (relative to the data directory, or absolute) holding the
    /// body font. Kept apart from Plato's `fonts/` because Plato opens every
    /// file in the search path to compare family names, which costs time
    /// per book. The header and footer stay in [`CHROME_FAMILY`] whatever
    /// the body font.
    pub font_dir: String,
    /// Body font size in points.
    pub font_size: f32,
    /// Line height in ems.
    pub line_height: f32,
    /// Margin around the text block in millimetres. Plato only takes whole
    /// millimetres, uniform on all sides of the area it lays out.
    pub margin_mm: i32,
    /// Distance in pixels from the top of the screen to the top of the
    /// text block; the header lives above it.
    pub text_top: u32,
    /// Distance in pixels from the bottom of the screen to the bottom of
    /// the text block; the footer lives below it.
    pub text_bottom: u32,
    /// Header and footer font size in points.
    pub chrome_size: f32,
    /// Gray level for header and footer text. Dark enough (below the
    /// threshold of [`crate::page::BW_THRESHOLD`]) to stay visible in the
    /// black and white A2 frame of a page turn.
    pub chrome_gray: u8,
    /// Hyphenation language for EPUB text, overriding the book's
    /// declaration (see [`crate::lang`]). `None`: decide per book.
    pub language: Option<String>,
}

impl Style {
    pub fn with_screen(mut self, screen: aristotle_screen::Screen, stride: usize) -> Self {
        assert!(
            stride >= screen.width as usize,
            "stride must cover the screen"
        );
        let ratio = screen.density.dpi() as f32 / self.dpi as f32;
        self.text_top = (self.text_top as f32 * ratio).round() as u32;
        self.text_bottom = (self.text_bottom as f32 * ratio).round() as u32;
        self.width = screen.width;
        self.height = screen.height;
        self.stride = stride;
        self.dpi = screen.density.dpi();
        self
    }

    /// Defaults for the PW4, chosen for Polish prose:
    ///
    /// - Literata: designed for e-reading, sturdy serifs and a large x-height
    ///   that survive e-ink's 16 gray levels, full Latin Extended-A
    ///   (ą ć ę ł ń ó ś ź ż) and Polish quotation marks, OFL licensed.
    /// - 10 pt at 300 DPI: Literata's large x-height reads like 11 pt of a
    ///   classical face such as Plato's default Libertinus.
    /// - 1.3 em leading and 5 mm side margins: about 60 characters per line
    ///   on the 91 mm wide panel, a comfortable measure for justified text
    ///   once Polish hyphenation removes the wide gaps.
    pub fn pw4() -> Style {
        Style {
            width: 1072,
            height: 1448,
            stride: 1088,
            dpi: 300,
            font_family: "Literata".to_string(),
            font_dir: "serif".to_string(),
            font_size: 10.0,
            line_height: 1.3,
            margin_mm: 5,
            text_top: 104,
            text_bottom: 100,
            chrome_size: 7.0,
            chrome_gray: 0x40,
            language: None,
        }
    }

    /// The margin in pixels, rounded the way Plato rounds it.
    pub fn margin_px(&self) -> i32 {
        (self.margin_mm as f32 * self.dpi as f32 / 25.4).round() as i32
    }

    /// The area handed to Plato for layout, as (top, height) in pixels.
    ///
    /// Plato puts the same margin on all four sides, but the page wants a
    /// header and a footer in the top and bottom margins. So the layout
    /// area spans the full width and is shifted vertically so that the
    /// text block lands exactly at `text_top` and `text_bottom`; the header
    /// and footer are drawn afterwards over Plato's (white) top and bottom
    /// margins.
    pub fn body(&self) -> (i32, u32) {
        let m = self.margin_px();
        let top = self.text_top as i32 - m;
        let bottom = self.height as i32 - self.text_bottom as i32 + m;
        (top, (bottom - top) as u32)
    }

    /// Point size in FreeType's 26.6 fixed point, as `Font::set_size` wants.
    pub fn chrome_size_26_6(&self) -> u32 {
        (self.chrome_size * 64.0).round() as u32
    }
}
