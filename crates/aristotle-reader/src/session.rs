//! One open book: where the reader is, the pages rendered around it, and
//! the page numbers once known.
//!
//! Positions are Plato locations (page starts), never page indices, so a
//! book opens at its saved place without laying out everything before it.
//! The pages next to the current one are rendered ahead ("prerender") while
//! the panel shows the current one, so a turn only costs the panel's time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aristotle_library::index::AnnotationKind;
use aristotle_library::{BookRecord, Library, Progress};
use aristotle_render::book::{Book, Engine, Link, Position, Word};
use aristotle_render::page::{Chrome, Page};
use aristotle_render::plato_core::anyhow::{format_err, Context, Error};
use aristotle_render::style::Style;

use crate::pagecache::{self, Paginator};
use crate::progress;
use crate::turns::Turn;

/// Rendered pages kept around: the current one and its two neighbours,
/// plus one spare so a back-and-forth does not render twice.
const KEEP_PAGES: usize = 4;

pub struct Session {
    pub id: String,
    pub title: String,
    path: PathBuf,
    stamp: (u64, i64),
    data_dir: PathBuf,
    mobi_cache: PathBuf,
    pub style: Style,
    book: Book,
    chrome: Chrome,
    /// Start of the page on the panel.
    pub current: usize,
    pages: Vec<(usize, Rc<Page>)>,
    next_of: HashMap<usize, Option<usize>>,
    prev_of: HashMap<usize, Option<usize>>,
    paginator: Option<Paginator>,
    /// Highlights, bookmarks and reading speed drawn on pages.
    deco: Deco,
    /// Words and links of the last page asked for.
    words: Option<(usize, Rc<Vec<Word>>)>,
    links: Option<(usize, Rc<Vec<Link>>)>,
}

/// What the timing log reports about opening a book.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenTimes {
    pub open: Duration,
    pub place: Duration,
    pub render: Duration,
    /// Rendering the pages on either side, when opened with neighbours.
    pub neighbours: Duration,
    pub pages_cached: bool,
}

/// Where a book is and where it was left: everything [`Session::prepare`]
/// needs from the library, owned, so it can go to another thread.
#[derive(Clone)]
pub struct Source {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    pub stamp: (u64, i64),
    pub data_dir: PathBuf,
    pub mobi_cache: PathBuf,
    pub saved: Option<Progress>,
    /// Highlights and bookmarks made here, as saved positions; resolved
    /// once the book is open, so the first page already shows them.
    pub marks: Vec<(Position, Position)>,
    pub bookmarks: Vec<Position>,
    pub secs_per_page: f32,
    /// A page start to open at instead of placing `saved`: the location of
    /// a snapshot made for exactly this progress, so the book opens on the
    /// page the snapshot shows, whatever rounding the locator went through.
    pub exact: Option<usize>,
    /// Whether to paginate the whole book in the background when its page
    /// numbers are not cached (not for snapshots made ahead of time).
    pub paginate: bool,
    /// Set by whoever stops waiting for this open; [`Session::prepare`]
    /// gives up between its steps.
    pub cancel: Option<Arc<AtomicBool>>,
}

/// Held by every thread that opens a book in the background (an open
/// worker, a snapshot made ahead of time) from the start of
/// [`Session::prepare`] until its result is handed over or dropped, so an
/// abandoned open never holds a laid-out book next to a new one.
pub static OPEN_GATE: Mutex<()> = Mutex::new(());

impl Source {
    pub fn new(lib: &Library, rec: &BookRecord, mobi_cache: &Path) -> Result<Source, Error> {
        let path = lib
            .absolute_path(rec)
            .ok_or_else(|| format_err!("the library has no root"))?;
        // The file itself, in case it changed since the last scan.
        let stamp = crate::snapshot::file_stamp(&path).unwrap_or((rec.file.size, rec.file.mtime));
        Ok(Source {
            id: rec.id.clone(),
            title: rec.meta.title.clone(),
            path,
            stamp,
            data_dir: lib.data_dir().to_path_buf(),
            mobi_cache: mobi_cache.to_path_buf(),
            saved: rec.progress.clone(),
            marks: crate::marks::ours(rec, AnnotationKind::Highlight)
                .into_iter()
                .map(|(_, s, e)| (s, e))
                .collect(),
            bookmarks: crate::marks::ours(rec, AnnotationKind::Bookmark)
                .into_iter()
                .map(|(_, s, _)| s)
                .collect(),
            secs_per_page: crate::speed::DEFAULT_SECS_PER_PAGE,
            exact: None,
            paginate: true,
            cancel: None,
        })
    }
}

/// A book opened, placed and rendered: the slow part of opening, done by
/// [`Session::prepare`] on any thread. Unlike a [`Session`] it is `Send`
/// (no `Rc` pages, no FreeType chrome fonts), so a worker thread can make
/// it while the main loop shows a snapshot and keeps taking input.
pub struct Prepared {
    src: Source,
    style: Style,
    book: Book,
    pub current: usize,
    pages: Vec<(usize, Page)>,
    next_of: HashMap<usize, Option<usize>>,
    prev_of: HashMap<usize, Option<usize>>,
    paginator: Option<Paginator>,
    deco: Deco,
    pub times: OpenTimes,
}

impl Prepared {
    /// The page start after the current one, if the worker found it.
    pub fn next(&self) -> Option<usize> {
        self.next_of.get(&self.current).copied().flatten()
    }
}

impl Session {
    /// Open the book `rec` at its saved position with `style`.
    pub fn open(
        lib: &Library,
        rec: &BookRecord,
        style: Style,
        mobi_cache: &Path,
    ) -> Result<(Session, OpenTimes), Error> {
        let prepared = Session::prepare(Source::new(lib, rec, mobi_cache)?, style, false)?;
        let times = prepared.times;
        Ok((Session::from_prepared(prepared)?, times))
    }

    /// Open, place at the saved position and render the first page (and,
    /// with `neighbours`, the pages on either side), without touching
    /// anything the main thread owns.
    pub fn prepare(src: Source, style: Style, neighbours: bool) -> Result<Prepared, Error> {
        let cancelled = || {
            if src
                .cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed))
            {
                Err(format_err!("cancelled"))
            } else {
                Ok(())
            }
        };
        cancelled()?;
        let t = Instant::now();
        let mut book = Book::open(&src.path, &style, Some(&src.mobi_cache))
            .with_context(|| format!("can't open {}", src.path.display()))?;
        // The chrome fonts of this thread: FreeType faces can't move to
        // another one, so the session loads its own.
        let mut chrome = Chrome::load(&style).context("can't load the chrome fonts")?;
        let t_open = t.elapsed();
        cancelled()?;
        let (pages_cached, paginator) = start_pagination(
            &mut book,
            &style,
            &src.data_dir,
            &src.id,
            src.stamp,
            &src.path,
            src.paginate.then_some(src.mobi_cache.as_path()),
        );
        let t = Instant::now();
        let target = src.exact.unwrap_or_else(|| {
            progress::positions(src.saved.as_ref())
                .iter()
                .find_map(|p| book.location_of(p))
                .unwrap_or(0)
        });
        let current = book
            .page_start(target)
            .ok_or_else(|| format_err!("the book has no pages"))?;
        let t_place = t.elapsed();
        cancelled()?;
        let t = Instant::now();
        let deco = Deco {
            marks: src
                .marks
                .iter()
                .filter_map(|(s, e)| Some((book.location_of(s)?, book.location_of(e)?)))
                .collect(),
            bookmarks: src
                .bookmarks
                .iter()
                .filter_map(|p| book.location_of(p))
                .collect(),
            secs_per_page: src.secs_per_page,
        };
        let first_next = if neighbours {
            book.next_page(current)
        } else {
            None
        };
        let mut pages = vec![(
            current,
            render_page(&mut book, &mut chrome, &deco, current, first_next)?,
        )];
        let t_render = t.elapsed();
        cancelled()?;
        let t = Instant::now();
        let (mut next_of, mut prev_of) = (HashMap::new(), HashMap::new());
        if neighbours {
            let next = book.next_page(current);
            let prev = book.previous_page(current);
            next_of.insert(current, next);
            prev_of.insert(current, prev);
            for loc in [next, prev].into_iter().flatten() {
                let after = if Some(loc) == next {
                    None
                } else {
                    Some(current)
                };
                match render_page(&mut book, &mut chrome, &deco, loc, after) {
                    Ok(page) => pages.push((loc, page)),
                    Err(e) => eprintln!("aristotle: prerender: {e:#}"),
                }
            }
        }
        let t_neighbours = t.elapsed();
        Ok(Prepared {
            src,
            style,
            book,
            current,
            pages,
            next_of,
            prev_of,
            paginator,
            deco,
            times: OpenTimes {
                open: t_open,
                place: t_place,
                render: t_render,
                neighbours: t_neighbours,
                pages_cached,
            },
        })
    }

    /// The session for a prepared book, on the thread that will use it.
    pub fn from_prepared(p: Prepared) -> Result<Session, Error> {
        let chrome = Chrome::load(&p.style).context("can't load the chrome fonts")?;
        Ok(Session {
            id: p.src.id,
            title: p.src.title,
            path: p.src.path,
            stamp: p.src.stamp,
            data_dir: p.src.data_dir,
            mobi_cache: p.src.mobi_cache,
            style: p.style,
            book: p.book,
            chrome,
            current: p.current,
            pages: p
                .pages
                .into_iter()
                .map(|(l, pg)| (l, Rc::new(pg)))
                .collect(),
            next_of: p.next_of,
            prev_of: p.prev_of,
            paginator: p.paginator,
            deco: p.deco,
            words: None,
            links: None,
        })
    }

    /// Use the stored pagination for this book and style, or start one in
    /// the background. Returns true on a cache hit.
    fn load_pagination(&mut self) -> bool {
        let (cached, paginator) = start_pagination(
            &mut self.book,
            &self.style,
            &self.data_dir,
            &self.id,
            self.stamp,
            &self.path,
            Some(&self.mobi_cache),
        );
        self.paginator = paginator;
        cached
    }

    /// Adopt a finished background pagination. Returns true if the page
    /// numbers just became known (rendered pages are then stale: their
    /// footer lacks "x / y").
    pub fn poll_pagination(&mut self) -> bool {
        let Some(result) = self.paginator.as_ref().and_then(Paginator::poll) else {
            return false;
        };
        self.paginator = None;
        match result {
            Some(pages) => {
                self.book.set_pagination(pages);
                self.pages.clear();
                true
            }
            None => false,
        }
    }

    /// Whether laying the book out again at the current page (a new font
    /// size, spacing or margins) will take long enough to show the busy
    /// mark: about 300 ms on the device. Plato lays out the current
    /// chapter from its start up to the page, at about 6 ms per KB of
    /// chapter text on the PW4 ("Pan Tadeusz": 420 KB in 2.5 s).
    pub fn slow_relayout(&self) -> bool {
        const SLOW_BYTES: usize = 48 * 1024;
        self.book.layout_bytes(self.current) > SLOW_BYTES
    }

    /// The book file's size and mtime when the library last saw it.
    pub fn stamp(&self) -> (u64, i64) {
        self.stamp
    }

    pub fn paginating(&self) -> bool {
        self.paginator.is_some()
    }

    /// The rendered page starting at `location`.
    pub fn page(&mut self, location: usize) -> Result<Rc<Page>, Error> {
        if let Some(i) = self.pages.iter().position(|(l, _)| *l == location) {
            let entry = self.pages.remove(i);
            let page = entry.1.clone();
            self.pages.push(entry);
            return Ok(page);
        }
        let next = self.neighbor(location, Turn::Forward);
        let page = Rc::new(render_page(
            &mut self.book,
            &mut self.chrome,
            &self.deco,
            location,
            next,
        )?);
        if self.pages.len() >= KEEP_PAGES {
            self.pages.remove(0);
        }
        self.pages.push((location, page.clone()));
        Ok(page)
    }

    pub fn is_rendered(&self, location: usize) -> bool {
        self.pages.iter().any(|(l, _)| *l == location)
    }

    /// The page at `location` if it is already rendered (never renders).
    pub fn rendered(&self, location: usize) -> Option<Rc<Page>> {
        self.pages
            .iter()
            .find(|(l, _)| *l == location)
            .map(|(_, p)| p.clone())
    }

    /// The page start after `location` if already known (never lays out).
    pub fn known_next(&self, location: usize) -> Option<usize> {
        self.next_of.get(&location).copied().flatten()
    }

    /// Whether `location` is known to be the last page (never lays out).
    pub fn known_last(&self, location: usize) -> bool {
        self.next_of.get(&location) == Some(&None)
    }

    /// The page start one page away from `location`.
    pub fn neighbor(&mut self, location: usize, dir: Turn) -> Option<usize> {
        let (map, book) = match dir {
            Turn::Forward => (&mut self.next_of, &mut self.book),
            Turn::Back => (&mut self.prev_of, &mut self.book),
        };
        *map.entry(location).or_insert_with(|| match dir {
            Turn::Forward => book.next_page(location),
            Turn::Back => book.previous_page(location),
        })
    }

    /// Render the pages on either side of the current one if they are not
    /// ready yet. Returns the time spent on each.
    pub fn prerender(&mut self) -> (Duration, Duration) {
        let mut spent = [Duration::ZERO; 2];
        for (i, dir) in [Turn::Forward, Turn::Back].into_iter().enumerate() {
            let t = Instant::now();
            if let Some(loc) = self.neighbor(self.current, dir) {
                if !self.is_rendered(loc) {
                    if let Err(e) = self.page(loc) {
                        eprintln!("aristotle: prerender: {e:#}");
                    }
                }
            }
            spent[i] = t.elapsed();
        }
        (spent[0], spent[1])
    }

    /// Whether `location` is the last page.
    pub fn at_end(&mut self) -> bool {
        self.neighbor(self.current, Turn::Forward).is_none()
    }

    /// Start of the next or previous chapter.
    pub fn chapter(&mut self, forward: bool) -> Option<usize> {
        self.book.chapter_page(self.current, forward)
    }

    /// Lay the book out again with `style`, keeping the reading position.
    pub fn restyle(&mut self, style: Style) -> Result<(), Error> {
        let here = self.book.position_at(self.current);
        let book = Book::open(&self.path, &style, Some(&self.mobi_cache))?;
        self.chrome = Chrome::load(&style)?;
        self.book = book;
        self.style = style;
        self.pages.clear();
        self.next_of.clear();
        self.prev_of.clear();
        self.paginator = None;
        self.words = None;
        self.links = None;
        self.load_pagination();
        let loc = self.book.location_of(&here).unwrap_or(0);
        self.current = self
            .book
            .page_start(loc)
            .ok_or_else(|| format_err!("the book has no pages"))?;
        Ok(())
    }

    /// The progress record for the current page.
    pub fn progress(&self, now: i64) -> Progress {
        let pos = self.book.position_at(self.current);
        progress::progress_at(&pos, self.book.fraction(self.current), now)
    }

    /// Chapter title, "Strona x z y" (or a percentage before the pages are
    /// known) and the fraction, for the menu.
    pub fn labels(&mut self) -> (String, String, f32) {
        let chapter = self.book.chapter_title(self.current).unwrap_or_default();
        let fraction = self.book.fraction(self.current);
        let page = match (
            self.book.index_of(self.current),
            self.book.known_page_count(),
        ) {
            (Some(i), Some(n)) => aristotle_ui::screens::library::page_of(i + 1, n),
            _ => format!("{:.0}%", (fraction * 100.0).floor()),
        };
        (chapter, page, fraction)
    }

    // ------------------------------------------------ reading interactions

    pub fn book(&mut self) -> &mut Book {
        &mut self.book
    }

    /// The words of the page starting at `location` (cached for the last
    /// page asked).
    pub fn words(&mut self, location: usize) -> Rc<Vec<Word>> {
        match &self.words {
            Some((l, w)) if *l == location => w.clone(),
            _ => {
                let w = Rc::new(self.book.words(location));
                self.words = Some((location, w.clone()));
                w
            }
        }
    }

    pub fn links(&mut self, location: usize) -> Rc<Vec<Link>> {
        match &self.links {
            Some((l, w)) if *l == location => w.clone(),
            _ => {
                let w = Rc::new(self.book.links(location));
                self.links = Some((location, w.clone()));
                w
            }
        }
    }

    /// New highlights and bookmarks: rendered pages are redrawn on demand.
    pub fn set_marks(&mut self, marks: Vec<(usize, usize)>, bookmarks: Vec<usize>) {
        if marks != self.deco.marks || bookmarks != self.deco.bookmarks {
            self.deco.marks = marks;
            self.deco.bookmarks = bookmarks;
            self.pages.clear();
        }
    }

    pub fn set_speed(&mut self, secs_per_page: f32) {
        self.deco.secs_per_page = secs_per_page;
    }

    /// Whether the page starting at `location` holds a bookmark.
    pub fn bookmarked(&mut self, location: usize) -> Option<usize> {
        let next = self.neighbor(location, Turn::Forward);
        self.deco.bookmark_on(location, next)
    }
}

/// What is drawn on a page besides the book: the progress line, the
/// highlights and the bookmark ribbon.
#[derive(Clone, Debug, Default)]
pub struct Deco {
    /// Highlights as Plato location ranges (first word to last word).
    pub marks: Vec<(usize, usize)>,
    /// Bookmarks as Plato locations.
    pub bookmarks: Vec<usize>,
    pub secs_per_page: f32,
}

impl Deco {
    fn bookmark_on(&self, location: usize, next: Option<usize>) -> Option<usize> {
        let next = next.unwrap_or(usize::MAX);
        self.bookmarks
            .iter()
            .copied()
            .find(|&b| b >= location && b < next)
    }
}

/// Render the page at `location` with the decorations. `next` is the next
/// page's start if known (bookmarks and the minutes estimate use it).
fn render_page(
    book: &mut Book,
    chrome: &mut Chrome,
    deco: &Deco,
    location: usize,
    next: Option<usize>,
) -> Result<Page, Error> {
    let footer = progress_line(book, deco.secs_per_page, location, next);
    let mut page = book.render_location_with(location, chrome, footer)?;
    if !deco.marks.is_empty() {
        let words = book.words(location);
        let marked = |w: &Word| {
            deco.marks
                .iter()
                .any(|&(s, e)| w.offset >= s && w.offset <= e)
        };
        let mut i = 0;
        while i < words.len() {
            if !marked(&words[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i + 1 < words.len() && marked(&words[i + 1]) {
                i += 1;
            }
            for b in crate::selection::line_boxes(&words, start, i) {
                darken(&mut page, &crate::selection::marker_box(&b));
            }
            i += 1;
        }
    }
    if deco.bookmark_on(location, next).is_some() {
        ribbon(&mut page);
    }
    Ok(page)
}

/// "Rozdz. 3/12 · 7 min": the chapter and the minutes left in it at the
/// measured speed.
fn progress_line(
    book: &mut Book,
    secs_per_page: f32,
    location: usize,
    next: Option<usize>,
) -> Option<String> {
    let ch = book.chapter_info(location)?;
    let pages_left = match book.pagination() {
        Some(pages) => {
            let here = pages.partition_point(|&p| p <= location);
            let end = pages.partition_point(|&p| p < ch.end);
            end.saturating_sub(here.saturating_sub(1)) as f32
        }
        None => {
            // Bytes left over bytes per page, from this page's size.
            let per_page = next.map_or(2000, |n| n.saturating_sub(location)).max(400) as f32;
            (ch.end.saturating_sub(location)) as f32 / per_page
        }
    };
    Some(crate::speed::progress_line(
        ch.number,
        ch.count,
        pages_left,
        secs_per_page,
    ))
}

/// Multiply the pixels of a line of marked words by 0.75: a
/// light gray marker that keeps the text black.
fn darken(page: &mut Page, r: &aristotle_render::book::Bounds) {
    let (w, h) = (page.width as i32, page.height as i32);
    let x0 = r.x0.clamp(0, w);
    let x1 = r.x1.clamp(0, w);
    for y in r.y0.clamp(0, h)..r.y1.clamp(0, h) {
        let row = y as usize * page.stride;
        for p in &mut page.data[row + x0 as usize..row + x1 as usize] {
            *p = (*p as u16 * 3 / 4) as u8;
        }
    }
}

/// A black bookmark ribbon in the top right corner, with a notch.
fn ribbon(page: &mut Page) {
    let (x0, w, h) = (page.width as i32 - 96, 44, 84);
    for y in 0..h {
        let notch = (y - (h - 18)).max(0); // the V cut at the bottom
        for x in x0..x0 + w {
            let dx = (x - x0 - w / 2).abs();
            if dx < notch {
                continue;
            }
            page.data[y as usize * page.stride + x as usize] = 0;
        }
    }
}

/// Use the stored pagination for `book` and `style`, or start one in the
/// background (with `paginate_with`, the MOBI cache directory). Returns
/// whether it was cached, and the pagination thread.
fn start_pagination(
    book: &mut Book,
    style: &Style,
    data_dir: &Path,
    id: &str,
    stamp: (u64, i64),
    path: &Path,
    paginate_with: Option<&Path>,
) -> (bool, Option<Paginator>) {
    if book.engine() != Engine::Plato {
        return (true, None);
    }
    let file = pagecache::cache_path(data_dir, id, stamp.0, stamp.1, style);
    if let Some(pages) = pagecache::load(&file) {
        book.set_pagination(pages);
        return (true, None);
    }
    let Some(mobi_cache) = paginate_with else {
        return (false, None);
    };
    let paginator = Paginator::start(
        path.to_path_buf(),
        style.clone(),
        mobi_cache.to_path_buf(),
        file,
    );
    (false, Some(paginator))
}
