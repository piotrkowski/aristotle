// SPDX-License-Identifier: AGPL-3.0-or-later
//! Snapshots made ahead of time, so almost every open is a snapshot open.
//!
//! A book gets a [snapshot](crate::snapshot) when it is left, but not one
//! that was never opened here, and
//! a font size change makes every snapshot stale. So while the library sits
//! on screen untouched, one thread at a time opens the books most likely to
//! be opened next, renders the saved page and the one after it, and stores
//! them as their snapshot:
//!
//! - **Which books:** the book to continue and the next few by recency
//!   ([`CANDIDATES`] in all), each with a saved position and without a
//!   valid snapshot for the current style. Each is tried once per run,
//!   position, style and UI language, so the work is bounded and then stops.
//!   Imported Kindle positions wait until a real open saves an owned position.
//! - **When:** after [`QUIET`] without input, with the library on screen and
//!   no book opening. Any touch cancels the job; the thread runs at
//!   `SCHED_IDLE`, so on the single core of the PW4 it only gets the CPU
//!   while the app waits for input and never delays a tap. The layout of a
//!   chapter cannot be interrupted, so a cancelled job finishes it at idle
//!   priority and then throws everything away.
//! - **No page numbers:** the job does not paginate whole books (the page
//!   numbers come with the first real open), so without page numbers on
//!   disk the snapshot shows the page without "x / y", as the first real
//!   open renders it. The footer's minutes use the reader's measured speed,
//!   as a real open does.
//!
//! Nothing here keeps the device awake: when powerd suspends it, the thread
//! is frozen with the rest of the process.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use aristotle_library::{BookRecord, Library};
use aristotle_render::plato_core::anyhow::{format_err, Error};
use aristotle_render::style::Style;

use crate::session::{Session, Source, OPEN_GATE};
use crate::snapshot::{self, Key, Snapshot};

/// How many books, by recency, get a snapshot ahead of time.
pub const CANDIDATES: usize = 5;

/// How long the library must be left alone before a job starts.
pub const QUIET: Duration = Duration::from_secs(2);

pub struct Prewarm {
    job: Option<Job>,
    /// Books tried in this run, by id, style, UI language and position.
    tried: HashSet<String>,
    quiet_since: Instant,
    /// Set when no candidate was left; cleared by input.
    exhausted: bool,
}

struct Job {
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl Default for Prewarm {
    fn default() -> Self {
        Prewarm {
            job: None,
            tried: HashSet::new(),
            quiet_since: Instant::now(),
            exhausted: false,
        }
    }
}

impl Prewarm {
    /// Any input: cancel the job in progress and wait for quiet again.
    pub fn input(&mut self) {
        self.quiet_since = Instant::now();
        self.exhausted = false;
        if let Some(j) = &self.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// When [`Prewarm::tick`] next has something to do, for the poll timeout.
    pub fn deadline(&self) -> Option<Instant> {
        if self.job.is_some() {
            Some(Instant::now() + Duration::from_millis(200))
        } else if self.exhausted {
            None
        } else {
            Some(self.quiet_since + QUIET)
        }
    }

    /// Call between events while the library is on screen and no book is
    /// opening: reaps a finished job and starts the next one when quiet.
    /// `secs_per_page` is the reading speed the progress line uses, as for
    /// a real open, or the footers would differ.
    pub fn tick(&mut self, lib: &Library, style: &Style, mobi_cache: &Path, secs_per_page: f32) {
        if let Some(j) = &self.job {
            if !j.handle.is_finished() {
                return;
            }
            if let Some(j) = self.job.take() {
                let _ = j.handle.join();
            }
        }
        if self.exhausted || self.quiet_since.elapsed() < QUIET {
            return;
        }
        for rec in candidates(lib) {
            let Some((src, key)) = job_for(lib, rec, style, mobi_cache, secs_per_page) else {
                continue;
            };
            if !self.tried.insert(format!(
                "{} {} {} {:?}",
                rec.id, key.look, key.language, key.locator
            )) {
                continue;
            }
            if snapshot::is_valid(lib.data_dir(), &key, style) {
                continue;
            }
            self.job = start(src, style.clone(), key);
            return;
        }
        self.exhausted = true;
    }
}

/// The source (without background pagination) and the key of a snapshot
/// made ahead of time for `rec`, with the reading speed `secs_per_page`
/// (the footer's minutes left); `None` without snapshot-compatible progress.
pub fn job_for(
    lib: &Library,
    rec: &BookRecord,
    style: &Style,
    mobi_cache: &Path,
    secs_per_page: f32,
) -> Option<(Source, Key)> {
    let progress = rec.progress.as_ref()?;
    if !snapshot::compatible_progress(progress) {
        return None;
    }
    let mut src = Source::new(lib, rec, mobi_cache).ok()?;
    src.secs_per_page = secs_per_page;
    let key = Key::new(&rec.id, src.stamp, style, progress.locator.clone());
    src.paginate = false;
    Some((src, key))
}

/// The books to prepare, most likely first: the book to continue, then by
/// when they were last opened here or their progress last changed.
pub fn candidates(lib: &Library) -> Vec<&BookRecord> {
    let mut books: Vec<&BookRecord> = lib
        .books()
        .iter()
        .filter(|b| {
            b.progress
                .as_ref()
                .is_some_and(snapshot::compatible_progress)
        })
        .collect();
    books.sort_by_key(|b| {
        std::cmp::Reverse((b.last_opened, b.progress.as_ref().map(|p| p.updated)))
    });
    books.truncate(CANDIDATES);
    books
}

fn start(mut src: Source, style: Style, key: Key) -> Option<Job> {
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    src.cancel = Some(cancel.clone());
    let spawned = thread::Builder::new()
        .name("prewarm".into())
        .spawn(move || {
            crate::pagecache::lower_priority();
            let _gate = OPEN_GATE.lock().unwrap_or_else(|e| e.into_inner());
            let title = src.title.clone();
            let t = Instant::now();
            match make(src, style, key, &stop) {
                Ok(Some(n)) => crate::perf!(
                    "prewarm: snapshot of \"{title}\" made in {:.0} ms ({} KB, idle priority)",
                    crate::ms(t.elapsed()),
                    n / 1024
                ),
                Ok(None) => crate::perf!(
                    "prewarm: \"{title}\" cancelled after {:.0} ms",
                    crate::ms(t.elapsed())
                ),
                Err(e) => eprintln!("aristotle: prewarm: {title}: {e:#}"),
            }
        });
    match spawned {
        Ok(handle) => Some(Job { cancel, handle }),
        Err(e) => {
            eprintln!("aristotle: can't start the prewarm thread: {e}");
            None
        }
    }
}

/// Open the book, render its saved page and the next, and store them.
/// `None` when cancelled.
pub fn make(
    src: Source,
    style: Style,
    key: Key,
    stop: &AtomicBool,
) -> Result<Option<usize>, Error> {
    let data_dir: PathBuf = src.data_dir.clone();
    if stop.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let prepared = match Session::prepare(src, style, true) {
        Err(_) if stop.load(Ordering::Relaxed) => return Ok(None),
        other => other?,
    };
    if stop.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let mut session = Session::from_prepared(prepared)?;
    let here = session.current;
    let current = session
        .rendered(here)
        .ok_or_else(|| format_err!("the page was not rendered"))?;
    let mut info = crate::open::page_info(&mut session, here);
    // The snapshot belongs to the position the index holds, whatever
    // locator this page would be saved under.
    info.locator = key.locator.clone();
    let mut pages = vec![(info, (*current).clone())];
    if let Some(next) = session.known_next(here) {
        if let Some(p) = session.rendered(next) {
            pages.push((crate::open::page_info(&mut session, next), (*p).clone()));
        }
    }
    if stop.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let n = snapshot::store(&data_dir, &Snapshot { key, pages })?;
    Ok(Some(n))
}
