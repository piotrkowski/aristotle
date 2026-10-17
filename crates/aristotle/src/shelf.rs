// SPDX-License-Identifier: AGPL-3.0-or-later
//! The library screens fed from the real library index.
//!
//! [`Nav`] is where the reader is: a view of the home screen (books with a
//! shelf filter, folders, series), or inside a folder or a series.
//! [`ShelfModel`] turns the index into the rows of that place and remembers
//! what each row opens.
//!
//! Folders mirror `documents/` exactly: a book's folder is the directory of
//! its path in the index, so "Mickiewicz, Adam/Dziady.epub" is in the folder
//! "Mickiewicz, Adam", and nested folders nest. Inside a folder its
//! subfolders come first, then its own books.
//!
//! The book to continue is the most recently opened one (by `last_opened`,
//! which stock-imported progress does not set, so before the first book is
//! opened here, the most recently updated progress counts).

use std::collections::{BTreeMap, HashMap};

use aristotle_i18n::{tf, tr, Msg};
use aristotle_library::cover::read_pgm;
use aristotle_library::text::fold;
use aristotle_library::{BookRecord, Library, Query, SortKey, Status};
use aristotle_ui::screens::library::{
    self as ui_lib, Book, CoverMode, Filter, Item, Place, Shelf, StatusLine, View,
};
use aristotle_ui::widgets::GrayImage;

/// Sort orders offered by the sort button, in the order it cycles.
pub const SORTS: [(SortKey, Msg); 3] = [
    (SortKey::Recent, Msg::LibrarySortRecent),
    (SortKey::Title, Msg::LibrarySortTitle),
    (SortKey::Author, Msg::LibrarySortAuthor),
];

/// Where the reader is in the library.
#[derive(Clone, Debug, PartialEq)]
pub struct Nav {
    pub view: View,
    pub filter: Filter,
    /// Index into [`SORTS`].
    pub sort: usize,
    /// The open folder, as path components below `documents/`.
    pub folder: Vec<String>,
    /// The open series.
    pub series: Option<String>,
}

impl Default for Nav {
    fn default() -> Self {
        Nav {
            view: View::Books,
            filter: Filter::All,
            sort: 0,
            folder: Vec::new(),
            series: None,
        }
    }
}

impl Nav {
    pub fn at_home(&self) -> bool {
        self.folder.is_empty() && self.series.is_none()
    }

    /// One level up: out of the series, or to the parent folder.
    pub fn up(&mut self) {
        if self.series.take().is_none() {
            self.folder.pop();
        }
    }
}

/// What a row opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Book(String),
    Folder(String),
    Series(String),
}

/// The library as one screen shows it.
pub struct ShelfModel {
    pub hero: Option<(String, Book)>,
    pub items: Vec<Item>,
    /// What `items[i]` opens.
    pub targets: Vec<Target>,
    pub nav: Nav,
    /// The folder or series name shown in the header of a shelf.
    pub name: String,
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

/// The folder of a book path relative to `documents/`, as components.
pub fn folder_of(path: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    parts.pop();
    parts
}

/// One level of the folder tree: the subfolders of `prefix` with their book
/// counts (all books below them, and how many are being read), and the
/// indices of the books lying directly in `prefix`.
pub fn folder_level(
    books: &[(&str, Status)],
    prefix: &[String],
) -> (Vec<(String, usize, usize)>, Vec<usize>) {
    let mut sub: BTreeMap<String, (String, usize, usize)> = BTreeMap::new();
    let mut own = Vec::new();
    for (i, (path, status)) in books.iter().enumerate() {
        let dir = folder_of(path);
        if dir.len() < prefix.len() || dir.iter().zip(prefix).any(|(a, b)| *a != b.as_str()) {
            continue;
        }
        match dir.get(prefix.len()) {
            None => own.push(i),
            Some(name) => {
                let e = sub
                    .entry(fold(name))
                    .or_insert_with(|| (name.to_string(), 0, 0));
                e.1 += 1;
                e.2 += (*status == Status::Reading) as usize;
            }
        }
    }
    (sub.into_values().collect(), own)
}

/// A library record as the screens show it.
pub fn screen_book(lib: &Library, rec: &BookRecord, covers: &mut Covers, photos: bool) -> Book {
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
        b.chapter = tf!(Msg::LibraryPage, page = page);
    }
    let dir = folder_of(&rec.file.path);
    if !dir.is_empty() {
        b.folder = Some(dir.join("/"));
    }
    if let Some(s) = &rec.meta.series {
        b.series = Some((s.name.clone(), s.index.unwrap_or(0.0) as f32));
    }
    if photos {
        b.cover = covers.get(lib, rec);
    }
    b
}

impl ShelfModel {
    pub fn build(lib: &Library, nav: &Nav, mode: CoverMode, covers: &mut Covers) -> ShelfModel {
        let photos = mode == CoverMode::Photos;
        let sort = SORTS[nav.sort % SORTS.len()].0;
        let sorted = lib.query(&Query {
            sort,
            ..Query::default()
        });
        let mut items = Vec::new();
        let mut targets = Vec::new();
        // Books come after any folders; collected here, added at the end.
        let mut book_rows: Vec<&BookRecord> = Vec::new();
        let hero_id = current_book(lib).map(|b| b.id.clone());
        let mut name = String::new();
        if let Some(series) = &nav.series {
            name = series.clone();
            let mut books: Vec<&BookRecord> = sorted
                .iter()
                .copied()
                .filter(|r| r.meta.series.as_ref().is_some_and(|s| &s.name == series))
                .collect();
            books.sort_by(|a, b| {
                let i = |r: &BookRecord| r.meta.series.as_ref().and_then(|s| s.index);
                i(a).partial_cmp(&i(b)).unwrap_or(std::cmp::Ordering::Equal)
            });
            book_rows = books;
        } else if !nav.folder.is_empty() || nav.view == View::Folders {
            name = nav.folder.last().cloned().unwrap_or_default();
            let paths: Vec<(&str, Status)> = sorted
                .iter()
                .map(|r| (r.file.path.as_str(), r.status))
                .collect();
            let (subs, own) = folder_level(&paths, &nav.folder);
            for (folder, books, reading) in subs {
                targets.push(Target::Folder(folder.clone()));
                items.push(Item::Folder {
                    name: folder,
                    books,
                    reading,
                });
            }
            book_rows.extend(own.into_iter().map(|i| sorted[i]));
        } else if nav.view == View::Series {
            let mut series: BTreeMap<String, (String, usize, usize)> = BTreeMap::new();
            for r in &sorted {
                if let Some(s) = &r.meta.series {
                    let e = series
                        .entry(fold(&s.name))
                        .or_insert_with(|| (s.name.clone(), 0, 0));
                    e.1 += 1;
                    e.2 += (r.status == Status::Finished) as usize;
                }
            }
            for (name, books, finished) in series.into_values() {
                targets.push(Target::Series(name.clone()));
                items.push(Item::Series {
                    name,
                    books,
                    finished,
                });
            }
        } else {
            for r in &sorted {
                // The hero is on top already; the narrower shelves still
                // list it, so "Reading" holds every book being read.
                if nav.filter == Filter::All && Some(&r.id) == hero_id.as_ref() {
                    continue;
                }
                book_rows.push(r);
            }
        }
        for r in book_rows {
            items.push(Item::Book(screen_book(lib, r, covers, photos)));
            targets.push(Target::Book(r.id.clone()));
        }
        let hero = if nav.at_home() {
            hero_id
                .and_then(|id| lib.book(&id))
                .map(|r| (r.id.clone(), screen_book(lib, r, covers, photos)))
        } else {
            None
        };
        ShelfModel {
            hero,
            items,
            targets,
            nav: nav.clone(),
            name,
        }
    }

    pub fn shelf<'a>(&'a self, status: StatusLine<'a>, covers: CoverMode) -> Shelf<'a> {
        let place = if self.nav.series.is_some() {
            Place::Series { name: &self.name }
        } else if !self.nav.folder.is_empty() {
            Place::Folder { name: &self.name }
        } else {
            Place::Home {
                view: self.nav.view,
                filter: self.nav.filter,
            }
        };
        let sort = if self.nav.series.is_some() {
            ""
        } else {
            tr(SORTS[self.nav.sort % SORTS.len()].1)
        };
        Shelf {
            place,
            hero: self.hero.as_ref().map(|(_, b)| b),
            items: &self.items,
            sort,
            status,
            covers,
        }
    }

    /// What `action::ITEM + i` opens.
    pub fn target(&self, i: usize) -> Option<&Target> {
        self.targets.get(i)
    }

    pub fn hero_id(&self) -> Option<&str> {
        self.hero.as_ref().map(|(id, _)| id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn folders_mirror_documents() {
        let books = [
            ("Mickiewicz, Adam/Pan Tadeusz.epub", Status::Reading),
            ("Mickiewicz, Adam/Dziady.epub", Status::Unread),
            ("Platon/Fedon.epub", Status::Finished),
            ("Filozofia/Stoicy/Medytacje.epub", Status::Reading),
            ("Filozofia/Etyka.pdf", Status::Unread),
            ("Moby Dick.mobi", Status::Unread),
        ];
        let (subs, own) = folder_level(&books, &[]);
        let names: Vec<&str> = subs.iter().map(|s| s.0.as_str()).collect();
        assert_eq!(names, ["Filozofia", "Mickiewicz, Adam", "Platon"]);
        assert_eq!(subs[0], ("Filozofia".into(), 2, 1), "nested books count");
        assert_eq!(subs[1], ("Mickiewicz, Adam".into(), 2, 1));
        assert_eq!(own, [5], "loose books at the top");
        let (subs, own) = folder_level(&books, &s(&["Filozofia"]));
        assert_eq!(subs, [("Stoicy".to_string(), 1, 1)]);
        assert_eq!(own, [4]);
        let (subs, own) = folder_level(&books, &s(&["Filozofia", "Stoicy"]));
        assert!(subs.is_empty());
        assert_eq!(own, [3]);
        // A sibling whose name starts the same is not inside.
        let (_, own) = folder_level(&[("Platonizm/x.epub", Status::Unread)], &s(&["Platon"]));
        assert!(own.is_empty());
    }

    #[test]
    fn nav_goes_up_one_level() {
        let mut n = Nav {
            folder: s(&["Filozofia", "Stoicy"]),
            ..Nav::default()
        };
        n.up();
        assert_eq!(n.folder, s(&["Filozofia"]));
        n.series = Some("Trylogia".into());
        n.up();
        assert_eq!(n.folder, s(&["Filozofia"]), "leaving a series first");
        n.up();
        assert!(n.at_home());
    }

    #[test]
    fn folder_of_paths() {
        assert_eq!(folder_of("a/b/c.epub"), ["a", "b"]);
        assert!(folder_of("c.epub").is_empty());
    }
}
