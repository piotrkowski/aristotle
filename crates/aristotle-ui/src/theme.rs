//! Design tokens: grays, spacing and the type scale.
//!
//! E-ink shows 16 gray levels and light grays fade under the frontlight, so
//! the palette is deliberately short. Hierarchy comes from size, weight and
//! space first and from gray second, and no text is lighter than `INK_3`.

use crate::geom::{mm, pt};
use crate::text::{FamilyId, TextStyle};

pub const INK: u8 = 0x00;
/// Secondary text: authors, metadata.
pub const INK_2: u8 = 0x44;
/// Tertiary text: captions, page numbers. The lightest text allowed.
pub const INK_3: u8 = 0x6a;
/// Hairlines and inactive tracks.
pub const RULE: u8 = 0xaa;
pub const PAPER: u8 = 0xff;

/// Side margin of UI screens, 6 mm.
pub fn margin() -> i32 {
    mm(6.0)
}

/// The UI type scale, in points so it reads the same on any density.
pub struct Type;

impl Type {
    pub const DISPLAY: f32 = 15.0;
    pub const TITLE: f32 = 11.0;
    pub const UI: f32 = 8.5;
    pub const SMALL: f32 = 7.5;
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

    pub fn title(&self) -> TextStyle {
        self.style(Type::TITLE).bold().line_height(1.2)
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
