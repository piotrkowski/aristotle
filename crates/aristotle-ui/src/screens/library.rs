//! Library home.
//!
//! One job: get back into a book, or pick the next one. So the screen holds
//! the current book large at the top, then the shelf as a paged list with
//! filters and a sort order. There is no store, no recommendations, no
//! reading lists and no "more for you": nothing on the screen exists to
//! sell something.
//!
//! Labels are Polish: this is the text the owner reads on the device.

use super::icon_button;
use crate::geom::{mm, pt, Insets, MIN_TARGET};
use crate::icons::Icon;
use crate::text::{Align, Paragraph, TextEngine};
use crate::theme::{margin, Styles, INK, INK_2, INK_3, RULE};
use crate::tree::{pages, progress, ActionId, Cross, Feedback, Kind, Node, NodeId, Tree};
use crate::widgets::{Battery, Cover, GrayImage};

/// Reading state of a book, as the library index keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Unread,
    Reading,
    Finished,
}

#[derive(Clone, Debug)]
pub struct Book {
    pub title: String,
    pub author: String,
    /// 0.0 is the start, 1.0 the end.
    pub progress: f32,
    pub status: Status,
    /// Where the reader is, in words: a chapter title or a printed page
    /// ("Strona 4 z 180"). Shown on the "continue reading" card; may be empty.
    pub chapter: String,
    /// Reading time left, when known.
    pub minutes_left: Option<u32>,
    /// The cover thumbnail, if the book has one; otherwise a typographic
    /// cover is drawn.
    pub cover: Option<GrayImage>,
}

impl Book {
    pub fn new(title: &str, author: &str, progress: f32) -> Self {
        let status = if progress >= 1.0 {
            Status::Finished
        } else if progress > 0.0 {
            Status::Reading
        } else {
            Status::Unread
        };
        Self {
            title: title.into(),
            author: author.into(),
            progress,
            status,
            chapter: String::new(),
            minutes_left: None,
            cover: None,
        }
    }
}

/// A plausible shelf, most recently opened first.
pub fn sample_books() -> Vec<Book> {
    let mut lalka = Book::new("Lalka", "Bolesław Prus", 0.43);
    lalka.chapter = "Rozdział VII · Idylla".into();
    lalka.minutes_left = Some(742);
    vec![
        lalka,
        Book::new("Pan Tadeusz", "Adam Mickiewicz", 0.02),
        Book::new("Duma i uprzedzenie", "Jane Austen", 0.01),
        Book::new("Nad Niemnem", "Eliza Orzeszkowa", 0.61),
        Book::new("Moby Dick", "Herman Melville", 0.0),
        Book::new("Zbrodnia i kara", "Fiodor Dostojewski", 0.18),
        Book::new("Pan Tadeusz", "Adam Mickiewicz", 1.0),
        Book::new("Przedwiośnie", "Stefan Żeromski", 0.0),
        Book::new("Sklepy cynamonowe", "Bruno Schulz", 1.0),
        Book::new("Faraon", "Bolesław Prus", 0.0),
        Book::new("Ogniem i mieczem", "Henryk Sienkiewicz", 0.88),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Reading,
    Unread,
    Finished,
}

impl Filter {
    pub const ALL: [Filter; 4] = [
        Filter::All,
        Filter::Reading,
        Filter::Unread,
        Filter::Finished,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "Wszystkie",
            Filter::Reading => "Czytane",
            Filter::Unread => "Nieprzeczytane",
            Filter::Finished => "Przeczytane",
        }
    }

    pub fn accepts(self, b: &Book) -> bool {
        match self {
            Filter::All => true,
            Filter::Reading => b.status == Status::Reading,
            Filter::Unread => b.status == Status::Unread,
            Filter::Finished => b.status == Status::Finished,
        }
    }
}

pub mod action {
    use crate::tree::ActionId;
    pub const RESUME: ActionId = 1;
    pub const SORT: ActionId = 4;
    pub const PREV_PAGE: ActionId = 5;
    pub const NEXT_PAGE: ActionId = 6;
    /// `FILTER + i` selects `Filter::ALL[i]`.
    pub const FILTER: ActionId = 10;
    /// `BOOK + i` opens `Shelf::books[i]`.
    pub const BOOK: ActionId = 100;
}

/// Everything the library screen shows.
pub struct Shelf<'a> {
    /// The book to continue, shown large at the top.
    pub hero: Option<&'a Book>,
    /// The shelf below it; `action::BOOK + i` opens `books[i]`.
    pub books: &'a [Book],
    pub filter: Filter,
    /// Name of the current sort order, shown on the sort button.
    pub sort: &'a str,
    /// Wall-clock time for the status line, e.g. "21:38".
    pub clock: &'a str,
    /// Battery charge from 0 to 1, when known.
    pub battery: Option<f32>,
}

/// Handles to the nodes the library updates in place.
pub struct Library {
    pub list: NodeId,
    pub pager_label: NodeId,
    pub prev: NodeId,
    pub next: NodeId,
    /// The screen title. Holding it quits the app on the device.
    pub title: NodeId,
    pub filter: Filter,
    /// Node of each shelf row, for tests and the preview.
    pub rows: Vec<NodeId>,
}

const ROW_COVER: (i32, i32) = (76, 112);
const HERO_COVER: (i32, i32) = (190, 285);

/// "Strona 2 z 5": the pager and reader position label.
pub fn page_of(page: usize, count: usize) -> String {
    format!("Strona {page} z {count}")
}

impl Library {
    /// The prototype entry point: `books[0]` is the book being read, the
    /// rest is the shelf, with sample clock and battery.
    pub fn build(
        text: &mut TextEngine,
        s: &Styles,
        books: &[Book],
        filter: Filter,
    ) -> (Tree, Self) {
        let shelf = Shelf {
            hero: books.first(),
            books: books.get(1..).unwrap_or(&[]),
            filter,
            sort: "Ostatnie",
            clock: "21:38",
            battery: Some(0.82),
        };
        Self::build_shelf(text, s, &shelf)
    }

    /// Builds the library tree for `shelf`.
    pub fn build_shelf(text: &mut TextEngine, s: &Styles, shelf: &Shelf) -> (Tree, Self) {
        let mut t = Tree::new();
        let fam = s.family;

        // Status line: the time and the battery, nothing else.
        let time = t.label(shelf.clock, s.small());
        let fill = t.spacer(1);
        let mut status = vec![time, fill];
        if let Some(level) = shelf.battery {
            let bat = t.custom(Battery { level });
            let gap = t.gap(12, 1);
            let pct = t.label(&format!("{:.0}%", level * 100.0), s.small());
            status.extend([bat, gap, pct]);
        }
        let status = t.add(Node::row(0), status);
        let status = t.fixed(None, Some(mm(5.5)), Some(status));

        // Header: the title and the sort order.
        let title = t.label("Biblioteka", s.display());
        let fill = t.spacer(1);
        let sort_label = t.label(shelf.sort, s.ui());
        let chevron = t.icon(Icon::ChevronDown, 34, INK);
        let g = t.gap(6, 1);
        let sort = t.add(Node::row(0), [sort_label, g, chevron]);
        let sort = t.pad(Insets::vh(0, 12), sort);
        let sort = t.button(Node::plain(), sort, action::SORT);
        let header = t.add(Node::row(0), [title, fill, sort]);

        let mut column = vec![status, header];
        if let Some(hero) = shelf.hero {
            let g = t.gap(1, 8);
            let h = Self::hero(&mut t, s, hero, fam);
            column.extend([g, h]);
        }
        let g = t.gap(1, 20);
        column.push(g);

        // Filters.
        let mut tabs = Vec::new();
        for (i, f) in Filter::ALL.iter().enumerate() {
            let selected = *f == shelf.filter;
            let style = if selected {
                s.ui_bold()
            } else {
                s.ui().gray(INK_2)
            };
            let w = text.measure(f.label(), &style).ceil() as i32;
            let label = t.label(f.label(), style);
            let line = t.add(
                Node::new(Kind::Fixed {
                    w: Some(w),
                    h: Some(5),
                })
                .bg(if selected { INK } else { 255 }),
                [],
            );
            let col = t.add(Node::column(8).cross(Cross::Center), [label, line]);
            let col = t.pad(Insets::new(14, 14, 0, 14), col);
            tabs.push(t.button(Node::plain(), col, action::FILTER + i as ActionId));
        }
        let fill = t.spacer(1);
        tabs.push(fill);
        let filters = t.add(Node::row(0), tabs);
        let rule = t.rule(2, RULE);

        // The shelf, one page at a time.
        let mut rows = Vec::new();
        for (i, b) in shelf.books.iter().enumerate() {
            if shelf.filter.accepts(b) {
                rows.push(Self::row(&mut t, s, b, fam, action::BOOK + i as ActionId));
            }
        }
        let mut children = rows.clone();
        if rows.is_empty() {
            let msg = if shelf.books.is_empty() && shelf.hero.is_none() {
                "Brak książek. Skopiuj je do folderu documents."
            } else {
                "Brak książek w tym widoku."
            };
            let l = t.text(Paragraph::plain(msg, s.ui().gray(INK_2), Align::Center));
            children.push(t.pad(Insets::vh(mm(8.0), 0), l));
        }
        let list = t.add(pages(0).flex(1), children);

        // Pager.
        let prev = icon_button(&mut t, Icon::ChevronLeft, 48, action::PREV_PAGE);
        let next = icon_button(&mut t, Icon::ChevronRight, 48, action::NEXT_PAGE);
        let pager_label = t.text(Paragraph::plain(page_of(1, 1), s.small(), Align::Center));
        let pager_label = t.add(Node::column(0).flex(1), [pager_label]);
        let pager = t.add(Node::row(0), [prev, pager_label, next]);

        column.extend([filters, rule, list, pager]);
        let col = t.add(Node::column(0), column);
        let root = t.pad(Insets::new(0, margin(), mm(2.0), margin()), col);
        t.set_root(root);
        let lib = Self {
            list,
            pager_label: t.node(pager_label).children[0],
            prev: t.node(prev).children[0],
            next: t.node(next).children[0],
            title,
            filter: shelf.filter,
            rows,
        };
        (t, lib)
    }

    fn hero(t: &mut Tree, s: &Styles, b: &Book, fam: crate::text::FamilyId) -> NodeId {
        let cover = t.custom(Cover::new(&b.title, &b.author, fam).with_image(b.cover.clone()));
        let cover = t.fixed(Some(HERO_COVER.0), Some(HERO_COVER.1), Some(cover));
        let caps = t.label("Czytaj dalej", s.caps());
        let mut tp = Paragraph::plain(&b.title, s.title().size(pt(13.0)), Align::Left);
        tp.style.max_lines = Some(2);
        let title = t.text(tp);
        let mut ap = Paragraph::plain(&b.author, s.ui().gray(INK_2), Align::Left);
        ap.style.max_lines = Some(1);
        let author = t.text(ap);
        let fill = t.spacer(1);
        let mut cp = Paragraph::plain(&b.chapter, s.small(), Align::Left);
        cp.style.max_lines = Some(1);
        let chapter = t.text(cp);
        let bar = t.add(progress(b.progress, 12), []);
        let pct = (b.progress * 100.0).floor();
        let left = match b.minutes_left {
            Some(min) => {
                let (h, m) = (min / 60, min % 60);
                format!("{pct:.0}% · zostało ok. {h} godz. {m:02} min")
            }
            None => format!("{pct:.0}%"),
        };
        let left = t.label(&left, s.small().gray(INK_3));
        let g = t.gap(1, 4);
        let info = t.add(
            Node::column(10).flex(1),
            [caps, title, author, fill, chapter, g, bar, left],
        );
        let row = t.add(Node::row(44).cross(Cross::Stretch), [cover, info]);
        let row = t.fixed(None, Some(HERO_COVER.1), Some(row));
        let row = t.pad(Insets::vh(22, 0), row);
        t.add(
            Node::new(Kind::Padding(Insets::all(0)))
                .action(action::RESUME)
                .feedback(Feedback::Outline),
            [row],
        )
    }

    fn row(t: &mut Tree, s: &Styles, b: &Book, fam: crate::text::FamilyId, id: ActionId) -> NodeId {
        let cover = t.custom(Cover::new(&b.title, &b.author, fam).with_image(b.cover.clone()));
        let cover = t.fixed(Some(ROW_COVER.0), Some(ROW_COVER.1), Some(cover));
        let title_style = s.ui_bold().size(pt(9.5)).line_height(1.15);
        let mut tp = Paragraph::plain(&b.title, title_style, Align::Left);
        tp.style.max_lines = Some(1);
        let title = t.text(tp);
        let mut ap = Paragraph::plain(&b.author, s.small().line_height(1.15), Align::Left);
        ap.style.max_lines = Some(1);
        let author = t.text(ap);
        let status = match b.status {
            Status::Unread => t.label("Nowa", s.caps().gray(INK)),
            Status::Finished => {
                let check = t.icon(Icon::Check, 30, INK_2);
                let l = t.label("Przeczytana", s.small());
                let g = t.gap(8, 1);
                t.add(Node::row(0), [check, g, l])
            }
            Status::Reading => {
                let bar = t.add(progress(b.progress, 12), []);
                let bar = t.fixed(Some(200), None, Some(bar));
                let pct = (b.progress * 100.0).floor();
                let pct = t.label(&format!("{pct:.0}%"), s.small());
                let g = t.gap(16, 1);
                t.add(Node::row(0), [bar, g, pct])
            }
        };
        let g = t.gap(1, 4);
        let info = t.add(
            Node::column(2).cross(Cross::Start).flex(1),
            [title, author, g, status],
        );
        let row = t.add(Node::row(32), [cover, info]);
        let row = t.pad(Insets::vh(14, 0), row);
        let line = t.rule(1, RULE);
        let item = t.add(Node::column(0), [row, line]);
        t.add(
            Node::new(Kind::MinSize(crate::geom::Size::new(0, MIN_TARGET)))
                .action(id)
                .feedback(Feedback::Outline),
            [item],
        )
    }

    /// Updates the pager after layout: page label and arrow states.
    pub fn sync(&self, t: &mut Tree) {
        let (page, count) = t.page(self.list);
        t.set_text(self.pager_label, &page_of(page + 1, count.max(1)));
        t.set_icon_gray(self.prev, if page == 0 { RULE } else { INK });
        t.set_icon_gray(self.next, if page + 1 >= count { RULE } else { INK });
    }

    /// Applies an action that stays within this screen. Returns true if it
    /// was handled.
    pub fn handle(&mut self, t: &mut Tree, id: ActionId) -> bool {
        let (page, _) = t.page(self.list);
        match id {
            action::PREV_PAGE => {
                t.set_page(self.list, page.saturating_sub(1));
            }
            action::NEXT_PAGE => {
                t.set_page(self.list, page + 1);
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{SCREEN_HEIGHT, SCREEN_WIDTH};
    use crate::text::{FamilyId, Fonts};
    use crate::ui::Ui;

    fn ui_for(shelf: &Shelf) -> (Ui, Library) {
        let mut text = TextEngine::new(Fonts::embedded());
        let styles = Styles::new(FamilyId(0));
        let (tree, lib) = Library::build_shelf(&mut text, &styles, shelf);
        let mut ui = Ui::new(tree, text, SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
        ui.layout();
        lib.sync(&mut ui.tree);
        ui.render();
        (ui, lib)
    }

    #[test]
    fn filters_follow_the_status() {
        let books = sample_books();
        let shelf = |filter| Shelf {
            hero: None,
            books: &books,
            filter,
            sort: "Ostatnie",
            clock: "12:00",
            battery: None,
        };
        let count = |f: Filter| books.iter().filter(|b| f.accepts(b)).count();
        assert_eq!(count(Filter::All), books.len());
        assert_eq!(
            count(Filter::Reading) + count(Filter::Unread) + count(Filter::Finished),
            books.len()
        );
        let (_, lib) = ui_for(&shelf(Filter::Finished));
        assert_eq!(lib.rows.len(), 2);
    }

    #[test]
    fn empty_library_still_builds_with_one_page() {
        let shelf = Shelf {
            hero: None,
            books: &[],
            filter: Filter::All,
            sort: "Ostatnie",
            clock: "12:00",
            battery: Some(0.5),
        };
        let (ui, lib) = ui_for(&shelf);
        assert!(lib.rows.is_empty());
        assert_eq!(ui.tree.page(lib.list).1, 1);
    }

    #[test]
    fn real_covers_are_drawn() {
        let mut books = sample_books();
        // A black cover: the hero's cover box must turn dark.
        books[0].cover = Some(GrayImage::new(200, 300, vec![0; 200 * 300]));
        let shelf = Shelf {
            hero: Some(&books[0]),
            books: &books[1..],
            filter: Filter::All,
            sort: "Ostatnie",
            clock: "12:00",
            battery: None,
        };
        let (ui, _) = ui_for(&shelf);
        let dark = (0..SCREEN_HEIGHT)
            .step_by(4)
            .filter(|&y| ui.screen().get(margin() + 95, y) < 40)
            .count();
        assert!(dark > 50, "{dark}");
    }
}
