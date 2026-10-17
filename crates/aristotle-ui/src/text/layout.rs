//! Paragraph layout: line breaking, hyphenation and justification.
//!
//! Break opportunities come from the Unicode line breaking algorithm
//! (UAX #14, `unicode-linebreak`), so punctuation, dashes, quotes and
//! non-breaking spaces behave as Unicode specifies. Each opportunity becomes
//! an item in Knuth and Plass's box-glue-penalty model:
//!
//! - a **box** is a shaped piece of a word (a whole word, or a part between
//!   two hyphenation points);
//! - **glue** is an inter-word space that may stretch or shrink;
//! - a **penalty** is a possible break that is not a space: a hyphenation
//!   point (costing a hyphen's width and a penalty) or a forced break.
//!
//! Justified paragraphs are broken with the Knuth–Plass total-fit
//! algorithm: it chooses all breaks of a paragraph together to minimise
//! uneven spacing, instead of filling each line greedily as most e-readers
//! do. The visible result is fewer rivers, fewer hyphens and no single very
//! loose line. Ragged (left, centred, right) text uses first-fit, which is
//! what UI labels need.
//!
//! Polish typesetting convention is supported: a one-letter word (a, i, o,
//! u, w, z) never ends a line, because a lone conjunction or preposition at
//! the right edge reads as an error to Polish readers.

use super::engine::{ShapedRun, TextEngine, TextStyle};
use super::hyphen::Hyphenator;
use crate::canvas::Painter;
use std::ops::Range;
use unicode_linebreak::{linebreaks, BreakOpportunity};

/// A run of text in one style.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: TextStyle,
}

impl Span {
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    /// Both edges flush; the last line is set flush left.
    Justify,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParagraphStyle {
    pub align: Align,
    /// First-line indent in pixels.
    pub indent: f32,
    /// Lines beyond this are dropped and the last kept line ends in "…".
    pub max_lines: Option<usize>,
    /// Ask the engine's hyphenator for break points. Soft hyphens in the
    /// text apply regardless.
    pub hyphenate: bool,
    /// Keep one-letter words off the end of a line (Polish convention).
    pub no_orphan_letters: bool,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        Self {
            align: Align::Left,
            indent: 0.0,
            max_lines: None,
            hyphenate: false,
            no_orphan_letters: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    pub spans: Vec<Span>,
    pub style: ParagraphStyle,
}

impl Paragraph {
    pub fn new(spans: Vec<Span>, style: ParagraphStyle) -> Self {
        Self { spans, style }
    }

    pub fn plain(text: impl Into<String>, style: TextStyle, align: Align) -> Self {
        Self {
            spans: vec![Span::new(text, style)],
            style: ParagraphStyle {
                align,
                ..Default::default()
            },
        }
    }

    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }
}

/// A placed piece of a shaped run.
#[derive(Clone, Debug)]
pub struct Piece {
    pub run: usize,
    pub clusters: Range<usize>,
    pub x: f32,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub pieces: Vec<Piece>,
    /// Width of the set line, from the left edge of the first piece to the
    /// right edge of the last.
    pub width: f32,
    /// Baseline, relative to the top of the paragraph.
    pub baseline: f32,
    /// True if the line ends with an inserted hyphen.
    pub hyphenated: bool,
}

/// A paragraph broken into lines at one width.
#[derive(Clone, Debug)]
pub struct ParagraphLayout {
    pub runs: Vec<ShapedRun>,
    pub lines: Vec<Line>,
    /// Width of the longest line.
    pub width: f32,
    /// Distance between baselines.
    pub pitch: f32,
    /// Total height: lines times pitch.
    pub height: f32,
}

impl ParagraphLayout {
    /// Draws `lines`, with the top of the first of them at `(x, y)`.
    pub fn draw_lines(
        &self,
        engine: &mut TextEngine,
        p: &mut Painter,
        x: f32,
        y: f32,
        lines: Range<usize>,
    ) {
        let Some(first) = self.lines.get(lines.start) else {
            return;
        };
        let top = first.baseline - self.ascent_offset(lines.start);
        for line in &self.lines[lines] {
            let baseline = y + line.baseline - top;
            for piece in &line.pieces {
                engine.draw(
                    p,
                    &self.runs[piece.run],
                    piece.clusters.clone(),
                    x + piece.x,
                    baseline,
                );
            }
        }
    }

    pub fn draw(&self, engine: &mut TextEngine, p: &mut Painter, x: f32, y: f32) {
        self.draw_lines(engine, p, x, y, 0..self.lines.len());
    }

    /// Distance from the top of line `i` to its baseline.
    fn ascent_offset(&self, i: usize) -> f32 {
        self.lines[i].baseline - i as f32 * self.pitch
    }

    /// The text of line `i` as laid out, for tests and debugging.
    pub fn line_text(&self, i: usize) -> String {
        let mut s = String::new();
        let mut last_end: Option<f32> = None;
        for piece in &self.lines[i].pieces {
            let run = &self.runs[piece.run];
            if let Some(end) = last_end {
                if piece.x - end > 0.5 {
                    s.push(' ');
                }
            }
            if run.clusters.is_empty() {
                continue;
            }
            let src = run_source(run, piece.clusters.clone());
            s.push_str(&src);
            last_end = Some(piece.x + run.width_of(piece.clusters.clone()));
        }
        s
    }
}

fn run_source(run: &ShapedRun, clusters: Range<usize>) -> String {
    match &run.text {
        Some(t) => {
            let a = run.clusters[clusters.start].source.start as usize;
            let b = run.clusters[clusters.end - 1].source.end as usize;
            t[a..b].to_string()
        }
        None => String::new(),
    }
}

const INFINITE: f32 = 10_000.0;
const HYPHEN_PENALTY: f32 = 100.0;
const DOUBLE_HYPHEN_DEMERITS: f32 = 10_000.0;
const FITNESS_DEMERITS: f32 = 5_000.0;
const LINE_PENALTY: f32 = 10.0;

#[derive(Clone, Debug)]
enum Item {
    /// `run == usize::MAX` marks an empty box (the first-line indent).
    Box {
        w: f32,
        run: usize,
        clusters: Range<usize>,
    },
    Glue {
        w: f32,
        stretch: f32,
        shrink: f32,
    },
    Penalty {
        w: f32,
        cost: f32,
        flagged: bool,
        /// Run holding the hyphen to draw if the line breaks here.
        hyphen: Option<usize>,
    },
}

impl Item {
    fn is_box(&self) -> bool {
        matches!(self, Item::Box { .. })
    }

    fn forced(&self) -> bool {
        matches!(self, Item::Penalty { cost, .. } if *cost <= -INFINITE)
    }
}

/// Characters that end a segment's body; non-breaking spaces stay inside
/// words on purpose.
fn is_break_space(c: char) -> bool {
    c.is_whitespace() && !matches!(c, '\u{a0}' | '\u{202f}' | '\u{2007}')
}

struct Builder<'a> {
    engine: &'a mut TextEngine,
    styles: Vec<TextStyle>,
    runs: Vec<ShapedRun>,
    space: Vec<Option<f32>>,
    hyphen: Vec<Option<usize>>,
    items: Vec<Item>,
}

impl Builder<'_> {
    fn shape(&mut self, text: &str, style: usize) -> usize {
        let mut run = self.engine.shape(text, &self.styles[style]);
        run.text = Some(text.to_string());
        run.style = style;
        self.runs.push(run);
        self.runs.len() - 1
    }

    fn space_width(&mut self, style: usize) -> f32 {
        if let Some(w) = self.space[style] {
            return w;
        }
        let w = self.engine.measure(" ", &self.styles[style]);
        self.space[style] = Some(w);
        w
    }

    fn hyphen_run(&mut self, style: usize) -> usize {
        if let Some(r) = self.hyphen[style] {
            return r;
        }
        let r = self.shape("-", style);
        self.hyphen[style] = Some(r);
        r
    }

    fn push_box(&mut self, run: usize, clusters: Range<usize>) {
        let w = self.runs[run].width_of(clusters.clone());
        self.items.push(Item::Box { w, run, clusters });
    }
}

/// Lays out a paragraph at `width` pixels.
pub fn layout_paragraph(
    engine: &mut TextEngine,
    para: &Paragraph,
    width: f32,
    hyphenator: &dyn Hyphenator,
) -> ParagraphLayout {
    // 1. Flatten the spans, dropping soft hyphens but remembering where
    //    they were: they are hyphenation points, not characters to shape.
    let mut text = String::new();
    let mut style_runs: Vec<(Range<usize>, usize)> = Vec::new();
    let mut shy: Vec<usize> = Vec::new();
    for (i, span) in para.spans.iter().enumerate() {
        let start = text.len();
        for c in span.text.chars() {
            if c == '\u{ad}' {
                shy.push(text.len());
            } else {
                text.push(c);
            }
        }
        if text.len() > start {
            style_runs.push((start..text.len(), i));
        }
    }
    let styles: Vec<TextStyle> = para.spans.iter().map(|s| s.style.clone()).collect();
    let base_style = styles
        .first()
        .cloned()
        .unwrap_or_else(|| TextStyle::new(super::FamilyId(0), 16.0));
    let n_styles = styles.len().max(1);
    let mut b = Builder {
        engine,
        styles: if styles.is_empty() {
            vec![base_style.clone()]
        } else {
            styles
        },
        runs: Vec::new(),
        space: vec![None; n_styles],
        hyphen: vec![None; n_styles],
        items: Vec::new(),
    };
    let style_at = |pos: usize| -> usize {
        style_runs
            .iter()
            .find(|(r, _)| r.contains(&pos))
            .or(style_runs.last())
            .map(|(_, s)| *s)
            .unwrap_or(0)
    };

    if para.style.indent > 0.0 {
        b.items.push(Item::Box {
            w: para.style.indent,
            run: usize::MAX,
            clusters: 0..0,
        });
    }

    // 2. Turn each UAX #14 segment into boxes, glue and penalties.
    let mut points = Vec::new();
    let mut seg_start = 0;
    for (end, opportunity) in linebreaks(&text) {
        let seg = &text[seg_start..end];
        let body_len = seg.trim_end_matches(is_break_space).len();
        let body = seg_start..seg_start + body_len;
        let ws = &text[body.end..end];
        let at_end = end == text.len();

        // Split the body where the style changes.
        let mut pieces: Vec<(Range<usize>, usize)> = Vec::new();
        let mut p = body.start;
        while p < body.end {
            let s = style_at(p);
            let run_end = style_runs
                .iter()
                .find(|(r, _)| r.contains(&p))
                .map(|(r, _)| r.end)
                .unwrap_or(body.end)
                .min(body.end);
            pieces.push((p..run_end, s));
            p = run_end;
        }

        for (range, style) in &pieces {
            let run = b.shape(&text[range.clone()], *style);
            let n = b.runs[run].clusters.len();
            // Hyphenation points, relative to the piece.
            points.clear();
            if para.style.hyphenate {
                hyphenator.hyphenate(&text[range.clone()], &mut points);
            }
            points.extend(
                shy.iter()
                    .filter(|&&s| s > range.start && s < range.end)
                    .map(|&s| s - range.start),
            );
            points.sort_unstable();
            points.dedup();
            let mut from = 0;
            for &pt in &points {
                let Some(c) = b.runs[run].cluster_at(pt as u32) else {
                    continue; // inside a ligature: not a usable point
                };
                if c == 0 || c >= n || c <= from {
                    continue;
                }
                b.push_box(run, from..c);
                let hy = b.hyphen_run(*style);
                b.items.push(Item::Penalty {
                    w: b.runs[hy].width,
                    cost: HYPHEN_PENALTY,
                    flagged: true,
                    hyphen: Some(hy),
                });
                from = c;
            }
            b.push_box(run, from..n);
        }

        let mandatory = matches!(opportunity, BreakOpportunity::Mandatory);
        if at_end {
            break;
        }
        if mandatory {
            b.items.push(Item::Glue {
                w: 0.0,
                stretch: INFINITE,
                shrink: 0.0,
            });
            b.items.push(Item::Penalty {
                w: 0.0,
                cost: -INFINITE,
                flagged: false,
                hyphen: None,
            });
        } else if !ws.is_empty() {
            let single_letter = {
                let mut cs = text[body.clone()].chars();
                matches!(
                    (cs.next(), cs.next()),
                    (
                        Some('a' | 'i' | 'o' | 'u' | 'w' | 'z' | 'A' | 'I' | 'O' | 'U' | 'W' | 'Z'),
                        None
                    )
                )
            };
            if para.style.no_orphan_letters && single_letter {
                // A penalty between box and glue makes the glue an illegal
                // break, so the letter stays with the next word.
                b.items.push(Item::Penalty {
                    w: 0.0,
                    cost: INFINITE,
                    flagged: false,
                    hyphen: None,
                });
            }
            let w = b.space_width(style_at(body.end));
            b.items.push(Item::Glue {
                w,
                stretch: w * 1.0,
                shrink: w * 0.3,
            });
        } else if !body.is_empty() {
            // A break with no space, e.g. after a hyphen or a slash.
            let after_hyphen = text[body.clone()].ends_with(['-', '\u{2010}']);
            b.items.push(Item::Penalty {
                w: 0.0,
                cost: if after_hyphen { HYPHEN_PENALTY } else { 0.0 },
                flagged: after_hyphen,
                hyphen: None,
            });
        }
        seg_start = end;
    }
    b.items.push(Item::Glue {
        w: 0.0,
        stretch: INFINITE,
        shrink: 0.0,
    });
    b.items.push(Item::Penalty {
        w: 0.0,
        cost: -INFINITE,
        flagged: false,
        hyphen: None,
    });

    // 3. Choose breaks.
    let items = std::mem::take(&mut b.items);
    let breaks = if para.style.align == Align::Justify {
        knuth_plass(&items, width, 200.0)
            .or_else(|| knuth_plass(&items, width, 2000.0))
            .unwrap_or_else(|| first_fit(&items, width))
    } else {
        first_fit(&items, width)
    };

    // 4. Set the lines.
    let metrics = b.engine.metrics(&base_style);
    let pitch = metrics.pitch;
    let half_leading = (pitch - (metrics.ascent + metrics.descent.abs())) / 2.0;
    let ascent = (half_leading + metrics.ascent).round();
    let mut lines = set_lines(&items, &breaks, width, para.style.align);
    if let Some(max) = para.style.max_lines {
        if lines.len() > max {
            lines.truncate(max);
            if let Some(last) = lines.last_mut() {
                ellipsize(&mut b, last, width);
            }
        }
    }
    let mut max_w: f32 = 0.0;
    for (i, line) in lines.iter_mut().enumerate() {
        line.baseline = i as f32 * pitch + ascent;
        let shift = match para.style.align {
            Align::Center => ((width - line.width) / 2.0).max(0.0),
            Align::Right => (width - line.width).max(0.0),
            _ => 0.0,
        };
        for piece in &mut line.pieces {
            piece.x += shift;
        }
        max_w = max_w.max(line.width);
    }
    let height = lines.len() as f32 * pitch;
    ParagraphLayout {
        runs: b.runs,
        lines,
        width: max_w,
        pitch,
        height,
    }
}

/// Index of the first item of a line that starts after a break at `brk`
/// (`None` for the paragraph start): spaces and penalties after a break are
/// discarded, as in TeX.
fn line_start(items: &[Item], brk: Option<usize>) -> usize {
    let mut i = brk.map_or(0, |b| b + 1);
    if brk.is_none() {
        return 0;
    }
    while i < items.len() && !items[i].is_box() && !items[i].forced() {
        i += 1;
    }
    i
}

fn legal_break(items: &[Item], i: usize) -> bool {
    match &items[i] {
        Item::Penalty { cost, .. } => *cost < INFINITE,
        Item::Glue { .. } => i > 0 && items[i - 1].is_box(),
        Item::Box { .. } => false,
    }
}

struct Sums {
    w: Vec<f32>,
    stretch: Vec<f32>,
    shrink: Vec<f32>,
}

/// Prefix sums of widths, stretch and shrink: `w[i]` covers `items[..i]`.
/// Penalty widths count only when the line breaks at them.
fn sums(items: &[Item]) -> Sums {
    let mut s = Sums {
        w: vec![0.0],
        stretch: vec![0.0],
        shrink: vec![0.0],
    };
    let (mut w, mut y, mut z) = (0.0, 0.0, 0.0);
    for item in items {
        match item {
            Item::Box { w: bw, .. } => w += bw,
            Item::Glue {
                w: gw,
                stretch,
                shrink,
            } => {
                w += gw;
                y += stretch;
                z += shrink;
            }
            Item::Penalty { .. } => {}
        }
        s.w.push(w);
        s.stretch.push(y);
        s.shrink.push(z);
    }
    s
}

/// Natural width, stretch and shrink of a line from `start` to the break
/// at `brk` (exclusive), including the penalty width if it breaks there.
fn measure(items: &[Item], s: &Sums, start: usize, brk: usize) -> (f32, f32, f32) {
    let mut w = s.w[brk] - s.w[start];
    if let Item::Penalty { w: pw, .. } = items[brk] {
        w += pw;
    }
    (
        w,
        s.stretch[brk] - s.stretch[start],
        s.shrink[brk] - s.shrink[start],
    )
}

fn ratio(width: f32, natural: f32, stretch: f32, shrink: f32) -> f32 {
    if natural < width {
        if stretch > 0.0 {
            (width - natural) / stretch
        } else {
            INFINITE
        }
    } else if natural > width {
        if shrink > 0.0 {
            (width - natural) / shrink
        } else {
            -INFINITE
        }
    } else {
        0.0
    }
}

fn fitness(r: f32) -> u8 {
    if r < -0.5 {
        0
    } else if r <= 0.5 {
        1
    } else if r <= 1.0 {
        2
    } else {
        3
    }
}

struct Node {
    brk: Option<usize>,
    fitness: u8,
    demerits: f32,
    prev: Option<usize>,
}

/// Knuth–Plass optimal line breaking. Returns the break indices, ending
/// with the final forced break, or `None` if no set of breaks keeps every
/// line within `tolerance` (badness, 100 × ratio³).
fn knuth_plass(items: &[Item], width: f32, tolerance: f32) -> Option<Vec<usize>> {
    let s = sums(items);
    let mut nodes = vec![Node {
        brk: None,
        fitness: 1,
        demerits: 0.0,
        prev: None,
    }];
    let mut active: Vec<usize> = vec![0];
    for b in 0..items.len() {
        if !legal_break(items, b) {
            continue;
        }
        let (cost, flagged) = match items[b] {
            Item::Penalty { cost, flagged, .. } => (cost, flagged),
            _ => (0.0, false),
        };
        let forced = items[b].forced();
        let mut best: [Option<(f32, usize)>; 4] = [None; 4];
        let mut k = 0;
        while k < active.len() {
            let a = &nodes[active[k]];
            let start = line_start(items, a.brk);
            if start > b {
                k += 1;
                continue;
            }
            let (nat, y, z) = measure(items, &s, start, b);
            let r = if forced && nat <= width {
                0.0
            } else {
                ratio(width, nat, y, z)
            };
            if (-1.0..).contains(&r) {
                let badness = 100.0 * r.abs().powi(3);
                if badness <= tolerance || forced {
                    let base = (LINE_PENALTY + badness).powi(2);
                    let mut d = if cost >= 0.0 {
                        base + cost * cost
                    } else if cost > -INFINITE {
                        base - cost * cost
                    } else {
                        base
                    };
                    let prev_flagged = a
                        .brk
                        .is_some_and(|p| matches!(items[p], Item::Penalty { flagged: true, .. }));
                    if flagged && prev_flagged {
                        d += DOUBLE_HYPHEN_DEMERITS;
                    }
                    let f = fitness(r);
                    if a.brk.is_some() && f.abs_diff(a.fitness) > 1 {
                        d += FITNESS_DEMERITS;
                    }
                    let total = a.demerits + d;
                    let slot = &mut best[f as usize];
                    if slot.is_none_or(|(t, _)| total < t) {
                        *slot = Some((total, active[k]));
                    }
                }
            }
            if r < -1.0 || forced {
                active.remove(k);
            } else {
                k += 1;
            }
        }
        for (f, slot) in best.iter().enumerate() {
            if let Some((demerits, prev)) = *slot {
                nodes.push(Node {
                    brk: Some(b),
                    fitness: f as u8,
                    demerits,
                    prev: Some(prev),
                });
                active.push(nodes.len() - 1);
            }
        }
        if active.is_empty() {
            return None;
        }
    }
    let last = items.len() - 1;
    let mut node = active
        .iter()
        .copied()
        .filter(|&i| nodes[i].brk == Some(last))
        .min_by(|&a, &b| nodes[a].demerits.total_cmp(&nodes[b].demerits))?;
    let mut breaks = Vec::new();
    while let Some(brk) = nodes[node].brk {
        breaks.push(brk);
        node = nodes[node].prev?;
    }
    breaks.reverse();
    Some(breaks)
}

/// First-fit line breaking: take the last break that still fits. A box
/// wider than the line overflows rather than being split.
fn first_fit(items: &[Item], width: f32) -> Vec<usize> {
    let s = sums(items);
    let mut breaks = Vec::new();
    let mut start = 0;
    let mut last_fit: Option<usize> = None;
    let mut b = 0;
    while b < items.len() {
        if !legal_break(items, b) || b < start {
            b += 1;
            continue;
        }
        let (nat, _, z) = measure(items, &s, start, b);
        let fits = nat - z.min(0.0) <= width + 0.01;
        if fits {
            if items[b].forced() {
                breaks.push(b);
                start = line_start(items, Some(b));
                last_fit = None;
            } else {
                last_fit = Some(b);
            }
            b += 1;
        } else if let Some(lf) = last_fit {
            breaks.push(lf);
            start = line_start(items, Some(lf));
            last_fit = None;
            // Re-examine b against the new line.
        } else {
            // Nothing fits: overflow at this break.
            breaks.push(b);
            start = line_start(items, Some(b));
            b += 1;
        }
    }
    if breaks.last() != Some(&(items.len() - 1)) {
        breaks.push(items.len() - 1);
    }
    breaks
}

fn set_lines(items: &[Item], breaks: &[usize], width: f32, align: Align) -> Vec<Line> {
    let s = sums(items);
    let mut lines = Vec::new();
    let mut prev: Option<usize> = None;
    for &brk in breaks {
        let start = line_start(items, prev);
        prev = Some(brk);
        let start = start.min(brk);
        let (nat, y, z) = measure(items, &s, start, brk);
        let last = items[brk].forced();
        let mut r = if align == Align::Justify && !last {
            ratio(width, nat, y, z).max(-1.0)
        } else {
            0.0
        };
        if r > 3.0 {
            // Only reachable from the first-fit fallback: a line that would
            // need huge gaps looks better ragged than stretched.
            r = 0.0;
        }
        let mut x = 0.0;
        let mut pieces = Vec::new();
        for item in &items[start..brk] {
            match item {
                Item::Box { w, run, clusters } => {
                    if *run != usize::MAX && !clusters.is_empty() {
                        pieces.push(Piece {
                            run: *run,
                            clusters: clusters.clone(),
                            x,
                        });
                    }
                    x += w;
                }
                Item::Glue { w, stretch, shrink } => {
                    if *stretch >= INFINITE {
                        continue;
                    }
                    x += w + if r >= 0.0 { r * stretch } else { r * shrink };
                }
                Item::Penalty { .. } => {}
            }
        }
        let mut hyphenated = false;
        if let Item::Penalty {
            hyphen: Some(hy),
            w,
            ..
        } = items[brk]
        {
            pieces.push(Piece {
                run: hy,
                clusters: 0..1,
                x,
            });
            x += w;
            hyphenated = true;
        }
        lines.push(Line {
            pieces,
            width: x,
            baseline: 0.0,
            hyphenated,
        });
    }
    lines
}

/// Shortens a line so it ends in "…" within `width`, cutting inside a word
/// if needed (titles are often one long word in a narrow column).
fn ellipsize(b: &mut Builder, line: &mut Line, width: f32) {
    let style = line.pieces.last().map(|p| b.runs[p.run].style).unwrap_or(0);
    let ell = b.shape("\u{2026}", style);
    let ell_w = b.runs[ell].width;
    line.hyphenated = false;
    // Drop a trailing hyphen piece, then trim clusters until it fits.
    if let Some(p) = line.pieces.last() {
        if b.runs[p.run].text.as_deref() == Some("-") {
            line.pieces.pop();
        }
    }
    while let Some(p) = line.pieces.last_mut() {
        let run = &b.runs[p.run];
        let end = p.x + run.width_of(p.clusters.clone());
        if end + ell_w <= width {
            break;
        }
        if p.clusters.len() > 1 {
            p.clusters.end -= 1;
        } else {
            line.pieces.pop();
        }
    }
    let x = line
        .pieces
        .last()
        .map(|p| p.x + b.runs[p.run].width_of(p.clusters.clone()))
        .unwrap_or(0.0);
    let n = b.runs[ell].clusters.len();
    line.pieces.push(Piece {
        run: ell,
        clusters: 0..n,
        x,
    });
    line.width = x + ell_w;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{FamilyId, Fonts, NoHyphenation, PolishSyllables};

    const PRUS: &str = "W początkach roku 1878, kiedy świat polityczny zajmował się \
        pokojem san-stefańskim, wyborem nowego papieża albo szansami europejskiej \
        wojny, warszawscy kupcy tudzież inteligencja pewnej okolicy Krakowskiego \
        Przedmieścia nie mniej gorąco interesowała się przyszłością galanteryjnego \
        sklepu pod firmą J. Mincel i S. Wokulski.";

    fn engine() -> TextEngine {
        TextEngine::new(Fonts::embedded())
    }

    fn body() -> TextStyle {
        TextStyle::new(FamilyId(0), 42.0).line_height(1.5)
    }

    fn justified(text: &str) -> Paragraph {
        Paragraph::new(
            vec![Span::new(text, body())],
            ParagraphStyle {
                align: Align::Justify,
                hyphenate: true,
                ..Default::default()
            },
        )
    }

    #[test]
    fn justified_lines_fill_the_measure() {
        let mut e = engine();
        let l = layout_paragraph(&mut e, &justified(PRUS), 896.0, &PolishSyllables::default());
        assert!(l.lines.len() >= 6);
        for line in &l.lines[..l.lines.len() - 1] {
            assert!(
                (line.width - 896.0).abs() < 0.5,
                "line width {}",
                line.width
            );
        }
        let last = l.lines.last().unwrap();
        assert!(last.width < 896.0);
        // Baselines advance by the pitch.
        assert_eq!(l.pitch, 63.0);
        assert_eq!(l.lines[1].baseline - l.lines[0].baseline, 63.0);
    }

    #[test]
    fn single_letter_words_never_end_a_line() {
        let mut e = engine();
        for width in (500..900).step_by(37) {
            let l = layout_paragraph(
                &mut e,
                &justified(PRUS),
                width as f32,
                &PolishSyllables::default(),
            );
            for i in 0..l.lines.len() {
                let t = l.line_text(i);
                let last = t.split(' ').next_back().unwrap();
                assert!(
                    !matches!(last, "i" | "w" | "z" | "o" | "a" | "u" | "W"),
                    "width {width}: line ends with {last:?}: {t}"
                );
            }
        }
    }

    #[test]
    fn soft_hyphens_are_break_points() {
        let mut e = engine();
        let word = "Kon\u{ad}stan\u{ad}ty\u{ad}no\u{ad}pol";
        let p = Paragraph::plain(word, body(), Align::Left);
        let l = layout_paragraph(&mut e, &p, 160.0, &NoHyphenation);
        let lines: Vec<String> = (0..l.lines.len()).map(|i| l.line_text(i)).collect();
        assert!(lines.len() > 1, "{lines:?}");
        assert!(lines[0].ends_with('-'), "{lines:?}");
        let joined: String = lines.concat().replace('-', "");
        assert_eq!(joined, "Konstantynopol");
        // Unbroken, the soft hyphens are invisible.
        let l = layout_paragraph(&mut e, &p, 2000.0, &NoHyphenation);
        assert_eq!(l.line_text(0), "Konstantynopol");
    }

    #[test]
    fn hard_line_breaks_and_ragged_text() {
        let mut e = engine();
        let p = Paragraph::plain("Rozdział pierwszy\nLalka", body(), Align::Center);
        let l = layout_paragraph(&mut e, &p, 900.0, &NoHyphenation);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.line_text(1), "Lalka");
        // Centred: the short line starts further right.
        assert!(l.lines[1].pieces[0].x > l.lines[0].pieces[0].x);
    }

    #[test]
    fn ellipsis_after_max_lines() {
        let mut e = engine();
        let mut p = Paragraph::plain(PRUS, body(), Align::Left);
        p.style.max_lines = Some(2);
        let l = layout_paragraph(&mut e, &p, 600.0, &NoHyphenation);
        assert_eq!(l.lines.len(), 2);
        assert!(l.line_text(1).ends_with('\u{2026}'), "{}", l.line_text(1));
        assert!(l.lines[1].width <= 600.0);
    }

    #[test]
    fn polish_diacritics_are_covered_by_every_bundled_font() {
        let mut fonts = Fonts::embedded();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
        fonts.load_family("Charis", &dir, "Charis").unwrap();
        fonts.load_family("Gentium", &dir, "GentiumBook").unwrap();
        let e = TextEngine::new(fonts);
        let all = "ĄĆĘŁŃÓŚŹŻąćęłńóśźż„”«»–—…";
        for fam in 0..3 {
            for (bold, italic) in [(false, false), (false, true), (true, false), (true, true)] {
                let mut s = TextStyle::new(FamilyId(fam), 40.0);
                s.bold = bold;
                s.italic = italic;
                assert!(
                    e.covers(all, &s),
                    "family {fam} bold {bold} italic {italic}"
                );
            }
        }
    }

    #[test]
    fn kerning_is_applied() {
        let mut e = engine();
        let s = TextStyle::new(FamilyId(0), 100.0);
        let pair = e.measure("AV", &s);
        let apart = e.measure("A", &s) + e.measure("V", &s);
        assert!(pair < apart - 2.0, "AV {pair} vs A+V {apart}");
    }
}
