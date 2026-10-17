//! The library screen fed from the real library index.
//!
//! The book to continue is the most recently opened one (by `last_opened`,
//! which stock-imported progress does not set, so before the first book is
//! opened here, the most recently updated progress counts). The shelf holds
//! every other book in the chosen order. Covers come from the library's
//! thumbnail cache and are loaded once per run.

use std::collections::HashMap;

use aristotle_library::cover::read_pgm;
use aristotle_library::{BookRecord, Library, Query, SortKey, Status};
use aristotle_ui::screens::library::{self as ui_lib, Book, Filter, Shelf};
use aristotle_ui::widgets::GrayImage;

/// Sort orders offered by the sort button, in the order it cycles.
pub const SORTS: [(SortKey, &str); 3] = [
    (SortKey::Recent, "Ostatnie"),
    (SortKey::Title, "Tytuł"),
    (SortKey::Author, "Autor"),
];

/// The library as the screen shows it: books in screen order with the
/// record IDs they came from.
pub struct ShelfModel {
    pub hero: Option<(String, Book)>,
    /// Record IDs of `books`, index for index.
    pub ids: Vec<String>,
    pub books: Vec<Book>,
    pub filter: Filter,
    pub sort: usize,
}

/// Thumbnails already read from the cover cache, by cover key.
#[derive(Default)]
pub struct Covers {
    loaded: HashMap<String, Option<GrayImage>>,
}

impl Covers {
    fn get(&mut self, lib: &Library, rec: &BookRecord) -> Option<GrayImage> {
        let key = rec.cover.clone()?;
        self.loaded
            .entry(key)
            .or_insert_with(|| {
                let path = lib.cover_path(rec)?;
                let g = read_pgm(&path).ok()?;
                Some(GrayImage::new(g.width, g.height, g.pixels))
            })
            .clone()
    }
}

/// The book to continue: the last one opened here, else the one whose
/// progress (perhaps imported from the stock reader) changed last.
pub fn current_book(lib: &Library) -> Option<&BookRecord> {
    lib.books()
        .iter()
        .filter(|b| b.last_opened.is_some())
        .max_by_key(|b| b.last_opened)
        .or_else(|| {
            lib.books()
                .iter()
                .filter(|b| b.status == Status::Reading)
                .filter_map(|b| b.progress.as_ref().map(|p| (p.updated, b)))
                .max_by_key(|(t, _)| *t)
                .map(|(_, b)| b)
        })
}

fn screen_book(lib: &Library, rec: &BookRecord, covers: &mut Covers) -> Book {
    let author = rec
        .meta
        .authors
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut b = Book::new(&rec.meta.title, &author, 0.0);
    b.progress = rec.progress.as_ref().map_or(0.0, |p| p.fraction);
    b.status = match rec.status {
        Status::Unread => ui_lib::Status::Unread,
        Status::Reading => ui_lib::Status::Reading,
        Status::Finished => ui_lib::Status::Finished,
    };
    if let Some(page) = rec.progress.as_ref().and_then(|p| p.page.as_ref()) {
        b.chapter = format!("Strona {page}");
    }
    b.cover = covers.get(lib, rec);
    b
}

impl ShelfModel {
    pub fn build(lib: &Library, filter: Filter, sort: usize, covers: &mut Covers) -> ShelfModel {
        let hero_id = current_book(lib).map(|b| b.id.clone());
        let q = Query {
            sort: SORTS[sort % SORTS.len()].0,
            ..Query::default()
        };
        let (mut ids, mut books) = (Vec::new(), Vec::new());
        for r in lib.query(&q) {
            if Some(&r.id) != hero_id.as_ref() {
                ids.push(r.id.clone());
                books.push(screen_book(lib, r, covers));
            }
        }
        let hero = hero_id
            .and_then(|id| lib.book(&id))
            .map(|r| (r.id.clone(), screen_book(lib, r, covers)));
        ShelfModel {
            hero,
            ids,
            books,
            filter,
            sort: sort % SORTS.len(),
        }
    }

    pub fn shelf<'a>(&'a self, clock: &'a str, battery: Option<f32>) -> Shelf<'a> {
        Shelf {
            hero: self.hero.as_ref().map(|(_, b)| b),
            books: &self.books,
            filter: self.filter,
            sort: SORTS[self.sort].1,
            clock,
            battery,
        }
    }

    /// The record ID behind `action::BOOK + i`.
    pub fn book_id(&self, i: usize) -> Option<&str> {
        self.ids.get(i).map(String::as_str)
    }

    pub fn hero_id(&self) -> Option<&str> {
        self.hero.as_ref().map(|(id, _)| id.as_str())
    }
}
