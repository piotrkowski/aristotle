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

/// The text of words `start..=end`: pieces of a hyphenated word joined,
/// other words separated by a space.
pub fn text(words: &[Word], start: usize, end: usize) -> String {
    let mut s = String::new();
    for w in &words[start..=end.min(words.len().saturating_sub(1))] {
        if !s.is_empty() && !s.ends_with('\u{00AD}') {
            s.push(' ');
        }
        if s.ends_with('\u{00AD}') {
            s.pop();
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
}

/// The boxes to paint for words `start..=end`, one per line: words on the
/// same line merge into one box, which also covers the spaces between
/// them. Lines are told apart by vertical overlap.
pub fn line_boxes(words: &[Word], start: usize, end: usize) -> Vec<Bounds> {
    let mut out: Vec<Bounds> = Vec::new();
    for w in words.iter().take(end + 1).skip(start) {
        let r = w.rect;
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
        assert_eq!(s.text(&w), "nieśmiertelna,");
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
        assert_eq!(s.text(&w), "jest nieśmiertelna,");
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
        assert_eq!(z.0, w[6].rect.x1);
    }
}
