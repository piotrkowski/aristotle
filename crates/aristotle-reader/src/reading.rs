//! Reading interactions in the app: selecting words, the dictionary,
//! footnotes, highlights, bookmarks, the contents, "go to".
//!
//! How things reach the panel (`docs/reader.md`, "Reading interactions"):
//!
//! - **Appearing** (a popup, the action sheet, a list, the "go to" sheet)
//!   is one update: the frame is fully built, then sent as one full-screen
//!   mode-9 update when it has grays ([`App::present_full`]).
//! - **Feedback while the finger is down** (the held word when the
//!   dictionary is slow, a selection growing under a drag) is A2 on the
//!   selection's boxes only.
//! - **Closing** anything that covered the page shows the page again with
//!   the page-turn update, one full-screen mode-9 update
//!   ([`App::close_overlay`]), so nothing is left behind.
//!
//! Pages turn on release ([`crate::turns`]), so a long press finds the
//! page it started on: the selection and the popup come in one update.
//!
//! Night mode needs nothing here: the UI inverts its pixels (or the EPDC
//! does), so popups, selections and lists follow the night palette.
//! While a book still opens from its snapshot these gestures do nothing
//! (`App::opening_intent`).

use std::rc::Rc;
use std::time::{Duration, Instant};

use aristotle_library::index::AnnotationKind;
use aristotle_render::book::{Bounds, Engine, Link, Position, TocItem, Word};
use aristotle_render::plato_core::anyhow::Error;
use aristotle_render::richtext::{Block, RichText, Run};
use aristotle_ui::geom::{Point, Rect, MIN_TARGET};
use aristotle_ui::input::Direction;
use aristotle_ui::tree::{ActionId, NodeId, Refresh, Tree};
use aristotle_ui::ui::UiEvent;
use aristotle_ui::widgets::Slider;

use crate::dict::service::{Reply, Request};
use crate::dict::{self, Lookup};
use crate::marks;
use crate::overlay::{self, act, ListRow, Place, PopupSpec, SelectionMarks};
use crate::panel::Content;
use crate::selection::{self, Selection};
use crate::turns::bookmark_corner;
use crate::{perf, App, Screen};

/// How long a long press waits for the dictionary before it shows the
/// selected word on its own.
const LOOKUP_PATIENCE: Duration = Duration::from_millis(150);

/// What covers the page.
#[derive(Default)]
pub enum Overlay {
    #[default]
    None,
    /// A held word, the dictionary still looking.
    Waiting {
        sel: Selection,
        words: Rc<Vec<Word>>,
    },
    Dict {
        sel: Selection,
        words: Rc<Vec<Word>>,
        lookup: Box<Lookup>,
        /// Annotation index of a highlight the word is in.
        highlight: Option<usize>,
        page: usize,
    },
    /// A selection under a dragging finger, with handles.
    Dragging {
        sel: Selection,
        words: Rc<Vec<Word>>,
    },
    /// A dragged selection, released: what to do with it.
    Actions {
        sel: Selection,
        words: Rc<Vec<Word>>,
    },
    /// A word-by-word gloss of the selection.
    Gloss {
        sel: Selection,
        words: Rc<Vec<Word>>,
        text: RichText,
        page: usize,
    },
    Note {
        text: RichText,
        link: Link,
        page: usize,
    },
    Toc {
        items: Vec<TocItem>,
        current: Option<usize>,
        page: usize,
    },
    Marks {
        rows: Vec<MarkRow>,
        page: usize,
    },
    GoTo {
        value: f32,
    },
}

impl Overlay {
    pub fn is_none(&self) -> bool {
        matches!(self, Overlay::None)
    }

    fn selection(&self) -> Option<(&Selection, &Rc<Vec<Word>>)> {
        match self {
            Overlay::Waiting { sel, words }
            | Overlay::Dict { sel, words, .. }
            | Overlay::Dragging { sel, words }
            | Overlay::Actions { sel, words }
            | Overlay::Gloss { sel, words, .. } => Some((sel, words)),
            _ => None,
        }
    }

    fn page_mut(&mut self) -> Option<&mut usize> {
        match self {
            Overlay::Dict { page, .. }
            | Overlay::Gloss { page, .. }
            | Overlay::Note { page, .. }
            | Overlay::Toc { page, .. }
            | Overlay::Marks { page, .. } => Some(page),
            _ => None,
        }
    }
}

/// A line of the highlights list.
pub struct MarkRow {
    pub text: String,
    pub meta: String,
    pub target: Position,
}

/// Nodes of the current overlay that change without a rebuild.
#[derive(Default)]
pub struct OverlayNodes {
    pub selection: Option<NodeId>,
    pub pages: Option<NodeId>,
    pub pager: Option<NodeId>,
    /// The pager's arrows, hidden when there is one page.
    pub arrows: Option<(NodeId, NodeId)>,
    pub slider: Option<NodeId>,
    pub label: Option<NodeId>,
}

/// Bounds to a UI rect.
fn rect(b: &Bounds) -> Rect {
    Rect::new(b.x0, b.y0, b.x1 - b.x0, b.y1 - b.y0)
}

/// The word nearest to a point, however far (a drag may run into the
/// margin).
fn nearest(words: &[Word], p: Point) -> Option<usize> {
    (0..words.len()).min_by_key(|&i| words[i].rect.dist2(p.x, p.y))
}

impl App {
    fn reading(&mut self) -> Option<&mut crate::Reading> {
        match &mut self.screen {
            Screen::Reader(r) => Some(r),
            _ => None,
        }
    }

    // ------------------------------------------------------------ frames

    /// Build the reader screen for the current state (page, overlay, menu)
    /// and install it. Nothing is sent to the panel.
    pub(crate) fn rebuild_reader(&mut self, refresh: Refresh) -> Result<(), Error> {
        let styles = self.styles.clone();
        let dicts: Vec<String> = self
            .dict
            .as_ref()
            .map(|d| d.dicts.iter().map(|x| x.label.clone()).collect())
            .unwrap_or_default();
        let h = self.ui.bounds().h;
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let page = r.session.page(r.session.current)?;
        let page_words = r.session.words(r.session.current);
        let mut nodes = OverlayNodes::default();
        let overlay = &r.overlay;
        let mut extra = |t: &mut Tree| -> Vec<NodeId> {
            let mut layers = Vec::new();
            if let Some((sel, words)) = overlay.selection() {
                let handles = matches!(overlay, Overlay::Dragging { .. } | Overlay::Actions { .. });
                let boxes = selection::line_boxes(words, sel.start, sel.end);
                let n = overlay::selection_layer(t, SelectionMarks { boxes, handles });
                nodes.selection = Some(n);
                layers.push(n);
            }
            let word_y = overlay
                .selection()
                .and_then(|(s, w)| w.get(s.start).map(|x| (x.rect.y0 + x.rect.y1) / 2));
            let place = Place::away_from(word_y, h);
            let height = popup_height(&page_words, place, h);
            let built = match overlay {
                Overlay::Dict {
                    lookup, highlight, ..
                } => Some(dict_popup(
                    t,
                    &styles,
                    lookup,
                    &dicts,
                    highlight.is_some(),
                    place,
                    height,
                )),
                Overlay::Waiting { .. } | Overlay::Dragging { .. } | Overlay::None => None,
                Overlay::Actions { .. } => {
                    let body = RichText::default();
                    Some(overlay::popup(
                        t,
                        &styles,
                        &PopupSpec {
                            title: "Zaznaczenie".into(),
                            subtitle: None,
                            tabs: Vec::new(),
                            body: &body,
                            suggestions: Vec::new(),
                            actions: vec![
                                ("Zakreśl", act::HIGHLIGHT),
                                ("Słownik", act::LOOKUP),
                                ("Tłumacz", act::GLOSS),
                                ("Anuluj", act::CANCEL),
                            ],
                            place,
                            height,
                        },
                    ))
                }
                Overlay::Gloss { text, .. } => Some(overlay::popup(
                    t,
                    &styles,
                    &PopupSpec {
                        title: "Tłumaczenie słowo po słowie".into(),
                        subtitle: Some("Ze słowników, bez sieci".into()),
                        tabs: Vec::new(),
                        body: text,
                        suggestions: Vec::new(),
                        actions: vec![("Zakreśl", act::HIGHLIGHT), ("Zamknij", act::CANCEL)],
                        place,
                        height,
                    },
                )),
                Overlay::Note { text, .. } => Some(overlay::popup(
                    t,
                    &styles,
                    &PopupSpec {
                        title: "Przypis".into(),
                        subtitle: None,
                        tabs: Vec::new(),
                        body: text,
                        suggestions: Vec::new(),
                        actions: vec![
                            ("Przejdź do przypisu", act::GO_NOTE),
                            ("Zamknij", act::CANCEL),
                        ],
                        place: Place::Bottom,
                        height: popup_height(&page_words, Place::Bottom, h),
                    },
                )),
                Overlay::Toc { items, current, .. } => {
                    let rows: Vec<ListRow> = items
                        .iter()
                        .map(|it| ListRow {
                            text: if it.title.is_empty() {
                                "—".into()
                            } else {
                                it.title.clone()
                            },
                            meta: None,
                            indent: it.depth,
                            current: Some(it.index) == *current,
                            lines: 2,
                        })
                        .collect();
                    let l = overlay::list(
                        t,
                        &styles,
                        "Spis treści",
                        &rows,
                        "Ta książka nie ma spisu treści.",
                    );
                    nodes.pages = Some(l.pages);
                    nodes.pager = Some(l.pager);
                    nodes.arrows = Some(l.arrows);
                    layers.push(l.root);
                    None
                }
                Overlay::Marks { rows, .. } => {
                    let rows: Vec<ListRow> = rows
                        .iter()
                        .map(|m| ListRow {
                            text: m.text.clone(),
                            meta: Some(m.meta.clone()),
                            indent: 0,
                            current: false,
                            lines: 3,
                        })
                        .collect();
                    let l = overlay::list(
                        t,
                        &styles,
                        "Zakreślenia",
                        &rows,
                        "Nie ma jeszcze zakreśleń ani zakładek. Przytrzymaj słowo, aby je zaznaczyć.",
                    );
                    nodes.pages = Some(l.pages);
                    nodes.pager = Some(l.pager);
                    nodes.arrows = Some(l.arrows);
                    layers.push(l.root);
                    None
                }
                Overlay::GoTo { value } => {
                    let g = overlay::goto_sheet(t, &styles, *value, "");
                    nodes.slider = Some(g.slider);
                    nodes.label = Some(g.label);
                    layers.push(g.root);
                    None
                }
            };
            if let Some(p) = built {
                nodes.pages = Some(p.pages);
                nodes.pager = Some(p.pager);
                nodes.arrows = Some(p.arrows);
                layers.push(p.root);
            }
            layers
        };
        let (tree, menu, page_node, nav) = crate::reader_tree_with(
            &styles,
            &r.session.title,
            &r.settings,
            page,
            (0, 0),
            &mut extra,
        );
        r.menu = menu;
        r.page_node = page_node;
        r.nav = nav;
        r.nodes = nodes;
        self.ui.set_tree_with(tree, refresh);
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let menu_open = r.menu_open;
        let (root, nav) = (r.menu.root, r.nav);
        self.ui.tree.set_hidden(root, !menu_open);
        self.ui.tree.set_hidden(nav, !menu_open);
        if menu_open {
            if let Screen::Reader(r) = &mut self.screen {
                crate::update_menu_labels(&mut self.ui, r);
            }
        }
        self.sync_pager();
        Ok(())
    }

    /// Turn the overlay's `Pages` node to the overlay's page and label it.
    fn sync_pager(&mut self) {
        self.ui.layout();
        let Screen::Reader(r) = &mut self.screen else {
            return;
        };
        let want = r.overlay.page_mut().map(|p| *p).unwrap_or(0);
        let (Some(pages), Some(pager)) = (r.nodes.pages, r.nodes.pager) else {
            return;
        };
        self.ui.tree.set_page(pages, want);
        self.ui.layout();
        let (p, n) = self.ui.tree.page(pages);
        if let Some(cur) = r.overlay.page_mut() {
            *cur = p;
        }
        let label = if n > 1 {
            format!("{}/{}", p + 1, n)
        } else {
            String::new()
        };
        self.ui.tree.set_text(pager, &label);
        if let Some((a, b)) = r.nodes.arrows {
            self.ui.tree.set_hidden(a, n <= 1);
            self.ui.tree.set_hidden(b, n <= 1);
        }
        self.ui.layout();
    }

    /// Send the frame as one update: grays go out as a full-screen mode-9
    /// update (clean, like a page turn), black and white changes as DU.
    pub(crate) fn present_full(&mut self) -> Result<(), Error> {
        let damage = self.ui.render();
        self.panel.present(&self.ui, &damage, Content::Page)?;
        Ok(())
    }

    /// Send the frame with the UI's own waveforms (A2 feedback, DU, GL16).
    fn present_fast(&mut self) -> Result<(), Error> {
        let damage = self.ui.render();
        self.panel.present(&self.ui, &damage, Content::Chrome)?;
        Ok(())
    }

    /// Take the overlay away: the page comes back with one full mode-9
    /// update of the whole screen.
    pub(crate) fn close_overlay(&mut self) -> Result<(), Error> {
        let Some(r) = self.reading() else {
            return Ok(());
        };
        r.overlay = Overlay::None;
        r.gestures.reset();
        self.rebuild_reader(Refresh::Auto)?;
        // The UI's copy of the screen follows; the panel gets the page.
        self.ui.render();
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let page = r.session.page(r.session.current)?;
        self.panel.show_page(&page, false, "overlay closed")?;
        self.page_changed();
        Ok(())
    }

    /// After the page on the panel changed: quiet zones for the gestures
    /// (links, the bookmark corner).
    pub(crate) fn page_changed(&mut self) {
        let Some(r) = self.reading() else {
            return;
        };
        let loc = r.session.current;
        let links = r.session.links(loc);
        let zones = links
            .iter()
            .map(|l| rect(&l.rect).expand_to(MIN_TARGET * 2 / 3))
            .collect();
        r.gestures.set_quiet(zones);
    }

    /// Show the page at `location` (a jump: contents, highlights, a link)
    /// with one full update, and close whatever covered it.
    pub(crate) fn jump_to(&mut self, location: usize) -> Result<(), Error> {
        let Some(r) = self.reading() else {
            return Ok(());
        };
        r.session.current = location;
        r.menu_open = false;
        self.close_overlay()?;
        let now = aristotle_library::library::now();
        if let Screen::Reader(r) = &mut self.screen {
            self.lib
                .set_progress(&r.session.id, r.session.progress(now));
            r.session.prerender();
        }
        self.speed.page_shown(Instant::now());
        self.debounce.changed(Instant::now());
        Ok(())
    }

    // ---------------------------------------------------------- gestures

    /// A long press at `at`.
    pub(crate) fn hold(&mut self, at: Point) -> Result<(), Error> {
        let t0 = Instant::now();
        let Some(r) = self.reading() else {
            return Ok(());
        };
        if !r.overlay.is_none() || r.menu_open {
            return Ok(());
        }
        let loc = r.session.current;
        let words = r.session.words(loc);
        let Some(i) = selection::hit(&words, at.x, at.y) else {
            return Ok(());
        };
        let sel = Selection::word(&words, i);
        let word = sel.text(&words);
        let highlight = self.highlight_at(words[sel.start].offset);
        let lang = self.book_lang();
        let Some(r) = self.reading() else {
            return Ok(());
        };
        r.overlay = Overlay::Waiting {
            sel,
            words: words.clone(),
        };
        let reply = match &self.dict {
            Some(d) if !d.is_empty() => {
                d.ask(Request::Lookup {
                    word: word.clone(),
                    book_lang: lang,
                });
                d.wait(LOOKUP_PATIENCE)
            }
            _ => None,
        };
        let no_dicts = self.dict.as_ref().is_none_or(|d| d.is_empty());
        match reply {
            Some(Reply::Lookup(l)) => {
                perf!("hold: \"{word}\" looked up in {:.1} ms", l.millis);
                self.show_lookup(l, highlight)?;
            }
            _ if no_dicts => {
                let l = Lookup {
                    word: word.clone(),
                    lang: String::new(),
                    order: Vec::new(),
                    shown: None,
                    found: None,
                    suggestions: Vec::new(),
                    indexing: false,
                    millis: 0.0,
                };
                self.show_lookup(l, highlight)?;
            }
            _ => {
                // The dictionary is slow: the word first, the popup when
                // the answer comes (see `background`).
                perf!("hold: \"{word}\" selected, dictionary still looking");
                self.rebuild_reader(Refresh::Auto)?;
                if let Some(n) = self.reading().and_then(|r| r.nodes.selection) {
                    self.ui.tree.invalidate(n, Refresh::Transient);
                }
                self.present_fast()?;
            }
        }
        perf!(
            "hold: shown {:.0} ms after the long press",
            crate::ms(t0.elapsed())
        );
        Ok(())
    }

    /// The dictionary answered: show the popup (one update).
    fn show_lookup(&mut self, l: Lookup, highlight: Option<usize>) -> Result<(), Error> {
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let (sel, words) = match r.overlay.selection() {
            Some((s, w)) => (*s, w.clone()),
            None => return Ok(()),
        };
        r.overlay = Overlay::Dict {
            sel,
            words,
            lookup: Box::new(l),
            highlight,
            page: 0,
        };
        self.rebuild_reader(Refresh::Auto)?;
        self.present_full()
    }

    /// Work between events: a late dictionary answer.
    pub(crate) fn poll_lookup(&mut self) -> Result<(), Error> {
        let waiting = matches!(
            self.reading().map(|r| &r.overlay),
            Some(Overlay::Waiting { .. })
        );
        if !waiting {
            return Ok(());
        }
        let Some(reply) = self.dict.as_ref().and_then(|d| d.wait(Duration::ZERO)) else {
            return Ok(());
        };
        if let Reply::Lookup(l) = reply {
            let highlight = match self.reading().map(|r| &r.overlay) {
                Some(Overlay::Waiting { sel, words }) => {
                    let off = words[sel.start].offset;
                    self.highlight_at(off)
                }
                _ => None,
            };
            self.show_lookup(l, highlight)?;
        }
        Ok(())
    }

    /// The finger moved while holding: grow the selection.
    pub(crate) fn drag(&mut self, p: Point) -> Result<(), Error> {
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let (mut sel, words, had_popup) = match &r.overlay {
            Overlay::Waiting { sel, words } => (*sel, words.clone(), false),
            Overlay::Dict { sel, words, .. } => (*sel, words.clone(), true),
            Overlay::Dragging { sel, words } => (*sel, words.clone(), false),
            _ => return Ok(()),
        };
        let Some(j) = nearest(&words, p) else {
            return Ok(());
        };
        if !sel.extend_to(&words, j) {
            return Ok(());
        }
        let first = !matches!(r.overlay, Overlay::Dragging { .. });
        r.overlay = Overlay::Dragging {
            sel,
            words: words.clone(),
        };
        if first {
            // Handles appear; a popup, if there was one, goes with a clean
            // full update.
            self.rebuild_reader(Refresh::Auto)?;
            if had_popup {
                return self.present_full();
            }
            if let Some(n) = self.reading().and_then(|r| r.nodes.selection) {
                self.ui.tree.invalidate(n, Refresh::Transient);
            }
            return self.present_fast();
        }
        let Some(n) = self.reading().and_then(|r| r.nodes.selection) else {
            return Ok(());
        };
        if let Some(w) = self.ui.tree.widget_mut::<SelectionMarks>(n) {
            w.boxes = selection::line_boxes(&words, sel.start, sel.end);
        }
        self.ui.tree.invalidate(n, Refresh::Transient);
        self.present_fast()
    }

    /// The finger lifted after a hold.
    pub(crate) fn release(&mut self, _p: Point) -> Result<(), Error> {
        let Some(r) = self.reading() else {
            return Ok(());
        };
        if let Overlay::Dragging { sel, words } = &r.overlay {
            r.overlay = Overlay::Actions {
                sel: *sel,
                words: words.clone(),
            };
            self.rebuild_reader(Refresh::Auto)?;
            self.present_full()?;
        }
        Ok(())
    }

    /// A tap in a quiet zone: the bookmark corner or a link.
    pub(crate) fn quiet_tap(&mut self, p: Point) -> Result<(), Error> {
        let screen = self.ui.bounds();
        if bookmark_corner(screen).contains(p) {
            return self.toggle_bookmark();
        }
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let loc = r.session.current;
        let links = r.session.links(loc);
        let Some(link) = links
            .iter()
            .filter(|l| rect(&l.rect).expand_to(MIN_TARGET * 2 / 3).contains(p))
            .min_by_key(|l| l.rect.dist2(p.x, p.y))
            .cloned()
        else {
            return Ok(());
        };
        let words = r.session.words(loc);
        let marker: Vec<&str> = words
            .iter()
            .filter(|w| {
                let (x, y) = ((w.rect.x0 + w.rect.x1) / 2, (w.rect.y0 + w.rect.y1) / 2);
                link.rect.contains(x, y)
            })
            .map(|w| w.text.as_str())
            .collect();
        let marker = marker.join("");
        let t = Instant::now();
        let note = r.session.book().footnote(&link, &marker);
        perf!(
            "link {:?} ({:?}): {} in {:.0} ms",
            link.uri,
            marker,
            if note.is_some() { "footnote" } else { "a jump" },
            crate::ms(t.elapsed())
        );
        match note {
            Some(text) => {
                r.overlay = Overlay::Note {
                    text,
                    link,
                    page: 0,
                };
                self.rebuild_reader(Refresh::Auto)?;
                self.present_full()
            }
            None => match r.session.book().link_page(loc, &link.uri) {
                Some(target) => self.jump_to(target),
                None => Ok(()),
            },
        }
    }

    // ------------------------------------------------------ marks

    fn book_lang(&mut self) -> Option<String> {
        let r = self.reading()?;
        r.session.book().language().map(String::from)
    }

    /// Plato locations of this book's highlights and bookmarks, from the
    /// index, into the session (pages are redrawn with them).
    pub(crate) fn reload_marks(&mut self) {
        let Screen::Reader(r) = &mut self.screen else {
            return;
        };
        let Some(rec) = self.lib.book(&r.session.id) else {
            return;
        };
        let book = r.session.book();
        let hl = marks::ours(rec, AnnotationKind::Highlight)
            .into_iter()
            .filter_map(|(_, s, e)| Some((book.location_of(&s)?, book.location_of(&e)?)))
            .collect();
        let bm = marks::ours(rec, AnnotationKind::Bookmark)
            .into_iter()
            .filter_map(|(_, s, _)| book.location_of(&s))
            .collect();
        r.session.set_marks(hl, bm);
    }

    /// The annotation index of a highlight covering `offset`.
    fn highlight_at(&mut self, offset: usize) -> Option<usize> {
        let Screen::Reader(r) = &mut self.screen else {
            return None;
        };
        let rec = self.lib.book(&r.session.id)?;
        let book = r.session.book();
        marks::ours(rec, AnnotationKind::Highlight)
            .into_iter()
            .find(|(_, s, e)| {
                matches!((book.location_of(s), book.location_of(e)), (Some(a), Some(b)) if a <= offset && offset <= b)
            })
            .map(|(i, _, _)| i)
    }

    /// Save the selection as a highlight, export it, close.
    fn highlight_selection(&mut self) -> Result<(), Error> {
        let created = marks::local_time(aristotle_library::library::now());
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        let Some((sel, words)) = r.overlay.selection().map(|(s, w)| (*s, w.clone())) else {
            return Ok(());
        };
        let (s, e) = (words[sel.start].offset, words[sel.end].offset);
        let book = r.session.book();
        let start = book.position_at(s);
        let end = book.position_at(e);
        let bytes = |p: &Position, loc: usize| match p {
            Position::Kindle(k) => *k,
            _ => loc as u64,
        };
        let fixed = book.engine() != Engine::Plato;
        let a = marks::highlight(marks::NewHighlight {
            text: sel.text(&words),
            start_bytes: bytes(&start, s),
            end_bytes: bytes(&end, e) + words[sel.end].text.len() as u64,
            start,
            end,
            page: fixed.then_some(s + 1),
            chapter: book.chapter_title(s),
            created,
        });
        let id = r.session.id.clone();
        perf!("highlight: {:?}", a.text);
        self.lib.add_annotation(&id, a);
        self.saver.save(&self.lib);
        self.debounce.saved();
        if let Some(rec) = self.lib.book(&id) {
            marks::export(&self.exports, rec);
        }
        self.reload_marks();
        self.close_overlay()?;
        // The page now shows the change: the next open's snapshot too.
        self.save_snapshot();
        Ok(())
    }

    fn remove_highlight(&mut self, index: usize) -> Result<(), Error> {
        let Screen::Reader(r) = &self.screen else {
            return Ok(());
        };
        let id = r.session.id.clone();
        let target = self
            .lib
            .book(&id)
            .and_then(|b| b.annotations.get(index))
            .and_then(|a| a.position.clone());
        if let Some(pos) = target {
            self.lib.remove_annotations(&id, |a| {
                a.source == marks::SOURCE
                    && a.kind == AnnotationKind::Highlight
                    && a.position.as_ref() == Some(&pos)
            });
            self.saver.save(&self.lib);
            self.debounce.saved();
            if let Some(rec) = self.lib.book(&id) {
                marks::export(&self.exports, rec);
            }
        }
        self.reload_marks();
        self.close_overlay()?;
        // The page now shows the change: the next open's snapshot too.
        self.save_snapshot();
        Ok(())
    }

    /// The top right corner: add or remove a bookmark on this page. The
    /// ribbon is black on white, so it changes with a quick DU update.
    pub(crate) fn toggle_bookmark(&mut self) -> Result<(), Error> {
        let created = marks::local_time(aristotle_library::library::now());
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        let id = r.session.id.clone();
        let loc = r.session.current;
        match r.session.bookmarked(loc) {
            Some(b) => {
                let book = r.session.book();
                let pos = marks::encode(&book.position_at(b));
                self.lib.remove_annotations(&id, |a| {
                    a.source == marks::SOURCE
                        && a.kind == AnnotationKind::Bookmark
                        && a.position.as_ref().map(|p| &p.0) == Some(&pos)
                });
                perf!("bookmark removed");
            }
            None => {
                let book = r.session.book();
                let at = book.position_at(loc);
                let bytes = match at {
                    Position::Kindle(k) => k,
                    _ => loc as u64,
                };
                let chapter = book.chapter_title(loc);
                self.lib
                    .add_annotation(&id, marks::bookmark(&at, bytes, chapter, created));
                perf!("bookmark added");
            }
        }
        self.saver.save(&self.lib);
        self.debounce.saved();
        self.reload_marks();
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        let page = r.session.page(r.session.current)?;
        if let Some(w) = self.ui.tree.widget_mut::<crate::PageView>(r.page_node) {
            w.page = page;
        }
        self.ui.tree.invalidate(r.page_node, Refresh::Auto);
        self.present_fast()?;
        // The page now shows the change: the next open's snapshot too.
        self.save_snapshot();
        Ok(())
    }

    // ------------------------------------------------------ lists

    pub(crate) fn open_toc(&mut self) -> Result<(), Error> {
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let loc = r.session.current;
        let items = r.session.book().toc_items();
        let current = r.session.book().toc_current(loc);
        // Start on the page that shows the current chapter.
        let page = 0;
        r.overlay = Overlay::Toc {
            items,
            current,
            page,
        };
        r.menu_open = false;
        self.rebuild_reader(Refresh::Auto)?;
        // Turn the list to the current entry.
        let target = match self.reading() {
            Some(r) => match (&r.overlay, r.nodes.pages) {
                (
                    Overlay::Toc {
                        items,
                        current: Some(c),
                        ..
                    },
                    Some(pages),
                ) => items
                    .iter()
                    .position(|it| it.index == *c)
                    .map(|idx| (idx, pages)),
                _ => None,
            },
            None => None,
        };
        if let Some((idx, pages)) = target {
            let row = self.ui.tree.node(pages).children.get(idx).copied();
            let mut p = 0;
            loop {
                self.ui.tree.set_page(pages, p);
                self.ui.layout();
                let (cur, n) = self.ui.tree.page(pages);
                // Only rows on the shown page can be hit.
                let visible = row.is_some_and(|id| {
                    let r = self.ui.tree.rect(id);
                    let c = Point {
                        x: r.x + r.w / 2,
                        y: r.y + r.h / 2,
                    };
                    self.ui.tree.hit(c) == aristotle_ui::tree::Hit::Target(id)
                });
                if visible || cur + 1 >= n {
                    if let Some(Overlay::Toc { page, .. }) = self.reading().map(|r| &mut r.overlay)
                    {
                        *page = cur;
                    }
                    break;
                }
                p += 1;
            }
        }
        self.sync_pager();
        self.present_full()
    }

    pub(crate) fn open_marks(&mut self) -> Result<(), Error> {
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        let mut rows = Vec::new();
        if let Some(rec) = self.lib.book(&r.session.id) {
            let mut all: Vec<(usize, &aristotle_library::index::Annotation, Position)> = rec
                .annotations
                .iter()
                .filter(|a| a.source == marks::SOURCE)
                .filter_map(|a| {
                    let p = marks::decode(&a.position.as_ref()?.0)?;
                    let loc = r.session.book().location_of(&p)?;
                    Some((loc, a, p))
                })
                .collect();
            all.sort_by_key(|x| x.0);
            for (_, a, p) in all {
                let date = a.created.as_deref().map(|c| c.get(..10).unwrap_or(c));
                let meta: Vec<String> = [
                    a.chapter.clone(),
                    aristotle_library::export::location_pl(a.location),
                    date.map(String::from),
                ]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .collect();
                let text = match a.kind {
                    AnnotationKind::Bookmark => "Zakładka".to_string(),
                    _ => format!("„{}”", a.text.clone().unwrap_or_default()),
                };
                rows.push(MarkRow {
                    text,
                    meta: meta.join(" · "),
                    target: p,
                });
            }
        }
        r.overlay = Overlay::Marks { rows, page: 0 };
        r.menu_open = false;
        self.rebuild_reader(Refresh::Auto)?;
        self.present_full()
    }

    pub(crate) fn open_goto(&mut self) -> Result<(), Error> {
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let here = r.session.current;
        let value = r.session.book().fraction(here);
        r.overlay = Overlay::GoTo { value };
        r.menu_open = false;
        self.rebuild_reader(Refresh::Auto)?;
        self.update_goto_label();
        self.present_full()
    }

    fn update_goto_label(&mut self) {
        let Some(r) = self.reading() else {
            return;
        };
        let Overlay::GoTo { value } = r.overlay else {
            return;
        };
        let detail = match r.session.book().known_page_count() {
            Some(n) if r.session.book().engine() == Engine::Plato => {
                format!(
                    "strona {} z {n}",
                    ((value * n as f32).ceil() as usize).clamp(1, n)
                )
            }
            _ => String::new(),
        };
        let (label, slider) = (r.nodes.label, r.nodes.slider);
        if let Some(l) = label {
            self.ui
                .tree
                .set_text(l, &overlay::percent_label(value, &detail));
        }
        if let Some(s) = slider {
            if let Some(w) = self.ui.tree.widget_mut::<Slider>(s) {
                w.value = value;
            }
            self.ui.tree.invalidate(s, Refresh::Auto);
        }
    }

    // ------------------------------------------------------ UI events

    /// UI events while an overlay is open (or the reading row of the menu).
    pub(crate) fn on_overlay_event(&mut self, ev: UiEvent) -> Result<(), Error> {
        match ev {
            UiEvent::Tap(_) => {
                // Outside the sheet: close. Lists cover the screen, so
                // this only happens around popups.
                self.close_overlay()
            }
            UiEvent::Swipe(Direction::Left) => self.overlay_page(1),
            UiEvent::Swipe(Direction::Right) => self.overlay_page(-1),
            UiEvent::Action { id, node, at } => self.overlay_action(id, node, at),
            _ => Ok(()),
        }
    }

    fn overlay_page(&mut self, delta: i32) -> Result<(), Error> {
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let Some(p) = r.overlay.page_mut() else {
            return Ok(());
        };
        let new = (*p as i32 + delta).max(0) as usize;
        if new == *p {
            return Ok(());
        }
        *p = new;
        self.sync_pager();
        let full = matches!(
            self.reading().map(|r| &r.overlay),
            Some(Overlay::Toc { .. } | Overlay::Marks { .. })
        );
        if full {
            // A list page is a new screen of text: clean, like a page.
            self.present_full()
        } else {
            self.present_fast()
        }
    }

    fn overlay_action(&mut self, id: ActionId, node: NodeId, at: Point) -> Result<(), Error> {
        match id {
            act::NAV_TOC => self.open_toc(),
            act::NAV_MARKS => self.open_marks(),
            act::NAV_GOTO => self.open_goto(),
            act::PREV => self.overlay_page(-1),
            act::NEXT => self.overlay_page(1),
            act::CANCEL | act::BACK => self.close_overlay(),
            act::HIGHLIGHT => self.highlight_selection(),
            act::UNHIGHLIGHT => {
                let idx = match self.reading().map(|r| &r.overlay) {
                    Some(Overlay::Dict {
                        highlight: Some(i), ..
                    }) => Some(*i),
                    _ => None,
                };
                match idx {
                    Some(i) => self.remove_highlight(i),
                    None => self.close_overlay(),
                }
            }
            act::LOOKUP => self.lookup_selection(),
            act::GLOSS => self.gloss_selection(),
            act::GO_NOTE => {
                let Some(r) = self.reading() else {
                    return Ok(());
                };
                let target = match &r.overlay {
                    Overlay::Note { link, .. } => {
                        let (loc, uri) = (r.session.current, link.uri.clone());
                        r.session.book().link_page(loc, &uri)
                    }
                    _ => None,
                };
                match target {
                    Some(t) => self.jump_to(t),
                    None => self.close_overlay(),
                }
            }
            id if (act::TAB..act::TAB + 100).contains(&id) => {
                self.switch_dict((id - act::TAB) as usize)
            }
            id if (act::SUGGEST..act::SUGGEST + 100).contains(&id) => {
                self.suggestion((id - act::SUGGEST) as usize)
            }
            id if (act::ITEM..act::GOTO_SET).contains(&id) => {
                self.list_item((id - act::ITEM) as usize)
            }
            act::GOTO_SET => {
                let v = Slider::value_at(self.ui.tree.rect(node), at.x);
                self.set_goto(|_| v)
            }
            act::GOTO_MINUS10 => self.set_goto(|v| v - 0.10),
            act::GOTO_MINUS1 => self.set_goto(|v| v - 0.01),
            act::GOTO_PLUS1 => self.set_goto(|v| v + 0.01),
            act::GOTO_PLUS10 => self.set_goto(|v| v + 0.10),
            act::GOTO_GO => {
                let Some(r) = self.reading() else {
                    return Ok(());
                };
                let Overlay::GoTo { value } = r.overlay else {
                    return Ok(());
                };
                let book = r.session.book();
                let target = book
                    .location_of(&Position::Fraction(value.min(0.9999)))
                    .and_then(|l| book.page_start(l));
                match target {
                    Some(t) => self.jump_to(t),
                    None => self.close_overlay(),
                }
            }
            _ => Ok(()),
        }
    }

    fn set_goto(&mut self, f: impl Fn(f32) -> f32) -> Result<(), Error> {
        let Some(r) = self.reading() else {
            return Ok(());
        };
        if let Overlay::GoTo { value } = &mut r.overlay {
            *value = f(*value).clamp(0.0, 1.0);
        }
        self.update_goto_label();
        self.present_fast()
    }

    fn list_item(&mut self, i: usize) -> Result<(), Error> {
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let target = match &r.overlay {
            Overlay::Toc { items, .. } => {
                let index = items.get(i).map(|it| it.index);
                index.and_then(|ix| r.session.book().toc_page(ix))
            }
            Overlay::Marks { rows, .. } => {
                let pos = rows.get(i).map(|m| m.target.clone());
                let book = r.session.book();
                pos.and_then(|p| book.location_of(&p))
                    .and_then(|l| book.page_start(l))
            }
            _ => None,
        };
        match target {
            Some(t) => self.jump_to(t),
            None => Ok(()),
        }
    }

    fn switch_dict(&mut self, i: usize) -> Result<(), Error> {
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let Overlay::Dict { lookup, .. } = &r.overlay else {
            return Ok(());
        };
        // Tab `i` is the i-th dictionary of the lookup's order.
        let Some(&i) = lookup.order.get(i) else {
            return Ok(());
        };
        if lookup.shown == Some(i) {
            return Ok(());
        }
        let l = (**lookup).clone();
        let reply = self.dict.as_ref().and_then(|d| {
            d.ask(Request::Switch { lookup: l, dict: i });
            d.wait(Duration::from_secs(2))
        });
        if let Some(Reply::Lookup(l)) = reply {
            if let Some(r) = self.reading() {
                if let Overlay::Dict { lookup, page, .. } = &mut r.overlay {
                    **lookup = l;
                    *page = 0;
                }
            }
            self.rebuild_reader(Refresh::Auto)?;
            // Only the popup's inside changes: a partial update of it.
            self.present_fast()?;
        }
        Ok(())
    }

    fn suggestion(&mut self, i: usize) -> Result<(), Error> {
        let lang = self.book_lang();
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let Overlay::Dict { lookup, .. } = &r.overlay else {
            return Ok(());
        };
        let Some(word) = lookup.suggestions.get(i).cloned() else {
            return Ok(());
        };
        let reply = self.dict.as_ref().and_then(|d| {
            d.ask(Request::Lookup {
                word,
                book_lang: lang,
            });
            d.wait(Duration::from_secs(2))
        });
        if let Some(Reply::Lookup(l)) = reply {
            if let Some(r) = self.reading() {
                if let Overlay::Dict { lookup, page, .. } = &mut r.overlay {
                    **lookup = l;
                    *page = 0;
                }
            }
            self.rebuild_reader(Refresh::Auto)?;
            self.present_fast()?;
        }
        Ok(())
    }

    /// "Słownik" on a dragged selection: look the phrase up (dictionaries
    /// have multi-word entries), else its first word.
    fn lookup_selection(&mut self) -> Result<(), Error> {
        let lang = self.book_lang();
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let Some((sel, words)) = r.overlay.selection().map(|(s, w)| (*s, w.clone())) else {
            return Ok(());
        };
        let phrase = sel.text(&words);
        let first = selection::text(&words, sel.anchor.0, sel.anchor.1);
        let Some(d) = self.dict.as_ref() else {
            return Ok(());
        };
        d.ask(Request::Lookup {
            word: phrase,
            book_lang: lang.clone(),
        });
        let mut l = match d.wait(Duration::from_secs(2)) {
            Some(Reply::Lookup(l)) => l,
            _ => return Ok(()),
        };
        if l.found.is_none() {
            d.ask(Request::Lookup {
                word: first,
                book_lang: lang,
            });
            if let Some(Reply::Lookup(x)) = d.wait(Duration::from_secs(2)) {
                l = x;
            }
        }
        let highlight = self.highlight_at(words[sel.start].offset);
        if let Some(r) = self.reading() {
            r.overlay = Overlay::Dict {
                sel,
                words,
                lookup: Box::new(l),
                highlight,
                page: 0,
            };
        }
        self.rebuild_reader(Refresh::Auto)?;
        self.present_full()
    }

    /// "Tłumacz": each word's first sense from the dictionaries.
    fn gloss_selection(&mut self) -> Result<(), Error> {
        let lang = self.book_lang();
        let Some(r) = self.reading() else {
            return Ok(());
        };
        let Some((sel, words)) = r.overlay.selection().map(|(s, w)| (*s, w.clone())) else {
            return Ok(());
        };
        let list: Vec<String> = sel
            .text(&words)
            .split_whitespace()
            .map(dict::clean_word)
            .filter(|w| !w.is_empty())
            .take(40)
            .collect();
        let Some(d) = self.dict.as_ref() else {
            return Ok(());
        };
        let t = Instant::now();
        d.ask(Request::Gloss {
            words: list,
            book_lang: lang,
        });
        let Some(Reply::Gloss(lines)) = d.wait(Duration::from_secs(5)) else {
            return Ok(());
        };
        perf!(
            "gloss: {} words in {:.0} ms",
            lines.len(),
            crate::ms(t.elapsed())
        );
        let text = RichText {
            blocks: lines
                .into_iter()
                .map(|(w, g)| Block {
                    runs: vec![
                        Run {
                            text: w,
                            bold: true,
                            italic: false,
                        },
                        Run {
                            text: format!("  {g}"),
                            bold: false,
                            italic: false,
                        },
                    ],
                    heading: false,
                })
                .collect(),
        };
        if let Some(r) = self.reading() {
            r.overlay = Overlay::Gloss {
                sel,
                words,
                text,
                page: 0,
            };
        }
        self.rebuild_reader(Refresh::Auto)?;
        self.present_full()
    }
}

/// The dictionary popup for a lookup.
fn dict_popup(
    t: &mut Tree,
    st: &aristotle_ui::theme::Styles,
    l: &Lookup,
    dicts: &[String],
    in_highlight: bool,
    place: Place,
    height: i32,
) -> overlay::Popup {
    // Tab k shows dictionary `l.order[k]` (see `App::switch_dict`).
    let tabs: Vec<(String, bool)> = l
        .order
        .iter()
        .map(|&i| {
            (
                dicts.get(i).cloned().unwrap_or_default(),
                l.shown == Some(i),
            )
        })
        .collect();
    let body = match &l.found {
        Some(f) => dict::as_rich_text(f),
        None if dicts.is_empty() => RichText::plain(
            "Brak słowników. Skopiuj słowniki StarDict (.ifo, .idx, .dict.dz) do /mnt/us/dictionaries.",
        ),
        None if l.indexing => RichText::plain(
            "Słownik jest indeksowany (tylko za pierwszym razem). Spróbuj za chwilę.",
        ),
        None => RichText::plain("Nie znaleziono w tym słowniku."),
    };
    // The title is the dictionary form; the word as tapped goes under it
    // when it differs.
    let headword = l
        .found
        .as_ref()
        .and_then(|f| f.entries.first())
        .map(|e| e.headword.clone());
    let title = headword.clone().unwrap_or_else(|| l.word.clone());
    let differs = headword.is_some_and(|h| h.to_lowercase() != l.word.to_lowercase());
    let subtitle = match &l.found {
        Some(f) if f.approximate => Some(format!("{} ≈ forma podstawowa (zgadnięta)", l.word)),
        Some(_) if differs => Some(format!("{} → forma podstawowa", l.word)),
        _ => None,
    };
    let actions = vec![
        if in_highlight {
            ("Usuń zakreślenie", act::UNHIGHLIGHT)
        } else {
            ("Zakreśl", act::HIGHLIGHT)
        },
        ("Zamknij", act::CANCEL),
    ];
    overlay::popup(
        t,
        st,
        &PopupSpec {
            title,
            subtitle,
            tabs,
            body: &body,
            suggestions: l.suggestions.clone(),
            actions,
            place,
            height,
        },
    )
}

/// Height of a popup sheet that ends between two lines of the page
/// instead of cutting through one: [`overlay::POPUP_H`], or a little less
/// when its edge would cross a line.
fn popup_height(words: &[Word], place: Place, screen_h: i32) -> i32 {
    let max = overlay::POPUP_H;
    let edge = match place {
        Place::Bottom => screen_h - max,
        Place::Top => max,
    };
    // A line of text the edge would cross (word boxes on one line overlap
    // vertically; any word will do).
    let Some(w) = words.iter().find(|w| w.rect.y0 < edge && w.rect.y1 > edge) else {
        return max;
    };
    // Word boxes span the font's whole ascender and descender, a little
    // more than the line pitch, and glyphs keep inside them: a sheet edge
    // at the end of one line's box leaves the next line's glyphs alone.
    match place {
        // Start the sheet under that line's box.
        Place::Bottom => screen_h - (w.rect.y1 + 1),
        // End the sheet under the box of the line before it.
        Place::Top => {
            words
                .iter()
                .filter(|x| x.rect.y0 < w.rect.y0 - 10)
                .map(|x| x.rect.y1)
                .max()
                .unwrap_or(w.rect.y0)
                + 1
        }
    }
    .clamp(max * 3 / 4, max)
}
