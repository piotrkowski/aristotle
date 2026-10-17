// SPDX-License-Identifier: AGPL-3.0-or-later
//! Icons: one family of vector outlines on a 24-unit grid.
//!
//! # Grid and weight
//!
//! Every icon is drawn on a 24 × 24 grid with a 2-unit safe border (live
//! area 2 to 22), as outlines with round caps and joins. The stroke is one
//! weight for the whole family, chosen per size in whole device pixels
//! ([`stroke_px`]): 3 px up to 36 px icons, 4 px at 44 to 48 px, 5 px at 56 to
//! 64 px and 6 px from 72 px. Small icons therefore get a relatively heavier
//! stroke (optical sizing: 2.25 units at 32 px, 2 units at 48 px), so the
//! status bar's icons do not look thinner than the toolbar's.
//!
//! The standard size is 48 px (4 mm at 300 DPI, 2 px per unit). There, the
//! 4 px stroke centred on whole grid units covers whole pixels, so the
//! horizontal and vertical strokes of most icons are pure black with no
//! anti-aliased edge: they stay sharp in 16 grays and under DU and A2, which
//! drop grays. Coordinates are therefore whole units wherever the drawing
//! allows.
//!
//! Paths instead of bitmaps: one definition renders at every size, costs a
//! few dozen bytes, and inverts cleanly for night mode and pressed states.

use crate::canvas::Painter;
use crate::geom::Rect;
use zeno::Transform;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// Home: the library (three books on a shelf).
    Library,
    ArrowLeft,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    ChevronDown,
    Menu,
    More,
    Settings,
    Search,
    /// Typography ("Aa").
    Font,
    Sun,
    SunSmall,
    Moon,
    Bookmark,
    BookmarkFilled,
    Folder,
    /// A series or collection: a stack of volumes.
    Series,
    Sort,
    Contents,
    Close,
    Plus,
    Minus,
    Check,
    Battery,
    BatteryCharging,
    Usb,
    Info,
    Storage,
    Clock,
    Power,
    SpacingTight,
    SpacingNormal,
    SpacingLoose,
    MarginsNarrow,
    MarginsNormal,
    MarginsWide,
    /// The UI language: a globe.
    Language,
}

impl Icon {
    /// Every icon, in the order of the preview sheet.
    pub const ALL: [Icon; 38] = [
        Icon::Library,
        Icon::ArrowLeft,
        Icon::Menu,
        Icon::Settings,
        Icon::Search,
        Icon::Font,
        Icon::Sun,
        Icon::SunSmall,
        Icon::Moon,
        Icon::Bookmark,
        Icon::BookmarkFilled,
        Icon::Folder,
        Icon::Series,
        Icon::Sort,
        Icon::Contents,
        Icon::Close,
        Icon::ChevronLeft,
        Icon::ChevronRight,
        Icon::ChevronUp,
        Icon::ChevronDown,
        Icon::More,
        Icon::Battery,
        Icon::BatteryCharging,
        Icon::Usb,
        Icon::Info,
        Icon::Storage,
        Icon::Clock,
        Icon::Power,
        Icon::Plus,
        Icon::Minus,
        Icon::Check,
        Icon::SpacingTight,
        Icon::SpacingNormal,
        Icon::SpacingLoose,
        Icon::MarginsNarrow,
        Icon::MarginsNormal,
        Icon::MarginsWide,
        Icon::Language,
    ];

    /// The English name, for the preview sheet and debugging.
    pub fn name(self) -> &'static str {
        match self {
            Icon::Library => "library",
            Icon::ArrowLeft => "back",
            Icon::ChevronLeft => "chevron-left",
            Icon::ChevronRight => "chevron-right",
            Icon::ChevronUp => "chevron-up",
            Icon::ChevronDown => "chevron-down",
            Icon::Menu => "menu",
            Icon::More => "more",
            Icon::Settings => "settings",
            Icon::Search => "search",
            Icon::Font => "font",
            Icon::Sun => "brightness",
            Icon::SunSmall => "dim",
            Icon::Moon => "night",
            Icon::Bookmark => "bookmark",
            Icon::BookmarkFilled => "bookmarked",
            Icon::Folder => "folder",
            Icon::Series => "series",
            Icon::Sort => "sort",
            Icon::Contents => "contents",
            Icon::Close => "close",
            Icon::Plus => "plus",
            Icon::Minus => "minus",
            Icon::Check => "check",
            Icon::Battery => "battery",
            Icon::BatteryCharging => "charging",
            Icon::Usb => "usb",
            Icon::Info => "info",
            Icon::Storage => "storage",
            Icon::Clock => "clock",
            Icon::Power => "sleep",
            Icon::SpacingTight => "spacing-1",
            Icon::SpacingNormal => "spacing-2",
            Icon::SpacingLoose => "spacing-3",
            Icon::MarginsNarrow => "margins-1",
            Icon::MarginsNormal => "margins-2",
            Icon::MarginsWide => "margins-3",
            Icon::Language => "language",
        }
    }
}

/// Stroked outlines, then filled shapes, both in grid units.
struct Def {
    stroke: &'static str,
    fill: &'static str,
}

const NONE: &str = "";

const GEAR: &str = "M18.44 10.15L20.9 10.67L20.9 13.33L18.44 13.85L17.86 15.25L19.23 17.35\
     L17.35 19.23L15.25 17.86L13.85 18.44L13.33 20.9L10.67 20.9L10.15 18.44L8.75 17.86\
     L6.65 19.23L4.77 17.35L6.14 15.25L5.56 13.85L3.1 13.33L3.1 10.67L5.56 10.15L6.14 8.75\
     L4.77 6.65L6.65 4.77L8.75 6.14L10.15 5.56L10.67 3.1L13.33 3.1L13.85 5.56L15.25 6.14\
     L17.35 4.77L19.23 6.65L17.86 8.75Z\
     M15 12A3 3 0 1 1 9 12A3 3 0 1 1 15 12Z";

fn def(icon: Icon) -> Def {
    let (stroke, fill) = match icon {
        Icon::Library => (
            "M3 4H7V20H3ZM9.5 4H13.5V20H9.5ZM15.2 5.3L18.9 4.3L21.4 18.9L17.7 20Z",
            NONE,
        ),
        Icon::ArrowLeft => ("M20 12H4M10 6L4 12L10 18", NONE),
        Icon::ChevronLeft => ("M15 5L8 12L15 19", NONE),
        Icon::ChevronRight => ("M9 5L16 12L9 19", NONE),
        Icon::ChevronUp => ("M5 15L12 8L19 15", NONE),
        Icon::ChevronDown => ("M5 9L12 16L19 9", NONE),
        Icon::Menu => ("M4 6H20M4 12H20M4 18H20", NONE),
        Icon::More => (
            NONE,
            "M14 5A2 2 0 1 1 10 5A2 2 0 1 1 14 5ZM14 12A2 2 0 1 1 10 12A2 2 0 1 1 14 12Z\
             M14 19A2 2 0 1 1 10 19A2 2 0 1 1 14 19Z",
        ),
        Icon::Settings => (GEAR, NONE),
        Icon::Search => (
            "M18 11A7 7 0 1 1 4 11A7 7 0 1 1 18 11ZM16 16L21 21",
            NONE,
        ),
        Icon::Font => (
            "M2 20L8 4L14 20M4.25 14H11.75M21 13V20M21 17A3 3 0 1 1 15 17A3 3 0 1 1 21 17Z",
            NONE,
        ),
        Icon::Sun => (
            "M16 12A4 4 0 1 1 8 12A4 4 0 1 1 16 12Z\
             M12 2V4M12 20V22M2 12H4M20 12H22\
             M4.93 4.93L6.34 6.34M17.66 17.66L19.07 19.07M4.93 19.07L6.34 17.66M17.66 6.34L19.07 4.93",
            NONE,
        ),
        Icon::SunSmall => (
            "M15 12A3 3 0 1 1 9 12A3 3 0 1 1 15 12Z\
             M12 5V5.01M12 19V19.01M5 12H5.01M19 12H19.01\
             M7.05 7.05H7.06M16.95 16.95H16.96M7.05 16.95H7.06M16.95 7.05H16.96",
            NONE,
        ),
        Icon::Moon => ("M20 14.5A8.5 8.5 0 1 1 9.5 4A6.5 6.5 0 0 0 20 14.5Z", NONE),
        Icon::Bookmark => ("M6 3H18V21L12 16.5L6 21Z", NONE),
        Icon::BookmarkFilled => ("M6 3H18V21L12 16.5L6 21Z", "M6 3H18V21L12 16.5L6 21Z"),
        Icon::Folder => (
            "M3 6.5A1.5 1.5 0 0 1 4.5 5H9L11 7.5H19.5A1.5 1.5 0 0 1 21 9V18.5\
             A1.5 1.5 0 0 1 19.5 20H4.5A1.5 1.5 0 0 1 3 18.5Z",
            NONE,
        ),
        Icon::Series => ("M8 8H19V21H8ZM5 18V5H16", NONE),
        Icon::Sort => ("M7 4V20M3.5 16.5L7 20L10.5 16.5M13 6H21M13 11H19M13 16H17", NONE),
        Icon::Contents => (
            "M9 6H20M9 12H20M9 18H20M4 6H4.01M4 12H4.01M4 18H4.01",
            NONE,
        ),
        Icon::Close => ("M6 6L18 18M18 6L6 18", NONE),
        Icon::Plus => ("M12 5V19M5 12H19", NONE),
        Icon::Minus => ("M5 12H19", NONE),
        Icon::Check => ("M5 12.5L10 17.5L19 7", NONE),
        Icon::Battery => ("M3 7H18V17H3ZM21 10.5V13.5", "M5.5 9.5H13V14.5H5.5Z"),
        Icon::BatteryCharging => (
            "M3 7H18V17H3ZM21 10.5V13.5",
            "M11.5 8.5L6.5 12.6H10L8.5 15.5L13.5 11.4H10Z",
        ),
        Icon::Usb => (
            "M12 3V17M9.5 5.5L12 3L14.5 5.5M12 14L7 11.5V9M12 11.5L17 9V7",
            "M14 19A2 2 0 1 1 10 19A2 2 0 1 1 14 19ZM8.6 8.2A1.6 1.6 0 1 1 5.4 8.2\
             A1.6 1.6 0 1 1 8.6 8.2ZM15.5 4.5H18.5V7.5H15.5Z",
        ),
        Icon::Info => (
            "M21 12A9 9 0 1 1 3 12A9 9 0 1 1 21 12ZM12 11V17M12 7.5V7.51",
            NONE,
        ),
        Icon::Storage => ("M7 3H15L19 7V21H7ZM10 7V10M13 7V10M16 9V10", NONE),
        Icon::Clock => ("M21 12A9 9 0 1 1 3 12A9 9 0 1 1 21 12ZM12 7V12L15 14", NONE),
        Icon::Power => ("M12 3V11M7 5.5A8 8 0 1 0 17 5.5", NONE),
        Icon::SpacingTight => ("M5 8H19M5 12H19M5 16H19", NONE),
        Icon::SpacingNormal => ("M5 6H19M5 12H19M5 18H19", NONE),
        Icon::SpacingLoose => ("M5 3.5H19M5 12H19M5 20.5H19", NONE),
        Icon::MarginsNarrow => ("M4 3H20V21H4ZM7 8H17M7 12H17M7 16H17", NONE),
        Icon::MarginsNormal => ("M4 3H20V21H4ZM9 8H15M9 12H15M9 16H15", NONE),
        Icon::MarginsWide => ("M4 3H20V21H4ZM10.5 8H13.5M10.5 12H13.5M10.5 16H13.5", NONE),
        Icon::Language => (
            "M21 12A9 9 0 1 1 3 12A9 9 0 1 1 21 12ZM3 12H21M12 3A4 9 0 0 1 12 21A4 9 0 0 1 12 3Z",
            NONE,
        ),
    };
    Def { stroke, fill }
}

/// The family's stroke for an icon `size` px wide, in whole pixels: 3 up to
/// 36 px, 4 up to 52 px, 5 up to 68 px, 6 above. See the module docs.
pub fn stroke_px(size: i32) -> f32 {
    ((size as f32 / 12.0).round()).clamp(3.0, 6.0)
}

/// Draws `icon` scaled into the square `rect` (the smaller side wins).
/// `weight` is the stroke width in grid units; 0 picks the family's
/// optical weight for the size ([`stroke_px`]), which is what UI chrome
/// should use.
pub fn draw(p: &mut Painter, icon: Icon, rect: Rect, gray: u8, weight: f32) {
    let side = rect.w.min(rect.h);
    let s = side as f32 / 24.0;
    // Whole-pixel origin, so whole grid units land on whole pixels at 48 px.
    let x = (rect.x + (rect.w - side) / 2) as f32;
    let y = (rect.y + (rect.h - side) / 2) as f32;
    let t = Transform::scale(s, s).then_translate(x, y);
    let width = if weight > 0.0 {
        weight
    } else {
        stroke_px(side) / s
    };
    let d = def(icon);
    if !d.fill.is_empty() {
        p.fill_path(d.fill, Some(t), gray);
    }
    if !d.stroke.is_empty() {
        p.stroke_path(d.stroke, width, Some(t), gray);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Canvas;

    #[test]
    fn every_icon_draws_inside_its_box() {
        for icon in Icon::ALL {
            for size in [32, 48, 72] {
                let mut c = Canvas::new(120, 120);
                let mut p = Painter::new(&mut c, Rect::new(0, 0, 120, 120));
                draw(&mut p, icon, Rect::new(20, 20, size, size), 0, 0.0);
                let ink = c.diff_bounds(&Canvas::new(120, 120), c.bounds());
                let ink = ink.unwrap_or_else(|| panic!("{icon:?} drew nothing"));
                // At most a pixel past the grid.
                let frame = Rect::new(19, 19, size + 2, size + 2);
                assert!(
                    frame.intersect(&ink) == Some(ink),
                    "{icon:?} at {size} spills out: {ink:?}"
                );
            }
        }
    }

    #[test]
    fn standard_size_strokes_are_pixel_sharp() {
        // The menu icon's bars at 48 px: 4 px tall, fully black, no gray rows.
        let mut c = Canvas::new(48, 48);
        let mut p = Painter::new(&mut c, Rect::new(0, 0, 48, 48));
        draw(&mut p, Icon::Menu, Rect::new(0, 0, 48, 48), 0, 0.0);
        let column: Vec<u8> = (0..48).map(|y| c.get(24, y)).collect();
        assert!(column.iter().all(|&v| v == 0 || v == 255), "{column:?}");
        assert_eq!(column.iter().filter(|&&v| v == 0).count(), 12);
    }

    #[test]
    fn optical_weights() {
        assert_eq!(stroke_px(32), 3.0);
        assert_eq!(stroke_px(48), 4.0);
        assert_eq!(stroke_px(64), 5.0);
        assert_eq!(stroke_px(96), 6.0);
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = Icon::ALL.iter().map(|i| i.name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Icon::ALL.len());
    }
}
