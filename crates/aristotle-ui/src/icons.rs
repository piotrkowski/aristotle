//! Icons as vector paths on a 24-unit grid.
//!
//! Paths instead of bitmaps: one definition renders crisply at every size
//! and density, costs a few dozen bytes, and the strokes are thick enough
//! (about 1/13 of the icon) to stay solid black after the panel's 16-level
//! quantization and under A2, which drops all grays.

use crate::canvas::Painter;
use crate::geom::Rect;
use zeno::Transform;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    ArrowLeft,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    Search,
    More,
    Sort,
    Contents,
    Bookmark,
    Close,
    Plus,
    Minus,
    Sun,
    SunSmall,
    Moon,
    Check,
    SpacingTight,
    SpacingNormal,
    SpacingLoose,
    MarginsNarrow,
    MarginsNormal,
    MarginsWide,
}

/// Stroked outlines, then filled shapes, both in grid units.
struct Def {
    stroke: &'static str,
    fill: &'static str,
}

const NONE: &str = "";

fn def(icon: Icon) -> Def {
    let (stroke, fill) = match icon {
        Icon::ArrowLeft => ("M19 12H5M11 6L5 12L11 18", NONE),
        Icon::ChevronLeft => ("M15 5L8 12L15 19", NONE),
        Icon::ChevronRight => ("M9 5L16 12L9 19", NONE),
        Icon::ChevronDown => ("M6 9L12 15L18 9", NONE),
        Icon::Search => (
            "M17 10.5A6.5 6.5 0 1 1 4 10.5A6.5 6.5 0 1 1 17 10.5ZM15.2 15.2L20.5 20.5",
            NONE,
        ),
        Icon::More => (
            NONE,
            "M14 5A2 2 0 1 1 10 5A2 2 0 1 1 14 5ZM14 12A2 2 0 1 1 10 12A2 2 0 1 1 14 12Z\
             M14 19A2 2 0 1 1 10 19A2 2 0 1 1 14 19Z",
        ),
        Icon::Sort => (
            "M7 4V20M3.5 16.5L7 20L10.5 16.5M13 6H21M13 11H19M13 16H17",
            NONE,
        ),
        Icon::Contents => ("M9 6H20M9 12H20M9 18H20M4.5 6H5M4.5 12H5M4.5 18H5", NONE),
        Icon::Bookmark => ("M6.5 3.5H17.5V20.5L12 16L6.5 20.5Z", NONE),
        Icon::Close => ("M6 6L18 18M18 6L6 18", NONE),
        Icon::Plus => ("M12 5V19M5 12H19", NONE),
        Icon::Minus => ("M5 12H19", NONE),
        Icon::Sun => (
            "M16 12A4 4 0 1 1 8 12A4 4 0 1 1 16 12Z\
             M12 2V4.5M12 19.5V22M2 12H4.5M19.5 12H22\
             M4.9 4.9L6.6 6.6M17.4 17.4L19.1 19.1M4.9 19.1L6.6 17.4M17.4 6.6L19.1 4.9",
            NONE,
        ),
        Icon::SunSmall => (
            "M14.5 12A2.5 2.5 0 1 1 9.5 12A2.5 2.5 0 1 1 14.5 12Z\
             M12 6V7M12 17V18M6 12H7M17 12H18",
            NONE,
        ),
        Icon::Moon => ("M20 14.5A8.5 8.5 0 1 1 9.5 4A7 7 0 0 0 20 14.5Z", NONE),
        Icon::Check => ("M5 12.5L10 17.5L19 7", NONE),
        Icon::SpacingTight => ("M5 9H19M5 12H19M5 15H19", NONE),
        Icon::SpacingNormal => ("M5 7H19M5 12H19M5 17H19", NONE),
        Icon::SpacingLoose => ("M5 4.5H19M5 12H19M5 19.5H19", NONE),
        Icon::MarginsNarrow => ("M4 3H20V21H4ZM6.5 8H17.5M6.5 12H17.5M6.5 16H17.5", NONE),
        Icon::MarginsNormal => ("M4 3H20V21H4ZM8 8H16M8 12H16M8 16H16", NONE),
        Icon::MarginsWide => ("M4 3H20V21H4ZM10 8H14M10 12H14M10 16H14", NONE),
    };
    Def { stroke, fill }
}

/// Draws `icon` scaled into the square `rect` (the smaller side wins).
/// `weight` is the stroke width in grid units; 1.8 suits UI chrome.
pub fn draw(p: &mut Painter, icon: Icon, rect: Rect, gray: u8, weight: f32) {
    let side = rect.w.min(rect.h) as f32;
    let s = side / 24.0;
    let x = rect.x as f32 + (rect.w as f32 - side) / 2.0;
    let y = rect.y as f32 + (rect.h as f32 - side) / 2.0;
    let t = Transform::scale(s, s).then_translate(x, y);
    let d = def(icon);
    if !d.fill.is_empty() {
        p.fill_path(d.fill, Some(t), gray);
    }
    if !d.stroke.is_empty() {
        p.stroke_path(d.stroke, weight * s, Some(t), gray);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Canvas;

    #[test]
    fn every_icon_draws_inside_its_box() {
        let all = [
            Icon::ArrowLeft,
            Icon::ChevronLeft,
            Icon::ChevronRight,
            Icon::ChevronDown,
            Icon::Search,
            Icon::More,
            Icon::Sort,
            Icon::Contents,
            Icon::Bookmark,
            Icon::Close,
            Icon::Plus,
            Icon::Minus,
            Icon::Sun,
            Icon::SunSmall,
            Icon::Moon,
            Icon::Check,
            Icon::SpacingTight,
            Icon::SpacingNormal,
            Icon::SpacingLoose,
            Icon::MarginsNarrow,
            Icon::MarginsNormal,
            Icon::MarginsWide,
        ];
        for icon in all {
            let mut c = Canvas::new(80, 80);
            let mut p = Painter::new(&mut c, Rect::new(0, 0, 80, 80));
            draw(&mut p, icon, Rect::new(16, 16, 48, 48), 0, 1.8);
            let ink = c.diff_bounds(&Canvas::new(80, 80), c.bounds());
            let ink = ink.unwrap_or_else(|| panic!("{icon:?} drew nothing"));
            assert!(
                Rect::new(14, 14, 52, 52).intersect(&ink) == Some(ink),
                "{icon:?} spills out: {ink:?}"
            );
        }
    }
}
