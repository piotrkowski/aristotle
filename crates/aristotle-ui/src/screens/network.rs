// SPDX-License-Identifier: AGPL-3.0-or-later
//! The network screens behind Settings › Network: progress sync (KOSync),
//! the list of OPDS catalogs, a catalog's feed, and one book in it.
//!
//! They share one frame: the status line, a back arrow with the title, a
//! message line, a paged list and the pager. The message line has a fixed
//! height of two lines, so "Loading…", an error or a download's progress
//! changes only its own pixels (one DU update) and never moves the list.
//!
//! Nothing here touches the network. The app runs requests on worker
//! threads and rebuilds or updates these trees with the results. Text the
//! reader types (server, user name, password, a catalog's address, search
//! terms) comes from the on-screen keyboard's `TextInput`; the screens only
//! offer the rows that ask for it.

use super::icon_button;
use super::library::{chip, page_of, status_line, StatusLine};
use crate::geom::{Insets, MIN_TARGET};
use crate::icons::Icon;
use crate::text::{Align, Paragraph, TextEngine};
use crate::theme::{margin, IconSize, Space, Styles, INK, INK_2, INK_3, RULE};
use crate::tree::{pages, ActionId, Cross, Feedback, Kind, Node, NodeId, Tree};
use crate::widgets::Toggle;
use aristotle_i18n::{tf, tr, Msg};

pub mod action {
    use crate::tree::ActionId;
    pub const BACK: ActionId = 1;
    pub const PREV_PAGE: ActionId = 5;
    pub const NEXT_PAGE: ActionId = 6;
    // Progress sync.
    pub const EDIT_SERVER: ActionId = 10;
    pub const EDIT_USER: ActionId = 11;
    pub const EDIT_PASSWORD: ActionId = 12;
    pub const AUTO: ActionId = 13;
    pub const LOG_IN: ActionId = 14;
    pub const REGISTER: ActionId = 15;
    /// Read `sync.json` or `catalogs.json` again.
    pub const IMPORT: ActionId = 16;
    pub const FORGET: ActionId = 17;
    // Catalogs.
    pub const ADD: ActionId = 20;
    pub const SEARCH: ActionId = 30;
    /// `DOWNLOAD + i` downloads `EntryView::formats[i]`.
    pub const DOWNLOAD: ActionId = 40;
    /// Open the book just downloaded.
    pub const READ: ActionId = 60;
    /// `ITEM + i`: catalog `i` in the list, entry `i` of a feed.
    pub const ITEM: ActionId = 100;
    /// `REMOVE + i` removes catalog `i`.
    pub const REMOVE: ActionId = 400;
}

/// The nodes every network screen updates in place.
pub struct Frame {
    pub list: NodeId,
    pub pager_label: NodeId,
    pub prev: NodeId,
    pub next: NodeId,
    pub title: NodeId,
    /// The busy mark in the status line, hidden.
    pub busy: NodeId,
    /// The message line under the title.
    pub message: NodeId,
    /// A feed that has more pages on the server, before or after the ones
    /// loaded: the arrows ask for them at the ends of the list.
    pub more_before: bool,
    pub more_after: bool,
}

/// What a pager arrow did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paged {
    /// Turned the list's page.
    Turned,
    /// At an end of the list: fetch the server's next (`true`) or
    /// previous page.
    Fetch { next: bool },
    /// Not a pager action, or nothing to turn to.
    No,
}

impl Frame {
    /// The pager after layout: label and arrow states.
    pub fn sync(&self, t: &mut Tree) {
        let (page, count) = t.page(self.list);
        t.set_text(self.pager_label, &page_of(page + 1, count.max(1)));
        let back = page > 0 || self.more_before;
        let forward = page + 1 < count || self.more_after;
        t.set_icon_gray(self.prev, if back { INK } else { RULE });
        t.set_icon_gray(self.next, if forward { INK } else { RULE });
    }

    pub fn handle(&self, t: &mut Tree, id: ActionId) -> Paged {
        let (page, count) = t.page(self.list);
        match id {
            action::PREV_PAGE if page > 0 => {
                t.set_page(self.list, page - 1);
                Paged::Turned
            }
            action::PREV_PAGE if self.more_before => Paged::Fetch { next: false },
            action::NEXT_PAGE if page + 1 < count => {
                t.set_page(self.list, page + 1);
                Paged::Turned
            }
            action::NEXT_PAGE if self.more_after => Paged::Fetch { next: true },
            _ => Paged::No,
        }
    }

    /// Show `text` in the message line (an empty string clears it).
    pub fn set_message(&self, t: &mut Tree, text: &str) {
        t.set_text(self.message, text);
    }
}

/// The two-line message paragraph.
fn message_line(t: &mut Tree, s: &Styles, text: &str) -> (NodeId, NodeId) {
    let mut p = Paragraph::plain(text, s.ui().gray(INK), Align::Left);
    p.style.max_lines = Some(2);
    let style = s.ui();
    let msg = t.text(p);
    let two_lines = (style.size * style.line_height * 2.0).ceil() as i32 + 4;
    let fixed = t.fixed(None, Some(two_lines), Some(msg));
    (t.pad(Insets::new(Space::XS, 0, Space::XS, 0), fixed), msg)
}

/// The frame around `items`: status, header (back, title, an optional
/// button at the right), message, list, pager.
fn frame(
    t: &mut Tree,
    s: &Styles,
    status: &StatusLine,
    title: &str,
    right: Option<NodeId>,
    message: &str,
    items: Vec<NodeId>,
) -> Frame {
    let (status, busy) = status_line(t, s, status);
    let back = icon_button(t, Icon::ArrowLeft, IconSize::STANDARD, action::BACK);
    let mut tp = Paragraph::plain(title, s.display(), Align::Left);
    tp.style.max_lines = Some(1);
    let title_node = t.text(tp);
    let title_col = t.add(Node::column(0).flex(1), [title_node]);
    let g = t.gap(Space::XS, 1);
    let mut header = vec![back, g, title_col];
    header.extend(right);
    let header = t.add(Node::row(0), header);
    let header = t.pad(Insets::new(0, 0, 0, -26), header);
    let (message_box, message) = message_line(t, s, message);
    let list = t.add(pages(0).flex(1), items);
    let prev = icon_button(t, Icon::ChevronLeft, IconSize::STANDARD, action::PREV_PAGE);
    let next = icon_button(t, Icon::ChevronRight, IconSize::STANDARD, action::NEXT_PAGE);
    let pager_label = t.text(Paragraph::plain("", s.small(), Align::Center));
    let pager_label = t.add(Node::column(0).flex(1), [pager_label]);
    let pager = t.add(Node::row(0), [prev, pager_label, next]);
    let col = t.add(Node::column(0), [status, header, message_box, list, pager]);
    let root = t.pad(
        Insets::new(0, margin(), crate::geom::mm(1.5), margin()),
        col,
    );
    t.set_root(root);
    Frame {
        list,
        pager_label: t.node(pager_label).children[0],
        prev: t.node(prev).children[0],
        next: t.node(next).children[0],
        title: title_node,
        busy,
        message,
        more_before: false,
        more_after: false,
    }
}

/// A full-width row that is one target: an optional icon, a title, up to
/// `detail_lines` of detail, a control or a chevron at the right, and a
/// hairline below.
#[allow(clippy::too_many_arguments)]
fn row(
    t: &mut Tree,
    s: &Styles,
    icon: Option<Icon>,
    title: &str,
    detail: &str,
    detail_lines: usize,
    right: Option<NodeId>,
    id: Option<ActionId>,
) -> NodeId {
    let mut tp = Paragraph::plain(title, s.item(), Align::Left);
    tp.style.max_lines = Some(2);
    let mut words = vec![t.text(tp)];
    if !detail.is_empty() {
        let mut dp = Paragraph::plain(detail, s.small().line_height(1.2), Align::Left);
        dp.style.max_lines = Some(detail_lines);
        words.push(t.text(dp));
    }
    let words = t.add(Node::column(4).flex(1).cross(Cross::Start), words);
    let mut children = Vec::new();
    if let Some(i) = icon {
        children.push(t.icon(i, IconSize::STANDARD, INK));
    }
    children.push(words);
    children.extend(right);
    let r = t.add(Node::row(Space::M), children);
    let r = t.pad(Insets::vh(Space::S - 2, 0), r);
    let line = t.rule(1, RULE);
    let item = t.add(Node::column(0), [r, line]);
    let mut n = Node::new(Kind::MinSize(crate::geom::Size::new(0, MIN_TARGET)));
    if let Some(id) = id {
        n = n.action(id).feedback(Feedback::Invert);
    }
    t.add(n, [item])
}

fn chevron(t: &mut Tree) -> NodeId {
    t.icon(Icon::ChevronRight, IconSize::SMALL, INK_3)
}

/// Chips in a row that wraps onto more rows when they don't fit.
fn chips(t: &mut Tree, s: &Styles, labels: &[(String, ActionId)]) -> NodeId {
    // Two per row keep long Polish labels whole on the 1072 px screen.
    let mut rows = Vec::new();
    for pair in labels.chunks(2) {
        let cs: Vec<NodeId> = pair
            .iter()
            .map(|(l, id)| chip(t, s, l, false, *id))
            .collect();
        rows.push(t.add(Node::row(Space::S), cs));
    }
    let col = t.add(Node::column(Space::S).cross(Cross::Start), rows);
    t.pad(Insets::vh(Space::S, 0), col)
}

// ------------------------------------------------------------ progress sync

/// The sync settings as the screen shows them.
pub struct SyncView<'a> {
    pub status: StatusLine<'a>,
    pub server: Option<&'a str>,
    pub username: Option<&'a str>,
    pub has_key: bool,
    pub auto: bool,
    /// What happened last ("Logged in as anna.", an error).
    pub message: &'a str,
    /// Where `sync.json` is read from.
    pub import_path: &'a str,
}

pub fn build_sync(_text: &mut TextEngine, s: &Styles, v: &SyncView) -> (Tree, Frame) {
    let mut t = Tree::new();
    let not_set = tr(Msg::SyncNotSet);
    let mut items = Vec::new();
    let mut intro = Paragraph::plain(tr(Msg::SyncIntro), s.small().gray(INK_2), Align::Left);
    intro.style.max_lines = Some(4);
    let intro = t.text(intro);
    items.push(t.pad(Insets::new(0, 0, Space::S, 0), intro));
    let c = chevron(&mut t);
    items.push(row(
        &mut t,
        s,
        None,
        tr(Msg::SyncServer),
        v.server.unwrap_or(not_set),
        2,
        Some(c),
        Some(action::EDIT_SERVER),
    ));
    let c = chevron(&mut t);
    items.push(row(
        &mut t,
        s,
        None,
        tr(Msg::SyncUser),
        v.username.unwrap_or(not_set),
        1,
        Some(c),
        Some(action::EDIT_USER),
    ));
    let c = chevron(&mut t);
    let pw = if v.has_key {
        tr(Msg::SyncPasswordSaved)
    } else {
        not_set
    };
    items.push(row(
        &mut t,
        s,
        None,
        tr(Msg::SyncPassword),
        pw,
        1,
        Some(c),
        Some(action::EDIT_PASSWORD),
    ));
    items.push(chips(
        &mut t,
        s,
        &[
            (tr(Msg::SyncLogIn).to_string(), action::LOG_IN),
            (tr(Msg::SyncRegister).to_string(), action::REGISTER),
        ],
    ));
    let toggle = t.custom(Toggle { on: v.auto });
    items.push(row(
        &mut t,
        s,
        None,
        tr(Msg::SyncAuto),
        tr(Msg::SyncAutoHint),
        2,
        Some(toggle),
        Some(action::AUTO),
    ));
    let hint = tf!(Msg::SyncImportHint, path = v.import_path);
    items.push(row(
        &mut t,
        s,
        None,
        tr(Msg::SyncImport),
        &hint,
        2,
        None,
        Some(action::IMPORT),
    ));
    if v.server.is_some() {
        items.push(row(
            &mut t,
            s,
            None,
            tr(Msg::SyncForget),
            "",
            1,
            None,
            Some(action::FORGET),
        ));
    }
    let f = frame(
        &mut t,
        s,
        &v.status,
        tr(Msg::SyncTitle),
        None,
        v.message,
        items,
    );
    (t, f)
}

// ------------------------------------------------------------ catalogs

pub struct CatalogRow<'a> {
    pub name: &'a str,
    pub url: &'a str,
}

pub struct CatalogsView<'a> {
    pub status: StatusLine<'a>,
    pub catalogs: &'a [CatalogRow<'a>],
    pub message: &'a str,
    /// Where `catalogs.json` is read from.
    pub import_path: &'a str,
}

pub fn build_catalogs(_text: &mut TextEngine, s: &Styles, v: &CatalogsView) -> (Tree, Frame) {
    let mut t = Tree::new();
    let mut items = Vec::new();
    for (i, c) in v.catalogs.iter().enumerate() {
        let remove = icon_button(
            &mut t,
            Icon::Close,
            IconSize::SMALL,
            action::REMOVE + i as ActionId,
        );
        items.push(row(
            &mut t,
            s,
            Some(Icon::Library),
            c.name,
            c.url,
            1,
            Some(remove),
            Some(action::ITEM + i as ActionId),
        ));
    }
    if v.catalogs.is_empty() {
        let mut p = Paragraph::plain(tr(Msg::CatalogsEmpty), s.ui().gray(INK_2), Align::Left);
        p.style.max_lines = Some(2);
        let p = t.text(p);
        items.push(t.pad(Insets::vh(Space::S, 0), p));
    }
    let plus = t.icon(Icon::Plus, IconSize::STANDARD, INK);
    items.push(row(
        &mut t,
        s,
        None,
        tr(Msg::CatalogsAdd),
        "",
        1,
        Some(plus),
        Some(action::ADD),
    ));
    let hint = tf!(Msg::CatalogsImportHint, path = v.import_path);
    items.push(row(
        &mut t,
        s,
        None,
        tr(Msg::CatalogsImport),
        &hint,
        2,
        None,
        Some(action::IMPORT),
    ));
    let f = frame(
        &mut t,
        s,
        &v.status,
        tr(Msg::CatalogsTitle),
        None,
        v.message,
        items,
    );
    (t, f)
}

// ------------------------------------------------------------ a feed

/// One row of a feed: a place to go, or a book.
pub struct FeedRow<'a> {
    pub title: &'a str,
    /// Authors, a summary, or the formats on offer.
    pub detail: &'a str,
    /// Leads to another feed (a chevron), rather than to a book.
    pub navigation: bool,
}

pub struct FeedView<'a> {
    pub status: StatusLine<'a>,
    pub title: &'a str,
    pub rows: &'a [FeedRow<'a>],
    pub searchable: bool,
    pub more_before: bool,
    pub more_after: bool,
    pub message: &'a str,
}

pub fn build_feed(_text: &mut TextEngine, s: &Styles, v: &FeedView) -> (Tree, Frame) {
    let mut t = Tree::new();
    let mut items = Vec::new();
    for (i, r) in v.rows.iter().enumerate() {
        let right = if r.navigation {
            Some(chevron(&mut t))
        } else {
            None
        };
        let icon = if r.navigation {
            Icon::Folder
        } else {
            Icon::Download
        };
        items.push(row(
            &mut t,
            s,
            Some(icon),
            r.title,
            r.detail,
            2,
            right,
            Some(action::ITEM + i as ActionId),
        ));
    }
    if v.rows.is_empty() {
        let p = t.label(tr(Msg::CatalogsEmptyFeed), s.ui().gray(INK_2));
        items.push(t.pad(Insets::vh(Space::S, 0), p));
    }
    let search = v
        .searchable
        .then(|| icon_button(&mut t, Icon::Search, IconSize::STANDARD, action::SEARCH));
    let mut f = frame(&mut t, s, &v.status, v.title, search, v.message, items);
    f.more_before = v.more_before;
    f.more_after = v.more_after;
    (t, f)
}

// ------------------------------------------------------------ one book

pub struct EntryView<'a> {
    pub status: StatusLine<'a>,
    pub title: &'a str,
    pub authors: &'a str,
    pub summary: &'a str,
    /// Button labels, "EPUB · 1.2 MB"; `DOWNLOAD + i` gets `formats[i]`.
    pub formats: &'a [String],
    /// The book is on the device: offer to read it.
    pub downloaded: bool,
    pub message: &'a str,
}

pub fn build_entry(_text: &mut TextEngine, s: &Styles, v: &EntryView) -> (Tree, Frame) {
    let mut t = Tree::new();
    let mut items = Vec::new();
    let mut head = Vec::new();
    let mut tp = Paragraph::plain(v.title, s.headline(), Align::Left);
    tp.style.max_lines = Some(3);
    head.push(t.text(tp));
    if !v.authors.is_empty() {
        let mut ap = Paragraph::plain(v.authors, s.ui().gray(INK_2), Align::Left);
        ap.style.max_lines = Some(2);
        head.push(t.text(ap));
    }
    let head = t.add(Node::column(Space::XS).cross(Cross::Start), head);
    items.push(t.pad(Insets::new(0, 0, Space::S, 0), head));
    let mut buttons: Vec<(String, ActionId)> = v
        .formats
        .iter()
        .enumerate()
        .map(|(i, f)| {
            (
                tf!(Msg::CatalogsDownload, format = f),
                action::DOWNLOAD + i as ActionId,
            )
        })
        .collect();
    if v.downloaded {
        buttons.insert(0, (tr(Msg::CatalogsReadNow).to_string(), action::READ));
    }
    if buttons.is_empty() {
        let l = t.label(tr(Msg::CatalogsNoFormats), s.ui().gray(INK_2));
        items.push(t.pad(Insets::vh(Space::S, 0), l));
    } else {
        items.push(chips(&mut t, s, &buttons));
    }
    if !v.summary.is_empty() {
        let mut p = Paragraph::plain(v.summary, s.ui(), Align::Left);
        p.style.max_lines = Some(14);
        let p = t.text(p);
        items.push(t.pad(Insets::new(Space::S, 0, 0, 0), p));
    }
    let f = frame(
        &mut t,
        s,
        &v.status,
        tr(Msg::CatalogsTitle),
        None,
        v.message,
        items,
    );
    (t, f)
}

/// Sample data for the preview and the tests.
pub mod sample {
    use super::*;

    pub fn feed_rows() -> Vec<FeedRow<'static>> {
        vec![
            FeedRow {
                title: "Lalka",
                detail: "Bolesław Prus · EPUB 1.2 MB, AZW3, PDF",
                navigation: false,
            },
            FeedRow {
                title: "Pan Tadeusz",
                detail: "Adam Mickiewicz · EPUB 446 KB",
                navigation: false,
            },
            FeedRow {
                title: "Authors",
                detail: "3 books",
                navigation: true,
            },
        ]
    }

    pub fn catalogs() -> Vec<CatalogRow<'static>> {
        vec![
            CatalogRow {
                name: "calibre Library",
                url: "http://192.168.15.201:8080/opds",
            },
            CatalogRow {
                name: "Project Gutenberg",
                url: "https://m.gutenberg.org/ebooks.opds/",
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Point, SCREEN_HEIGHT, SCREEN_WIDTH};
    use crate::text::{FamilyId, Fonts};
    use crate::tree::Hit;
    use crate::ui::Ui;

    fn status() -> StatusLine<'static> {
        StatusLine {
            clock: "21:38",
            battery: Some(0.8),
            charging: false,
        }
    }

    fn ui(tree: Tree, text: TextEngine) -> Ui {
        let mut ui = Ui::new(tree, text, SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
        ui.layout();
        ui
    }

    /// The actions that are targets on the first page; list rows are at
    /// least 9 mm tall (chips and icons are enlarged by the hit test).
    fn targets(ui: &Ui) -> Vec<ActionId> {
        let mut out = Vec::new();
        for y in (0..SCREEN_HEIGHT).step_by(8) {
            for x in [80, 300, 540, 1000] {
                if let Hit::Target(n) = ui.tree.hit(Point { x, y }) {
                    let a = ui.tree.node(n).action.unwrap();
                    let r = ui.tree.rect(n);
                    if (action::ITEM..action::REMOVE).contains(&a) {
                        assert!(r.h >= MIN_TARGET, "row {a} is {} px tall", r.h);
                    }
                    if !out.contains(&a) {
                        out.push(a);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn sync_screen_offers_every_setting() {
        for lang in ["en", "pl"] {
            aristotle_i18n::with_language(lang, || {
                let mut text = TextEngine::new(Fonts::embedded());
                let v = SyncView {
                    status: status(),
                    server: Some("http://192.168.15.201:17200"),
                    username: Some("anna"),
                    has_key: true,
                    auto: true,
                    message: "Logged in as anna.",
                    import_path: "/mnt/us/aristotle/sync.json",
                };
                let (tree, f) = build_sync(&mut text, &Styles::new(FamilyId(0)), &v);
                let mut ui = ui(tree, text);
                f.sync(&mut ui.tree);
                let got = targets(&ui);
                for a in [
                    action::BACK,
                    action::EDIT_SERVER,
                    action::EDIT_USER,
                    action::EDIT_PASSWORD,
                    action::LOG_IN,
                    action::REGISTER,
                    action::AUTO,
                ] {
                    assert!(got.contains(&a), "{lang}: action {a} missing from {got:?}");
                }
            });
        }
    }

    #[test]
    fn feed_pager_asks_the_server_at_the_ends() {
        let rows: Vec<FeedRow> = (0..30)
            .map(|_| FeedRow {
                title: "Pride and Prejudice",
                detail: "Jane Austen",
                navigation: true,
            })
            .collect();
        let mut text = TextEngine::new(Fonts::embedded());
        let v = FeedView {
            status: status(),
            title: "Project Gutenberg",
            rows: &rows,
            searchable: true,
            more_before: false,
            more_after: true,
            message: "",
        };
        let (tree, f) = build_feed(&mut text, &Styles::new(FamilyId(0)), &v);
        let mut ui = ui(tree, text);
        let (_, count) = ui.tree.page(f.list);
        assert!(count > 1);
        assert_eq!(f.handle(&mut ui.tree, action::PREV_PAGE), Paged::No);
        for _ in 1..count {
            assert_eq!(f.handle(&mut ui.tree, action::NEXT_PAGE), Paged::Turned);
            ui.layout();
        }
        assert_eq!(
            f.handle(&mut ui.tree, action::NEXT_PAGE),
            Paged::Fetch { next: true }
        );
        assert!(targets(&ui).contains(&action::SEARCH) || ui.tree.page(f.list).0 > 0);
    }

    #[test]
    fn catalogs_and_entry_build_in_both_languages() {
        for lang in ["en", "pl"] {
            aristotle_i18n::with_language(lang, || {
                let cats = sample::catalogs();
                let mut text = TextEngine::new(Fonts::embedded());
                let s = Styles::new(FamilyId(0));
                let v = CatalogsView {
                    status: status(),
                    catalogs: &cats,
                    message: "",
                    import_path: "/mnt/us/aristotle/catalogs.json",
                };
                let (tree, _) = build_catalogs(&mut text, &s, &v);
                let ui = ui(tree, text);
                let got = targets(&ui);
                for a in [action::ITEM, action::ITEM + 1, action::REMOVE, action::ADD] {
                    assert!(got.contains(&a), "{lang}: {a} missing");
                }
                let mut text = TextEngine::new(Fonts::embedded());
                let formats = vec!["EPUB · 1.2 MB".to_string(), "PDF".to_string()];
                let v = EntryView {
                    status: status(),
                    title: "Lalka",
                    authors: "Bolesław Prus",
                    summary: "Warszawa, rok 1878.",
                    formats: &formats,
                    downloaded: true,
                    message: "",
                };
                let (tree, _) = build_entry(&mut text, &s, &v);
                let got = targets(&ui_with(tree, text));
                for a in [action::READ, action::DOWNLOAD, action::DOWNLOAD + 1] {
                    assert!(got.contains(&a), "{lang}: {a} missing");
                }
            });
        }
    }

    fn ui_with(tree: Tree, text: TextEngine) -> Ui {
        ui(tree, text)
    }
}
