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
pub struct OpenTimes {
    pub open: Duration,
    pub place: Duration,
    pub render: Duration,
    pub pages_cached: bool,
}

impl Session {
    /// Open the book `rec` at its saved position with `style`.
    pub fn open(
        lib: &Library,
        rec: &BookRecord,
        style: Style,
        mobi_cache: &Path,
    ) -> Result<(Session, OpenTimes), Error> {
        let path = lib
            .absolute_path(rec)
            .ok_or_else(|| format_err!("the library has no root"))?;
        let t = Instant::now();
        let book = Book::open(&path, &style, Some(mobi_cache))
            .with_context(|| format!("can't open {}", path.display()))?;
        let chrome = Chrome::load(&style).context("can't load the chrome fonts")?;
        let t_open = t.elapsed();
        let mut s = Session {
            id: rec.id.clone(),
            title: rec.meta.title.clone(),
            path,
            stamp: (rec.file.size, rec.file.mtime),
            data_dir: lib.data_dir().to_path_buf(),
            mobi_cache: mobi_cache.to_path_buf(),
            style,
            book,
            chrome,
            current: 0,
            pages: Vec::new(),
            next_of: HashMap::new(),
            prev_of: HashMap::new(),
            paginator: None,
        };
        let pages_cached = s.load_pagination();
        let t = Instant::now();
        s.current = s.place(rec.progress.as_ref())?;
        let t_place = t.elapsed();
        let t = Instant::now();
        s.page(s.current)?;
        let t_render = t.elapsed();
        Ok((
            s,
            OpenTimes {
                open: t_open,
                place: t_place,
                render: t_render,
                pages_cached,
            },
        ))
    }

    /// The page start for a saved progress record: its locator if the book
    /// can place it, else its fraction, else the first page.
    fn place(&mut self, saved: Option<&Progress>) -> Result<usize, Error> {
        let target = progress::positions(saved)
            .iter()
            .find_map(|p| self.book.location_of(p))
            .unwrap_or(0);
        self.book
            .page_start(target)
            .ok_or_else(|| format_err!("the book has no pages"))
    }

    /// Use the stored pagination for this book and style, or start one in
    /// the background. Returns true on a cache hit.
    fn load_pagination(&mut self) -> bool {
        if self.book.engine() != Engine::Plato {
            return true;
        }
        let file = self.cache_file();
        if let Some(pages) = pagecache::load(&file) {
            self.book.set_pagination(pages);
            return true;
        }
        self.paginator = Some(Paginator::start(
            self.path.clone(),
            self.style.clone(),
            self.mobi_cache.clone(),
            file,
        ));
        false
    }

    fn cache_file(&self) -> PathBuf {
        pagecache::cache_path(
            &self.data_dir,
            &self.id,
            self.stamp.0,
            self.stamp.1,
            &self.style,
        )
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
