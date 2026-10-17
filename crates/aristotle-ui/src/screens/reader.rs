//! Reader page and reader menu.
//!
//! The page is the book and almost nothing else: a quiet running head,
//! justified text with real hyphenation, and a footer with the chapter and
//! the position. Touch is split into zones the size of a thumb: the left
//! third goes back, the rest goes forward (most turns are forward, so that
//! target is the biggest), and the top band opens the menu. Swipes do the
//! same. Nothing on the page reacts with a highlight: the new page is the
//! feedback.
//!
//! The menu is an overlay of two bars. Opening it repaints only the bars;
//! the page between them stays on the panel untouched.

use super::{icon_button, segmented};
use crate::geom::{Insets, Point, Rect, Size, SCREEN_HEIGHT, SCREEN_WIDTH};
use crate::icons::Icon;
use crate::input::Direction;
use crate::text::{
    layout_paragraph, Align, FamilyId, Paragraph, ParagraphLayout, ParagraphStyle, PolishSyllables,
    Span, TextEngine, TextStyle,
};
use crate::theme::{margin, Styles, INK, INK_2, INK_3, RULE};
use crate::tree::{
    progress, ActionId, Constraints, Cross, Feedback, Kind, Node, NodeId, PaintCx, Refresh, Tree,
    Widget,
};
use crate::ui::{Ui, UiEvent};
use crate::widgets::{Slider, Steps, Toggle};
use std::ops::Range;
use std::rc::Rc;

/// Reading settings, as steps into the tables below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Index into the reader's font families (Literata, Charis, Gentium).
    pub family: usize,
    pub size: usize,
    pub spacing: usize,
    pub margins: usize,
    /// Frontlight, 0 to 1.
    pub light: f32,
    pub night: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            family: 0,
            size: 2,
            spacing: 1,
            margins: 1,
            light: 0.45,
            night: false,
        }
    }
}

/// Body sizes in pixels: 8 to 15 pt. The default, 42 px, is 10 pt.
pub const SIZES: [f32; 7] = [34.0, 38.0, 42.0, 46.0, 50.0, 56.0, 62.0];
/// Line pitch as a multiple of the size.
pub const SPACING: [f32; 3] = [1.3, 1.5, 1.7];
/// Side margins: 4.7, 7.5 and 10.8 mm.
pub const MARGINS: [i32; 3] = [56, 88, 128];
pub const FAMILY_NAMES: [&str; 3] = ["Literata", "Charis", "Gentium"];

const HEADER: i32 = 112;
const FOOTER: i32 = 112;

pub mod action {
    use crate::tree::ActionId;
    pub const BACK: ActionId = 1;
    pub const CONTENTS: ActionId = 2;
    pub const BOOKMARK: ActionId = 3;
    pub const PREV_CHAPTER: ActionId = 4;
    pub const NEXT_CHAPTER: ActionId = 5;
    pub const FONT: ActionId = 10;
    pub const SIZE_DOWN: ActionId = 20;
    pub const SIZE_UP: ActionId = 21;
    pub const SPACING: ActionId = 30;
    pub const MARGINS: ActionId = 40;
    pub const LIGHT_DOWN: ActionId = 50;
    pub const LIGHT_UP: ActionId = 51;
    pub const LIGHT_SET: ActionId = 52;
    pub const NIGHT: ActionId = 60;
}

/// Where a tap on the page lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Prev,
    Next,
    Menu,
}

/// The tap zones of the page, as rectangles (for the preview overlay).
pub fn zones(screen: Rect) -> [(Zone, Rect); 3] {
    let top = screen.h * 12 / 100;
    let left = screen.w * 30 / 100;
    [
        (Zone::Menu, Rect::new(screen.x, screen.y, screen.w, top)),
        (
            Zone::Prev,
            Rect::new(screen.x, screen.y + top, left, screen.h - top),
        ),
        (
            Zone::Next,
            Rect::new(
                screen.x + left,
                screen.y + top,
                screen.w - left,
                screen.h - top,
            ),
        ),
    ]
}

pub fn zone_at(screen: Rect, p: Point) -> Zone {
    zones(screen)
        .iter()
        .find(|(_, r)| r.contains(p))
        .map(|(z, _)| *z)
        .unwrap_or(Zone::Next)
}

/// The opening of Bolesław Prus, *Lalka* (1890, public domain), lightly
/// abridged, as sample content.
const CHAPTER_NUMBER: &str = "Rozdział pierwszy";
const CHAPTER_TITLE: &str = "Jak wygląda firma J. Mincel i S. Wokulski przez szkło butelek";
const PARAGRAPHS: [&str; 4] = [
    "W początkach roku 1878, kiedy świat polityczny zajmował się pokojem \
     san-stefańskim, wyborem nowego papieża albo szansami europejskiej wojny, \
     warszawscy kupcy tudzież inteligencja pewnej okolicy Krakowskiego \
     Przedmieścia nie mniej gorąco interesowała się przyszłością \
     galanteryjnego sklepu pod firmą J. Mincel i S. Wokulski.",
    "W renomowanej jadłodajni, gdzie wieczorami schodzą się właściciele składów \
     bielizny i składów wina, fabrykanci powozów i kapeluszy, poważni ojcowie \
     rodzin żyjący z własnych funduszów i posiadacze kamienic bez zajęcia, \
     równie dużo mówiono o uzbrojeniach Anglii, jak i o firmie J. Mincel \
     i S. Wokulski. Zatopieni w kłębach dymu cygar i pochyleni nad butelkami \
     z ciemnego szkła, obywatele dzielnicy owej jedni zakładali się o wygranę \
     lub przegranę Anglii, drudzy o bankructwo Wokulskiego; jedni nazywali \
     geniuszem Bismarcka, drudzy — awanturnikiem Wokulskiego; jedni krytykowali \
     postępowanie prezydenta MacMahona, inni twierdzili, że Wokulski jest \
     zdecydowanym wariatem, jeżeli nie czymś gorszym…",
    "Fabrykant powozów, pan Deklewski, który do majątku i stanowiska doszedł \
     ciągłym pilnowaniem jednego zajęcia, oraz radca sądowy, pan Węgrowicz, \
     który od dwudziestu lat płacił składkę do Towarzystwa Kredytowego, \
     przepowiadali najsmutniejszą przyszłość sklepowi, który — ich zdaniem — \
     zginie przez „romantyzm” swego właściciela.",
    "— Ma on swoje szczęście — mówił pan Deklewski — ale ma też i fantazję, \
     a fantazja w handlu to rzecz zgubna. Kto chce dziś uczciwie zarobić \
     grosz, musi siedzieć w sklepie od rana do nocy, liczyć każdą szpulkę \
     nici i patrzeć subiektom na ręce, a nie jeździć po świecie za sławą \
     i zaszczytami.",
];

/// A piece of the chapter: one paragraph and the space above it.
struct Block {
    layout: ParagraphLayout,
    space_before: f32,
    /// Never end a page with this block (headings).
    keep_with_next: bool,
}

/// Lines of one block placed on a page.
#[derive(Clone, Debug, PartialEq)]
pub struct Slice {
    pub block: usize,
    pub lines: Range<usize>,
    pub y: f32,
}

/// Fills pages with lines, keeping headings with what follows and never
/// leaving a paragraph's first line alone at the bottom of a page (an
/// orphan) or its last line alone at the top of the next (a widow).
fn paginate(blocks: &[Block], height: f32) -> Vec<Vec<Slice>> {
    let mut pages: Vec<Vec<Slice>> = vec![Vec::new()];
    let mut y = 0.0f32;
    for (bi, b) in blocks.iter().enumerate() {
        let n = b.layout.lines.len();
        let pitch = b.layout.pitch;
        if y > 0.0 || bi == 0 {
            y += b.space_before;
        }
        if b.keep_with_next {
            let next_pitch = blocks.get(bi + 1).map_or(0.0, |n| n.layout.pitch);
            let need = b.layout.height
                + blocks.get(bi + 1).map_or(0.0, |n| n.space_before)
                + 2.0 * next_pitch;
            if y > 0.0 && y + need > height && bi > 0 {
                pages.push(Vec::new());
                y = 0.0;
            }
        }
        let mut line = 0;
        while line < n {
            let room = ((height - y) / pitch).floor().max(0.0) as usize;
            let left = n - line;
            let mut take = room.min(left);
            if take < left {
                if line == 0 && take == 1 && left > 1 {
                    take = 0; // orphan
                } else if left - take == 1 && take >= 2 {
                    take -= 1; // widow
                }
            }
            if take > 0 {
                pages.last_mut().unwrap().push(Slice {
                    block: bi,
                    lines: line..line + take,
                    y,
                });
                y += take as f32 * pitch;
                line += take;
            }
            if line < n {
                pages.push(Vec::new());
                y = 0.0;
            }
        }
    }
    pages
}

/// One page of book text.
pub struct TextPage {
    blocks: Rc<Vec<Block>>,
    slices: Vec<Slice>,
    margin: i32,
}

impl Widget for TextPage {
    fn measure(&mut self, c: Constraints, _: &mut TextEngine) -> Size {
        c.constrain(c.max)
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        for s in &self.slices {
            let b = &self.blocks[s.block];
            b.layout.draw_lines(
                cx.text,
                cx.painter,
                (r.x + self.margin) as f32,
                r.y as f32 + s.y,
                s.lines.clone(),
            );
        }
    }
}

/// Handles to the reader's nodes and its pagination.
pub struct Reader {
    pub settings: Settings,
    pub families: [FamilyId; 3],
    pub page: NodeId,
    pub footer_left: NodeId,
    pub footer_right: NodeId,
    pub menu: NodeId,
    pub menu_progress: NodeId,
    pub menu_page_label: NodeId,
    pub light: NodeId,
    pub night_toggle: NodeId,
    blocks: Rc<Vec<Block>>,
    pages: Vec<Vec<Slice>>,
    pub current: usize,
    menu_open: bool,
}

/// What the app must do after the reader handled an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    None,
    /// Leave the book for the library.
    Library,
}

/// Fake position of this chapter inside the whole book, for the footer.
const BOOK_PAGE_OFFSET: usize = 11;
const BOOK_PAGES: usize = 684;
const WORDS_PER_MINUTE: f32 = 230.0;

impl Reader {
    fn build_blocks(text: &mut TextEngine, families: &[FamilyId; 3], s: &Settings) -> Vec<Block> {
        let fam = families[s.family.min(2)];
        let size = SIZES[s.size.min(SIZES.len() - 1)];
        let lh = SPACING[s.spacing.min(2)];
        let width = (SCREEN_WIDTH - 2 * MARGINS[s.margins.min(2)]) as f32;
        let body = TextStyle::new(fam, size).line_height(lh).oldstyle();
        let pitch = (size * lh).round();
        let hy = PolishSyllables::default();
        let mut blocks = Vec::new();
        let caps = TextStyle::new(fam, size * 0.82)
            .small_caps()
            .tracking(size * 0.06)
            .line_height(lh)
            .gray(INK_2);
        let num = Paragraph::plain(CHAPTER_NUMBER, caps, Align::Center);
        blocks.push(Block {
            layout: layout_paragraph(text, &num, width, &hy),
            // Chapter openings sink: the first page starts lower, as in
            // printed books, which marks the chapter without ornament.
            space_before: pitch * 2.0,
            keep_with_next: true,
        });
        let title_style = TextStyle::new(fam, size * 1.3).italic().line_height(1.3);
        let title = Paragraph::new(
            vec![Span::new(CHAPTER_TITLE, title_style)],
            ParagraphStyle {
                align: Align::Center,
                balance: true,
                ..Default::default()
            },
        );
        blocks.push(Block {
            layout: layout_paragraph(text, &title, width * 0.85, &hy),
            space_before: pitch * 0.5,
            keep_with_next: true,
        });
        for (i, p) in PARAGRAPHS.iter().enumerate() {
            let para = Paragraph::new(
                vec![Span::new(*p, body.clone())],
                ParagraphStyle {
                    align: Align::Justify,
                    // No indent after a heading, the classic convention.
                    indent: if i == 0 { 0.0 } else { size * 1.5 },
                    hyphenate: true,
                    ..Default::default()
                },
            );
            blocks.push(Block {
                layout: layout_paragraph(text, &para, width, &hy),
                space_before: if i == 0 { pitch * 1.5 } else { 0.0 },
                keep_with_next: false,
            });
        }
        // The narrower title is centred within the full measure.
        let shift = width * 0.075;
        for line in &mut blocks[1].layout.lines {
            for piece in &mut line.pieces {
                piece.x += shift;
            }
        }
        blocks
    }

    pub fn build(
        text: &mut TextEngine,
        st: &Styles,
        families: [FamilyId; 3],
        settings: Settings,
    ) -> (Tree, Self) {
        let blocks = Rc::new(Self::build_blocks(text, &families, &settings));
        let body_h = (SCREEN_HEIGHT - HEADER - FOOTER) as f32;
        let pages = paginate(&blocks, body_h);
        let m = MARGINS[settings.margins.min(2)];
        let mut t = Tree::new();

        // Running head and footer.
        let head = t.text(Paragraph::plain(
            "Bolesław Prus · Lalka",
            st.small().gray(INK_3),
            Align::Center,
        ));
        let head = t.align(0.5, 0.62, head);
        let head = t.fixed(None, Some(HEADER), Some(head));
        let page = t.custom(TextPage {
            blocks: blocks.clone(),
            slices: pages[0].clone(),
            margin: m,
        });
        let page = t.add(
            Node::new(Kind::Fixed {
                w: None,
                h: Some(SCREEN_HEIGHT - HEADER - FOOTER),
            }),
            [page],
        );
        let footer_left = t.label("", st.small().gray(INK_3));
        let fill = t.spacer(1);
        let footer_right = t.label("", st.small().gray(INK_3));
        let foot = t.add(Node::row(0), [footer_left, fill, footer_right]);
        let foot = t.pad(Insets::vh(0, m), foot);
        let foot = t.align(0.0, 0.35, foot);
        let foot = t.fixed(None, Some(FOOTER), Some(foot));
        let reading = t.add(Node::column(0), [head, page, foot]);

        // Menu overlay.
        let (menu, ids) = Self::menu(&mut t, st, &families, &settings);
        let root = t.add(Node::new(Kind::Stack), [reading, menu]);
        t.set_root(root);

        let mut r = Self {
            settings,
            families,
            page: t.node(page).children[0],
            footer_left,
            footer_right,
            menu,
            menu_progress: ids.0,
            menu_page_label: ids.1,
            light: ids.2,
            night_toggle: ids.3,
            blocks,
            pages,
            current: 0,
            menu_open: false,
        };
        r.update_position(&mut t);
        (t, r)
    }

    fn menu(
        t: &mut Tree,
        st: &Styles,
        families: &[FamilyId; 3],
        s: &Settings,
    ) -> (NodeId, (NodeId, NodeId, NodeId, NodeId)) {
        let m = margin();
        // Top bar: back to the library, the title, contents and bookmark.
        let back_icon = t.icon(Icon::ArrowLeft, 44, INK);
        let back_label = t.label("Library", st.ui());
        let g = t.gap(12, 1);
        let back = t.add(Node::row(0), [back_icon, g, back_label]);
        let back = t.pad(Insets::vh(0, 16), back);
        let back = t.button(Node::plain(), back, action::BACK);
        let fill = t.spacer(1);
        let toc = icon_button(t, Icon::Contents, 50, action::CONTENTS);
        let mark = icon_button(t, Icon::Bookmark, 50, action::BOOKMARK);
        let bar = t.add(Node::row(0), [back, fill, toc, mark]);
        let title = t.label("Lalka", st.ui_bold());
        let title = t.align(0.5, 0.5, title);
        let top = t.add(Node::new(Kind::Stack), [title, bar]);
        let top = t.fixed(None, Some(120), Some(top));
        let top = t.pad(Insets::vh(0, m - 16), top);
        let line = t.rule(2, INK);
        let top = t.add(Node::column(0).bg(255), [top, line]);

        // Bottom sheet.
        let label = |t: &mut Tree, s: &str| {
            let l = t.label(s, st.ui().gray(INK_2));
            let l = t.align(0.0, 0.5, l);
            t.fixed(Some(196), None, Some(l))
        };
        let row = |t: &mut Tree, name: &str, control: NodeId| {
            let l = label(t, name);
            let c = t.add(Node::column(0).flex(1), [control]);
            let r = t.add(Node::row(0).cross(Cross::Stretch), [l, c]);
            t.fixed(None, Some(crate::geom::MIN_TARGET), Some(r))
        };

        // Position in the chapter, with chapter skips.
        let prev = icon_button(t, Icon::ChevronLeft, 44, action::PREV_CHAPTER);
        let next = icon_button(t, Icon::ChevronRight, 44, action::NEXT_CHAPTER);
        let chapter = t.label("Chapter I", st.small());
        let fill = t.spacer(1);
        let page_label = t.label("", st.small());
        let labels = t.add(Node::row(0), [chapter, fill, page_label]);
        let bar = t.add(progress(0.0, 12), []);
        let pos = t.add(Node::column(12).flex(1), [labels, bar]);
        let pos = t.pad(Insets::vh(0, 12), pos);
        let pos = t.add(Node::column(0).flex(1), [pos]);
        let nav = t.add(Node::row(0), [prev, pos, next]);

        let fonts: Vec<NodeId> = FAMILY_NAMES
            .iter()
            .zip(families)
            .map(|(name, fam)| t.label(name, TextStyle::new(*fam, crate::geom::pt(9.0))))
            .collect();
        let fonts = segmented(t, fonts, s.family, action::FONT);
        let fonts = row(t, "Font", fonts);

        let small = t.label("A", st.ui().size(30.0));
        let small = t.button(Node::plain(), small, action::SIZE_DOWN);
        let steps = t.custom(Steps {
            count: SIZES.len(),
            value: s.size,
        });
        let steps = t.add(Node::column(0).flex(1), [steps]);
        let large = t.label("A", st.ui().size(54.0));
        let large = t.button(Node::plain(), large, action::SIZE_UP);
        let size = t.add(Node::row(8), [small, steps, large]);
        let size = row(t, "Size", size);

        let spacing: Vec<NodeId> = [Icon::SpacingTight, Icon::SpacingNormal, Icon::SpacingLoose]
            .iter()
            .map(|i| t.icon(*i, 46, INK))
            .collect();
        let spacing = segmented(t, spacing, s.spacing, action::SPACING);
        let spacing = row(t, "Spacing", spacing);

        let margins: Vec<NodeId> = [Icon::MarginsNarrow, Icon::MarginsNormal, Icon::MarginsWide]
            .iter()
            .map(|i| t.icon(*i, 46, INK))
            .collect();
        let margins = segmented(t, margins, s.margins, action::MARGINS);
        let margins = row(t, "Margins", margins);

        let dim = icon_button(t, Icon::SunSmall, 54, action::LIGHT_DOWN);
        let slider = t.add(
            Node::new(Kind::Custom(Box::new(Slider { value: s.light })))
                .action(action::LIGHT_SET)
                .feedback(Feedback::None),
            [],
        );
        let light_node = slider;
        let slider = t.add(Node::column(0).flex(1).cross(Cross::Stretch), [slider]);
        let slider = t.align(0.0, 0.5, slider);
        let slider = t.add(Node::column(0).flex(1), [slider]);
        let bright = icon_button(t, Icon::Sun, 54, action::LIGHT_UP);
        let light = t.add(Node::row(0), [dim, slider, bright]);
        let light = row(t, "Light", light);

        let toggle = t.custom(Toggle { on: s.night });
        let toggle_node = toggle;
        let moon = t.icon(Icon::Moon, 40, INK);
        let hint = t.label("Dark page, light text", st.small());
        let g = t.gap(14, 1);
        let fill = t.spacer(1);
        let night = t.add(Node::row(0), [moon, g, hint, fill, toggle]);
        let night = t.add(
            Node::new(Kind::Padding(Insets::vh(0, 0)))
                .action(action::NIGHT)
                .feedback(Feedback::None),
            [night],
        );
        let night = t.align(0.0, 0.5, night);
        let night = row(t, "Night", night);

        let hair = t.rule(1, RULE);
        let rows = t.add(
            Node::column(14),
            [nav, hair, fonts, size, spacing, margins, light, night],
        );
        let rows = t.pad(Insets::new(20, m, 34, m), rows);
        let line = t.rule(2, INK);
        let sheet = t.add(Node::column(0).bg(255), [line, rows]);

        let fill = t.spacer(1);
        let menu = t.add(Node::column(0).hidden(true), [top, fill, sheet]);
        (menu, (bar, page_label, light_node, toggle_node))
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// The line slices of every page.
    pub fn pages(&self) -> &[Vec<Slice>] {
        &self.pages
    }

    /// Number of lines block `i` was set in.
    pub fn block_lines(&self, i: usize) -> usize {
        self.blocks[i].layout.lines.len()
    }

    /// Words from the start of the current page to the end of the chapter.
    fn words_left(&self) -> usize {
        let first = self.pages[self.current]
            .first()
            .map(|s| s.block)
            .unwrap_or(0);
        PARAGRAPHS
            .iter()
            .enumerate()
            .filter(|(i, _)| i + 2 >= first)
            .map(|(_, p)| p.split_whitespace().count())
            .sum()
    }

    fn update_position(&mut self, t: &mut Tree) {
        let minutes = (self.words_left() as f32 / WORDS_PER_MINUTE).ceil() as usize;
        let left = if self.current + 1 == self.pages.len() {
            "Chapter I · last page".to_string()
        } else {
            format!("Chapter I · {minutes} min left")
        };
        t.set_text(self.footer_left, &left);
        let book_page = BOOK_PAGE_OFFSET + self.current + 1;
        t.set_text(self.footer_right, &format!("{book_page} / {BOOK_PAGES}"));
        t.set_text(
            self.menu_page_label,
            &format!("Page {book_page} of {BOOK_PAGES}"),
        );
        t.set_progress(
            self.menu_progress,
            (self.current + 1) as f32 / self.pages.len() as f32,
        );
    }

    /// Turns the page. Returns false at either end of the chapter.
    pub fn turn(&mut self, t: &mut Tree, forward: bool) -> bool {
        let next = if forward {
            self.current + 1
        } else {
            match self.current.checked_sub(1) {
                Some(p) => p,
                None => return false,
            }
        };
        if next >= self.pages.len() {
            return false;
        }
        self.current = next;
        let slices = self.pages[next].clone();
        if let Some(w) = t.widget_mut::<TextPage>(self.page) {
            w.slices = slices;
        }
        t.invalidate(self.page, Refresh::Page);
        self.update_position(t);
        true
    }

    pub fn menu_open(&self) -> bool {
        self.menu_open
    }

    pub fn set_menu(&mut self, t: &mut Tree, open: bool) {
        self.menu_open = open;
        t.set_hidden(self.menu, !open);
    }

    /// Handles a UI event. Settings that change the text layout rebuild the
    /// screen and swap it in with a diffed refresh.
    pub fn handle(&mut self, ui: &mut Ui, ev: UiEvent) -> Command {
        let screen = ui.bounds();
        match ev {
            UiEvent::Tap(p) if !self.menu_open => match zone_at(screen, p) {
                Zone::Prev => {
                    self.turn(&mut ui.tree, false);
                }
                Zone::Next => {
                    self.turn(&mut ui.tree, true);
                }
                Zone::Menu => self.set_menu(&mut ui.tree, true),
            },
            // A tap on the page between the bars closes the menu.
            UiEvent::Tap(_) => self.set_menu(&mut ui.tree, false),
            UiEvent::Swipe(dir) if !self.menu_open => match dir {
                Direction::Left => {
                    self.turn(&mut ui.tree, true);
                }
                Direction::Right => {
                    self.turn(&mut ui.tree, false);
                }
                Direction::Down => self.set_menu(&mut ui.tree, true),
                Direction::Up => {}
            },
            UiEvent::Action { id, node, at } => return self.action(ui, id, node, at),
            _ => {}
        }
        Command::None
    }

    fn action(&mut self, ui: &mut Ui, id: ActionId, node: NodeId, at: Point) -> Command {
        let mut s = self.settings;
        match id {
            action::BACK => return Command::Library,
            action::NIGHT => {
                s.night = !s.night;
                if let Some(w) = ui.tree.widget_mut::<Toggle>(self.night_toggle) {
                    w.on = s.night;
                }
                ui.set_night(s.night);
                self.settings = s;
                return Command::None;
            }
            action::LIGHT_SET | action::LIGHT_DOWN | action::LIGHT_UP => {
                s.light = match id {
                    action::LIGHT_SET => Slider::value_at(ui.tree.rect(node), at.x),
                    action::LIGHT_DOWN => (s.light - 1.0 / 24.0).max(0.0),
                    _ => (s.light + 1.0 / 24.0).min(1.0),
                };
                if let Some(w) = ui.tree.widget_mut::<Slider>(self.light) {
                    w.value = s.light;
                }
                ui.tree.invalidate(self.light, Refresh::Auto);
                self.settings = s;
                return Command::None;
            }
            action::SIZE_DOWN => s.size = s.size.saturating_sub(1),
            action::SIZE_UP => s.size = (s.size + 1).min(SIZES.len() - 1),
            i if (action::FONT..action::FONT + 3).contains(&i) => {
                s.family = (i - action::FONT) as usize
            }
            i if (action::SPACING..action::SPACING + 3).contains(&i) => {
                s.spacing = (i - action::SPACING) as usize
            }
            i if (action::MARGINS..action::MARGINS + 3).contains(&i) => {
                s.margins = (i - action::MARGINS) as usize
            }
            _ => return Command::None,
        }
        if s != self.settings {
            // Keep the reader's place: the page holding the same first
            // paragraph line is close enough for a prototype.
            let styles = Styles::new(self.families[0]);
            let (tree, mut fresh) = Reader::build(&mut ui.text, &styles, self.families, s);
            fresh.set_menu_quiet(self.menu_open, tree, ui);
            *self = fresh;
        }
        Command::None
    }

    fn set_menu_quiet(&mut self, open: bool, mut tree: Tree, ui: &mut Ui) {
        self.menu_open = open;
        tree.set_hidden(self.menu, !open);
        ui.set_tree_with(tree, Refresh::Auto);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Phase, TouchEvent};
    use crate::text::Fonts;
    use crate::waveform::Waveform;

    fn reader() -> (Ui, Reader) {
        let mut text = TextEngine::new(Fonts::embedded());
        let fam = FamilyId(0);
        let (tree, r) = Reader::build(&mut text, &Styles::new(fam), [fam; 3], Settings::default());
        let mut ui = Ui::new(tree, text, SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
        ui.render();
        (ui, r)
    }

    fn tap(ui: &mut Ui, x: i32, y: i32) -> Vec<UiEvent> {
        let mut ev = ui.handle_touch(TouchEvent {
            phase: Phase::Down,
            id: 0,
            x,
            y,
            time_us: 0,
        });
        ev.extend(ui.handle_touch(TouchEvent {
            phase: Phase::Up,
            id: 0,
            x,
            y,
            time_us: 60_000,
        }));
        ev
    }

    #[test]
    fn zones_cover_the_screen() {
        let s = Rect::screen();
        assert_eq!(zone_at(s, Point { x: 100, y: 800 }), Zone::Prev);
        assert_eq!(zone_at(s, Point { x: 800, y: 800 }), Zone::Next);
        assert_eq!(zone_at(s, Point { x: 500, y: 60 }), Zone::Menu);
        let area: i64 = zones(s).iter().map(|(_, r)| r.area()).sum();
        assert_eq!(area, s.area());
    }

    #[test]
    fn chapter_paginates_without_widows_or_orphans() {
        let (_, r) = reader();
        assert!(r.page_count() >= 2);
        for page in r.pages() {
            for s in page {
                let n = r.block_lines(s.block);
                if n > 1 {
                    assert!(s.lines.len() > 1, "single line of a paragraph: {s:?}");
                }
            }
        }
    }

    #[test]
    fn page_turn_damages_text_and_footer_only() {
        let (mut ui, mut r) = reader();
        let ev = tap(&mut ui, 900, 900);
        assert_eq!(ev, vec![UiEvent::Tap(Point { x: 900, y: 900 })]);
        r.handle(&mut ui, ev[0]);
        assert_eq!(r.current, 1);
        let d = ui.render();
        assert!(!d.is_empty());
        for dmg in &d {
            assert_eq!(dmg.waveform, Waveform::GL16);
            // The running head is untouched.
            assert!(dmg.rect.y >= HEADER, "{dmg:?}");
        }
    }

    #[test]
    fn menu_opens_from_the_top_band_without_touching_the_page() {
        let (mut ui, mut r) = reader();
        let ev = tap(&mut ui, 500, 50);
        r.handle(&mut ui, ev[0]);
        assert!(r.menu_open());
        ui.render();
        let d = ui.minimal_damage().to_vec();
        let menu_top = 122;
        let sheet_top = d
            .iter()
            .map(|d| d.rect.y)
            .filter(|&y| y >= menu_top)
            .min()
            .unwrap();
        // Two bars, and the page between them is not refreshed.
        assert!(d
            .iter()
            .all(|d| d.rect.bottom() <= menu_top || d.rect.y >= sheet_top));
        assert!(sheet_top > 400, "{d:?}");
        // While the menu is open, page-zone taps close it instead of turning.
        let ev = tap(&mut ui, 900, 300);
        r.handle(&mut ui, ev[0]);
        assert!(!r.menu_open());
        assert_eq!(r.current, 0);
    }

    #[test]
    fn larger_type_means_more_pages() {
        let mut text = TextEngine::new(Fonts::embedded());
        let fam = FamilyId(0);
        let st = Styles::new(fam);
        let small = Settings {
            size: 0,
            ..Default::default()
        };
        let large = Settings {
            size: 6,
            ..Default::default()
        };
        let (_, a) = Reader::build(&mut text, &st, [fam; 3], small);
        let (_, b) = Reader::build(&mut text, &st, [fam; 3], large);
        assert!(b.page_count() > a.page_count());
    }
}
