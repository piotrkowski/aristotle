//! Opening a book with Plato's document engine and turning it into pages.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use plato_core::anyhow::{format_err, Error};
use plato_core::document::epub::EpubDocument;
use plato_core::document::html::layout::{hyph_lang, DEFAULT_HYPH_LANG, HYPHENATION_PATTERNS};
use plato_core::document::{self, file_kind, Document, Location, TocEntry};
use plato_core::framebuffer::Pixmap;
use plato_core::metadata::TextAlign;

use plato_core::geom::CycleDir;

use crate::lang;
use crate::mobi::MobiInfo;
use crate::notes;
use crate::page::{Chrome, Folio, Page};
use crate::position::{PosMap, Spine};
use crate::richtext::RichText;
use crate::style::Style;

/// A reading position in units that outlive one layout (see
/// [`crate::position`]): what the library index stores.
#[derive(Clone, Debug, PartialEq)]
pub enum Position {
    /// EPUB spine item and byte offset inside it.
    Spine { item: usize, offset: usize },
    /// Stock Kindle position: byte offset into the MOBI text.
    Kindle(u64),
    /// Zero-based page of a fixed-layout document.
    Page(usize),
    /// Share of the book read, 0 to 1: the fallback when nothing exact is
    /// known (KOReader XPointers, a format the renderer reads differently).
    Fraction(f32),
}

/// Which engine lays the book out. It decides what a "location" is and how
/// the page is placed on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// Plato's own HTML engine (EPUB, HTML): our font, Knuth–Plass
    /// justification and Liang hyphenation. Locations are byte offsets into
    /// the book's text and pages are discovered by laying the book out.
    Plato,
    /// MuPDF's HTML layout (MOBI, FB2, TXT through Plato's PdfOpener).
    /// Locations are page indices; MuPDF paginates the whole book on layout
    /// and uses its own fonts and line breaking.
    MupdfReflow,
    /// Fixed-layout pages (PDF, CBZ, XPS, DjVu), scaled to fit.
    Fixed,
}

/// An open book, laid out for one [`Style`].
pub struct Book {
    doc: Box<dyn Document>,
    engine: Engine,
    style: Style,
    toc: Vec<TocEntry>,
    title: Option<String>,
    language: Option<String>,
    /// Locations of the pages found so far, in order. For the Plato engine
    /// they are found by walking `Location::Next`, which lays out one spine
    /// item (chapter file) at a time.
    pages: Vec<usize>,
    /// Whether `pages` covers the whole book.
    complete: bool,
    mobi: Option<MobiConversion>,
    /// Spine item sizes, to convert Plato locations to spine positions.
    spine: Option<Spine>,
    /// The file Plato reads (the converted EPUB for a MOBI book).
    source: PathBuf,
    /// The book's source, opened on the first footnote.
    notes: Option<notes::Source>,
    /// Locations of table of contents entries found without layout, by
    /// entry index (see [`Book::toc_location_cheap`]).
    toc_locations: HashMap<usize, Option<usize>>,
}

/// A rectangle on the screen, in pixels: `x0..x1`, `y0..y1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bounds {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Bounds {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    /// Squared distance from a point to the rectangle (0 inside).
    pub fn dist2(&self, x: i32, y: i32) -> i64 {
        let dx = (self.x0 - x).max(0).max(x - self.x1 + 1) as i64;
        let dy = (self.y0 - y).max(0).max(y - self.y1 + 1) as i64;
        dx * dx + dy * dy
    }

    pub fn union(&self, o: &Bounds) -> Bounds {
        Bounds {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }
}

/// A word (or a hyphenated piece of one) on a rendered page.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    /// As laid out; a piece before a hyphenation break ends in U+00AD.
    pub text: String,
    pub rect: Bounds,
    /// Plato location of the word's first byte (EPUB); for fixed-layout
    /// documents the page index.
    pub offset: usize,
}

/// A link on a rendered page.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// The `href` as written in the book (percent-decoded).
    pub uri: String,
    pub rect: Bounds,
    pub offset: usize,
}

/// A flattened table of contents line.
#[derive(Clone, Debug, PartialEq)]
pub struct TocItem {
    pub title: String,
    pub depth: usize,
    /// Plato's entry index, for [`Book::toc_page`].
    pub index: usize,
}

/// Where the current chapter is, for the progress line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChapterInfo {
    /// 1-based number among the chapters at the chosen level.
    pub number: usize,
    pub count: usize,
    pub start: usize,
    /// Where the next chapter starts (or the end of the book).
    pub end: usize,
}

fn flatten(entries: &[TocEntry], depth: usize, out: &mut Vec<(usize, TocEntry)>) {
    for e in entries {
        out.push((depth, e.clone()));
        flatten(&e.children, depth + 1, out);
    }
}

/// A MOBI book opened through its converted EPUB (see [`crate::mobi`]).
#[derive(Clone, Debug)]
pub struct MobiConversion {
    pub info: MobiInfo,
    /// The cached EPUB this book is read from.
    pub epub: PathBuf,
    /// False when the EPUB came from the cache.
    pub converted_now: bool,
    /// MOBI positions to EPUB locations; `None` if the map is missing.
    pub posmap: Option<PosMap>,
}

/// How the book will be hyphenated.
#[derive(Clone, Debug)]
pub struct Hyphenation {
    /// The language tag the engine uses (the book's, or Plato's fallback).
    pub tag: String,
    /// The pattern set it maps to, if any (`Polish`, `EnglishUS`, ...).
    pub language: Option<String>,
    /// Whether that pattern set was found in `hyphenation-patterns/`.
    pub loaded: bool,
}

impl Book {
    /// Open `path` and set it up for `style`. Nothing is laid out yet; the
    /// first call to [`Book::locate`] does that.
    ///
    /// MOBI books are converted to EPUB in `cache_dir` (once per file) and
    /// read with Plato's engine; without a cache directory, or when the
    /// conversion fails, they go through MuPDF.
    ///
    /// Plato resolves its fonts, stylesheets and hyphenation patterns
    /// relative to the current directory, so call [`crate::use_data_dir`]
    /// first.
    pub fn open(path: &Path, style: &Style, cache_dir: Option<&Path>) -> Result<Book, Error> {
        let mut kind = file_kind(path).unwrap_or_default();
        let mut source = path.to_path_buf();
        let mut mobi = None;
        let is_mobi = matches!(kind.as_str(), "mobi" | "azw" | "prc");
        if let (true, Some(dir)) = (is_mobi, cache_dir) {
            match crate::mobi::cached_epub(path, dir) {
                Ok((epub, info, converted_now)) => {
                    source = epub.clone();
                    kind = "epub".into();
                    mobi = Some(MobiConversion {
                        info,
                        posmap: crate::mobi::load_posmap(&epub),
                        epub,
                        converted_now,
                    });
                }
                Err(e) => eprintln!("{}: {e:#}; reading it with MuPDF", path.display()),
            }
        }
        let path = source.as_path();

        let mut language = None;
        let mut doc: Box<dyn Document> = if kind == "epub" {
            // Opened directly rather than through document::open, to pick
            // the hyphenation language when the book's declaration is
            // ambiguous (see crate::lang).
            let mut epub = EpubDocument::new(path)?;
            language = match &style.language {
                Some(l) => Some(l.clone()),
                None => lang::epub_language(path).unwrap_or_else(|e| {
                    eprintln!("{}: can't guess the language: {e:#}", path.display());
                    None
                }),
            };
            if language.is_some() {
                epub.set_language(language.clone());
            }
            Box::new(epub)
        } else {
            document::open(path)
                .ok_or_else(|| format_err!("can't open {} (kind {:?})", path.display(), kind))?
        };
        let language =
            language.or_else(|| doc.metadata("dc:language").filter(|l| !l.trim().is_empty()));
        let (_, body_height) = style.body();

        let engine = if matches!(kind.as_str(), "epub" | "html" | "htm") {
            Engine::Plato
        } else if doc.is_reflowable() {
            Engine::MupdfReflow
        } else {
            Engine::Fixed
        };

        match engine {
            Engine::Plato => {
                doc.layout(style.width, body_height, style.font_size, style.dpi);
                doc.set_margin_width(style.margin_mm);
                doc.set_font_family(&style.font_family, &style.font_dir);
                doc.set_text_align(TextAlign::Justify);
                doc.set_line_height(style.line_height);
            }
            Engine::MupdfReflow => {
                // MuPDF has no margin setting; lay out into the text block
                // and place the pixmap at the margin.
                let m = style.margin_px() as u32;
                doc.layout(
                    style.width - 2 * m,
                    body_height - 2 * m,
                    style.font_size,
                    style.dpi,
                );
            }
            Engine::Fixed => {}
        }

        let toc = doc.toc().unwrap_or_default();
        let title = doc.title().filter(|t| !t.trim().is_empty());
        // Only trusted when it agrees with Plato's own idea of the size.
        let spine = if engine == Engine::Plato {
            Spine::read(path)
                .ok()
                .filter(|s| s.total() == doc.pages_count())
        } else {
            None
        };
        Ok(Book {
            spine,
            doc,
            engine,
            style: style.clone(),
            toc,
            title,
            language,
            pages: Vec::new(),
            complete: false,
            mobi,
            source: path.to_path_buf(),
            notes: None,
            toc_locations: HashMap::new(),
        })
    }

    /// Set when this is a MOBI book read through its converted EPUB.
    pub fn mobi(&self) -> Option<&MobiConversion> {
        self.mobi.as_ref()
    }

    pub fn engine(&self) -> Engine {
        self.engine
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn author(&self) -> Option<String> {
        self.doc.author()
    }

    /// The language the text is hyphenated in: chosen by [`crate::lang`]
    /// or `--lang` for ambiguous EPUBs, else the book's first `dc:language`.
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    pub fn toc(&self) -> &[TocEntry] {
        &self.toc
    }

    /// The hyphenation the Plato engine will apply. Plato picks patterns
    /// per element from `lang`/`xml:lang` and falls back to the book's
    /// `dc:language`, then to English.
    pub fn hyphenation(&self) -> Hyphenation {
        let tag = self
            .language
            .clone()
            .unwrap_or_else(|| DEFAULT_HYPH_LANG.to_string());
        let lang = hyph_lang(&tag);
        Hyphenation {
            language: lang.map(|l| format!("{l:?}")),
            loaded: lang.is_some_and(|l| HYPHENATION_PATTERNS.contains_key(&l)),
            tag,
        }
    }

    /// Location of page `index` (0-based), laying out as much of the book
    /// as needed to find it. `None` past the end.
    pub fn locate(&mut self, index: usize) -> Option<usize> {
        if self.engine != Engine::Plato {
            return (index < self.doc.pages_count()).then_some(index);
        }
        if self.pages.is_empty() {
            let first = self.doc.resolve_location(Location::Exact(0))?;
            self.pages.push(first);
        }
        while self.pages.len() <= index && !self.complete {
            let last = *self.pages.last().expect("non-empty");
            match self.doc.resolve_location(Location::Next(last)) {
                // Guard against a location that does not advance, which
                // would loop forever.
                Some(next) if next > last => self.pages.push(next),
                _ => self.complete = true,
            }
        }
        self.pages.get(index).copied()
    }

    /// Lay out the whole book and return its page count. Cheap for MuPDF
    /// documents; for EPUB it lays out every spine item.
    pub fn paginate(&mut self) -> usize {
        if self.engine != Engine::Plato {
            return self.doc.pages_count();
        }
        while !self.complete {
            let len = self.pages.len();
            if self.locate(len).is_none() {
                break;
            }
        }
        self.pages.len()
    }

    /// Total pages if already known without further layout.
    pub fn known_page_count(&self) -> Option<usize> {
        match self.engine {
            Engine::Plato => self.complete.then_some(self.pages.len()),
            _ => Some(self.doc.pages_count()),
        }
    }

    /// Position of page `index` in the book, 0.0 to 1.0.
    pub fn progress(&self, index: usize, location: usize) -> f32 {
        match self.known_page_count() {
            // Agrees with the "x / y" next to it.
            Some(total) => (index + 1) as f32 / total.max(1) as f32,
            // Byte offset over total bytes: available before pagination,
            // though images and markup make it drift from the page count.
            None => location as f32 / self.doc.pages_count().max(1) as f32,
        }
    }

    /// Title of the chapter containing `location`.
    pub fn chapter_title(&mut self, location: usize) -> Option<String> {
        self.doc
            .chapter(location, &self.toc)
            .map(|(entry, _)| entry.title.trim().to_string())
            .filter(|t| !t.is_empty())
    }

    /// Rasterise the book's content at `location`, without header and
    /// footer, as a grayscale pixmap.
    pub fn render_body(&mut self, location: usize) -> Option<Pixmap> {
        let scale = match self.engine {
            Engine::Plato | Engine::MupdfReflow => 1.0,
            Engine::Fixed => {
                let (w, h) = self.doc.dims(location)?;
                let (_, body_h) = self.style.body();
                let m = self.style.margin_px() as f32;
                ((self.style.width as f32) / w).min((body_h as f32 - 2.0 * m) / h)
            }
        };
        self.doc
            .pixmap(Location::Exact(location), scale, 1)
            .map(|(pixmap, _)| pixmap)
    }

    /// Put `body` (from [`Book::render_body`]) and the chrome on a page.
    pub fn compose(&self, body: &Pixmap, folio: &Folio, chrome: &mut Chrome) -> Page {
        let style = &self.style;
        let mut page = Page::white(style);
        let (top, body_h) = style.body();
        let (x, y) = match self.engine {
            Engine::Plato => (0, top),
            Engine::MupdfReflow => (style.margin_px(), top + style.margin_px()),
            Engine::Fixed => (
                (style.width as i32 - body.width as i32) / 2,
                top + (body_h as i32 - body.height as i32) / 2,
            ),
        };
        page.blit_pixmap(body, x, y);
        chrome.draw(&mut page, folio);
        page
    }

    /// The header and footer contents for page `index` at `location`.
    pub fn folio(&mut self, index: usize, location: usize) -> Folio {
        let header = self
            .chapter_title(location)
            .or_else(|| self.title.clone())
            .unwrap_or_default();
        Folio {
            header,
            page: Some(index + 1),
            pages: self.known_page_count(),
            progress: self.progress(index, location),
            left: None,
        }
    }

    // Navigation by location, for the reader app. A reader opens a book at
    // a saved position and turns from there, so it cannot depend on page
    // indices, which need the whole book laid out first (seconds on the
    // device). Indices come in later from a background pagination
    // ([`Book::set_pagination`]) and only feed "x / y".

    /// Size of the location space: bytes of text for Plato's engine, pages
    /// otherwise.
    pub fn len(&self) -> usize {
        self.doc.pages_count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Start of the page that contains `location` (clamped to the book),
    /// laying out its chapter if needed.
    pub fn page_start(&mut self, location: usize) -> Option<usize> {
        let last = self.len().checked_sub(1)?;
        let location = location.min(last);
        match self.engine {
            Engine::Plato => self
                .doc
                .resolve_location(Location::Exact(location))
                .or_else(|| self.doc.resolve_location(Location::Exact(0))),
            _ => Some(location),
        }
    }

    /// Start of the page after the one starting at `location`.
    pub fn next_page(&mut self, location: usize) -> Option<usize> {
        self.doc
            .resolve_location(Location::Next(location))
            .filter(|&n| n > location)
    }

    /// Start of the page before the one starting at `location`.
    pub fn previous_page(&mut self, location: usize) -> Option<usize> {
        self.doc
            .resolve_location(Location::Previous(location))
            .filter(|&p| p < location)
    }

    /// Start of the next (or previous) chapter in the table of contents.
    pub fn chapter_page(&mut self, location: usize, forward: bool) -> Option<usize> {
        let dir = if forward {
            CycleDir::Next
        } else {
            CycleDir::Previous
        };
        let target = self
            .doc
            .chapter_relative(location, dir, &self.toc)
            .map(|entry| entry.location.clone())?;
        let found = self.doc.resolve_location(target)?;
        self.page_start(found)
    }

    /// Take page locations found elsewhere (a background pagination, or a
    /// cache on disk) as the whole book's pagination. They must come from
    /// the same file and style. Ignored for engines that know their pages.
    pub fn set_pagination(&mut self, pages: Vec<usize>) {
        if self.engine == Engine::Plato && !pages.is_empty() {
            self.pages = pages;
            self.complete = true;
        }
    }

    /// The start of every page, once the whole book has been laid out.
    pub fn pagination(&self) -> Option<&[usize]> {
        (self.engine == Engine::Plato && self.complete).then_some(&self.pages[..])
    }

    /// Index of the page starting at or before `location`, if known
    /// without laying anything out.
    pub fn index_of(&self, location: usize) -> Option<usize> {
        if self.engine != Engine::Plato {
            return Some(location);
        }
        let last = *self.pages.last()?;
        if !self.complete && location > last {
            return None;
        }
        Some(
            self.pages
                .partition_point(|&p| p <= location)
                .saturating_sub(1),
        )
    }

    /// Position in the book, 0 to 1: "x / y" when the pages are known,
    /// else the share of the text before `location`.
    pub fn fraction(&self, location: usize) -> f32 {
        match (self.known_page_count(), self.index_of(location)) {
            (Some(total), Some(index)) => (index + 1) as f32 / total.max(1) as f32,
            _ => location as f32 / self.len().max(1) as f32,
        }
    }

    /// The location of a saved position, if this book can place it exactly
    /// or by fraction. Not necessarily a page start (see
    /// [`Book::page_start`]).
    pub fn location_of(&self, position: &Position) -> Option<usize> {
        let len = self.len();
        let by_fraction =
            |f: f32| ((f.clamp(0.0, 1.0) * len as f32) as usize).min(len.saturating_sub(1));
        match (self.engine, position) {
            (_, Position::Fraction(f)) => Some(by_fraction(*f)),
            (Engine::Plato, Position::Spine { item, offset }) if self.mobi.is_none() => {
                self.spine.as_ref()?.location(*item, *offset)
            }
            (Engine::Plato, Position::Kindle(p)) => self
                .mobi
                .as_ref()
                .and_then(|m| m.posmap.as_ref())
                .map(|m| m.location(*p) as usize),
            (Engine::MupdfReflow | Engine::Fixed, Position::Page(i)) => {
                Some((*i).min(len.saturating_sub(1)))
            }
            _ => None,
        }
    }

    /// The most exact position of `location` this book can name.
    pub fn position_at(&self, location: usize) -> Position {
        match self.engine {
            Engine::Plato => {
                if let Some(map) = self.mobi.as_ref().and_then(|m| m.posmap.as_ref()) {
                    Position::Kindle(map.position(location as u64))
                } else if let (None, Some(spine)) = (&self.mobi, &self.spine) {
                    let (item, offset) = spine.coordinates(location);
                    Position::Spine { item, offset }
                } else {
                    Position::Fraction(location as f32 / self.len().max(1) as f32)
                }
            }
            _ => Position::Page(location),
        }
    }

    /// Render the page starting at `location`, with header and footer.
    pub fn render_location(&mut self, location: usize, chrome: &mut Chrome) -> Result<Page, Error> {
        self.render_location_with(location, chrome, None)
    }

    /// [`Book::render_location`] with `footer_left` at the left of the
    /// footer.
    pub fn render_location_with(
        &mut self,
        location: usize,
        chrome: &mut Chrome,
        footer_left: Option<String>,
    ) -> Result<Page, Error> {
        let body = self
            .render_body(location)
            .ok_or_else(|| format_err!("can't render the page at {location}"))?;
        let header = self
            .chapter_title(location)
            .or_else(|| self.title.clone())
            .unwrap_or_default();
        let index = self.index_of(location);
        let folio = Folio {
            header,
            page: index.map(|i| i + 1),
            pages: self.known_page_count(),
            progress: self.fraction(location),
            left: footer_left,
        };
        Ok(self.compose(&body, &folio, chrome))
    }

    /// Locate, render and compose page `index` (0-based).
    pub fn render_page(&mut self, index: usize, chrome: &mut Chrome) -> Result<Page, Error> {
        let location = self
            .locate(index)
            .ok_or_else(|| format_err!("page {} is past the end of the book", index + 1))?;
        let body = self
            .render_body(location)
            .ok_or_else(|| format_err!("can't render page {}", index + 1))?;
        let folio = self.folio(index, location);
        Ok(self.compose(&body, &folio, chrome))
    }
}

// Reading interactions: words and links on a page, footnotes, the table of
// contents. Coordinates are screen pixels of the composed page.
impl Book {
    /// How the body of the page at `location` is placed on the screen:
    /// (scale, x offset, y offset), as [`Book::render_body`] and
    /// [`Book::compose`] place it.
    fn placement(&mut self, location: usize) -> Option<(f32, i32, i32)> {
        let (top, body_h) = self.style.body();
        let m = self.style.margin_px();
        match self.engine {
            Engine::Plato => Some((1.0, 0, top)),
            Engine::MupdfReflow => Some((1.0, m, top + m)),
            Engine::Fixed => {
                let (w, h) = self.doc.dims(location)?;
                let scale =
                    ((self.style.width as f32) / w).min((body_h as f32 - 2.0 * m as f32) / h);
                let (pw, ph) = ((w * scale) as i32, (h * scale) as i32);
                Some((
                    scale,
                    (self.style.width as i32 - pw) / 2,
                    top + (body_h as i32 - ph) / 2,
                ))
            }
        }
    }

    fn to_screen(b: &plato_core::geom::Boundary, (scale, dx, dy): (f32, i32, i32)) -> Bounds {
        Bounds {
            x0: (b.min.x * scale).floor() as i32 + dx,
            y0: (b.min.y * scale).floor() as i32 + dy,
            x1: (b.max.x * scale).ceil() as i32 + dx,
            y1: (b.max.y * scale).ceil() as i32 + dy,
        }
    }

    /// The words of the page starting at `location`, in reading order.
    pub fn words(&mut self, location: usize) -> Vec<Word> {
        let Some(place) = self.placement(location) else {
            return Vec::new();
        };
        let Some((words, _)) = self.doc.words(Location::Exact(location)) else {
            return Vec::new();
        };
        words
            .into_iter()
            .filter(|w| !w.text.trim().is_empty())
            .map(|w| Word {
                rect: Self::to_screen(&w.rect, place),
                offset: w.location.location(),
                text: w.text,
            })
            .collect()
    }

    /// The links of the page starting at `location`.
    pub fn links(&mut self, location: usize) -> Vec<Link> {
        let Some(place) = self.placement(location) else {
            return Vec::new();
        };
        let Some((links, _)) = self.doc.links(Location::Exact(location)) else {
            return Vec::new();
        };
        links
            .into_iter()
            .map(|l| Link {
                rect: Self::to_screen(&l.rect, place),
                offset: l.location.location(),
                uri: l.text,
            })
            .collect()
    }

    /// The archive path of the spine item holding `location`.
    fn spine_path(&self, location: usize) -> Option<String> {
        let spine = self.spine.as_ref()?;
        let (item, _) = spine.coordinates(location);
        spine.paths.get(item).cloned()
    }

    /// The note behind `link` if it is a footnote (see [`crate::notes`]);
    /// `link_text` is the marker the link shows.
    pub fn footnote(&mut self, link: &Link, link_text: &str) -> Option<RichText> {
        if self.engine != Engine::Plato {
            return None;
        }
        let base = self.spine_path(link.offset)?;
        if self.notes.is_none() {
            self.notes = notes::Source::open(&self.source);
        }
        self.notes.as_mut()?.footnote(&base, &link.uri, link_text)
    }

    /// Start of the page a link in the page at `location` leads to.
    pub fn link_page(&mut self, location: usize, uri: &str) -> Option<usize> {
        let target = self
            .doc
            .resolve_location(Location::LocalUri(location, uri.to_string()))?;
        self.page_start(target)
    }

    /// The table of contents, flattened with depths.
    pub fn toc_items(&self) -> Vec<TocItem> {
        let mut flat = Vec::new();
        flatten(&self.toc, 0, &mut flat);
        flat.into_iter()
            .map(|(depth, e)| TocItem {
                title: e.title.trim().to_string(),
                depth,
                index: e.index,
            })
            .collect()
    }

    /// Start of the page a table of contents entry points at (lays out its
    /// chapter).
    pub fn toc_page(&mut self, index: usize) -> Option<usize> {
        let mut flat = Vec::new();
        flatten(&self.toc, 0, &mut flat);
        let (_, entry) = flat.into_iter().find(|(_, e)| e.index == index)?;
        let target = self.doc.resolve_location(entry.location)?;
        self.page_start(target)
    }

    /// The index of the table of contents entry for `location`.
    pub fn toc_current(&mut self, location: usize) -> Option<usize> {
        self.doc
            .chapter(location, &self.toc)
            .map(|(entry, _)| entry.index)
    }

    /// Where a table of contents entry starts, found from the spine and the
    /// target file's ids without laying anything out (Plato's own resolver
    /// lays out the chapter, a page turn's time per entry).
    fn toc_location_cheap(&mut self, entry: &TocEntry) -> Option<usize> {
        if let Some(found) = self.toc_locations.get(&entry.index) {
            return *found;
        }
        let found = match &entry.location {
            Location::Exact(n) => Some(*n),
            Location::Uri(uri) => {
                let (path, frag) = match uri.split_once('#') {
                    Some((p, f)) => (p, Some(f)),
                    None => (uri.as_str(), None),
                };
                let start = self
                    .spine
                    .as_ref()
                    .and_then(|s| s.item_of(path).and_then(|i| s.start(i)));
                match (start, frag) {
                    (Some(start), Some(frag)) => {
                        if self.notes.is_none() {
                            self.notes = notes::Source::open(&self.source);
                        }
                        let offset = self.notes.as_mut().and_then(|n| n.id_offset(path, frag));
                        Some(start + offset.unwrap_or(0))
                    }
                    (start, _) => start,
                }
            }
            _ => None,
        };
        self.toc_locations.insert(entry.index, found);
        found
    }

    /// Which chapter `location` is in and where it ends. Chapters are the
    /// table of contents entries at the first level with more than one
    /// entry (a book whose contents is one title with chapters under it
    /// counts the chapters).
    pub fn chapter_info(&mut self, location: usize) -> Option<ChapterInfo> {
        if self.engine != Engine::Plato || self.toc.is_empty() {
            return None;
        }
        let mut flat = Vec::new();
        flatten(&self.toc, 0, &mut flat);
        let max_depth = flat.iter().map(|(d, _)| *d).max().unwrap_or(0);
        let level = (0..=max_depth)
            .find(|&d| flat.iter().filter(|(x, _)| *x == d).count() > 1)
            .unwrap_or(0);
        let mut starts: Vec<usize> = Vec::new();
        for (_, e) in flat.iter().filter(|(d, _)| *d == level) {
            if let Some(l) = self.toc_location_cheap(e) {
                starts.push(l);
            }
        }
        starts.sort_unstable();
        starts.dedup();
        let number = starts.partition_point(|&s| s <= location);
        if number == 0 {
            return None;
        }
        Some(ChapterInfo {
            number,
            count: starts.len(),
            start: starts[number - 1],
            end: starts.get(number).copied().unwrap_or(self.len()),
        })
    }
}
