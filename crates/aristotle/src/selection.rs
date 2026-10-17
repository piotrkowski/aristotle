// SPDX-License-Identifier: AGPL-3.0-or-later
//! Words under the finger, and selections of words on a page.
//!
//! Plato lays text out word by word and reports each word's box
//! ([`aristotle_render::book::Word`]); a word broken by hyphenation comes as
//! two pieces, the first ending in a soft hyphen. Everything here works on
//! one page's list of words, by index, so it is cheap and host-tested.

use aristotle_render::book::{Bounds, Word};

/// How far from a word a finger may land and still pick it: 3 mm. Wider
/// than the gap between lines (about 1 mm at 10 pt), so the nearest word
/// wins, but not so wide that a press in the margin selects text.
pub const SLOP: i32 = 36;

/// The word under (or nearest to) a point, within [`SLOP`].
pub fn hit(words: &[Word], x: i32, y: i32) -> Option<usize> {
    let mut best: Option<(i64, usize)> = None;
    for (i, w) in words.iter().enumerate() {
        let d = w.rect.dist2(x, y);
        if d <= (SLOP as i64).pow(2) && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// The whole word around piece `i`: a piece ending in a soft hyphen
/// continues in the next one.
pub fn whole_word(words: &[Word], i: usize) -> (usize, usize) {
    let soft = |k: usize| words[k].text.ends_with('\u{00AD}');
    let mut start = i;
    while start > 0 && soft(start - 1) {
        start -= 1;
    }
    let mut end = i;
    while end + 1 < words.len() && soft(end) {
        end += 1;
    }
    (start, end)
}

/// Punctuation and quotes a selection leaves out at its ends: "dusza,"
/// selects and looks up "dusza".
fn edge_mark(c: char) -> bool {
    !c.is_alphanumeric()
}

/// How wide a character is, roughly, in letters: punctuation is narrow,
/// dashes are wide.
fn weight(c: char) -> f32 {
    match c {
        '—' | '–' | '…' => 1.0,
        c if edge_mark(c) => 0.45,
        _ => 1.0,
    }
}

/// The horizontal extent of a word's letters without the punctuation at
/// its ends. Plato reports one box per word; the share of the box is
/// estimated from the characters.
pub fn letters_x(w: &Word) -> (i32, i32) {
    let chars: Vec<char> = w.text.chars().collect();
    let first = chars.iter().position(|c| !edge_mark(*c));
    let last = chars.iter().rposition(|c| !edge_mark(*c));
    let (Some(first), Some(last)) = (first, last) else {
        return (w.rect.x0, w.rect.x1);
    };
    let total: f32 = chars.iter().map(|&c| weight(c)).sum();
    let lead: f32 = chars[..first].iter().map(|&c| weight(c)).sum();
    let trail: f32 = chars[last + 1..].iter().map(|&c| weight(c)).sum();
    let width = (w.rect.x1 - w.rect.x0) as f32;
    (
        w.rect.x0 + (width * lead / total).round() as i32,
        w.rect.x1 - (width * trail / total).round() as i32,
    )
}

/// The text of words `start..=end`, without punctuation at its two ends:
/// the word to look up.
pub fn text(words: &[Word], start: usize, end: usize) -> String {
    joined(words, start, end)
        .trim_matches(edge_mark)
        .to_string()
}

/// The text a highlight of words `start..=end` covers: what
/// [`line_boxes`] shades, so the punctuation at the ends of the first and
/// last words is left out, but a word of punctuation alone (Plato makes
/// "," of "art," a word of its own in a hyphenated paragraph) stays.
pub fn quote(words: &[Word], start: usize, end: usize) -> String {
    let end = end.min(words.len().saturating_sub(1));
    if start > end {
        return String::new();
    }
    let has_letters = |w: &Word| w.text.chars().any(|c| !edge_mark(c));
    let mut s = joined(words, start, end);
    if has_letters(&words[end]) {
        s.truncate(s.trim_end_matches(edge_mark).len());
    }
    if has_letters(&words[start]) {
        s = s.trim_start_matches(edge_mark).to_string();
    }
    s.trim().to_string()
}

/// Whether `b` follows `a` with no space between them: the next byte of
/// the book's text (Plato splits "many:" into "many" and ":" when it
/// hyphenates a paragraph, and breaks "ship-building" after the hyphen), or
/// touching it on the same line (a word continued in another element, as
/// in `<i>art</i>,`).
fn glued(a: &Word, b: &Word) -> bool {
    if a.offset + a.text.len() == b.offset {
        return true;
    }
    let same_line = a.rect.y0 < b.rect.y1 && b.rect.y0 < a.rect.y1;
    same_line && b.rect.x0 >= a.rect.x0 && b.rect.x0 - a.rect.x1 <= 1
}

/// Words `start..=end` as the book spells them: pieces of a hyphenated
/// word joined, a space only where the book has one.
fn joined(words: &[Word], start: usize, end: usize) -> String {
    let mut s = String::new();
    let end = end.min(words.len().saturating_sub(1));
    for (i, w) in words.iter().enumerate().take(end + 1).skip(start) {
        if s.ends_with('\u{00AD}') {
            s.pop();
        } else if i > start && !glued(&words[i - 1], w) {
            s.push(' ');
        }
        s.push_str(&w.text);
    }
    s.trim_end_matches('\u{00AD}').to_string()
}

/// A selection: the word it started on (the anchor) and its ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: (usize, usize),
    pub start: usize,
    pub end: usize,
}

impl Selection {
    /// The whole word at piece `i`.
    pub fn word(words: &[Word], i: usize) -> Selection {
        let (start, end) = whole_word(words, i);
        Selection {
            anchor: (start, end),
            start,
            end,
        }
    }

    /// Extend from the anchor to the whole word at piece `i`, in either
    /// direction. Returns whether anything changed.
    pub fn extend_to(&mut self, words: &[Word], i: usize) -> bool {
        let (ws, we) = whole_word(words, i);
        let (start, end) = if ws < self.anchor.0 {
            (ws, self.anchor.1)
        } else {
            (self.anchor.0, we.max(self.anchor.1))
        };
        let changed = (start, end) != (self.start, self.end);
        self.start = start;
        self.end = end;
        changed
    }

    pub fn text(&self, words: &[Word]) -> String {
        text(words, self.start, self.end)
    }

    /// The text to save as a highlight; see [`quote`].
    pub fn quote(&self, words: &[Word]) -> String {
        quote(words, self.start, self.end)
    }
}

/// The boxes to paint for words `start..=end`, one per line: words on the
/// same line merge into one box, which also covers the spaces between
/// them. Lines are told apart by vertical overlap.
pub fn line_boxes(words: &[Word], start: usize, end: usize) -> Vec<Bounds> {
    let mut out: Vec<Bounds> = Vec::new();
    for (i, w) in words.iter().enumerate().take(end + 1).skip(start) {
        // The ends leave punctuation out, like the text.
        let mut r = w.rect;
        if i == start {
            r.x0 = letters_x(w).0;
        }
        if i == end {
            r.x1 = letters_x(w).1;
        }
        match out.last_mut() {
            Some(last) if r.y0 < last.y1 && r.y1 > last.y0 && r.x0 >= last.x0 => {
                *last = last.union(&r);
            }
            _ => out.push(r),
        }
    }
    out
}

/// The box a marker covers for a line box: Plato's word boxes span the
/// font's whole ascender and descender, which reach into the next lines
/// at 1.3 em leading, so the marker keeps a little inside them
/// vertically and a little outside horizontally.
pub fn marker_box(b: &Bounds) -> Bounds {
    let h = b.y1 - b.y0;
    Bounds {
        x0: b.x0 - 4,
        y0: b.y0 + h / 9,
        x1: b.x1 + 4,
        y1: b.y1 - h / 18,
    }
}

/// A selection handle: x, and the top and bottom of its line.
pub type Handle = (i32, i32, i32);

/// Where the selection handles go: a bar at the start of the first box and
/// at the end of the last, as (x, top, bottom).
pub fn handles(boxes: &[Bounds]) -> Option<(Handle, Handle)> {
    let first = boxes.first()?;
    let last = boxes.last()?;
    Some(((first.x0, first.y0, first.y1), (last.x1, last.y0, last.y1)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A page of words laid out on a grid: `lines` of `(text, width)`.
    pub fn page(lines: &[&[&str]]) -> Vec<Word> {
        let mut out = Vec::new();
        let mut offset = 1000;
        for (row, line) in lines.iter().enumerate() {
            let y0 = 104 + row as i32 * 56;
            let mut x = 59;
            for t in *line {
                let w = 24 * t.chars().filter(|&c| c != '\u{AD}').count() as i32;
                out.push(Word {
                    text: t.to_string(),
                    rect: Bounds {
                        x0: x,
                        y0,
                        x1: x + w,
                        y1: y0 + 44,
                    },
                    offset,
                });
                offset += t.len() + 1;
                x += w + 14;
            }
        }
        out
    }

    fn sample() -> Vec<Word> {
        page(&[
            &["Dusza", "jest", "nieśmier\u{AD}"],
            &["telna,", "a", "ciało"],
            &["śmiertelne."],
        ])
    }

    #[test]
    fn hit_inside_between_and_outside() {
        let w = sample();
        // Inside "jest".
        let j = &w[1].rect;
        assert_eq!(hit(&w, j.x0 + 5, j.y0 + 5), Some(1));
        // In the gap between "Dusza" and "jest": the nearer one.
        let d = &w[0].rect;
        assert_eq!(hit(&w, d.x1 + 3, d.y0 + 10), Some(0));
        assert_eq!(hit(&w, j.x0 - 3, j.y0 + 10), Some(1));
        // Between lines: the nearer line.
        assert_eq!(hit(&w, d.x0 + 10, d.y1 + 4), Some(0));
        assert_eq!(hit(&w, d.x0 + 10, d.y1 + 10), Some(3));
        // Far in the margin: nothing.
        assert_eq!(hit(&w, 1000, 1300), None);
        assert_eq!(hit(&w, 5, 5), None);
    }

    #[test]
    fn hyphenated_words_are_one_word() {
        let w = sample();
        assert_eq!(whole_word(&w, 2), (2, 3));
        assert_eq!(whole_word(&w, 3), (2, 3));
        assert_eq!(whole_word(&w, 0), (0, 0));
        let s = Selection::word(&w, 3);
        assert_eq!(s.text(&w), "nieśmiertelna");
    }

    #[test]
    fn extending_both_ways_keeps_the_anchor() {
        let w = sample();
        let mut s = Selection::word(&w, 1); // "jest"
        assert!(s.extend_to(&w, 5));
        assert_eq!(s.text(&w), "jest nieśmiertelna, a ciało");
        // Back before the anchor: the anchor word stays the end.
        assert!(s.extend_to(&w, 0));
        assert_eq!(s.text(&w), "Dusza jest");
        assert!(!s.extend_to(&w, 0));
        // Into the middle of the hyphenated word: the whole word.
        s.extend_to(&w, 2);
        assert_eq!(s.text(&w), "jest nieśmiertelna");
    }

    #[test]
    fn ends_leave_punctuation_out() {
        let w = page(&[&["„Dusza,”", "jest"]]);
        assert_eq!(text(&w, 0, 0), "Dusza");
        assert_eq!(text(&w, 0, 1), "Dusza,” jest");
        let (x0, x1) = letters_x(&w[0]);
        assert!(x0 > w[0].rect.x0 && x1 < w[0].rect.x1);
        let b = line_boxes(&w, 0, 0);
        assert_eq!((b[0].x0, b[0].x1), (x0, x1));
        // Text only of punctuation keeps its box.
        let dash = page(&[&["—"]]);
        assert_eq!(letters_x(&dash[0]), (dash[0].rect.x0, dash[0].rect.x1));
    }

    /// Words with their offsets in the book's text. A `+` before a word
    /// means no space before it in the book, and no gap on the page.
    fn spelled(lines: &[&[&str]]) -> Vec<Word> {
        let mut out = Vec::new();
        let mut offset = 1000;
        for (n, line) in lines.iter().enumerate() {
            let y0 = 104 + 54 * n as i32;
            let mut x = 59;
            for t in *line {
                let (glue, t) = match t.strip_prefix('+') {
                    Some(t) => (true, t),
                    None => (false, *t),
                };
                if !glue {
                    offset += 1;
                    if x > 59 {
                        x += 14;
                    }
                }
                let w = 24 * t.chars().count() as i32;
                out.push(Word {
                    text: t.to_string(),
                    rect: Bounds {
                        x0: x,
                        y0,
                        x1: x + w,
                        y1: y0 + 44,
                    },
                    offset,
                });
                offset += t.len();
                x += w;
            }
        }
        out
    }

    #[test]
    fn punctuation_split_off_by_plato_takes_no_space() {
        // As Plato reports a hyphenated paragraph of Chase's translation.
        let w = spelled(&[
            &["many", "+:", "of", "the", "heal\u{AD}"],
            &["+ing", "art", "+,", "for", "instance", "+,"],
            &["health", "+;", "of", "the", "ship", "+-"],
            &["+building", "art"],
        ]);
        let last = w.len() - 1;
        assert_eq!(
            quote(&w, 0, last),
            "many: of the healing art, for instance, health; of the ship-building art"
        );
        assert_eq!(text(&w, 2, 6), "of the healing art");
        // The comma is a word of its own: shaded, so in the highlight.
        assert_eq!(quote(&w, 6, 7), "art,");
        assert_eq!(quote(&w, 6, 6), "art");
        // A word continued in another element touches the one before it,
        // though the offsets skip the markup between them.
        let mut tag = spelled(&[&["art", "+,", "for"]]);
        tag[1].offset += "</i>".len();
        tag[2].offset += "</i>".len();
        assert_eq!(quote(&tag, 0, 2), "art, for");
    }

    #[test]
    fn quote_keeps_what_is_shaded() {
        let w = page(&[&["„Dusza,”", "jest", "(nieśmiertelna)."]]);
        assert_eq!(quote(&w, 0, 2), "Dusza,” jest (nieśmiertelna");
        assert_eq!(quote(&w, 0, 0), "Dusza");
        let dash = page(&[&["—"]]);
        assert_eq!(quote(&dash, 0, 0), "—");
    }

    #[test]
    fn boxes_merge_per_line() {
        let w = sample();
        let b = line_boxes(&w, 1, 6);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].x0, w[1].rect.x0);
        assert_eq!(b[0].x1, w[2].rect.x1);
        assert_eq!(b[1].x1, w[5].rect.x1);
        let (a, z) = handles(&b).unwrap();
        assert_eq!(a, (w[1].rect.x0, w[1].rect.y0, w[1].rect.y1));
        // The last word ends in a period, which the box leaves out.
        assert_eq!(z.0, letters_x(&w[6]).1);
        assert!(z.0 < w[6].rect.x1);
    }
}
