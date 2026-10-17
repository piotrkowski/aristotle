//! Reader preferences: the menu's typography steps, mapped onto
//! aristotle-render's [`Style`], and the page-turn waveform.
//!
//! Stored as `reader.json` in the library data directory. They are global,
//! not per book: the owner sets a comfortable size once. Unknown or missing
//! fields fall back to the defaults, so the file can grow.

use std::path::Path;

use aristotle_render::style::Style;
use aristotle_ui::screens::reader::Settings;
use serde::{Deserialize, Serialize};

/// Body sizes in points for the menu's seven size steps. Step 2 is 10 pt,
/// the size chosen with the owner for Literata (`docs/reader.md`). The
/// steps match the UI prototype's pixel sizes (34 to 62 px at 300 DPI).
pub const FONT_PT: [f32; 7] = [8.0, 9.0, 10.0, 11.0, 12.0, 13.5, 15.0];
/// Line heights in ems for tight, normal and loose. Normal is the 1.3 em
/// the renderer was tuned with.
pub const LINE_HEIGHT: [f32; 3] = [1.2, 1.3, 1.5];
/// Side margins in whole millimetres (Plato takes whole millimetres).
/// Normal is the renderer's default 5 mm.
pub const MARGIN_MM: [i32; 3] = [3, 5, 8];

/// The stock reader's page-turn waveform (mode 9, a full update of the
/// whole screen). Measured 503 ms from tap to page on the device.
pub const STOCK_TURN: u32 = 9;
/// DU4 (mode 7): 317 ms in the same measurement, four gray levels. Offered
/// as an option; the owner decides on the panel.
pub const DU4_TURN: u32 = 7;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub size: usize,
    pub spacing: usize,
    pub margins: usize,
    /// EPDC waveform mode for page turns.
    pub turn_mode: u32,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            size: 2,
            spacing: 1,
            margins: 1,
            turn_mode: STOCK_TURN,
        }
    }
}

impl Prefs {
    pub const FILE: &'static str = "reader.json";

    /// Load from `dir`, or the defaults if there is no readable file.
    pub fn load(dir: &Path) -> Prefs {
        std::fs::read(dir.join(Self::FILE))
            .ok()
            .and_then(|d| serde_json::from_slice::<Prefs>(&d).ok())
            .map(Prefs::clamped)
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let data = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        aristotle_library::index::write_atomic(&dir.join(Self::FILE), &data)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }

    fn clamped(mut self) -> Prefs {
        self.size = self.size.min(FONT_PT.len() - 1);
        self.spacing = self.spacing.min(LINE_HEIGHT.len() - 1);
        self.margins = self.margins.min(MARGIN_MM.len() - 1);
        self
    }

    /// The page style for these preferences.
    pub fn style(&self) -> Style {
        let p = self.clone().clamped();
        let mut s = Style::pw4();
        s.font_size = FONT_PT[p.size];
        s.line_height = LINE_HEIGHT[p.spacing];
        s.margin_mm = MARGIN_MM[p.margins];
        s
    }

    /// The menu's view of these preferences, with the frontlight level.
    pub fn menu_settings(&self, light: f32) -> Settings {
        Settings {
            size: self.size,
            spacing: self.spacing,
            margins: self.margins,
            light,
            ..Settings::default()
        }
    }

    /// Take the typography steps from the menu.
    pub fn apply(&mut self, s: &Settings) {
        self.size = s.size;
        self.spacing = s.spacing;
        self.margins = s.margins;
        *self = self.clone().clamped();
    }
}

/// Frontlight: the menu slider is perceptual (0 to 1), the LM3695 takes raw
/// steps. Perceived brightness grows roughly with the square root of the
/// drive current, so the slider squares its value.
pub fn light_to_raw(v: f32, max: u32) -> u32 {
    let v = v.clamp(0.0, 1.0);
    (v * v * max as f32).round() as u32
}

pub fn raw_to_light(raw: u32, max: u32) -> f32 {
    (raw as f32 / max.max(1) as f32).clamp(0.0, 1.0).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_renderer_defaults() {
        let s = Prefs::default().style();
        let d = Style::pw4();
        assert_eq!(s.font_size, d.font_size);
        assert_eq!(s.line_height, d.line_height);
        assert_eq!(s.margin_mm, d.margin_mm);
        assert_eq!(Prefs::default().turn_mode, 9);
    }

    #[test]
    fn steps_follow_the_menu_tables() {
        use aristotle_ui::screens::reader::{MARGINS, SIZES, SPACING};
        assert_eq!(FONT_PT.len(), SIZES.len());
        assert_eq!(LINE_HEIGHT.len(), SPACING.len());
        assert_eq!(MARGIN_MM.len(), MARGINS.len());
        // Points grow with the step, like the prototype's pixel sizes.
        assert!(FONT_PT.windows(2).all(|w| w[0] < w[1]));
        let mut p = Prefs::default();
        p.apply(&Settings {
            size: 6,
            spacing: 2,
            margins: 0,
            ..Settings::default()
        });
        let s = p.style();
        assert_eq!((s.font_size, s.line_height, s.margin_mm), (15.0, 1.5, 3));
    }

    #[test]
    fn file_round_trip_and_bad_values() {
        let dir = crate::testdir("prefs");
        assert_eq!(Prefs::load(&dir), Prefs::default());
        let p = Prefs {
            size: 4,
            spacing: 0,
            margins: 2,
            turn_mode: DU4_TURN,
        };
        p.save(&dir).unwrap();
        assert_eq!(Prefs::load(&dir), p);
        std::fs::write(dir.join(Prefs::FILE), br#"{"size": 99, "future": 1}"#).unwrap();
        let q = Prefs::load(&dir);
        assert_eq!(q.size, FONT_PT.len() - 1);
        assert_eq!(q.turn_mode, STOCK_TURN);
    }

    #[test]
    fn light_mapping_round_trips() {
        assert_eq!(light_to_raw(0.0, 2047), 0);
        assert_eq!(light_to_raw(1.0, 2047), 2047);
        assert_eq!(light_to_raw(0.5, 2047), 512);
        for raw in [0, 1, 100, 512, 2047] {
            let back = light_to_raw(raw_to_light(raw, 2047), 2047);
            assert!(back.abs_diff(raw) <= 1, "{raw} -> {back}");
        }
    }
}
