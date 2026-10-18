// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reading progress: between the library index's locators and the
//! renderer's positions, and onto disk without stalling page turns.
//!
//! - [`positions`] lists where to open a book, most exact first: the
//!   stored locator, then the stored fraction. Stock-imported progress
//!   (`kindle` locators of single-part MOBI 7 books) maps through the converter.
//! - [`progress_at`] is what a page turn writes back.
//! - [`Debounce`] decides when the index goes to disk: after a pause in
//!   reading, or at the latest every [`Debounce::MAX_DELAY`] while turning
//!   steadily. The index lives on the FAT userstore, where an fsync can take
//!   tens of milliseconds.
//! - [`Saver`] writes on a thread of its own, so even that never lands
//!   between a tap and its page.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use aristotle_library::index::{write_atomic, INDEX_FILE};
use aristotle_library::{Library, Locator, Progress};
use aristotle_render::book::{Book, Position};

/// Whether imported Kindle offsets share the active converter's address space.
pub fn stock_positions_compatible(book: &Book) -> bool {
    book.mobi()
        .is_some_and(|m| m.info.version < 8 && m.info.kf8_boundary.is_none())
}

/// Where to open a book with this progress, most exact first.
pub fn positions(progress: Option<&Progress>, stock_position_compatible: bool) -> Vec<Position> {
    let Some(p) = progress else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match &p.locator {
        Locator::Epub { spine, offset } => out.push(Position::Spine {
            item: *spine as usize,
            offset: *offset as usize,
        }),
        Locator::Kindle { position }
            if stock_position_compatible || p.source.as_deref() == Some("aristotle") =>
        {
            out.push(Position::Kindle(*position));
        }
        Locator::Kindle { .. } => {}
        Locator::Page { index } => out.push(Position::Page(*index as usize)),
        // Plain text offsets and KOReader XPointers mean nothing to Plato's
        // engine; the fraction gets close.
        Locator::Text { .. } | Locator::XPointer { .. } => {}
    }
    out.push(Position::Fraction(p.fraction));
    out
}

/// The locator for a renderer position. A bare fraction (a book whose
/// spine could not be read) is stored as a page locator, which Plato's
/// engine does not place, so the next open falls back to the fraction.
pub fn locator(position: &Position) -> Locator {
    match *position {
        Position::Spine { item, offset } => Locator::Epub {
            spine: item as u32,
            offset: offset as u32,
        },
        Position::Kindle(position) => Locator::Kindle { position },
        Position::Page(index) => Locator::Page {
            index: index as u32,
        },
        Position::Fraction(_) => Locator::Page { index: 0 },
    }
}

/// The progress record for a page at `position`, `fraction` of the way in.
pub fn progress_at(position: &Position, fraction: f32, now: i64) -> Progress {
    progress_for(locator(position), fraction, now)
}

/// [`progress_at`] for a locator already made (a snapshot's page).
pub fn progress_for(locator: Locator, fraction: f32, now: i64) -> Progress {
    Progress {
        fraction: fraction.clamp(0.0, 1.0),
        locator,
        updated: now,
        source: Some("aristotle".into()),
        page: None,
    }
}

/// When to write the index after progress changed.
#[derive(Debug, Default)]
pub struct Debounce {
    first: Option<Instant>,
    last: Option<Instant>,
}

impl Debounce {
    /// Write once reading has paused this long...
    pub const QUIET: Duration = Duration::from_secs(3);
    /// ...or at the latest this long after the first unsaved change.
    pub const MAX_DELAY: Duration = Duration::from_secs(20);

    pub fn changed(&mut self, now: Instant) {
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    /// When [`Debounce::due`] will next say yes, for the poll timeout.
    pub fn deadline(&self) -> Option<Instant> {
        let (first, last) = (self.first?, self.last?);
        Some((last + Self::QUIET).min(first + Self::MAX_DELAY))
    }

    pub fn due(&self, now: Instant) -> bool {
        self.deadline().is_some_and(|d| now >= d)
    }

    pub fn saved(&mut self) {
        self.first = None;
        self.last = None;
    }
}

/// Writes the library index on a background thread. Snapshots are
/// serialised on the caller's thread (well under a millisecond for a few
/// dozen books) and only the newest pending one is written.
pub struct Saver {
    tx: Option<Sender<Vec<u8>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Saver {
    pub fn new(data_dir: &Path) -> Saver {
        let path: PathBuf = data_dir.join(INDEX_FILE);
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let thread = thread::Builder::new()
            .name("index-saver".into())
            .spawn(move || {
                while let Ok(mut data) = rx.recv() {
                    // Latest wins: skip snapshots superseded meanwhile.
                    while let Ok(newer) = rx.try_recv() {
                        data = newer;
                    }
                    let t = Instant::now();
                    match write_atomic(&path, &data) {
                        Ok(()) => crate::perf!(
                            "index: saved {} KB in {:.0} ms",
                            data.len() / 1024,
                            crate::ms(t.elapsed())
                        ),
                        Err(e) => eprintln!("aristotle: can't save {}: {e}", path.display()),
                    }
                }
            })
            .ok();
        Saver {
            tx: Some(tx),
            thread,
        }
    }

    /// Queue the current index for writing.
    pub fn save(&self, lib: &Library) {
        match serde_json::to_vec(lib.index()) {
            Ok(data) => {
                if let Some(tx) = &self.tx {
                    let _ = tx.send(data);
                }
            }
            Err(e) => eprintln!("aristotle: can't serialise the index: {e}"),
        }
    }

    /// Wait for queued writes, e.g. before quitting.
    pub fn flush(mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod mobi_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locators_round_trip_through_positions() {
        let cases = [
            Locator::Epub {
                spine: 3,
                offset: 1200,
            },
            Locator::Kindle { position: 7510 },
            Locator::Page { index: 41 },
        ];
        for loc in cases {
            let p = Progress {
                fraction: 0.25,
                locator: loc.clone(),
                updated: 1,
                source: Some("kindle".into()),
                page: Some("4".into()),
            };
            let pos = positions(Some(&p), true);
            assert_eq!(pos.len(), 2);
            assert_eq!(pos[1], Position::Fraction(0.25));
            assert_eq!(locator(&pos[0]), loc);
            let back = progress_at(&pos[0], 0.3, 99);
            assert_eq!(back.locator, loc);
            assert_eq!(back.source.as_deref(), Some("aristotle"));
        }
    }

    #[test]
    fn foreign_locators_fall_back_to_the_fraction() {
        let p = Progress {
            fraction: 0.6,
            locator: Locator::XPointer {
                xpointer: "/body/DocFragment[3]".into(),
            },
            updated: 1,
            source: Some("koreader".into()),
            page: None,
        };
        assert_eq!(positions(Some(&p), false), vec![Position::Fraction(0.6)]);
        assert!(positions(None, false).is_empty());
    }

    #[test]
    fn debounce_waits_for_a_pause_but_not_forever() {
        let t0 = Instant::now();
        let mut d = Debounce::default();
        assert!(d.deadline().is_none() && !d.due(t0));
        d.changed(t0);
        assert!(!d.due(t0 + Duration::from_secs(2)));
        assert!(d.due(t0 + Debounce::QUIET));
        // Steady turning every second keeps postponing, up to MAX_DELAY.
        let mut t = t0;
        for _ in 0..30 {
            t += Duration::from_secs(1);
            d.changed(t);
            if d.due(t) {
                break;
            }
        }
        assert!(d.due(t));
        assert!(
            t - t0 >= Debounce::MAX_DELAY && t - t0 < Debounce::MAX_DELAY + Duration::from_secs(2)
        );
        d.saved();
        assert!(d.deadline().is_none());
    }

    #[test]
    fn saver_writes_the_latest_index() {
        let dir = crate::testdir("saver");
        let mut lib = Library::open(&dir).unwrap();
        let saver = Saver::new(&dir);
        saver.save(&lib);
        // The index is empty; a scan of an empty folder still sets the root.
        let root = dir.join("docs");
        std::fs::create_dir_all(&root).unwrap();
        lib.scan(&root, &Default::default()).unwrap();
        saver.save(&lib);
        saver.flush();
        let again = Library::open(&dir).unwrap();
        assert!(again.root().is_some());
    }
}
