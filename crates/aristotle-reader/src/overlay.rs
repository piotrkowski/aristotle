//! What the reading interactions draw over the page: the selection, the
//! popups (dictionary, footnote, gloss, selection actions), the contents
//! and highlights lists, "go to", and the row of reading actions shown with
//! the menu. All labels are Polish.
//!
//! These only build `aristotle-ui` trees; [`crate::reading`] decides when,
//! and how the result reaches the panel.

use aristotle_render::book::Bounds;
use aristotle_render::richtext::{Block, RichText};
use aristotle_ui::geom::{pt, Insets, Point, Rect, Size, MIN_TARGET};
use aristotle_ui::icons::Icon;
use aristotle_ui::screens::{icon_button, text_button};
use aristotle_ui::text::TextEngine;
use aristotle_ui::text::{Align, Paragraph, ParagraphStyle, Span, TextStyle};
use aristotle_ui::theme::{margin, Styles, INK, INK_2, RULE};
use aristotle_ui::tree::{
    pages, ActionId, Constraints, Cross, Kind, Node, NodeId, PaintCx, Tree, Widget,
};
use aristotle_ui::widgets::Slider;

/// Action ids, clear of the reader menu's (below 100).
pub mod act {
    use aristotle_ui::tree::ActionId;
    pub const PREV: ActionId = 1001;
    pub const NEXT: ActionId = 1002;
    pub const HIGHLIGHT: ActionId = 1003;
    pub const UNHIGHLIGHT: ActionId = 1004;
    pub const LOOKUP: ActionId = 1005;
    pub const GLOSS: ActionId = 1006;
    pub const GO_NOTE: ActionId = 1007;
    pub const CANCEL: ActionId = 1008;
    pub const BACK: ActionId = 1009;
    /// + index of the dictionary in the service's list.
    pub const TAB: ActionId = 1100;
    /// + index of the suggestion.
    pub const SUGGEST: ActionId = 1200;
    /// + index of the list row.
    pub const ITEM: ActionId = 2000;
    pub const GOTO_SET: ActionId = 3000;
    pub const GOTO_MINUS10: ActionId = 3001;
    pub const GOTO_MINUS1: ActionId = 3002;
    pub const GOTO_PLUS1: ActionId = 3003;
    pub const GOTO_PLUS10: ActionId = 3004;
    pub const GOTO_GO: ActionId = 3010;
    pub const NAV_TOC: ActionId = 3100;
    pub const NAV_MARKS: ActionId = 3101;
    pub const NAV_GOTO: ActionId = 3102;
}

/// The selection over the page: inverted word boxes, and handles while
/// the selection is being dragged.
pub struct SelectionMarks {
    pub boxes: Vec<Bounds>,
    pub handles: bool,
}

impl Widget for SelectionMarks {
    fn measure(&mut self, c: Constraints, _: &mut TextEngine) -> Size {
        c.constrain(c.max)
    }

    fn paint(&self, _r: Rect, cx: &mut PaintCx) {
        let p = &mut *cx.painter;
        for b in &self.boxes {
            // A little air around the glyphs, as a marker pen would leave.
            let m = crate::selection::marker_box(b);
            p.invert_rect(Rect::new(m.x0, m.y0, m.x1 - m.x0, m.y1 - m.y0));
        }
        if !self.handles {
            return;
        }
        if let Some(((x0, t0, b0), (x1, t1, b1))) = crate::selection::handles(&self.boxes) {
            for (x, top, bottom) in [(x0 - 4, t0, b0), (x1 + 4, t1, b1)] {
                p.fill_rect(Rect::new(x - 3, top - 2, 6, bottom - top + 14), INK);
                p.fill_circle(Point { x, y: bottom + 22 }, 14.0, INK);
            }
        }
    }
}

/// Selection boxes of the full width of the screen, so a widget change
/// damages exactly the union of old and new boxes.
pub fn selection_layer(t: &mut Tree, marks: SelectionMarks) -> NodeId {
    t.custom(marks)
}

/// Where a popup sits: away from the word it is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Top,
    Bottom,
}

impl Place {
    /// Bottom unless the word is in the lower half.
    pub fn away_from(y: Option<i32>, screen_h: i32) -> Place {
        match y {
            Some(y) if y > screen_h / 2 => Place::Top,
            _ => Place::Bottom,
        }
    }
}

/// A popup's parts.
pub struct PopupSpec<'a> {
    pub title: String,
    /// A note under the title ("≈ forma podstawowa", "pozycja 120").
    pub subtitle: Option<String>,
    /// Dictionary switcher: (label, active).
    pub tabs: Vec<(String, bool)>,
    pub body: &'a RichText,
    /// Tappable words under the body ("Może: …").
    pub suggestions: Vec<String>,
    /// Buttons: (label, action).
    pub actions: Vec<(&'static str, ActionId)>,
    pub place: Place,
    /// Sheet height with a body (see `reading::popup_height`).
    pub height: i32,
}

/// Nodes of a built popup.
pub struct Popup {
    pub root: NodeId,
    pub pages: NodeId,
    pub pager: NodeId,
    pub arrows: (NodeId, NodeId),
}

/// Body text size in popups: a little larger than the UI, for reading.
const BODY_PT: f32 = 9.0;
/// Most text a popup paragraph holds before it is split: about one popup
/// page, so the pager can turn through long notes.
const CHUNK: usize = 320;
/// Height of a popup: under half the screen, so the word stays visible.
pub const POPUP_H: i32 = 700;

fn body_style(st: &Styles) -> TextStyle {
    TextStyle::new(st.family, pt(BODY_PT)).line_height(1.32)
}

/// Split a block into pieces of at most about [`CHUNK`] characters, at
/// sentence ends where possible, keeping run styles.
fn chunks(b: &Block) -> Vec<Block> {
    let total: usize = b.runs.iter().map(|r| r.text.chars().count()).sum();
    if total <= CHUNK {
        return vec![b.clone()];
    }
    let mut out = Vec::new();
    let mut cur = Block {
        runs: Vec::new(),
        heading: b.heading,
    };
    let mut len = 0;
    for r in &b.runs {
        for word in r.text.split_inclusive(' ') {
            let n = word.chars().count();
            let sentence_end = word.trim_end().ends_with(['.', ';', '?', '!']);
            match cur.runs.last_mut() {
                Some(last) if last.italic == r.italic && last.bold == r.bold => {
                    last.text.push_str(word)
                }
                _ => cur.runs.push(aristotle_render::richtext::Run {
                    text: word.to_string(),
                    italic: r.italic,
                    bold: r.bold,
                }),
            }
            len += n;
            if len >= CHUNK || (len >= CHUNK * 2 / 3 && sentence_end) {
                out.push(std::mem::take(&mut cur));
                cur.heading = b.heading;
                len = 0;
            }
        }
    }
    if !cur.runs.is_empty() {
        out.push(cur);
    }
    out
}

fn paragraph(st: &Styles, b: &Block) -> Paragraph {
    let base = body_style(st);
    let spans = b
        .runs
        .iter()
        .map(|r| {
            let mut s = base.clone();
            if r.italic {
                s = s.italic();
            }
            if r.bold || b.heading {
                s = s.bold();
            }
            Span::new(r.text.clone(), s)
        })
        .collect();
    Paragraph::new(
        spans,
        ParagraphStyle {
            align: Align::Left,
            hyphenate: false,
            ..Default::default()
        },
    )
}

/// A row of bordered text buttons.
fn button_row(t: &mut Tree, st: &Styles, actions: &[(&str, ActionId)]) -> NodeId {
    let mut row = Vec::new();
    for (label, id) in actions {
        let l = t.label(label, st.ui_bold());
        let l = t.pad(Insets::vh(0, 22), l);
        let b = t.button(Node::plain().border(2, INK, 14), l, *id);
        row.push(b);
    }
    t.add(Node::row(18), row)
}

/// Build a popup sheet at the top or bottom of the screen. Returns the
/// layer (full screen, transparent around the sheet).
///
/// Layout, top to bottom: the title with the dictionary switcher beside it,
/// the body (paged; a note such as "≈ forma podstawowa" first, in gray),
/// and the actions with the pager at the right.
pub fn popup(t: &mut Tree, st: &Styles, spec: &PopupSpec) -> Popup {
    let m = margin();
    let mut rows = Vec::new();

    // Title, then the dictionary switcher: one tap per dictionary.
    let mut title = Paragraph::plain(spec.title.clone(), st.title(), Align::Left);
    title.style.max_lines = Some(1);
    let title = t.text(title);
    let title = t.add(Node::column(0).flex(1), [title]);
    let mut head = vec![title];
    for (i, (label, active)) in spec.tabs.iter().enumerate() {
        let l = t.label(label, if *active { st.ui_bold() } else { st.ui() });
        let l = t.pad(Insets::vh(0, 14), l);
        let node = Node::plain().border(2, INK, 12).inverted(*active);
        head.push(t.button(node, l, act::TAB + i as ActionId));
    }
    let head = t.add(Node::row(10), head);
    let head = t.fixed(None, Some(MIN_TARGET), Some(head));
    rows.push(head);

    // Body, paged.
    let mut paras = Vec::new();
    if let Some(sub) = &spec.subtitle {
        let mut p = Paragraph::plain(sub.clone(), st.small(), Align::Left);
        p.style.max_lines = Some(2);
        paras.push(t.text(p));
    }
    for b in spec.body.blocks.iter().flat_map(chunks) {
        paras.push(t.text(paragraph(st, &b)));
    }
    if !spec.suggestions.is_empty() {
        let l = t.label("Może chodziło o:", st.small());
        paras.push(l);
        let mut row = Vec::new();
        for (i, s) in spec.suggestions.iter().enumerate() {
            let b = text_button(t, s, st.ui(), act::SUGGEST + i as ActionId);
            row.push(b);
        }
        let row = t.add(Node::row(8), row);
        paras.push(row);
    }
    // Without a body (the selection's action sheet) the sheet takes its
    // natural height.
    let has_body = !spec.body.blocks.is_empty() || !spec.suggestions.is_empty();
    let body = t.add(pages(8), paras);
    let pages_node = body;
    if has_body {
        let body = t.add(Node::column(0).flex(1), [body]);
        rows.push(body);
    }

    // Actions, and the pager at the right.
    let mut foot = vec![button_row(t, st, &spec.actions)];
    let fill = t.spacer(1);
    foot.push(fill);
    let prev = icon_button(t, Icon::ChevronLeft, 40, act::PREV);
    let pager = t.label("", st.small());
    let next = icon_button(t, Icon::ChevronRight, 40, act::NEXT);
    if has_body {
        foot.extend([prev, pager, next]);
    }
    let foot = t.add(Node::row(4), foot);
    rows.push(foot);

    let content = t.add(Node::column(14).cross(Cross::Stretch), rows);
    let content = t.pad(Insets::new(18, m, 18, m), content);
    let rule = t.rule(3, INK);
    let sheet_children = match spec.place {
        Place::Bottom => [rule, content],
        Place::Top => [content, rule],
    };
    let sheet = t.add(Node::column(0).bg(255), sheet_children);
    let sheet = if has_body {
        t.fixed(None, Some(spec.height), Some(sheet))
    } else {
        sheet
    };
    let fill = t.spacer(1);
    let layer = match spec.place {
        Place::Bottom => [fill, sheet],
        Place::Top => [sheet, fill],
    };
    let root = t.add(Node::column(0), layer);
    // The page is set once laid out (a `Pages` node counts its pages in
    // layout); see `App::sync_pager`.
    Popup {
        root,
        pages: pages_node,
        pager,
        arrows: (prev, next),
    }
}

/// A full-screen list (contents, highlights) with a title bar and a pager.
pub struct ListRow {
    pub text: String,
    /// Smaller second line (chapter, location, date).
    pub meta: Option<String>,
    pub indent: usize,
    pub current: bool,
    /// Lines of `text` shown at most.
    pub lines: usize,
}

pub struct List {
    pub root: NodeId,
    pub pages: NodeId,
    pub pager: NodeId,
    pub arrows: (NodeId, NodeId),
}

pub fn list(t: &mut Tree, st: &Styles, title: &str, rows: &[ListRow], empty: &str) -> List {
    let m = margin();
    let back_icon = t.icon(Icon::ArrowLeft, 44, INK);
    let back_label = t.label("Wróć", st.ui());
    let g = t.gap(12, 1);
    let back = t.add(Node::row(0), [back_icon, g, back_label]);
    let back = t.pad(Insets::vh(0, 16), back);
    let back = t.button(Node::plain(), back, act::BACK);
    let mut tl = Paragraph::plain(title, st.title(), Align::Center);
    tl.style.max_lines = Some(1);
    let tl = t.text(tl);
    let tl = t.add(Node::column(0).flex(1), [tl]);
    let pad = t.gap(160, 1);
    let bar = t.add(Node::row(0), [back, tl, pad]);
    let bar = t.fixed(None, Some(120), Some(bar));
    let bar = t.pad(Insets::vh(0, m - 16), bar);
    let line = t.rule(2, INK);

    let mut items = Vec::new();
    if rows.is_empty() {
        let l = t.label(empty, st.ui().gray(INK_2));
        let l = t.pad(Insets::vh(40, 0), l);
        items.push(l);
    }
    for (i, r) in rows.iter().enumerate() {
        let style = if r.current { st.ui_bold() } else { st.ui() };
        let mut p = Paragraph::plain(r.text.clone(), style, Align::Left);
        p.style.max_lines = Some(r.lines.max(1));
        let text = t.text(p);
        let mut col = vec![text];
        if let Some(meta) = &r.meta {
            let mut mp = Paragraph::plain(meta.clone(), st.small(), Align::Left);
            mp.style.max_lines = Some(1);
            col.push(t.text(mp));
        }
        let col = t.add(Node::column(6), col);
        let col = t.pad(Insets::new(18, 0, 18, 36 * r.indent.min(4) as i32), col);
        let hair = t.rule(1, RULE);
        let row = t.add(Node::column(0), [col, hair]);
        let row = t.add(
            Node::new(Kind::Padding(Insets::all(0)))
                .action(act::ITEM + i as ActionId)
                .feedback(aristotle_ui::tree::Feedback::Outline),
            [row],
        );
        items.push(row);
    }
    let list = t.add(pages(0), items);
    let pages_node = list;
    let list = t.add(Node::column(0).flex(1), [list]);
    let list = t.pad(Insets::new(8, m, 0, m), list);

    let prev = icon_button(t, Icon::ChevronLeft, 48, act::PREV);
    let pager = t.label("", st.small());
    let pager_c = t.align(0.5, 0.5, pager);
    let pager_c = t.add(Node::column(0).flex(1), [pager_c]);
    let next = icon_button(t, Icon::ChevronRight, 48, act::NEXT);
    let foot = t.add(Node::row(0), [prev, pager_c, next]);
    let foot = t.pad(Insets::new(0, m - 16, 24, m - 16), foot);

    let root = t.add(Node::column(0).bg(255), [bar, line, list, foot]);
    List {
        root,
        pages: pages_node,
        pager,
        arrows: (prev, next),
    }
}

/// The "go to" sheet: percentage, slider, steps.
pub struct GoTo {
    pub root: NodeId,
    pub label: NodeId,
    pub slider: NodeId,
}

pub fn goto_sheet(t: &mut Tree, st: &Styles, value: f32, detail: &str) -> GoTo {
    let m = margin();
    let title = t.label("Przejdź do miejsca", st.title());
    let label = t.label(&percent_label(value, detail), st.ui());
    let slider = t.add(
        Node::new(Kind::Custom(Box::new(Slider { value })))
            .action(act::GOTO_SET)
            .feedback(aristotle_ui::tree::Feedback::None),
        [],
    );
    let slider_c = t.add(Node::column(0).cross(Cross::Stretch), [slider]);
    let slider_c = t.fixed(None, Some(aristotle_ui::geom::MIN_TARGET), Some(slider_c));
    let steps = button_row(
        t,
        st,
        &[
            ("−10%", act::GOTO_MINUS10),
            ("−1%", act::GOTO_MINUS1),
            ("+1%", act::GOTO_PLUS1),
            ("+10%", act::GOTO_PLUS10),
        ],
    );
    let go = button_row(t, st, &[("Przejdź", act::GOTO_GO), ("Anuluj", act::CANCEL)]);
    let col = t.add(
        Node::column(22).cross(Cross::Stretch),
        [title, label, slider_c, steps, go],
    );
    let col = t.pad(Insets::new(28, m, 34, m), col);
    let rule = t.rule(3, INK);
    let sheet = t.add(Node::column(0).bg(255), [rule, col]);
    let fill = t.spacer(1);
    let root = t.add(Node::column(0), [fill, sheet]);
    GoTo {
        root,
        label,
        slider,
    }
}

pub fn percent_label(value: f32, detail: &str) -> String {
    let p = (value * 100.0).round() as u32;
    if detail.is_empty() {
        format!("{p}%")
    } else {
        format!("{p}% · {detail}")
    }
}

/// The reading actions shown under the menu's top bar.
pub fn nav_bar(t: &mut Tree, st: &Styles) -> NodeId {
    let m = margin();
    let mut cells = Vec::new();
    for (label, id) in [
        ("Spis treści", act::NAV_TOC),
        ("Zakreślenia", act::NAV_MARKS),
        ("Przejdź do…", act::NAV_GOTO),
    ] {
        let l = t.label(label, st.ui());
        let l = t.align(0.5, 0.5, l);
        let b = t.button(Node::plain().flex(1), l, id);
        cells.push(b);
    }
    let row = t.add(Node::row(0).cross(Cross::Stretch), cells);
    let row = t.fixed(None, Some(112), Some(row));
    let row = t.pad(Insets::vh(0, m - 16), row);
    let line = t.rule(2, INK);
    let bar = t.add(Node::column(0).bg(255), [row, line]);
    // Below the menu's top bar (120 px and a 2 px rule).
    let above = t.gap(1, 122);
    let fill = t.spacer(1);
    t.add(Node::column(0).hidden(true), [above, bar, fill])
}
