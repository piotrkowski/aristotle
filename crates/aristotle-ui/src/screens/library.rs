//! Library home.
//!
//! One job: get back into a book, or pick the next one. So the screen holds
//! the current book large at the top, then the shelf as a paged list with
//! filters and a sort order. There is no store, no recommendations, no
//! reading lists and no "more for you": nothing on the screen exists to
//! sell something.

use super::icon_button;
use crate::geom::{mm, pt, Insets, MIN_TARGET};
use crate::icons::Icon;
use crate::text::{Align, Paragraph, TextEngine};
use crate::theme::{margin, Styles, INK, INK_2, INK_3, RULE};
use crate::tree::{pages, progress, ActionId, Cross, Feedback, Kind, Node, NodeId, Tree};
use crate::widgets::{Battery, Cover};

#[derive(Clone, Debug)]
pub struct Book {
    pub title: String,
    pub author: String,
    /// 0.0 is unread, 1.0 finished.
    pub progress: f32,
    pub chapter: String,
    pub minutes_left: u32,
}

impl Book {
    fn new(title: &str, author: &str, progress: f32) -> Self {
        Self {
            title: title.into(),
            author: author.into(),
            progress,
            chapter: String::new(),
            minutes_left: 0,
        }
    }
}

/// A plausible shelf, most recently opened first.
pub fn sample_books() -> Vec<Book> {
    let mut lalka = Book::new("Lalka", "Bolesław Prus", 0.43);
    lalka.chapter = "Chapter VII · Idylla".into();
    lalka.minutes_left = 742;
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

    fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Reading => "Reading",
            Filter::Unread => "Unread",
            Filter::Finished => "Finished",
        }
    }

    fn accepts(self, b: &Book) -> bool {
        match self {
            Filter::All => true,
            Filter::Reading => b.progress > 0.0 && b.progress < 1.0,
            Filter::Unread => b.progress == 0.0,
            Filter::Finished => b.progress >= 1.0,
        }
    }
}

pub mod action {
    use crate::tree::ActionId;
    pub const RESUME: ActionId = 1;
    pub const SEARCH: ActionId = 2;
    pub const MORE: ActionId = 3;
    pub const SORT: ActionId = 4;
    pub const PREV_PAGE: ActionId = 5;
    pub const NEXT_PAGE: ActionId = 6;
    /// `FILTER + i` selects `Filter::ALL[i]`.
    pub const FILTER: ActionId = 10;
    /// `BOOK + i` opens the i-th book of the shelf.
    pub const BOOK: ActionId = 100;
}

/// Handles to the nodes the library updates in place.
pub struct Library {
    pub list: NodeId,
    pub pager_label: NodeId,
    pub prev: NodeId,
    pub next: NodeId,
    pub filter: Filter,
    /// Node of each shelf row, for tests and the preview.
    pub rows: Vec<NodeId>,
}

const ROW_COVER: (i32, i32) = (76, 112);
const HERO_COVER: (i32, i32) = (190, 285);

impl Library {
    /// Builds the library tree. `books[0]` is the book being read.
    pub fn build(
        text: &mut TextEngine,
        s: &Styles,
        books: &[Book],
        filter: Filter,
    ) -> (Tree, Self) {
        let mut t = Tree::new();
        let fam = s.family;

        // Status line: the time and the battery, nothing else.
        let time = t.label("21:38", s.small());
        let bat = t.custom(Battery { level: 0.82 });
        let pct = t.label("82%", s.small());
        let gap = t.gap(12, 1);
        let fill = t.spacer(1);
        let status = t.add(Node::row(0), [time, fill, bat, gap, pct]);
        let status = t.fixed(None, Some(mm(5.5)), Some(status));

        // Header.
        let title = t.label("Library", s.display());
        let fill = t.spacer(1);
        let search = icon_button(&mut t, Icon::Search, 52, action::SEARCH);
        let more = icon_button(&mut t, Icon::More, 52, action::MORE);
        let header = t.add(Node::row(0), [title, fill, search, more]);

        // The book being read, large.
        let hero = Self::hero(&mut t, s, &books[0], fam);

        // Filters and sort.
        let mut tabs = Vec::new();
        for (i, f) in Filter::ALL.iter().enumerate() {
            let selected = *f == filter;
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
            let col = t.pad(Insets::new(14, 18, 0, 18), col);
            tabs.push(t.button(Node::plain(), col, action::FILTER + i as ActionId));
        }
        let fill = t.spacer(1);
        let sort_label = t.label("Recent", s.ui());
        let chevron = t.icon(Icon::ChevronDown, 34, INK);
        let g = t.gap(6, 1);
        let sort = t.add(Node::row(0), [sort_label, g, chevron]);
        let sort = t.pad(Insets::vh(0, 12), sort);
        let sort = t.button(Node::plain(), sort, action::SORT);
        let mut filters = tabs;
        filters.push(fill);
        filters.push(sort);
        let filters = t.add(Node::row(0), filters);
        let rule = t.rule(2, RULE);

        // The shelf: everything but the current book, one page at a time.
        let mut rows = Vec::new();
        for (i, b) in books.iter().enumerate().skip(1) {
            if filter.accepts(b) {
                rows.push(Self::row(&mut t, s, b, fam, action::BOOK + i as ActionId));
            }
        }
        let list = t.add(pages(0).flex(1), rows.clone());

        // Pager.
        let prev = icon_button(&mut t, Icon::ChevronLeft, 48, action::PREV_PAGE);
        let next = icon_button(&mut t, Icon::ChevronRight, 48, action::NEXT_PAGE);
        let pager_label = t.text(Paragraph::plain("Page 1 of 1", s.small(), Align::Center));
        let pager_label = t.add(Node::column(0).flex(1), [pager_label]);
        let pager = t.add(Node::row(0), [prev, pager_label, next]);

        let g1 = t.gap(1, 8);
        let g2 = t.gap(1, 20);
        let col = t.add(
            Node::column(0),
            [status, header, g1, hero, g2, filters, rule, list, pager],
        );
        let root = t.pad(Insets::new(0, margin(), mm(2.0), margin()), col);
        t.set_root(root);
        let lib = Self {
            list,
            pager_label: t.node(pager_label).children[0],
            prev: t.node(prev).children[0],
            next: t.node(next).children[0],
            filter,
            rows,
        };
        (t, lib)
    }

    fn hero(t: &mut Tree, s: &Styles, b: &Book, fam: crate::text::FamilyId) -> NodeId {
        let cover = t.custom(Cover::new(&b.title, &b.author, fam));
        let cover = t.fixed(Some(HERO_COVER.0), Some(HERO_COVER.1), Some(cover));
        let caps = t.label("Continue reading", s.caps());
        let mut tp = Paragraph::plain(&b.title, s.title().size(pt(13.0)), Align::Left);
        tp.style.max_lines = Some(2);
        let title = t.text(tp);
        let author = t.label(&b.author, s.ui().gray(INK_2));
        let fill = t.spacer(1);
        let chapter = t.label(&b.chapter, s.small());
        let bar = t.add(progress(b.progress, 12), []);
        let h = b.minutes_left / 60;
        let m = b.minutes_left % 60;
        let left = format!("{:.0}% · about {h} h {m:02} min left", b.progress * 100.0);
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
        let cover = t.custom(Cover::new(&b.title, &b.author, fam));
        let cover = t.fixed(Some(ROW_COVER.0), Some(ROW_COVER.1), Some(cover));
        let title_style = s.ui_bold().size(pt(9.5)).line_height(1.15);
        let mut tp = Paragraph::plain(&b.title, title_style, Align::Left);
        tp.style.max_lines = Some(1);
        let title = t.text(tp);
        let author = t.label(&b.author, s.small().line_height(1.15));
        let status = if b.progress <= 0.0 {
            t.label("New", s.caps().gray(INK))
        } else if b.progress >= 1.0 {
            let check = t.icon(Icon::Check, 30, INK_2);
            let l = t.label("Finished", s.small());
            let g = t.gap(8, 1);
            t.add(Node::row(0), [check, g, l])
        } else {
            let bar = t.add(progress(b.progress, 12), []);
            let bar = t.fixed(Some(200), None, Some(bar));
            let pct = t.label(&format!("{:.0}%", b.progress * 100.0), s.small());
            let g = t.gap(16, 1);
            t.add(Node::row(0), [bar, g, pct])
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
        t.set_text(self.pager_label, &format!("Page {} of {}", page + 1, count));
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
