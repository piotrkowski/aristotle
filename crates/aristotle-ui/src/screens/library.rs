// SPDX-License-Identifier: AGPL-3.0-or-later
//! Library home, and the folder and series shelves under it.
//!
//! One job: get back into a book, or find the next one. So the home screen
//! holds the book being read at the top, then the library in three views:
//!
//! - **Books**: every book, narrowed by a shelf chip: All, Reading (being
//!   read), New (never opened), Finished.
//! - **Folders**: the folders of `documents/` as they are on the device
//!   (author folders such as "Mickiewicz, Adam", topics such as
//!   "Filozofia"), then the books lying loose at the top. A folder opens as
//!   its own shelf, with subfolders first.
//! - **Series**: one shelf per series, its books in series order.
//!
//! Lists turn pages, never scroll, and say where you are ("Page 1 of 3").
//! There is no store, no recommendations, no reading lists and no "more
//! for you": nothing on the screen exists to sell something.
//!
//! Covers are optional ([`CoverMode`]): none by default, a clean list of
//! titles; typographic cards (`widgets::Cover`); or the books' own covers.
//!
//! Labels come from `aristotle-i18n`, in the UI language.

use super::icon_button;
use crate::components::{chip, CHIP_PADDING_X};
use crate::geom::{mm, Insets, MIN_TARGET};
use crate::icons::Icon;
use crate::text::{Align, Paragraph, TextEngine};
use crate::theme::{margin, IconSize, Space, Styles, INK, INK_2, INK_3, RULE};
use crate::tree::{pages, progress, ActionId, Cross, Feedback, Kind, Node, NodeId, Tree};
use crate::widgets::{series_number, Battery, BusyMark, Cover, CoverMark, GrayImage};
use aristotle_i18n::{fmt_duration, tf, tr, trn, Msg, Plural};

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
    /// ("Page 4 of 180"). Shown on the "continue reading" card; may be empty.
    pub chapter: String,
    /// Reading time left, when known.
    pub minutes_left: Option<u32>,
    /// The cover thumbnail, if the book has one and photo covers are on.
    pub cover: Option<GrayImage>,
    /// Series and shelf, for the typographic card and the list.
    pub series: Option<(String, f32)>,
    /// The folder the book lies in (relative to `documents/`), if any.
    pub folder: Option<String>,
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
            series: None,
            folder: None,
        }
    }

    pub fn in_folder(mut self, folder: &str) -> Self {
        self.folder = Some(folder.into());
        self
    }

    pub fn in_series(mut self, name: &str, index: f32) -> Self {
        self.series = Some((name.into(), index));
        self
    }

    /// What the typographic card shows: the series, else the folder,
    /// decides the binding.
    pub fn mark(&self) -> CoverMark {
        CoverMark {
            group: self
                .series
                .as_ref()
                .map(|(s, _)| s.clone())
                .or_else(|| self.folder.clone()),
            series_index: self.series.as_ref().map(|(_, i)| *i),
        }
    }
}

/// A plausible library of public-domain books, most recently opened first:
/// author folders and topic folders, one series.
pub fn sample_books() -> Vec<Book> {
    let mut lalka = Book::new("Lalka", "Bolesław Prus", 0.43).in_folder("Prus, Bolesław");
    lalka.chapter = "Rozdział VII · Idylla".into();
    lalka.minutes_left = Some(742);
    vec![
        lalka,
        Book::new("Pan Tadeusz", "Adam Mickiewicz", 0.02).in_folder("Mickiewicz, Adam"),
        Book::new("Duma i uprzedzenie", "Jane Austen", 0.01).in_folder("Powieści"),
        Book::new("Państwo", "Platon", 0.61).in_folder("Platon"),
        Book::new("Dziady", "Adam Mickiewicz", 0.0).in_folder("Mickiewicz, Adam"),
        Book::new("Obrona Sokratesa", "Platon", 1.0).in_folder("Platon"),
        Book::new("Ogniem i mieczem", "Henryk Sienkiewicz", 0.88)
            .in_series("Trylogia", 1.0)
            .in_folder("Sienkiewicz, Henryk"),
        Book::new("Potop", "Henryk Sienkiewicz", 0.0)
            .in_series("Trylogia", 2.0)
            .in_folder("Sienkiewicz, Henryk"),
        Book::new("Rozmyślania", "Marek Aureliusz", 0.0).in_folder("Filozofia"),
        Book::new("Fedon", "Platon", 0.18).in_folder("Platon"),
        Book::new("Moby Dick", "Herman Melville", 0.0),
    ]
}

/// The home screen's views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Books,
    Folders,
    Series,
}

impl View {
    pub const ALL: [View; 3] = [View::Books, View::Folders, View::Series];

    pub fn label(self) -> &'static str {
        tr(match self {
            View::Books => Msg::LibraryViewBooks,
            View::Folders => Msg::LibraryViewFolders,
            View::Series => Msg::LibraryViewSeries,
        })
    }
}

/// The shelves of the books view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Reading,
    New,
    Finished,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::Reading, Filter::New, Filter::Finished];

    pub fn label(self) -> &'static str {
        tr(match self {
            Filter::All => Msg::LibraryFilterAll,
            Filter::Reading => Msg::LibraryFilterReading,
            Filter::New => Msg::LibraryFilterNew,
            Filter::Finished => Msg::LibraryFilterFinished,
        })
    }

    pub fn accepts(self, b: &Book) -> bool {
        match self {
            Filter::All => true,
            Filter::Reading => b.status == Status::Reading,
            Filter::New => b.status == Status::Unread,
            Filter::Finished => b.status == Status::Finished,
        }
    }
}

/// How books are pictured in lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CoverMode {
    /// Text only: the calmest list, and the default.
    #[default]
    None,
    /// Typographic cards.
    Cards,
    /// The books' own covers where they have one, cards otherwise.
    Photos,
}

impl CoverMode {
    pub const ALL: [CoverMode; 3] = [CoverMode::None, CoverMode::Cards, CoverMode::Photos];

    pub fn label(self) -> &'static str {
        tr(match self {
            CoverMode::None => Msg::CoversNone,
            CoverMode::Cards => Msg::CoversCards,
            CoverMode::Photos => Msg::CoversPhotos,
        })
    }
}

/// One line of a list.
#[derive(Clone, Debug)]
pub enum Item {
    Book(Book),
    Folder {
        name: String,
        books: usize,
        reading: usize,
    },
    Series {
        name: String,
        books: usize,
        finished: usize,
    },
}

/// Where the list is: the home screen with its views, or one shelf.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place<'a> {
    Home { view: View, filter: Filter },
    Folder { name: &'a str },
    Series { name: &'a str },
}

pub mod action {
    use crate::tree::ActionId;
    pub const RESUME: ActionId = 1;
    pub const SORT: ActionId = 4;
    pub const PREV_PAGE: ActionId = 5;
    pub const NEXT_PAGE: ActionId = 6;
    pub const SETTINGS: ActionId = 7;
    /// Up from a folder or series shelf.
    pub const BACK: ActionId = 8;
    /// `FILTER + i` selects `Filter::ALL[i]`.
    pub const FILTER: ActionId = 10;
    /// `VIEW + i` selects `View::ALL[i]`.
    pub const VIEW: ActionId = 20;
    /// `ITEM + i` opens `Shelf::items[i]`.
    pub const ITEM: ActionId = 100;
}

/// The status line: the time and the battery, nothing else.
#[derive(Clone, Copy, Debug, Default)]
pub struct StatusLine<'a> {
    /// Wall-clock time, e.g. "21:38".
    pub clock: &'a str,
    /// Battery charge from 0 to 1, when known.
    pub battery: Option<f32>,
    pub charging: bool,
}

/// Everything a library screen shows.
pub struct Shelf<'a> {
    pub place: Place<'a>,
    /// The book to continue, shown large on the home screen.
    pub hero: Option<&'a Book>,
    /// The list; `action::ITEM + i` opens `items[i]`. In the books view
    /// the screen applies the filter itself.
    pub items: &'a [Item],
    /// Name of the current sort order, shown on the sort button; empty
    /// hides the button (a series keeps its own order).
    pub sort: &'a str,
    pub status: StatusLine<'a>,
    pub covers: CoverMode,
}

/// Handles to the nodes the library updates in place.
pub struct Library {
    pub list: NodeId,
    pub pager_label: NodeId,
    pub prev: NodeId,
    pub next: NodeId,
    /// The screen title. Holding it quits the app on the device.
    pub title: NodeId,
    /// The busy mark in the status line, hidden.
    pub busy: NodeId,
    /// Node of each list row, for tests and the preview.
    pub rows: Vec<NodeId>,
}

const ROW_COVER: (i32, i32) = (72, 106);
const HERO_COVER: (i32, i32) = (156, 234);

/// Touch-down feedback for a book row or the book to continue. Inverting
/// is the clearest signal and suits text and typographic cards (a card
/// simply shows its inverse binding), but it would turn a cover photo into
/// a negative; with photos the row gets a heavy frame instead.
fn press_feedback(covers: CoverMode) -> Feedback {
    if covers == CoverMode::Photos {
        Feedback::Outline
    } else {
        Feedback::Invert
    }
}

/// "Page 2 of 5": the pager and reader position label.
pub fn page_of(page: usize, count: usize) -> String {
    tf!(Msg::CommonPageOf, page = page, count = count)
}

/// "3 books".
pub fn books_label(n: usize) -> String {
    trn(Plural::LibraryBooks, n)
}

/// The status line node, and its busy mark (hidden; show it with
/// `Tree::set_hidden` before a slow action, see `docs/ui-framework.md`,
/// "Transitions").
pub fn status_line(t: &mut Tree, s: &Styles, line: &StatusLine) -> (NodeId, NodeId) {
    let busy = t.add(Node::new(Kind::Custom(Box::new(BusyMark))).hidden(true), []);
    let busy_c = t.align(0.5, 0.5, busy);
    let time = t.label(line.clock, s.small());
    let fill = t.spacer(1);
    let mut status = vec![time, fill];
    if let Some(level) = line.battery {
        let pct = t.label(&format!("{:.0}%", level * 100.0), s.small());
        let gap = t.gap(Space::S, 1);
        let bat = t.custom(Battery {
            level,
            charging: line.charging,
        });
        status.extend([pct, gap, bat]);
    }
    let status = t.add(Node::row(0), status);
    let both = t.add(Node::new(Kind::Stack), [status, busy_c]);
    (t.fixed(None, Some(mm(5.5)), Some(both)), busy)
}

/// Underlined text tabs.
fn tabs(
    t: &mut Tree,
    text: &mut TextEngine,
    s: &Styles,
    labels: &[(&str, bool, ActionId)],
) -> Vec<NodeId> {
    let mut out = Vec::new();
    for &(label, selected, id) in labels {
        let style = if selected {
            s.ui_bold()
        } else {
            s.ui().gray(INK_2)
        };
        let w = text.measure(label, &style).ceil() as i32;
        let l = t.label(label, style);
        let line = t.add(
            Node::new(Kind::Fixed {
                w: Some(w),
                h: Some(s.density.px(5)),
            })
            .in_pixels()
            .bg(if selected { INK } else { 255 }),
            [],
        );
        let col = t.add(Node::column(Space::XS).cross(Cross::Center), [l, line]);
        let col = t.pad(Insets::new(Space::S, Space::S, 0, Space::S), col);
        out.push(t.button(Node::plain(), col, id));
    }
    out
}

impl Library {
    /// The prototype entry point: `books[0]` is the book being read, the
    /// rest is the books view, with a sample clock and battery.
    pub fn build(
        text: &mut TextEngine,
        s: &Styles,
        books: &[Book],
        filter: Filter,
    ) -> (Tree, Self) {
        let items: Vec<Item> = books.iter().skip(1).cloned().map(Item::Book).collect();
        let shelf = Shelf {
            place: Place::Home {
                view: View::Books,
                filter,
            },
            hero: books.first(),
            items: &items,
            sort: tr(Msg::LibrarySortRecent),
            status: StatusLine {
                clock: "21:38",
                battery: Some(0.82),
                charging: false,
            },
            covers: CoverMode::None,
        };
        Self::build_shelf(text, s, &shelf)
    }

    /// Builds the library tree for `shelf`.
    pub fn build_shelf(text: &mut TextEngine, s: &Styles, shelf: &Shelf) -> (Tree, Self) {
        Self::build_shelf_for_width(text, s, shelf, crate::geom::SCREEN_WIDTH)
    }

    /// Wraps shelf filters to keep their labels visible at the supplied width.
    pub fn build_shelf_for_width(
        text: &mut TextEngine,
        s: &Styles,
        shelf: &Shelf,
        width: i32,
    ) -> (Tree, Self) {
        let mut t = Tree::new();
        let (status, busy) = status_line(&mut t, s, &shelf.status);
        let mut column = vec![status];

        // The sort order: next to the views at home, in the header of a
        // shelf.
        let sort_label = t.label(shelf.sort, s.ui());
        let chevron = t.icon(Icon::ChevronDown, IconSize::SMALL, INK);
        let g = t.gap(Space::XS, 1);
        let sort = t.add(Node::row(0), [sort_label, g, chevron]);
        let sort = t.pad(Insets::vh(0, Space::S), sort);
        let sort = t.button(Node::plain(), sort, action::SORT);

        // Header: the title with settings, or back and the shelf's name.
        let title;
        match shelf.place {
            Place::Home { .. } => {
                title = t.label(tr(Msg::LibraryTitle), s.display());
                let fill = t.spacer(1);
                let settings =
                    icon_button(&mut t, Icon::Settings, IconSize::STANDARD, action::SETTINGS);
                let header = t.add(Node::row(0), [title, fill, settings]);
                column.push(header);
            }
            Place::Folder { name } | Place::Series { name } => {
                let back = icon_button(&mut t, Icon::ArrowLeft, IconSize::STANDARD, action::BACK);
                let mut p = Paragraph::plain(name, s.headline(), Align::Left);
                p.style.max_lines = Some(2);
                title = t.text(p);
                let kind = tr(if matches!(shelf.place, Place::Folder { .. }) {
                    Msg::LibraryFolder
                } else {
                    Msg::LibrarySeries
                });
                let count = shelf
                    .items
                    .iter()
                    .filter(|i| matches!(i, Item::Book(_)))
                    .count();
                let sub = t.label(&format!("{kind} · {}", books_label(count)), s.small());
                let words = t.add(Node::column(2).flex(1), [title, sub]);
                let g = t.gap(Space::XS, 1);
                let mut parts = vec![back, g, words];
                if !shelf.sort.is_empty() {
                    parts.push(sort);
                }
                let header = t.add(Node::row(0), parts);
                // The back arrow's 9 mm target overhangs the margin.
                let header = t.pad(Insets::new(0, 0, 0, -26), header);
                let g = t.gap(1, Space::S);
                let rule = t.rule(2, RULE);
                column.extend([header, g, rule]);
            }
        }

        if let (Place::Home { .. }, Some(hero)) = (shelf.place, shelf.hero) {
            let g = t.gap(1, Space::S);
            let h = Self::hero(&mut t, s, hero, shelf.covers);
            column.extend([g, h]);
        }

        // Views and sort order.
        if let Place::Home { view, .. } = shelf.place {
            let labels: Vec<(&str, bool, ActionId)> = View::ALL
                .iter()
                .enumerate()
                .map(|(i, v)| (v.label(), *v == view, action::VIEW + i as ActionId))
                .collect();
            let mut bar = tabs(&mut t, text, s, &labels);
            // Tabs start flush with the margin, their padding outside it.
            let first = bar[0];
            bar[0] = t.pad(Insets::new(0, 0, 0, -Space::S), first);
            let fill = t.spacer(1);
            bar.extend([fill, sort]);
            let bar = t.add(Node::row(0).cross(Cross::End), bar);
            let g = t.gap(1, Space::S);
            let rule = t.rule(2, RULE);
            column.extend([g, bar, rule]);
        }

        // Shelf chips in the books view.
        let filter = match shelf.place {
            Place::Home {
                view: View::Books,
                filter,
            } => {
                let available = (width - 2 * s.density.px(margin())).max(1);
                let mut rows = Vec::new();
                let mut chips = Vec::new();
                let mut used = 0;
                for (i, f) in Filter::ALL.iter().enumerate() {
                    let style = if *f == filter { s.ui_bold() } else { s.ui() };
                    let chip_width = text.measure(f.label(), &style).ceil() as i32
                        + 2 * s.density.px(CHIP_PADDING_X);
                    if !chips.is_empty() && used + s.density.px(Space::S) + chip_width > available {
                        rows.push(t.add(Node::row(Space::S), std::mem::take(&mut chips)));
                        used = 0;
                    }
                    if !chips.is_empty() {
                        used += s.density.px(Space::S);
                    }
                    used += chip_width;
                    chips.push(chip(
                        &mut t,
                        s,
                        f.label(),
                        *f == filter,
                        action::FILTER + i as ActionId,
                    ));
                }
                rows.push(t.add(Node::row(Space::S), chips));
                let chips = t.add(Node::column(Space::XL), rows);
                column.push(t.pad(Insets::new(Space::M, 0, Space::XS, 0), chips));
                Some(filter)
            }
            _ => None,
        };

        // The list, one page at a time.
        let mut rows = Vec::new();
        for (i, item) in shelf.items.iter().enumerate() {
            let id = action::ITEM + i as ActionId;
            let row = match item {
                Item::Book(b) => {
                    if filter.is_some_and(|f| !f.accepts(b)) {
                        continue;
                    }
                    let in_series = matches!(shelf.place, Place::Series { .. });
                    Self::book_row(&mut t, s, b, shelf.covers, in_series, id)
                }
                Item::Folder {
                    name,
                    books,
                    reading,
                } => {
                    let mut sub = books_label(*books);
                    if *reading > 0 {
                        sub.push_str(" · ");
                        sub.push_str(&tf!(Msg::LibraryFolderReading, n = reading));
                    }
                    Self::shelf_row(&mut t, s, Icon::Folder, name, &sub, id)
                }
                Item::Series {
                    name,
                    books,
                    finished,
                } => {
                    let mut sub = trn(Plural::LibraryVolumes, *books);
                    if *finished > 0 {
                        sub.push_str(" · ");
                        sub.push_str(&tf!(Msg::LibrarySeriesFinished, n = finished));
                    }
                    Self::shelf_row(&mut t, s, Icon::Series, name, &sub, id)
                }
            };
            rows.push(row);
        }
        let mut children = rows.clone();
        if rows.is_empty() {
            let msg = tr(match shelf.place {
                Place::Home {
                    view: View::Folders,
                    ..
                } => Msg::LibraryNoFolders,
                Place::Home {
                    view: View::Series, ..
                } => Msg::LibraryNoSeries,
                _ if shelf.items.is_empty() && shelf.hero.is_none() => Msg::LibraryNoBooks,
                _ => Msg::LibraryEmptyShelf,
            });
            let l = t.text(Paragraph::plain(msg, s.ui().gray(INK_2), Align::Center));
            children.push(t.pad(Insets::vh(mm(10.0), Space::L), l));
        }
        let list = t.add(pages(0).flex(1), children);

        // Pager.
        let prev = icon_button(
            &mut t,
            Icon::ChevronLeft,
            IconSize::STANDARD,
            action::PREV_PAGE,
        );
        let next = icon_button(
            &mut t,
            Icon::ChevronRight,
            IconSize::STANDARD,
            action::NEXT_PAGE,
        );
        let pager_label = t.text(Paragraph::plain(page_of(1, 1), s.small(), Align::Center));
        let pager_label = t.add(Node::column(0).flex(1), [pager_label]);
        let pager = t.add(Node::row(0), [prev, pager_label, next]);

        column.extend([list, pager]);
        let col = t.add(Node::column(0), column);
        let root = t.pad(Insets::new(0, margin(), mm(1.5), margin()), col);
        t.set_root(root);
        let lib = Self {
            list,
            pager_label: t.node(pager_label).children[0],
            prev: t.node(prev).children[0],
            next: t.node(next).children[0],
            title,
            busy,
            rows,
        };
        (t, lib)
    }

    fn hero(t: &mut Tree, s: &Styles, b: &Book, covers: CoverMode) -> NodeId {
        let caps = t.label(tr(Msg::LibraryContinueReading), s.caps());
        let mut tp = Paragraph::plain(&b.title, s.headline(), Align::Left);
        tp.style.max_lines = Some(2);
        let title = t.text(tp);
        let mut ap = Paragraph::plain(&b.author, s.ui().gray(INK_2), Align::Left);
        ap.style.max_lines = Some(1);
        let author = t.text(ap);
        let fill = t.spacer(1);
        let pct = (b.progress * 100.0).floor();
        let mut cp = Paragraph::plain(&b.chapter, s.small(), Align::Left);
        cp.style.max_lines = Some(1);
        let chapter = t.text(cp);
        let chapter = t.add(Node::column(0).flex(1), [chapter]);
        let left = match b.minutes_left {
            Some(min) => format!("{pct:.0}% · {}", fmt_duration(min)),
            None => format!("{pct:.0}%"),
        };
        let left = t.label(&left, s.small());
        let g = t.gap(Space::S, 1);
        let where_ = t.add(Node::row(0), [chapter, g, left]);
        let bar = t.add(progress(b.progress, 12), []);
        let row = if covers == CoverMode::None {
            // Without a card the words alone make the block, at their
            // natural height.
            let g = t.gap(1, Space::XS);
            t.add(
                Node::column(Space::XS),
                [caps, title, author, g, where_, bar],
            )
        } else {
            let info = t.add(
                Node::column(Space::XS).flex(1),
                [caps, title, author, fill, where_, bar],
            );
            let cover = Cover::new(&b.title, &b.author, s.family)
                .with_mark(b.mark())
                .with_image(b.cover.clone().filter(|_| covers == CoverMode::Photos));
            let cover = t.custom(cover);
            let cover = t.fixed(Some(HERO_COVER.0), Some(HERO_COVER.1), Some(cover));
            let row = t.add(Node::row(Space::L).cross(Cross::Stretch), [cover, info]);
            t.fixed(None, Some(HERO_COVER.1), Some(row))
        };
        let row = t.pad(Insets::vh(Space::M, 0), row);
        t.add(
            Node::new(Kind::Padding(Insets::all(0)))
                .action(action::RESUME)
                .feedback(press_feedback(covers)),
            [row],
        )
    }

    fn book_row(
        t: &mut Tree,
        s: &Styles,
        b: &Book,
        covers: CoverMode,
        in_series: bool,
        id: ActionId,
    ) -> NodeId {
        let mut tp = Paragraph::plain(&b.title, s.item(), Align::Left);
        tp.style.max_lines = Some(1);
        let title = t.text(tp);
        let mut ap = Paragraph::plain(&b.author, s.small().line_height(1.2), Align::Left);
        ap.style.max_lines = Some(1);
        let author = t.text(ap);
        let series = b
            .series
            .as_ref()
            .and_then(|(name, i)| series_label(name, *i, in_series));
        let status = match b.status {
            Status::Reading => {
                let bar = t.add(progress(b.progress, 12), []);
                let bar = t.fixed(Some(180), None, Some(bar));
                let pct = (b.progress * 100.0).floor();
                let pct = t.label(&format!("{pct:.0}%"), s.small());
                let g = t.gap(Space::S, 1);
                t.add(Node::row(0), [bar, g, pct])
            }
            Status::Finished => {
                let check = t.icon(Icon::Check, 30, INK_2);
                let l = t.label(tr(Msg::LibraryBookFinished), s.small());
                let g = t.gap(Space::XS, 1);
                t.add(Node::row(0), [check, g, l])
            }
            Status::Unread => match &series {
                Some(text) => t.label(text, s.small().gray(INK_3)),
                None => t.label(tr(Msg::LibraryBookNew), s.caps()),
            },
        };
        let status = t.fixed(None, Some(34), Some(status));
        let info = t.add(
            Node::column(2).cross(Cross::Start).flex(1),
            [title, author, status],
        );
        let row = if covers == CoverMode::None {
            t.add(Node::row(0), [info])
        } else {
            let cover = Cover::new(&b.title, &b.author, s.family)
                .with_mark(b.mark())
                .with_image(b.cover.clone().filter(|_| covers == CoverMode::Photos));
            let cover = t.custom(cover);
            let cover = t.fixed(Some(ROW_COVER.0), Some(ROW_COVER.1), Some(cover));
            t.add(Node::row(Space::M), [cover, info])
        };
        Self::list_item(t, row, id, press_feedback(covers))
    }

    fn shelf_row(
        t: &mut Tree,
        s: &Styles,
        icon: Icon,
        name: &str,
        sub: &str,
        id: ActionId,
    ) -> NodeId {
        let icon = t.icon(icon, IconSize::STANDARD, INK);
        let mut tp = Paragraph::plain(name, s.item(), Align::Left);
        tp.style.max_lines = Some(1);
        let title = t.text(tp);
        let sub = t.label(sub, s.small());
        let words = t.add(Node::column(4).flex(1), [title, sub]);
        let chevron = t.icon(Icon::ChevronRight, IconSize::SMALL, INK_3);
        let row = t.add(Node::row(Space::M), [icon, words, chevron]);
        let row = t.fixed(None, Some(MIN_TARGET + 6), Some(row));
        Self::list_item(t, row, id, Feedback::Invert)
    }

    /// A row with its padding and hairline, the whole of it one target.
    fn list_item(t: &mut Tree, row: NodeId, id: ActionId, feedback: Feedback) -> NodeId {
        let row = t.pad(Insets::vh(Space::S - 2, 0), row);
        let line = t.rule(1, RULE);
        let item = t.add(Node::column(0), [row, line]);
        t.add(
            Node::new(Kind::MinSize(crate::geom::Size::new(0, MIN_TARGET)))
                .action(id)
                .feedback(feedback),
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

/// Folder and series shelves of a sample library, for the preview.
pub fn sample_items(books: &[Book], view: View) -> Vec<Item> {
    use std::collections::BTreeMap;
    match view {
        View::Books => books.iter().cloned().map(Item::Book).collect(),
        View::Folders => {
            let mut folders: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
            let mut loose = Vec::new();
            for b in books {
                match &b.folder {
                    Some(f) => {
                        let e = folders.entry(f).or_default();
                        e.0 += 1;
                        e.1 += (b.status == Status::Reading) as usize;
                    }
                    None => loose.push(Item::Book(b.clone())),
                }
            }
            let mut items: Vec<Item> = folders
                .into_iter()
                .map(|(name, (books, reading))| Item::Folder {
                    name: name.into(),
                    books,
                    reading,
                })
                .collect();
            items.extend(loose);
            items
        }
        View::Series => {
            let mut series: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
            for b in books {
                if let Some((name, _)) = &b.series {
                    let e = series.entry(name).or_default();
                    e.0 += 1;
                    e.1 += (b.status == Status::Finished) as usize;
                }
            }
            series
                .into_iter()
                .map(|(name, (books, finished))| Item::Series {
                    name: name.into(),
                    books,
                    finished,
                })
                .collect()
        }
    }
}

/// The series line of a library card: "Name · vol. n", or "Volume n"
/// inside the series' own view. A missing index arrives as 0 (and a 0 or
/// negative one numbers nothing), so it is never shown as "vol. 0": the
/// name alone, or no line inside the series.
fn series_label(name: &str, index: f32, in_series: bool) -> Option<String> {
    let numbered = index > 0.0;
    let n = series_number(index);
    match (in_series, numbered) {
        (true, true) => Some(tf!(Msg::LibraryVolume, n = n)),
        (true, false) => None,
        (false, true) => Some(tf!(Msg::LibrarySeriesVolume, series = name, n = n)),
        (false, false) => Some(name.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_index_zero_is_never_shown() {
        assert_eq!(
            series_label("Gazeta", 0.0, false).as_deref(),
            Some("Gazeta")
        );
        assert_eq!(series_label("Gazeta", 0.0, true), None);
        aristotle_i18n::with_language("en", || {
            assert_eq!(
                series_label("Dialogues", 3.0, false).as_deref(),
                Some("Dialogues · vol. 3")
            );
            assert_eq!(
                series_label("Dialogues", 2.5, true).as_deref(),
                Some("Volume 2.5")
            );
        });
        aristotle_i18n::with_language("pl", || {
            assert_eq!(
                series_label("Dialogi", 3.0, false).as_deref(),
                Some("Dialogi · tom 3")
            );
            assert_eq!(series_label("Dialogi", 3.0, true).as_deref(), Some("Tom 3"));
            assert_eq!(
                series_label("Dialogi", 2.5, true).as_deref(),
                Some("Tom 2,5")
            );
        });
    }
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

    fn home<'a>(items: &'a [Item], view: View, filter: Filter, covers: CoverMode) -> Shelf<'a> {
        Shelf {
            place: Place::Home { view, filter },
            hero: None,
            items,
            sort: "Recent",
            status: StatusLine {
                clock: "12:00",
                battery: Some(0.5),
                charging: false,
            },
            covers,
        }
    }

    #[test]
    fn shelves_follow_the_status() {
        let books = sample_books();
        let items = sample_items(&books, View::Books);
        let count = |f: Filter| books.iter().filter(|b| f.accepts(b)).count();
        assert_eq!(count(Filter::All), books.len());
        assert_eq!(
            count(Filter::Reading) + count(Filter::New) + count(Filter::Finished),
            books.len()
        );
        let (_, lib) = ui_for(&home(
            &items,
            View::Books,
            Filter::Finished,
            CoverMode::None,
        ));
        assert_eq!(lib.rows.len(), count(Filter::Finished));
    }

    #[test]
    fn folders_mirror_the_tree_with_loose_books_last() {
        let books = sample_books();
        let items = sample_items(&books, View::Folders);
        assert!(matches!(&items[0], Item::Folder { name, .. } if name == "Filozofia"));
        assert!(matches!(items.last(), Some(Item::Book(b)) if b.title == "Moby Dick"));
        let platon = items.iter().find_map(|i| match i {
            Item::Folder {
                name,
                books,
                reading,
            } if name == "Platon" => Some((*books, *reading)),
            _ => None,
        });
        assert_eq!(platon, Some((3, 2)));
        let (_, lib) = ui_for(&home(&items, View::Folders, Filter::All, CoverMode::None));
        assert_eq!(lib.rows.len(), items.len());
    }

    #[test]
    fn empty_library_still_builds_with_one_page() {
        let (ui, lib) = ui_for(&home(&[], View::Books, Filter::All, CoverMode::Cards));
        assert!(lib.rows.is_empty());
        assert_eq!(ui.tree.page(lib.list).1, 1);
    }

    #[test]
    fn photo_covers_only_when_asked() {
        let mut books = sample_books();
        // A black cover: the row's cover box turns dark only with photos.
        books[1].cover = Some(GrayImage::new(200, 300, vec![0; 200 * 300]));
        let items = sample_items(&books[1..2], View::Books);
        let dark = |covers| {
            let (ui, lib) = ui_for(&home(&items, View::Books, Filter::All, covers));
            let r = ui.tree.rect(lib.rows[0]);
            (r.y..r.bottom())
                .filter(|&y| ui.screen().get(margin() + 36, y) < 40)
                .count()
        };
        assert!(dark(CoverMode::Photos) > 80);
        assert!(dark(CoverMode::None) < 40);
    }

    #[test]
    fn plurals_follow_the_language() {
        aristotle_i18n::with_language("pl", || {
            assert_eq!(books_label(1), "1 książka");
            assert_eq!(books_label(3), "3 książki");
            assert_eq!(books_label(5), "5 książek");
            assert_eq!(books_label(12), "12 książek");
            assert_eq!(books_label(22), "22 książki");
        });
        aristotle_i18n::with_language("en", || {
            assert_eq!(books_label(1), "1 book");
            assert_eq!(books_label(22), "22 books");
        });
    }

    #[test]
    fn every_row_is_a_full_target() {
        let books = sample_books();
        for view in View::ALL {
            let items = sample_items(&books, view);
            let (ui, lib) = ui_for(&home(&items, view, Filter::All, CoverMode::Cards));
            for r in &lib.rows {
                let rect = ui.tree.rect(*r);
                if !rect.is_empty() {
                    assert!(rect.h >= MIN_TARGET, "{view:?}: {rect:?}");
                }
            }
        }
    }

    #[test]
    fn filters_keep_their_labels_and_touch_targets_on_narrow_screens() {
        for lang in ["en", "pl"] {
            aristotle_i18n::with_language(lang, || {
                for (width, height, dpi) in [
                    (800, 1200, 300),
                    (1072, 1448, 300),
                    (1236, 1648, 300),
                    (600, 800, 167),
                    (758, 1024, 212),
                ] {
                    let profile = crate::geom::Screen::new(width, height, dpi).unwrap();
                    for selected in Filter::ALL {
                        let mut text = TextEngine::new(Fonts::embedded());
                        let styles = Styles::new(FamilyId(0)).with_density(profile.density);
                        let shelf = home(&[], View::Books, selected, CoverMode::None);
                        let (tree, _) = Library::build_shelf_for_width(
                            &mut text,
                            &styles,
                            &shelf,
                            width as i32,
                        );
                        let mut ui = Ui::with_screen(tree, text, profile);
                        ui.layout();
                        for (i, f) in Filter::ALL.iter().enumerate() {
                            let action = action::FILTER + i as ActionId;
                            let id = (0..ui.tree.len())
                                .map(|n| NodeId(n as u32))
                                .find(|n| ui.tree.node(*n).action == Some(action))
                                .unwrap();
                            let rect = ui.tree.rect(id);
                            assert_eq!(rect.intersect(&ui.bounds()), Some(rect));
                            let style = if *f == selected {
                                styles.ui_bold()
                            } else {
                                styles.ui()
                            };
                            let label = ui.text.measure(f.label(), &style).ceil() as i32;
                            assert!(
                                rect.w >= label + 2 * profile.density.px(CHIP_PADDING_X),
                                "{lang} {width}: {f:?} {rect:?}"
                            );
                            assert_eq!(ui.tree.hit(rect.center()), crate::tree::Hit::Target(id));
                        }
                    }
                }
            });
        }
    }

    #[test]
    fn titles_and_pager_fit_on_screen() {
        let books = sample_books();
        let items = sample_items(&books, View::Books);
        let mut shelf = home(&items, View::Books, Filter::All, CoverMode::Cards);
        shelf.hero = Some(&books[0]);
        let (ui, lib) = ui_for(&shelf);
        let pager = ui.tree.rect(lib.next);
        assert!(pager.bottom() <= SCREEN_HEIGHT, "{pager:?}");
        assert!(ui.tree.page(lib.list).1 >= 2);
    }
}
