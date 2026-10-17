//! Opening a book with Plato's document engine and turning it into pages.

use std::path::{Path, PathBuf};

use plato_core::anyhow::{format_err, Error};
use plato_core::document::epub::EpubDocument;
use plato_core::document::html::layout::{hyph_lang, DEFAULT_HYPH_LANG, HYPHENATION_PATTERNS};
use plato_core::document::{self, file_kind, Document, Location, TocEntry};
use plato_core::framebuffer::Pixmap;
use plato_core::metadata::TextAlign;

use crate::lang;
use crate::mobi::MobiInfo;
use crate::page::{Chrome, Folio, Page};
use crate::style::Style;

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
}

/// A MOBI book opened through its converted EPUB (see [`crate::mobi`]).
#[derive(Clone, Debug)]
pub struct MobiConversion {
    pub info: MobiInfo,
    /// The cached EPUB this book is read from.
    pub epub: PathBuf,
    /// False when the EPUB came from the cache.
    pub converted_now: bool,
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
        Ok(Book {
            doc,
            engine,
            style: style.clone(),
            toc,
            title,
            language,
            pages: Vec::new(),
            complete: false,
            mobi,
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
            page: index + 1,
            pages: self.known_page_count(),
            progress: self.progress(index, location),
        }
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
