//! Reading speed, for "minutes left in the chapter".
//!
//! Every forward page turn after a plausible time on the page (5 s to 10
//! min; shorter is skimming or a double tap, longer is a break) feeds an
//! exponential average of seconds per page. The average is kept in
//! `reading.json` in the library data directory, so the estimate is right
//! from the first page of the next session.

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Before any measurement: about 250 words a page (Literata 10 pt) at 200
/// words a minute.
pub const DEFAULT_SECS_PER_PAGE: f32 = 75.0;
const MIN_SECS: f32 = 5.0;
const MAX_SECS: f32 = 600.0;
/// Weight of the newest page in the average.
const ALPHA: f32 = 0.15;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stored {
    pub secs_per_page: f32,
    pub samples: u32,
}

impl Default for Stored {
    fn default() -> Self {
        Stored {
            secs_per_page: DEFAULT_SECS_PER_PAGE,
            samples: 0,
        }
    }
}

pub struct Speed {
    pub stored: Stored,
    file: Option<PathBuf>,
    page_since: Option<Instant>,
    dirty: bool,
}

impl Speed {
    pub const FILE: &'static str = "reading.json";

    pub fn load(dir: &Path) -> Speed {
        let file = dir.join(Self::FILE);
        let stored = std::fs::read(&file)
            .ok()
            .and_then(|d| serde_json::from_slice::<Stored>(&d).ok())
            .filter(|s| s.secs_per_page.is_finite() && s.secs_per_page > 0.0)
            .unwrap_or_default();
        Speed {
            stored,
            file: Some(file),
            page_since: None,
            dirty: false,
        }
    }

    #[cfg(test)]
    pub fn in_memory() -> Speed {
        Speed {
            stored: Stored::default(),
            file: None,
            page_since: None,
            dirty: false,
        }
    }

    pub fn secs_per_page(&self) -> f32 {
        self.stored.secs_per_page
    }

    /// A page appeared (opening a book, jumping): start timing it.
    pub fn page_shown(&mut self, now: Instant) {
        self.page_since = Some(now);
    }

    /// A forward turn: the page just left counts if its time is plausible.
    pub fn turned_forward(&mut self, now: Instant) {
        if let Some(t) = self.page_since {
            self.add(now.saturating_duration_since(t).as_secs_f32());
        }
        self.page_since = Some(now);
    }

    fn add(&mut self, secs: f32) {
        if !(MIN_SECS..=MAX_SECS).contains(&secs) {
            return;
        }
        let s = &mut self.stored;
        // The first samples count more, so a reader faster or slower than
        // the default sees a sensible estimate within a few pages.
        let alpha = (1.0 / (s.samples as f32 + 1.0)).max(ALPHA);
        s.secs_per_page += alpha * (secs - s.secs_per_page);
        s.samples = s.samples.saturating_add(1);
        self.dirty = true;
    }

    /// Write the average if it changed (with the index, not on every turn).
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        if let (Some(f), Ok(data)) = (&self.file, serde_json::to_vec_pretty(&self.stored)) {
            if aristotle_library::index::write_atomic(f, &data).is_ok() {
                self.dirty = false;
            }
        }
    }
}

/// The progress line in the footer: "Ch. 3/12 · 7 min" (minutes left in
/// the chapter).
pub fn progress_line(
    chapter: usize,
    chapters: usize,
    pages_left: f32,
    secs_per_page: f32,
) -> String {
    use aristotle_i18n::{tf, tr, Msg};
    let minutes = (pages_left.max(0.0) * secs_per_page / 60.0).round() as u32;
    let time = if minutes == 0 {
        tr(Msg::ReaderProgressUnderAMinute).to_string()
    } else if minutes < 60 {
        tf!(Msg::FormatMinutes, m = minutes)
    } else {
        tf!(
            Msg::ReaderProgressHoursMinutes,
            h = minutes / 60,
            m = minutes % 60
        )
    };
    tf!(
        Msg::ReaderProgress,
        chapter = chapter,
        chapters = chapters,
        time = time
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn average_follows_the_reader_and_ignores_breaks() {
        let mut s = Speed::in_memory();
        s.add(40.0);
        assert_eq!(
            s.secs_per_page(),
            40.0,
            "the first sample replaces the default"
        );
        for _ in 0..20 {
            s.add(60.0);
        }
        assert!((s.secs_per_page() - 60.0).abs() < 1.0);
        s.add(2.0); // a double tap
        s.add(3600.0); // a break
        assert!((s.secs_per_page() - 60.0).abs() < 1.0);
    }

    #[test]
    fn progress_lines() {
        aristotle_i18n::with_language("pl", || {
            assert_eq!(progress_line(3, 12, 7.0, 60.0), "Rozdz. 3/12 · 7 min");
            assert_eq!(progress_line(1, 4, 0.2, 60.0), "Rozdz. 1/4 · <1 min");
            assert_eq!(progress_line(2, 2, 90.0, 60.0), "Rozdz. 2/2 · 1 h 30 min");
        });
        aristotle_i18n::with_language("en", || {
            assert_eq!(progress_line(3, 12, 7.0, 60.0), "Ch. 3/12 · 7 min");
        });
    }
}
