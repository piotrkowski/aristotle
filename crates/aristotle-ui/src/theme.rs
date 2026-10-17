//! Design tokens: grays, spacing and the type scale.
//!
//! E-ink shows 16 gray levels and light grays fade under the frontlight, so
//! the palette is deliberately short. Hierarchy comes from size, weight and
//! space first and from gray second, and no text is lighter than `INK_3`.
//! Every gray is one of the panel's 16 levels (a multiple of 17), so what
//! the preview shows is what GL16 reproduces, without dithering.
//!
//! Night mode inverts the final pixels (`Ui::set_night`), so the night
//! palette is the day palette mirrored: `INK` 0x00 becomes 0xff (white
//! text), `INK_2` 0x44 becomes 0xbb, `INK_3` 0x66 becomes 0x99, `RULE` 0xaa
//! becomes 0x55 and `TONE` 0xdd becomes 0x22. Black-and-white chrome stays
//! black and white, so it keeps qualifying for DU.

use crate::geom::{mm, pt};
use crate::text::{FamilyId, TextStyle};

pub const INK: u8 = 0x00;
/// Secondary text: authors, metadata.
pub const INK_2: u8 = 0x44;
/// Tertiary text: captions, page numbers. The lightest text allowed.
pub const INK_3: u8 = 0x66;
/// Hairlines and inactive tracks.
pub const RULE: u8 = 0xaa;
/// Quiet fills: inactive chips, card tones.
pub const TONE: u8 = 0xdd;
pub const PAPER: u8 = 0xff;

/// Side margin of UI screens, 6 mm.
pub fn margin() -> i32 {
    mm(6.0)
}

/// The spacing scale: multiples of an 8 px unit (0.68 mm at 300 DPI).
/// Every gap and padding in the screens comes from here.
pub struct Space;

impl Space {
    pub const XS: i32 = 8;
    pub const S: i32 = 16;
    pub const M: i32 = 24;
    pub const L: i32 = 32;
    pub const XL: i32 = 48;
    pub const XXL: i32 = 64;
}

/// The UI type scale, in points so it reads the same on any density.
/// Roughly a 1.15 ratio between neighbours, with a jump to the display
/// size for the one title per screen.
pub struct Type;

impl Type {
    /// The screen title ("Biblioteka", "Ustawienia").
    pub const DISPLAY: f32 = 15.0;
    /// The book to continue, the folder title.
    pub const HEADLINE: f32 = 12.0;
    pub const TITLE: f32 = 11.0;
    /// A list item's main line.
    pub const ITEM: f32 = 9.5;
    /// Body text of the chrome: tabs, labels, buttons.
    pub const UI: f32 = 8.5;
    /// Secondary lines: authors, values, hints.
    pub const SMALL: f32 = 7.5;
}

/// Icon sizes in pixels. 48 is the standard (see `icons`); 36 for the
/// status line and inline marks.
pub struct IconSize;

impl IconSize {
    pub const SMALL: i32 = 36;
    pub const STANDARD: i32 = 48;
    pub const LARGE: i32 = 60;
}

/// Text styles for the UI chrome, all in the UI family (Literata).
#[derive(Clone)]
pub struct Styles {
    pub family: FamilyId,
}

impl Styles {
    pub fn new(family: FamilyId) -> Self {
        Self { family }
    }

    fn style(&self, points: f32) -> TextStyle {
        TextStyle::new(self.family, pt(points)).line_height(1.3)
    }

    pub fn display(&self) -> TextStyle {
        self.style(Type::DISPLAY).bold().line_height(1.15)
    }

    pub fn headline(&self) -> TextStyle {
        self.style(Type::HEADLINE).bold().line_height(1.15)
    }

    pub fn title(&self) -> TextStyle {
        self.style(Type::TITLE).bold().line_height(1.2)
    }

    pub fn item(&self) -> TextStyle {
        self.style(Type::ITEM).bold().line_height(1.15)
    }

    pub fn ui(&self) -> TextStyle {
        self.style(Type::UI)
    }

    pub fn ui_bold(&self) -> TextStyle {
        self.style(Type::UI).bold()
    }

    pub fn small(&self) -> TextStyle {
        self.style(Type::SMALL).gray(INK_2)
    }

    /// Small capitals with a little tracking, for section labels.
    pub fn caps(&self) -> TextStyle {
        self.style(Type::SMALL)
            .small_caps()
            .tracking(pt(Type::SMALL) * 0.08)
            .gray(INK_3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grays_are_panel_levels() {
        for g in [INK, INK_2, INK_3, RULE, TONE, PAPER] {
            assert_eq!(g % 17, 0, "{g:#x} is not one of the 16 levels");
        }
    }

    #[test]
    fn spacing_is_on_the_grid() {
        for s in [
            Space::XS,
            Space::S,
            Space::M,
            Space::L,
            Space::XL,
            Space::XXL,
        ] {
            assert_eq!(s % 8, 0);
        }
    }
}
