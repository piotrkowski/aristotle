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
use std::time::{Duration, Instant};

use aristotle_library::{BookRecord, Library, Progress};
use aristotle_render::book::{Book, Engine};
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
    /// A page start to open at instead of placing `saved`: the location of
    /// a snapshot made for exactly this progress, so the book opens on the
    /// page the snapshot shows, whatever rounding the locator went through.
    pub exact: Option<usize>,
    /// Whether to paginate the whole book in the background when its page
    /// numbers are not cached (not for snapshots made ahead of time).
    pub paginate: bool,
}

impl Source {
    pub fn new(lib: &Library, rec: &BookRecord, mobi_cache: &Path) -> Result<Source, Error> {
        let path = lib
            .absolute_path(rec)
            .ok_or_else(|| format_err!("the library has no root"))?;
        Ok(Source {
            id: rec.id.clone(),
            title: rec.meta.title.clone(),
            path,
            stamp: (rec.file.size, rec.file.mtime),
            data_dir: lib.data_dir().to_path_buf(),
            mobi_cache: mobi_cache.to_path_buf(),
            saved: rec.progress.clone(),
            exact: None,
            paginate: true,
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
        let t = Instant::now();
        let mut book = Book::open(&src.path, &style, Some(&src.mobi_cache))
            .with_context(|| format!("can't open {}", src.path.display()))?;
        // The chrome fonts of this thread: FreeType faces can't move to
        // another one, so the session loads its own.
        let mut chrome = Chrome::load(&style).context("can't load the chrome fonts")?;
        let t_open = t.elapsed();
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
        let t = Instant::now();
        let mut pages = vec![(current, book.render_location(current, &mut chrome)?)];
        let t_render = t.elapsed();
        let t = Instant::now();
        let (mut next_of, mut prev_of) = (HashMap::new(), HashMap::new());
        if neighbours {
            let next = book.next_page(current);
            let prev = book.previous_page(current);
            next_of.insert(current, next);
            prev_of.insert(current, prev);
            for loc in [next, prev].into_iter().flatten() {
                match book.render_location(loc, &mut chrome) {
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
        let page = Rc::new(self.book.render_location(location, &mut self.chrome)?);
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
