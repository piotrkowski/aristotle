// SPDX-License-Identifier: AGPL-3.0-or-later
//! The retained widget tree: nodes, box layout, painting and hit testing.
//!
//! Retained rather than immediate mode because e-ink inverts the usual
//! trade-off. Redrawing is cheap but *showing* is expensive (120 to 550 ms
//! per update, one at a time), so the toolkit must know exactly which
//! pixels changed. A retained tree remembers every node's rectangle, so a
//! change to one node can be turned into a small damage rectangle instead
//! of a full-screen refresh.
//!
//! Layout is a single pass of box constraints, in the style of Flutter: a
//! parent hands each child a minimum and maximum size, the child picks its
//! size within them, and the parent positions it. Rows and columns share
//! leftover space between `flex` children. That is enough for the screens
//! of an e-reader, and keeps layout linear in the number of nodes.

use crate::canvas::Painter;
use crate::geom::{Density, Insets, Point, Rect, Size, MIN_TARGET};
use crate::icons::{self, Icon};
use crate::text::{layout_paragraph, Paragraph, ParagraphLayout, TextEngine, TextStyle};
use std::any::Any;

/// Stands in for "unbounded" in constraints.
pub const INF: i32 = i32::MAX / 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

/// Application-defined identifier delivered when a node is tapped.
pub type ActionId = u32;

/// Size limits handed from parent to child.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Constraints {
    pub min: Size,
    pub max: Size,
}

impl Constraints {
    pub fn loose(max: Size) -> Self {
        Self {
            min: Size::default(),
            max,
        }
    }

    pub fn tight(s: Size) -> Self {
        Self { min: s, max: s }
    }

    pub fn constrain(&self, s: Size) -> Size {
        Size::new(
            s.w.clamp(self.min.w, self.max.w.max(self.min.w)),
            s.h.clamp(self.min.h, self.max.h.max(self.min.h)),
        )
    }

    fn deflate(&self, i: Insets) -> Self {
        let d = |v: i32, by: i32| if v >= INF { INF } else { (v - by).max(0) };
        Self {
            min: Size::new(d(self.min.w, i.horizontal()), d(self.min.h, i.vertical())),
            max: Size::new(d(self.max.w, i.horizontal()), d(self.max.h, i.vertical())),
        }
    }
}

/// Placement of children across a row's or column's main axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cross {
    Start,
    Center,
    End,
    /// Children take the full cross size.
    Stretch,
}

/// What a node shows while a finger is on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feedback {
    /// Invert the node (black background, white content). The strongest
    /// signal, used for buttons.
    Invert,
    /// A thick black outline. Calmer for large targets such as list rows,
    /// where inverting would flash a big black block.
    Outline,
    /// Nothing; the result of the action is the feedback (page turns).
    None,
}

/// How a content change should reach the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refresh {
    /// Pick by content: DU if the new pixels are pure black and white,
    /// otherwise GL16.
    Auto,
    /// Transient feedback that is replaced shortly: A2.
    Transient,
    /// A page turn. Usually GL16, but every n-th one becomes a full-screen
    /// GC16 to clear accumulated ghosting.
    Page,
    /// A flashing GC16 refresh of the node.
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Border {
    pub width: i32,
    pub gray: u8,
    pub radius: i32,
}

/// Context handed to custom widgets when painting.
pub struct PaintCx<'a, 'b> {
    pub painter: &'a mut Painter<'b>,
    pub text: &'a mut TextEngine,
}

/// A leaf with its own measuring and painting, for things the generic
/// kinds do not cover (covers, sliders, a page of book text).
pub trait Widget: Any {
    fn measure(&mut self, c: Constraints, text: &mut TextEngine) -> Size;
    fn paint(&self, rect: Rect, cx: &mut PaintCx);
}

pub enum Kind {
    Column {
        gap: i32,
        cross: Cross,
    },
    Row {
        gap: i32,
        cross: Cross,
    },
    /// Children on top of each other, each filling the stack. Used for
    /// overlays.
    Stack,
    Padding(Insets),
    /// Positions its child by fractions of the free space (0.5, 0.5 is
    /// centred) and fills the available space itself.
    Align {
        x: f32,
        y: f32,
    },
    /// Forces a width and/or height.
    Fixed {
        w: Option<i32>,
        h: Option<i32>,
    },
    /// Raises the minimum size, e.g. to keep a touch target at 9 mm.
    MinSize(Size),
    Text {
        para: Paragraph,
        cache: Option<(i32, ParagraphLayout)>,
    },
    Icon {
        icon: Icon,
        size: i32,
        gray: u8,
        weight: f32,
    },
    /// A horizontal hairline across the available width.
    Rule {
        thickness: i32,
        gray: u8,
    },
    /// Reading progress: a thin track with a heavier filled part.
    Progress {
        value: f32,
        height: i32,
    },
    /// Empty space; give it `flex` to push siblings apart.
    Spacer,
    /// Shows one page of its children and hides the rest. E-ink lists page
    /// instead of scrolling: every scroll step would cost a full update and
    /// leave ghosting, while a page turn is one clean update.
    Pages {
        gap: i32,
        page: usize,
        count: usize,
    },
    Custom(Box<dyn Widget>),
}

pub struct Node {
    native_units: bool,
    pub kind: Kind,
    pub children: Vec<NodeId>,
    pub flex: u16,
    pub bg: Option<u8>,
    pub border: Option<Border>,
    pub action: Option<ActionId>,
    pub feedback: Feedback,
    pub hidden: bool,
    /// Drawn inverted (white on black) as a persistent selection state.
    pub inverted: bool,
    pub(crate) pressed: bool,
    /// Hidden because a `Pages` parent shows another page.
    off_page: bool,
    pos: Point,
    size: Size,
    pub(crate) rect: Rect,
}

impl Node {
    pub fn new(kind: Kind) -> Self {
        Self {
            native_units: false,
            kind,
            children: Vec::new(),
            flex: 0,
            bg: None,
            border: None,
            action: None,
            feedback: Feedback::Invert,
            hidden: false,
            inverted: false,
            pressed: false,
            off_page: false,
            pos: Point::default(),
            size: Size::default(),
            rect: Rect::default(),
        }
    }

    /// A node that only carries looks (background, border, action), for
    /// constructors such as [`Tree::button`] that supply the kind.
    pub fn plain() -> Self {
        Self::new(Kind::Spacer)
    }

    /// Keeps measured dimensions in native pixels instead of reference pixels.
    pub fn in_pixels(mut self) -> Self {
        self.native_units = true;
        self
    }

    fn at_density(&mut self, d: Density) {
        if let Some(b) = &mut self.border {
            b.width = d.px(b.width);
            b.radius = d.px(b.radius);
        }
        if let Kind::Text { para, cache } = &mut self.kind {
            for span in &mut para.spans {
                span.style = span.style.clone().at_density(d);
            }
            if !self.native_units {
                para.style.indent = d.scale(para.style.indent);
            }
            *cache = None;
        }
        if self.native_units {
            return;
        }
        match &mut self.kind {
            Kind::Column { gap, .. } | Kind::Row { gap, .. } | Kind::Pages { gap, .. } => {
                *gap = d.px(*gap)
            }
            Kind::Padding(i) => {
                *i = Insets::new(d.px(i.top), d.px(i.right), d.px(i.bottom), d.px(i.left))
            }
            Kind::Fixed { w, h } => {
                *w = w.map(|v| d.px(v));
                *h = h.map(|v| d.px(v));
            }
            Kind::MinSize(s) => *s = Size::new(d.px(s.w), d.px(s.h)),
            Kind::Icon { size, .. } => *size = d.px(*size),
            Kind::Rule { thickness, .. } => *thickness = d.px(*thickness),
            Kind::Progress { height, .. } => *height = d.px(*height),
            _ => {}
        }
    }

    pub fn column(gap: i32) -> Self {
        Self::new(Kind::Column {
            gap,
            cross: Cross::Stretch,
        })
    }

    pub fn row(gap: i32) -> Self {
        Self::new(Kind::Row {
            gap,
            cross: Cross::Center,
        })
    }

    pub fn cross(mut self, c: Cross) -> Self {
        match &mut self.kind {
            Kind::Column { cross, .. } | Kind::Row { cross, .. } => *cross = c,
            _ => {}
        }
        self
    }

    pub fn flex(mut self, f: u16) -> Self {
        self.flex = f;
        self
    }

    pub fn bg(mut self, gray: u8) -> Self {
        self.bg = Some(gray);
        self
    }

    pub fn border(mut self, width: i32, gray: u8, radius: i32) -> Self {
        self.border = Some(Border {
            width,
            gray,
            radius,
        });
        self
    }

    pub fn action(mut self, id: ActionId) -> Self {
        self.action = Some(id);
        self
    }

    pub fn feedback(mut self, f: Feedback) -> Self {
        self.feedback = f;
        self
    }

    pub fn hidden(mut self, h: bool) -> Self {
        self.hidden = h;
        self
    }

    pub fn inverted(mut self, on: bool) -> Self {
        self.inverted = on;
        self
    }

    /// Whether this node puts pixels on screen itself (as opposed to only
    /// arranging children). Only such nodes generate damage when they move.
    fn paints(&self) -> bool {
        // Deliberately independent of `pressed`: press feedback is damaged
        // through an explicit Transient change, and must not also show up
        // as a node "appearing" with Auto refresh.
        self.bg.is_some()
            || self.border.is_some()
            || self.inverted
            || matches!(
                self.kind,
                Kind::Text { .. }
                    | Kind::Icon { .. }
                    | Kind::Rule { .. }
                    | Kind::Progress { .. }
                    | Kind::Custom(_)
            )
    }

    fn visible(&self) -> bool {
        !self.hidden && !self.off_page
    }
}

/// Result of hit testing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// A node with an action.
    Target(NodeId),
    /// An opaque node with no action under the finger, e.g. an empty part
    /// of a menu sheet: the touch must not fall through to what is below.
    Blocked,
    /// Nothing; the screen may interpret the touch (reader tap zones).
    Nothing,
}

/// Pending change, consumed by [`crate::ui::Ui::render`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Change {
    pub node: NodeId,
    pub refresh: Refresh,
}

pub struct Tree {
    density: Density,
    nodes: Vec<Node>,
    root: Option<NodeId>,
    pub(crate) changes: Vec<Change>,
    pub(crate) needs_layout: bool,
    /// Background painted under everything.
    pub paper: u8,
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

impl Tree {
    pub fn new() -> Self {
        Self {
            density: Density::default(),
            nodes: Vec::new(),
            root: None,
            changes: Vec::new(),
            needs_layout: true,
            paper: 255,
        }
    }

    pub fn add(&mut self, node: Node, children: impl IntoIterator<Item = NodeId>) -> NodeId {
        let mut node = node;
        if self.density != Density::default() {
            node.at_density(self.density);
        }
        node.children = children.into_iter().collect();
        self.nodes.push(node);
        NodeId(self.nodes.len() as u32 - 1)
    }

    pub(crate) fn set_density(&mut self, density: Density) {
        if self.density == density {
            return;
        }
        assert_eq!(
            self.density,
            Density::default(),
            "build a new tree when changing density"
        );
        for node in &mut self.nodes {
            node.at_density(density);
        }
        self.density = density;
        self.needs_layout = true;
    }

    pub fn set_root(&mut self, id: NodeId) {
        self.root = Some(id);
        self.needs_layout = true;
    }

    pub fn root(&self) -> Option<NodeId> {
        self.root
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The node's rectangle on screen after the last layout.
    pub fn rect(&self, id: NodeId) -> Rect {
        self.node(id).rect
    }

    // --- Convenience constructors -------------------------------------

    pub fn text(&mut self, para: Paragraph) -> NodeId {
        self.add(Node::new(Kind::Text { para, cache: None }), [])
    }

    pub fn label(&mut self, text: &str, style: TextStyle) -> NodeId {
        self.text(Paragraph::plain(text, style, crate::text::Align::Left))
    }

    pub fn icon(&mut self, icon: Icon, size: i32, gray: u8) -> NodeId {
        self.add(
            Node::new(Kind::Icon {
                icon,
                size,
                gray,
                weight: 0.0,
            }),
            [],
        )
    }

    pub fn spacer(&mut self, flex: u16) -> NodeId {
        self.add(Node::new(Kind::Spacer).flex(flex), [])
    }

    pub fn gap(&mut self, w: i32, h: i32) -> NodeId {
        self.add(
            Node::new(Kind::Fixed {
                w: Some(w),
                h: Some(h),
            }),
            [],
        )
    }

    pub fn rule(&mut self, thickness: i32, gray: u8) -> NodeId {
        self.add(Node::new(Kind::Rule { thickness, gray }), [])
    }

    pub fn pad(&mut self, insets: Insets, child: NodeId) -> NodeId {
        self.add(Node::new(Kind::Padding(insets)), [child])
    }

    pub fn align(&mut self, x: f32, y: f32, child: NodeId) -> NodeId {
        self.add(Node::new(Kind::Align { x, y }), [child])
    }

    pub fn fixed(&mut self, w: Option<i32>, h: Option<i32>, child: Option<NodeId>) -> NodeId {
        self.add(Node::new(Kind::Fixed { w, h }), child)
    }

    pub fn fixed_pixels(
        &mut self,
        w: Option<i32>,
        h: Option<i32>,
        child: Option<NodeId>,
    ) -> NodeId {
        self.add(Node::new(Kind::Fixed { w, h }).in_pixels(), child)
    }

    pub fn gap_pixels(&mut self, w: i32, h: i32) -> NodeId {
        self.fixed_pixels(Some(w), Some(h), None)
    }

    pub fn custom(&mut self, w: impl Widget) -> NodeId {
        self.add(Node::new(Kind::Custom(Box::new(w))), [])
    }

    /// A button: content centred in a box at least 9 mm tall and wide,
    /// inverted while pressed. `node` carries the look (border, background,
    /// flex); its kind is replaced.
    pub fn button(&mut self, node: Node, content: NodeId, action: ActionId) -> NodeId {
        self.add(
            Node {
                kind: Kind::MinSize(Size::new(MIN_TARGET, MIN_TARGET)),
                ..node
            }
            .action(action),
            [content],
        )
    }

    // --- Mutation, each recording what needs repainting ----------------

    /// Records that `id` changed and how it should be refreshed. Call after
    /// mutating a custom widget through [`Tree::widget_mut`].
    pub fn invalidate(&mut self, id: NodeId, refresh: Refresh) {
        self.changes.push(Change { node: id, refresh });
        self.needs_layout = true;
    }

    pub fn set_text(&mut self, id: NodeId, text: &str) {
        if let Kind::Text { para, cache } = &mut self.node_mut(id).kind {
            if para.text() == text && para.spans.len() == 1 {
                return;
            }
            para.spans.truncate(1);
            para.spans[0].text = text.to_string();
            *cache = None;
        }
        self.invalidate(id, Refresh::Auto);
    }

    pub fn set_progress(&mut self, id: NodeId, v: f32) {
        if let Kind::Progress { value, .. } = &mut self.node_mut(id).kind {
            *value = v.clamp(0.0, 1.0);
        }
        self.invalidate(id, Refresh::Auto);
    }

    /// Shows or hides a subtree. Damage comes from the rectangles of the
    /// painting nodes inside it, not from its own (possibly full-screen)
    /// box, so showing a menu with a transparent middle repaints only its
    /// bars.
    pub fn set_hidden(&mut self, id: NodeId, hidden: bool) {
        if self.node(id).hidden != hidden {
            self.node_mut(id).hidden = hidden;
            self.needs_layout = true;
        }
    }

    pub fn set_icon_gray(&mut self, id: NodeId, g: u8) {
        if let Kind::Icon { gray, .. } = &mut self.node_mut(id).kind {
            if *gray != g {
                *gray = g;
                self.invalidate(id, Refresh::Auto);
            }
        }
    }

    pub fn set_inverted(&mut self, id: NodeId, on: bool) {
        if self.node(id).inverted != on {
            self.node_mut(id).inverted = on;
            self.invalidate(id, Refresh::Auto);
        }
    }

    /// Turns a `Pages` node to `page` (clamped). Returns whether it moved.
    pub fn set_page(&mut self, id: NodeId, page: usize) -> bool {
        if let Kind::Pages { page: p, count, .. } = &mut self.node_mut(id).kind {
            let new = page.min(count.saturating_sub(1));
            if new != *p {
                *p = new;
                self.needs_layout = true;
                return true;
            }
        }
        false
    }

    pub fn page(&self, id: NodeId) -> (usize, usize) {
        match self.node(id).kind {
            Kind::Pages { page, count, .. } => (page, count),
            _ => (0, 1),
        }
    }

    pub fn widget_mut<T: Widget>(&mut self, id: NodeId) -> Option<&mut T> {
        match &mut self.node_mut(id).kind {
            Kind::Custom(w) => (w.as_mut() as &mut dyn Any).downcast_mut::<T>(),
            _ => None,
        }
    }

    pub fn widget<T: Widget>(&self, id: NodeId) -> Option<&T> {
        match &self.node(id).kind {
            Kind::Custom(w) => (w.as_ref() as &dyn Any).downcast_ref::<T>(),
            _ => None,
        }
    }

    pub(crate) fn set_pressed(&mut self, id: NodeId, on: bool) {
        self.node_mut(id).pressed = on;
    }

    // --- Layout ---------------------------------------------------------

    /// Lays out the whole tree into `screen`.
    pub fn layout(&mut self, screen: Rect, text: &mut TextEngine) {
        if let Some(root) = self.root {
            self.layout_node(root, Constraints::tight(screen.size()), text);
            self.place(
                root,
                Point {
                    x: screen.x,
                    y: screen.y,
                },
                true,
            );
        }
        self.needs_layout = false;
    }

    fn place(&mut self, id: NodeId, origin: Point, visible: bool) {
        let n = self.node_mut(id);
        let visible = visible && n.visible();
        let at = Point {
            x: origin.x + n.pos.x,
            y: origin.y + n.pos.y,
        };
        n.rect = if visible {
            Rect::new(at.x, at.y, n.size.w, n.size.h)
        } else {
            Rect::default()
        };
        for i in 0..self.node(id).children.len() {
            let c = self.node(id).children[i];
            self.place(c, at, visible);
        }
    }

    fn layout_node(&mut self, id: NodeId, c: Constraints, text: &mut TextEngine) -> Size {
        if self.node(id).hidden {
            self.node_mut(id).size = Size::default();
            return Size::default();
        }
        let children = self.node(id).children.clone();
        let size = match &mut self.node_mut(id).kind {
            Kind::Column { gap, cross } => {
                let (gap, cross) = (*gap, *cross);
                self.layout_flex(&children, c, gap, cross, true, text)
            }
            Kind::Row { gap, cross } => {
                let (gap, cross) = (*gap, *cross);
                self.layout_flex(&children, c, gap, cross, false, text)
            }
            Kind::Stack => {
                let s = c.constrain(c.max);
                for &ch in &children {
                    self.layout_node(ch, Constraints::tight(s), text);
                    self.node_mut(ch).pos = Point::default();
                }
                s
            }
            Kind::Padding(i) => {
                let i = *i;
                let mut s = Size::new(i.horizontal(), i.vertical());
                if let Some(&ch) = children.first() {
                    let cs = self.layout_node(ch, c.deflate(i), text);
                    self.node_mut(ch).pos = Point {
                        x: i.left,
                        y: i.top,
                    };
                    s.w += cs.w;
                    s.h += cs.h;
                }
                c.constrain(s)
            }
            Kind::Align { x, y } => {
                let (fx, fy) = (*x, *y);
                let mut s = c.constrain(c.max);
                if let Some(&ch) = children.first() {
                    let cs = self.layout_node(ch, Constraints::loose(c.max), text);
                    if c.max.w >= INF {
                        s.w = c.constrain(cs).w;
                    }
                    if c.max.h >= INF {
                        s.h = c.constrain(cs).h;
                    }
                    self.node_mut(ch).pos = Point {
                        x: ((s.w - cs.w) as f32 * fx).round() as i32,
                        y: ((s.h - cs.h) as f32 * fy).round() as i32,
                    };
                }
                s
            }
            Kind::Fixed { w, h } => {
                let mut cc = c;
                if let Some(w) = *w {
                    let w = w.clamp(c.min.w, c.max.w);
                    cc.min.w = w;
                    cc.max.w = w;
                }
                if let Some(h) = *h {
                    let h = h.clamp(c.min.h, c.max.h);
                    cc.min.h = h;
                    cc.max.h = h;
                }
                match children.first() {
                    Some(&ch) => {
                        self.node_mut(ch).pos = Point::default();
                        cc.constrain(self.layout_node(ch, cc, text))
                    }
                    // A childless box fills the free axis when it can, so
                    // `Fixed { h: Some(2), .. }` is a full-width bar.
                    None => Size::new(
                        if cc.max.w < INF { cc.max.w } else { cc.min.w },
                        if cc.max.h < INF { cc.max.h } else { cc.min.h },
                    ),
                }
            }
            Kind::MinSize(m) => {
                // Shrink-wrap the child, grow to the minimum, centre it.
                let m = *m;
                let cs = match children.first() {
                    Some(&ch) => self.layout_node(ch, Constraints::loose(c.max), text),
                    None => Size::default(),
                };
                let s = c.constrain(Size::new(cs.w.max(m.w), cs.h.max(m.h)));
                if let Some(&ch) = children.first() {
                    self.node_mut(ch).pos = Point {
                        x: (s.w - cs.w) / 2,
                        y: (s.h - cs.h) / 2,
                    };
                }
                s
            }
            Kind::Text { para, cache } => {
                let width = if c.max.w >= INF { 100_000 } else { c.max.w };
                if cache.as_ref().map(|(w, _)| *w) != Some(width) {
                    let layout = layout_paragraph(
                        text,
                        para,
                        width as f32,
                        crate::text::ui_hyphenator(aristotle_i18n::language()),
                    );
                    *cache = Some((width, layout));
                }
                let l = &cache.as_ref().unwrap().1;
                let natural = Size::new(l.width.ceil() as i32, l.height.ceil() as i32);
                let w = match para.style.align {
                    crate::text::Align::Left => natural.w,
                    _ if c.max.w < INF => c.max.w,
                    _ => natural.w,
                };
                c.constrain(Size::new(w, natural.h))
            }
            Kind::Icon { size, .. } => c.constrain(Size::new(*size, *size)),
            Kind::Rule { thickness, .. } => c.constrain(Size::new(c.max.w, *thickness)),
            Kind::Progress { height, .. } => c.constrain(Size::new(c.max.w, *height)),
            Kind::Spacer => c.min,
            Kind::Pages { gap, page, .. } => {
                let (gap, want) = (*gap, *page);
                let s = c.constrain(c.max);
                let item_c = Constraints {
                    min: Size::new(s.w, 0),
                    max: Size::new(s.w, INF),
                };
                // Paginate: fill each page until the next item would not fit.
                let mut pages: Vec<usize> = vec![0];
                let mut y = 0;
                let mut heights = Vec::with_capacity(children.len());
                for (i, &ch) in children.iter().enumerate() {
                    let h = self.layout_node(ch, item_c, text).h;
                    heights.push(h);
                    if y > 0 && y + h > s.h {
                        pages.push(i);
                        y = 0;
                    }
                    y += h + gap;
                }
                let n_pages = pages.len();
                let current = want.min(n_pages - 1);
                if let Kind::Pages { page, count, .. } = &mut self.node_mut(id).kind {
                    *page = current;
                    *count = n_pages;
                }
                let start = pages[current];
                let end = pages.get(current + 1).copied().unwrap_or(children.len());
                let mut y = 0;
                for (i, &ch) in children.iter().enumerate() {
                    let on = (start..end).contains(&i);
                    let n = self.node_mut(ch);
                    n.off_page = !on;
                    if on {
                        n.pos = Point { x: 0, y };
                        y += heights[i] + gap;
                    }
                }
                s
            }
            Kind::Custom(w) => w.measure(c, text),
        };
        self.node_mut(id).size = size;
        size
    }

    fn layout_flex(
        &mut self,
        children: &[NodeId],
        c: Constraints,
        gap: i32,
        cross: Cross,
        vertical: bool,
        text: &mut TextEngine,
    ) -> Size {
        // Work in (main, cross) coordinates.
        let (max_main, max_cross) = if vertical {
            (c.max.h, c.max.w)
        } else {
            (c.max.w, c.max.h)
        };
        let min_cross = if vertical { c.min.w } else { c.min.h };
        let child_cross_min = if cross == Cross::Stretch && max_cross < INF {
            max_cross
        } else {
            0
        };
        let mk = |main_min: i32, main_max: i32| {
            if vertical {
                Constraints {
                    min: Size::new(child_cross_min, main_min),
                    max: Size::new(max_cross, main_max),
                }
            } else {
                Constraints {
                    min: Size::new(main_min, child_cross_min),
                    max: Size::new(main_max, max_cross),
                }
            }
        };
        let visible: Vec<NodeId> = children
            .iter()
            .copied()
            .filter(|&ch| !self.node(ch).hidden)
            .collect();
        let gaps = gap * (visible.len() as i32 - 1).max(0);
        let mut used = gaps;
        let mut sizes = vec![Size::default(); visible.len()];
        let mut total_flex = 0u32;
        for (i, &ch) in visible.iter().enumerate() {
            let f = self.node(ch).flex;
            if f > 0 {
                total_flex += f as u32;
                continue;
            }
            let remaining = if max_main >= INF {
                INF
            } else {
                (max_main - used).max(0)
            };
            let s = self.layout_node(ch, mk(0, remaining), text);
            used += if vertical { s.h } else { s.w };
            sizes[i] = s;
        }
        if total_flex > 0 {
            let free = if max_main >= INF {
                0
            } else {
                (max_main - used).max(0)
            };
            let mut given = 0;
            let mut seen = 0u32;
            for (i, &ch) in visible.iter().enumerate() {
                let f = self.node(ch).flex as u32;
                if f == 0 {
                    continue;
                }
                seen += f;
                // Cumulative rounding so the shares add up exactly.
                let share = (free as i64 * seen as i64 / total_flex as i64) as i32 - given;
                given += share;
                sizes[i] = self.layout_node(ch, mk(share, share), text);
                used += share;
            }
        }
        let content_cross = sizes
            .iter()
            .map(|s| if vertical { s.w } else { s.h })
            .max()
            .unwrap_or(0);
        let main = if total_flex > 0 && max_main < INF {
            max_main
        } else {
            used
        };
        let cross_size = if cross == Cross::Stretch && max_cross < INF {
            max_cross
        } else {
            content_cross.max(min_cross)
        };
        let size = if vertical {
            c.constrain(Size::new(cross_size, main))
        } else {
            c.constrain(Size::new(main, cross_size))
        };
        let cross_avail = if vertical { size.w } else { size.h };
        let mut at = 0;
        for (i, &ch) in visible.iter().enumerate() {
            let (m, x) = if vertical {
                (sizes[i].h, sizes[i].w)
            } else {
                (sizes[i].w, sizes[i].h)
            };
            let off = match cross {
                Cross::Start | Cross::Stretch => 0,
                Cross::Center => (cross_avail - x) / 2,
                Cross::End => cross_avail - x,
            };
            self.node_mut(ch).pos = if vertical {
                Point { x: off, y: at }
            } else {
                Point { x: at, y: off }
            };
            at += m + gap;
        }
        size
    }

    // --- Painting ---------------------------------------------------------

    /// Paints every visible node that intersects the painter's clip.
    pub fn paint(&self, p: &mut Painter, text: &mut TextEngine) {
        p.fill_rect(p.clip, self.paper);
        if let Some(root) = self.root {
            self.paint_node(root, p, text);
        }
    }

    fn paint_node(&self, id: NodeId, p: &mut Painter, text: &mut TextEngine) {
        let n = self.node(id);
        if !n.visible() || n.rect.is_empty() || !n.rect.intersects(&p.clip) {
            return;
        }
        let r = n.rect;
        let radius = n.border.map_or(0.0, |b| b.radius as f32);
        if let Some(bg) = n.bg {
            p.fill_round_rect(r, radius, bg);
        }
        match &n.kind {
            Kind::Text {
                cache: Some((_, l)),
                ..
            } => l.draw(text, p, r.x as f32, r.y as f32),
            Kind::Icon {
                icon, gray, weight, ..
            } => icons::draw(p, *icon, r, *gray, *weight),
            Kind::Rule { gray, .. } => p.fill_rect(r, *gray),
            Kind::Progress { value, .. } => paint_progress(p, r, *value),
            Kind::Custom(w) => w.paint(r, &mut PaintCx { painter: p, text }),
            _ => {}
        }
        for &ch in &n.children {
            self.paint_node(ch, p, text);
        }
        // A zero-width border only lends its radius to the background and
        // the pressed or inverted state.
        if let Some(b) = n.border.filter(|b| b.width > 0) {
            p.stroke_round_rect(r, radius, b.width as f32, b.gray);
        }
        // Pressed toggles the inversion, so a selected (inverted) item
        // still visibly reacts to touch.
        let invert = n.inverted ^ (n.pressed && n.feedback == Feedback::Invert);
        if invert {
            p.invert_round_rect(r, radius);
        }
        if n.pressed && n.feedback == Feedback::Outline {
            p.stroke_round_rect(
                r,
                radius.max(self.density.scale(8.0)),
                self.density.scale(5.0),
                0,
            );
        }
    }

    // --- Hit testing ------------------------------------------------------

    /// Finds what is under `pt`. Targets smaller than 9 mm are hit-tested
    /// as if they were 9 mm, centred on themselves.
    pub fn hit(&self, pt: Point) -> Hit {
        match self.root {
            Some(root) => self.hit_node(root, pt),
            None => Hit::Nothing,
        }
    }

    fn hit_node(&self, id: NodeId, pt: Point) -> Hit {
        let n = self.node(id);
        if !n.visible() || n.rect.is_empty() {
            return Hit::Nothing;
        }
        let area = if n.action.is_some() {
            n.rect.expand_to(self.density.mm(9.0))
        } else {
            n.rect
        };
        // Children may extend past a parent only through target expansion,
        // so test them first and topmost (last painted) first.
        for &ch in n.children.iter().rev() {
            match self.hit_node(ch, pt) {
                Hit::Nothing => {}
                h => return h,
            }
        }
        if !area.contains(pt) {
            return Hit::Nothing;
        }
        if n.action.is_some() {
            Hit::Target(id)
        } else if n.bg.is_some() {
            Hit::Blocked
        } else {
            Hit::Nothing
        }
    }

    /// Rectangles of the painting nodes in a subtree, as they are now.
    /// Used to compute damage when nodes move, appear or disappear.
    pub(crate) fn snapshot(&self) -> Vec<Rect> {
        self.nodes
            .iter()
            .map(|n| if n.paints() { n.rect } else { Rect::default() })
            .collect()
    }

    pub(crate) fn rects_of(&self, id: NodeId, out: &mut Vec<Rect>) {
        let n = self.node(id);
        // A target's whole box is its unit of change: press feedback
        // covers all of it, not just the label inside.
        if n.paints() || n.action.is_some() {
            out.push(n.rect);
        } else {
            for &c in &n.children {
                self.rects_of(c, out);
            }
        }
    }

    pub(crate) fn action(&self, id: NodeId) -> Option<ActionId> {
        self.node(id).action
    }

    pub(crate) fn feedback(&self, id: NodeId) -> Feedback {
        self.node(id).feedback
    }
}

/// A `Pages` node; see [`Kind::Pages`].
pub fn pages(gap: i32) -> Node {
    Node::new(Kind::Pages {
        gap,
        page: 0,
        count: 1,
    })
}

pub fn progress(value: f32, height: i32) -> Node {
    Node::new(Kind::Progress {
        value: value.clamp(0.0, 1.0),
        height,
    })
}

/// A 2 px track in mid-gray with a 6 px black fill, vertically centred.
/// The thin track keeps the bar quiet. The fill is a plain black rectangle
/// with no anti-aliased ends, so a progress step is pure black and white
/// and goes out with the faster DU waveform.
fn paint_progress(p: &mut Painter, r: Rect, value: f32) {
    let track = 2;
    let fill = r.h.min(6);
    let cy = r.y + r.h / 2;
    p.fill_rect(Rect::new(r.x, cy - track / 2, r.w, track), 0x99);
    let w = (r.w as f32 * value).round() as i32;
    if w > 0 {
        p.fill_rect(Rect::new(r.x, cy - fill / 2, w, fill), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Fonts, TextEngine};

    fn engine() -> TextEngine {
        TextEngine::new(Fonts::embedded())
    }

    fn fixed(t: &mut Tree, w: i32, h: i32) -> NodeId {
        t.add(
            Node::new(Kind::Fixed {
                w: Some(w),
                h: Some(h),
            })
            .bg(0),
            [],
        )
    }

    #[test]
    fn column_with_flex_and_padding() {
        let mut t = Tree::new();
        let a = fixed(&mut t, 100, 50);
        let s = t.spacer(1);
        let b = fixed(&mut t, 100, 30);
        let col = t.add(Node::column(10), [a, s, b]);
        let root = t.pad(Insets::all(20), col);
        t.set_root(root);
        t.layout(Rect::new(0, 0, 400, 600), &mut engine());
        assert_eq!(t.rect(root), Rect::new(0, 0, 400, 600));
        // Stretch makes children as wide as the column.
        assert_eq!(t.rect(a), Rect::new(20, 20, 360, 50));
        // The spacer takes everything left: 560 - 50 - 30 - 2 * 10.
        assert_eq!(t.rect(s).h, 460);
        assert_eq!(t.rect(b), Rect::new(20, 550, 360, 30));
    }

    #[test]
    fn row_centres_cross_axis_and_shares_flex() {
        let mut t = Tree::new();
        let a = fixed(&mut t, 50, 20);
        let f1 = t.spacer(1);
        let f2 = t.spacer(3);
        let b = fixed(&mut t, 50, 60);
        let row = t.add(Node::row(0), [a, f1, b, f2]);
        t.set_root(row);
        t.layout(Rect::new(0, 0, 500, 100), &mut engine());
        assert_eq!(t.rect(a), Rect::new(0, 40, 50, 20));
        assert_eq!(t.rect(f1).w, 100);
        assert_eq!(t.rect(f2).w, 300);
        assert_eq!(t.rect(b), Rect::new(150, 20, 50, 60));
    }

    #[test]
    fn align_and_min_target() {
        let mut t = Tree::new();
        let icon = t.icon(Icon::Search, 48, 0);
        let btn = t.button(Node::new(Kind::Spacer), icon, 7);
        let root = t.align(1.0, 0.0, btn);
        t.set_root(root);
        t.layout(Rect::new(0, 0, 300, 300), &mut engine());
        // The button grows to the 9 mm minimum and sits top right.
        assert_eq!(t.rect(btn), Rect::new(300 - 106, 0, 106, 106));
        // The icon is centred inside it.
        assert_eq!(t.rect(icon), Rect::new(194 + 29, 29, 48, 48));
        assert_eq!(t.hit(Point { x: 250, y: 50 }), Hit::Target(btn));
        assert_eq!(t.hit(Point { x: 50, y: 50 }), Hit::Nothing);
    }

    #[test]
    fn small_targets_are_hit_tested_at_9mm() {
        let mut t = Tree::new();
        let dot = t.add(
            Node::new(Kind::Fixed {
                w: Some(20),
                h: Some(20),
            })
            .action(1),
            [],
        );
        let root = t.align(0.5, 0.5, dot);
        t.set_root(root);
        t.layout(Rect::new(0, 0, 400, 400), &mut engine());
        assert_eq!(t.rect(dot), Rect::new(190, 190, 20, 20));
        assert_eq!(t.hit(Point { x: 150, y: 150 }), Hit::Target(dot));
        assert_eq!(t.hit(Point { x: 140, y: 140 }), Hit::Nothing);
    }

    #[test]
    fn pages_paginate_and_hide_other_pages() {
        let mut t = Tree::new();
        let items: Vec<NodeId> = (0..7).map(|_| fixed(&mut t, 10, 100)).collect();
        let list = t.add(pages(10), items.clone());
        t.set_root(list);
        let mut e = engine();
        t.layout(Rect::new(0, 0, 200, 330), &mut e);
        // 100 + 10 + 100 + 10 + 100 = 320 fits; a fourth would not.
        assert_eq!(t.page(list), (0, 3));
        assert_eq!(t.rect(items[2]), Rect::new(0, 220, 200, 100));
        assert!(t.rect(items[3]).is_empty());
        assert!(t.set_page(list, 2));
        t.layout(Rect::new(0, 0, 200, 330), &mut e);
        assert_eq!(t.rect(items[6]), Rect::new(0, 0, 200, 100));
        assert!(t.rect(items[0]).is_empty());
        assert!(!t.set_page(list, 9), "clamped to the last page");
    }

    #[test]
    fn overlay_blocks_touches_below() {
        let mut t = Tree::new();
        let under = t.add(Node::new(Kind::Fixed { w: None, h: None }).action(1), []);
        let sheet = t.add(
            Node::new(Kind::Fixed {
                w: None,
                h: Some(100),
            })
            .bg(255),
            [],
        );
        let top = t.align(0.0, 1.0, sheet);
        let root = t.add(Node::new(Kind::Stack), [under, top]);
        t.set_root(root);
        t.layout(Rect::new(0, 0, 300, 300), &mut engine());
        assert_eq!(t.hit(Point { x: 10, y: 10 }), Hit::Target(under));
        assert_eq!(t.hit(Point { x: 10, y: 250 }), Hit::Blocked);
    }

    #[test]
    fn left_text_shrinks_and_justified_text_fills() {
        let mut e = engine();
        let style = TextStyle::new(crate::text::FamilyId(0), 40.0);
        let mut t = Tree::new();
        let a = t.label("Library", style.clone());
        let b = t.text(Paragraph::plain(
            "Library",
            style,
            crate::text::Align::Justify,
        ));
        let col = t.add(Node::column(0).cross(Cross::Start), [a, b]);
        t.set_root(col);
        t.layout(Rect::new(0, 0, 800, 400), &mut e);
        assert!(t.rect(a).w > 100 && t.rect(a).w < 200, "{:?}", t.rect(a));
        assert_eq!(t.rect(b).w, 800);
        assert_eq!(t.rect(a).h, (40.0f32 * 1.35).round() as i32);
    }
}
