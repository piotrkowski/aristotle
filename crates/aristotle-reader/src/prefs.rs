//! Preferences: the menu's typography steps (mapped onto aristotle-render's
//! [`Style`]), the page-turn waveform, night mode, library covers and view,
//! and the screensaver delay.
//!
//! Stored as `reader.json` in the library data directory. They are global,
//! not per book: the owner sets a comfortable size once. Unknown or missing
//! fields fall back to the defaults, so the file can grow, and a file from
//! an older version keeps its values. The frontlight is not stored here: the
//! LM3695 keeps its level, and the app reads it back.

use std::path::Path;

use aristotle_render::style::Style;
use aristotle_ui::screens::library::{CoverMode, View};
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

/// How night mode inverts the screen (`docs/ui-framework.md`, "Night mode").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NightMethod {
    /// Our pixels are inverted before they reach the framebuffer: the UI
    /// canvas by `Ui::set_night`, book pages in the copy to the framebuffer.
    #[default]
    Software,
    /// The framebuffer keeps day pixels and every update asks the PXP to
    /// invert (`EPDC_FLAG_ENABLE_INVERSION`).
    Hardware,
}

/// Library covers, as stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Covers {
    #[default]
    None,
    Cards,
    Photos,
}

impl Covers {
    pub fn mode(self) -> CoverMode {
        match self {
            Covers::None => CoverMode::None,
            Covers::Cards => CoverMode::Cards,
            Covers::Photos => CoverMode::Photos,
        }
    }

    pub fn from_mode(m: CoverMode) -> Covers {
        match m {
            CoverMode::None => Covers::None,
            CoverMode::Cards => Covers::Cards,
            CoverMode::Photos => Covers::Photos,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub size: usize,
    pub spacing: usize,
    pub margins: usize,
    /// EPDC waveform mode for page turns.
    pub turn_mode: u32,
    pub night: bool,
    pub night_method: NightMethod,
    pub covers: Covers,
    /// Minutes without a touch before the screensaver; 0 is never.
    pub sleep_minutes: u32,
    /// Library view: 0 books, 1 folders, 2 series.
    pub library_view: usize,
    /// Index into `shelf::SORTS`.
    pub sort: usize,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            size: 2,
            spacing: 1,
            margins: 1,
            turn_mode: STOCK_TURN,
            night: false,
            night_method: NightMethod::Software,
            covers: Covers::None,
            sleep_minutes: 15,
            library_view: 0,
            sort: 0,
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
        self.library_view = self.library_view.min(View::ALL.len() - 1);
        self.sort = self.sort.min(crate::shelf::SORTS.len() - 1);
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

    /// The menu's view of these preferences, with the frontlight level
    /// (0 to 1).
    pub fn menu_settings(&self, light: f32) -> Settings {
        Settings {
            size: self.size,
            spacing: self.spacing,
            margins: self.margins,
            light,
            night: self.night,
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

    /// The screensaver delay as the settings screen offers it.
    pub fn sleep(&self) -> Option<u32> {
        (self.sleep_minutes > 0).then_some(self.sleep_minutes)
    }

    pub fn set_sleep(&mut self, minutes: Option<u32>) {
        self.sleep_minutes = minutes.unwrap_or(0);
    }

    /// The library place to open at start.
    pub fn view(&self) -> View {
        View::ALL[self.library_view.min(View::ALL.len() - 1)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aristotle_ui::screens::settings::SLEEP_MINUTES;

    /// Whether `minutes` is one of the delays the settings screen offers.
    fn offered_sleep(minutes: Option<u32>) -> bool {
        SLEEP_MINUTES.contains(&minutes)
    }

    #[test]
    fn defaults_are_the_renderer_defaults() {
        let s = Prefs::default().style();
        let d = Style::pw4();
        assert_eq!(s.font_size, d.font_size);
        assert_eq!(s.line_height, d.line_height);
        assert_eq!(s.margin_mm, d.margin_mm);
        let p = Prefs::default();
        assert_eq!(p.turn_mode, 9);
        assert!(!p.night);
        assert_eq!(p.covers, Covers::None, "covers are off by default");
        assert!(offered_sleep(p.sleep()));
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
            night: true,
            night_method: NightMethod::Hardware,
            covers: Covers::Cards,
            sleep_minutes: 30,
            library_view: 1,
            sort: 2,
        };
        p.save(&dir).unwrap();
        assert_eq!(Prefs::load(&dir), p);
        std::fs::write(dir.join(Prefs::FILE), br#"{"size": 99, "future": 1}"#).unwrap();
        let q = Prefs::load(&dir);
        assert_eq!(q.size, FONT_PT.len() - 1);
        assert_eq!(q.turn_mode, STOCK_TURN);
        assert!(!q.night);
    }

    #[test]
    fn settings_persist_as_readable_json() {
        let dir = crate::testdir("prefs-json");
        let mut p = Prefs {
            night: true,
            covers: Covers::Photos,
            ..Prefs::default()
        };
        p.set_sleep(None);
        p.save(&dir).unwrap();
        let text = std::fs::read_to_string(dir.join(Prefs::FILE)).unwrap();
        assert!(text.contains(r#""night": true"#), "{text}");
        assert!(text.contains(r#""night_method": "software""#), "{text}");
        assert!(text.contains(r#""covers": "photos""#), "{text}");
        let q = Prefs::load(&dir);
        assert_eq!(q.sleep(), None, "never is kept");
        assert_eq!(q.covers.mode(), CoverMode::Photos);
        // An older file without the new fields keeps its typography and
        // gets the defaults for the rest.
        std::fs::write(
            dir.join(Prefs::FILE),
            br#"{"size": 3, "spacing": 2, "margins": 0, "turn_mode": 7}"#,
        )
        .unwrap();
        let old = Prefs::load(&dir);
        assert_eq!((old.size, old.turn_mode), (3, 7));
        assert_eq!(old.sleep_minutes, Prefs::default().sleep_minutes);
        assert_eq!(old.night_method, NightMethod::Software);
        // Out-of-range library choices are clamped.
        std::fs::write(dir.join(Prefs::FILE), br#"{"library_view": 9, "sort": 9}"#).unwrap();
        let q = Prefs::load(&dir);
        assert_eq!(q.view(), View::Series);
        assert_eq!(q.sort, crate::shelf::SORTS.len() - 1);
    }

    #[test]
    fn cover_modes_round_trip() {
        for m in CoverMode::ALL {
            assert_eq!(Covers::from_mode(m).mode(), m);
        }
    }
}
