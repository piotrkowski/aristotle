// SPDX-License-Identifier: AGPL-3.0-or-later
//! The on-screen keyboard and the text field it types into.
//!
//! # Layout
//!
//! Four rows on a grid of tenths of the screen width, the same in every
//! layer, so switching a layer only relabels keys:
//!
//! ```text
//! letters    q w e r t y u i o p     symbols   1 2 3 4 5 6 7 8 9 0     more    [ ] { } # ^ * < > ~
//!             a s d f g h j k l                 - / : ; ( ) $ & @               \ | ` € £ ¥ § ° •
//!           ⇧  z x c v b n m  ⌫              #+= , ? ! ' " _ %  ⌫           ?123 „ ” « » – — …  ⌫
//!          ?123 ąę  [space]  . ⏎             abc  +  [space]  . ⏎            abc  =  [space]  . ⏎
//! ```
//!
//! Every printable ASCII character is on one of the layers (a test checks
//! it), so any Wi-Fi password can be typed.
//!
//! # Shift and Polish letters
//!
//! Shift is one-shot: tap it for the next letter, tap it again for caps
//! lock, a third time to turn it off. Polish letters come two ways, both
//! without a popup, because a popup is one more panel update:
//!
//! - hold a letter for half a second: `a` types `ą`, `z` types `ż` and `x`
//!   types `ź`, as AltGr does on the Polish programmer's layout;
//! - or tap `ąę`, which relabels the nine letters that have a Polish form
//!   for the next letter only, then tap the letter.
//!
//! # E-ink
//!
//! Keys and the text field are drawn in pure black and white (glyphs are
//! thresholded after anti-aliasing, [`crate::canvas::Painter::threshold`]):
//!
//! - touch-down inverts only the key, as one A2 update (120 to 144 ms on
//!   the PW4);
//! - the release restores the key and draws the new character as one DU
//!   update over the changed pixels, never GL16 (about 544 ms) and never a
//!   flash. While the panel is busy the display thread merges keys typed
//!   meanwhile into its next update, so fast typing never queues up.

use crate::geom::{Point, Rect, Size};
use crate::icons::{self, Icon};
use crate::text::{TextEngine, TextStyle};
use crate::theme::{Styles, INK, PAPER};
use crate::tree::{
    ActionId, Constraints, Feedback, Kind, Node, NodeId, PaintCx, Refresh, Tree, Widget,
};
use crate::ui::UiEvent;
use aristotle_i18n::{tr, Msg};

/// What a key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Types a character; letters follow shift and the Polish layer.
    Char(char),
    Shift,
    Backspace,
    Enter,
    Space,
    ToLetters,
    ToSymbols,
    ToMore,
    /// The one-shot layer of Polish letters.
    Polish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Letters,
    Symbols,
    More,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    Off,
    /// For the next letter.
    Once,
    Lock,
}

/// What a key press does to the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Insert(char),
    Backspace,
    /// Delete back to the start of the word (holding backspace).
    DeleteWord,
    Enter,
    /// Only the keyboard changed (shift, a layer).
    None,
}

/// Width of each key in tenths of the keyboard, by row. Row 2 has half a
/// key of margin on each side, which belongs to no key.
pub const ROWS: [&[u8]; 4] = [
    &[10; 10],
    &[10; 9],
    &[15, 10, 10, 10, 10, 10, 10, 10, 15],
    &[15, 10, 45, 10, 20],
];

/// Number of keys.
pub const KEYS: usize = 10 + 9 + 9 + 5;

fn row(chars: &str) -> impl Iterator<Item = Key> + '_ {
    chars.chars().map(Key::Char)
}

/// The keys of `layer`, row by row (`KEYS` of them).
pub fn keys(layer: Layer) -> Vec<Key> {
    let (r1, r2, r3, left, extra) = match layer {
        Layer::Letters => (
            "qwertyuiop",
            "asdfghjkl",
            "zxcvbnm",
            Key::Shift,
            Key::Polish,
        ),
        Layer::Symbols => (
            "1234567890",
            "-/:;()$&@",
            ",?!'\"_%",
            Key::ToMore,
            Key::Char('+'),
        ),
        Layer::More => (
            "[]{}#^*<>~",
            "\\|`€£¥§°•",
            "„”«»–—…",
            Key::ToSymbols,
            Key::Char('='),
        ),
    };
    let layer_key = if layer == Layer::Letters {
        Key::ToSymbols
    } else {
        Key::ToLetters
    };
    let mut k: Vec<Key> = row(r1).chain(row(r2)).collect();
    k.push(left);
    k.extend(row(r3));
    k.push(Key::Backspace);
    k.extend([layer_key, extra, Key::Space, Key::Char('.'), Key::Enter]);
    debug_assert_eq!(k.len(), KEYS);
    k
}

/// The Polish letter typed by holding `c`, or by `c` on the `ąę` layer.
pub fn polish(c: char) -> Option<char> {
    let lower = match c.to_lowercase().next()? {
        'a' => 'ą',
        'c' => 'ć',
        'e' => 'ę',
        'l' => 'ł',
        'n' => 'ń',
        'o' => 'ó',
        's' => 'ś',
        'z' => 'ż',
        'x' => 'ź',
        _ => return None,
    };
    Some(if c.is_uppercase() {
        lower.to_uppercase().next()?
    } else {
        lower
    })
}

/// What a key shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cap {
    Text(String),
    Icon(Icon),
}

/// A key's face: what it shows and whether it is drawn solid (a modifier
/// that is on).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    pub cap: Cap,
    pub active: bool,
}

/// The keyboard's state machine, without any drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub layer: Layer,
    pub shift: Shift,
    /// The `ąę` layer, for one letter.
    pub polish: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            layer: Layer::Letters,
            shift: Shift::Off,
            polish: false,
        }
    }
}

impl State {
    pub fn key(&self, slot: usize) -> Key {
        keys(self.layer)[slot]
    }

    /// The character `c` types now.
    pub fn char_of(&self, c: char) -> char {
        if self.layer != Layer::Letters || !c.is_alphabetic() {
            return c;
        }
        let c = if self.shift == Shift::Off {
            c
        } else {
            c.to_uppercase().next().unwrap_or(c)
        };
        if self.polish {
            polish(c).unwrap_or(c)
        } else {
            c
        }
    }

    /// One-shot modifiers end with a letter.
    fn typed_letter(&mut self) {
        if self.shift == Shift::Once {
            self.shift = Shift::Off;
        }
        self.polish = false;
    }

    /// A tap on `slot`.
    pub fn tap(&mut self, slot: usize) -> Edit {
        match self.key(slot) {
            Key::Char(c) => {
                let out = self.char_of(c);
                if self.layer == Layer::Letters && c.is_alphabetic() {
                    self.typed_letter();
                }
                Edit::Insert(out)
            }
            Key::Space => Edit::Insert(' '),
            Key::Backspace => Edit::Backspace,
            Key::Enter => Edit::Enter,
            Key::Shift => {
                self.shift = match self.shift {
                    Shift::Off => Shift::Once,
                    Shift::Once => Shift::Lock,
                    Shift::Lock => Shift::Off,
                };
                Edit::None
            }
            Key::Polish => {
                self.polish = !self.polish;
                Edit::None
            }
            Key::ToLetters => self.switch(Layer::Letters),
            Key::ToSymbols => self.switch(Layer::Symbols),
            Key::ToMore => self.switch(Layer::More),
        }
    }

    fn switch(&mut self, layer: Layer) -> Edit {
        self.layer = layer;
        self.polish = false;
        Edit::None
    }

    /// A long press on `slot`: a letter's Polish form, a word deleted;
    /// anything else as a tap.
    pub fn hold(&mut self, slot: usize) -> Edit {
        match self.key(slot) {
            Key::Char(c) if self.layer == Layer::Letters => {
                let shifted = State {
                    polish: false,
                    ..*self
                }
                .char_of(c);
                match polish(shifted) {
                    Some(p) => {
                        self.typed_letter();
                        Edit::Insert(p)
                    }
                    None => self.tap(slot),
                }
            }
            Key::Backspace => Edit::DeleteWord,
            _ => self.tap(slot),
        }
    }

    /// What `slot` shows. `enter` labels the enter key ("Connect").
    pub fn face(&self, slot: usize, enter: &str) -> Face {
        let text = |s: &str| Cap::Text(s.to_string());
        let (cap, active) = match self.key(slot) {
            Key::Char(c) => (Cap::Text(self.char_of(c).to_string()), false),
            Key::Space => (text(""), false),
            Key::Backspace => (Cap::Icon(Icon::Backspace), false),
            Key::Enter if enter.is_empty() => (Cap::Icon(Icon::Return), false),
            Key::Enter => (text(enter), false),
            Key::Shift => (
                Cap::Icon(match self.shift {
                    Shift::Off => Icon::Shift,
                    Shift::Once => Icon::ShiftOn,
                    Shift::Lock => Icon::ShiftLock,
                }),
                false,
            ),
            Key::Polish => (text("ąę"), self.polish),
            Key::ToLetters => (text(tr(Msg::KeyboardLetters)), false),
            Key::ToSymbols => (text(tr(Msg::KeyboardSymbols)), false),
            Key::ToMore => (text(tr(Msg::KeyboardMoreSymbols)), false),
        };
        Face { cap, active }
    }
}

/// Applies an edit to `text`, keeping at most `max` characters. Returns
/// whether the text changed; `false` for an insert at the limit.
pub fn apply(text: &mut String, edit: Edit, max: usize) -> bool {
    match edit {
        Edit::Insert(c) => {
            if text.chars().count() >= max {
                return false;
            }
            text.push(c);
            true
        }
        Edit::Backspace => text.pop().is_some(),
        Edit::DeleteWord => {
            if text.is_empty() {
                return false;
            }
            let trimmed = text.trim_end_matches(' ').len();
            let start = text[..trimmed].rfind(' ').map_or(0, |i| i + 1);
            text.truncate(start);
            true
        }
        Edit::Enter | Edit::None => false,
    }
}

// --------------------------------------------------------------- drawing

/// Below this a pixel of a key or the field turns black.
const THRESHOLD: u8 = 140;
/// Key height: 9.5 mm.
pub const KEY_H: i32 = 112;
/// Space between keys.
pub const GUTTER: i32 = 8;

/// One key, drawn in pure black and white.
pub struct KeyCap {
    pub face: Face,
    /// For single characters.
    pub glyph: TextStyle,
    /// For words ("?123", "Connect").
    pub word: TextStyle,
}

impl Widget for KeyCap {
    fn measure(&mut self, c: Constraints, _: &mut TextEngine) -> Size {
        c.constrain(Size::new(c.min.w, KEY_H))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let radius = 12.0;
        let (ink, paper) = if self.face.active {
            (PAPER, INK)
        } else {
            (INK, PAPER)
        };
        cx.painter.fill_round_rect(r, radius, paper);
        cx.painter.stroke_round_rect(r, radius, 3.0, INK);
        match &self.face.cap {
            Cap::Icon(icon) => {
                let s = 48;
                let at = Rect::new(r.x + (r.w - s) / 2, r.y + (r.h - s) / 2, s, s);
                icons::draw(cx.painter, *icon, at, ink, 0.0);
            }
            Cap::Text(t) if !t.is_empty() => {
                let style = if t.chars().count() == 1 {
                    &self.glyph
                } else {
                    &self.word
                };
                let style = style.clone().gray(ink);
                let run = cx.text.shape(t, &style);
                let m = cx.text.metrics(&style);
                let x = r.x as f32 + (r.w as f32 - run.width) / 2.0;
                // Centre the x-height band, so "a" and "A" sit alike.
                let baseline = r.y as f32 + (r.h as f32 + m.x_height) / 2.0;
                let n = run.clusters.len();
                cx.text.draw(cx.painter, &run, 0..n, x, baseline);
            }
            Cap::Text(_) => {}
        }
        cx.painter.threshold(r, THRESHOLD);
    }
}

/// A one-line text field with the cursor at the end. In password mode it
/// shows a dot per character until `reveal` is set. Text wider than the
/// field scrolls: the end stays visible after an ellipsis.
pub struct TextField {
    pub text: String,
    pub password: bool,
    pub reveal: bool,
    pub style: TextStyle,
}

/// Height of [`TextField`]: 11 mm.
pub const FIELD_H: i32 = 130;
const FIELD_PAD: i32 = 20;

impl TextField {
    /// The string drawn: the text, or a dot per character.
    pub fn shown(&self) -> String {
        if self.password && !self.reveal {
            "•".repeat(self.text.chars().count())
        } else {
            self.text.clone()
        }
    }
}

/// The longest tail of `s` (with a leading ellipsis if cut) no wider than
/// `width`, by binary search over the cut point.
pub fn fit_tail(text: &mut TextEngine, s: &str, style: &TextStyle, width: f32) -> String {
    if text.measure(s, style) <= width {
        return s.to_string();
    }
    let starts: Vec<usize> = s.char_indices().map(|(i, _)| i).collect();
    let (mut lo, mut hi) = (0, starts.len());
    // Find the smallest cut whose tail fits.
    while lo < hi {
        let mid = (lo + hi) / 2;
        let t = format!("…{}", &s[starts[mid]..]);
        if text.measure(&t, style) <= width {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    match starts.get(lo) {
        Some(&i) => format!("…{}", &s[i..]),
        None => "…".to_string(),
    }
}

impl Widget for TextField {
    fn measure(&mut self, c: Constraints, _: &mut TextEngine) -> Size {
        c.constrain(Size::new(c.max.w, FIELD_H))
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        cx.painter.fill_rect(r, PAPER);
        cx.painter.stroke_round_rect(r, 14.0, 3.0, INK);
        let cursor_w = 5;
        let avail = (r.w - 2 * FIELD_PAD - cursor_w - 4) as f32;
        let shown = fit_tail(cx.text, &self.shown(), &self.style, avail);
        let m = cx.text.metrics(&self.style);
        let baseline = r.y as f32 + (r.h as f32 + m.cap_height) / 2.0;
        let run = cx.text.shape(&shown, &self.style);
        let n = run.clusters.len();
        let x = (r.x + FIELD_PAD) as f32;
        cx.text.draw(cx.painter, &run, 0..n, x, baseline);
        // The cursor: a bar from the ascent to below the baseline.
        let cx0 = (x + run.width).ceil() as i32 + 3;
        let top = (baseline - m.ascent * 0.85).round() as i32;
        let bottom = (baseline + m.descent.abs() * 0.6).round() as i32;
        cx.painter
            .fill_rect(Rect::new(cx0, top, cursor_w, bottom - top), INK);
        cx.painter.threshold(r, THRESHOLD);
    }
}

/// The keyboard in a tree: one action node per key, `first + slot`.
pub struct Keyboard {
    pub root: NodeId,
    /// The [`KeyCap`] of each slot (also its action node).
    caps: Vec<NodeId>,
    pub state: State,
    first: ActionId,
    enter: String,
}

impl Keyboard {
    /// Builds the keyboard as wide as its parent allows. `enter` labels the
    /// enter key; empty shows the return arrow.
    pub fn build(t: &mut Tree, s: &Styles, first: ActionId, enter: &str) -> Keyboard {
        let state = State::default();
        let glyph = TextStyle::new(s.family, crate::geom::pt(12.5));
        let word = TextStyle::new(s.family, crate::geom::pt(8.5)).bold();
        let mut caps = Vec::with_capacity(KEYS);
        let mut rows = Vec::new();
        let mut slot = 0;
        for (r, widths) in ROWS.iter().enumerate() {
            let mut cells = Vec::new();
            if r == 1 {
                cells.push(t.add(Node::new(Kind::Spacer).flex(5), []));
            }
            for &w in widths.iter() {
                let cap = t.add(
                    Node::new(Kind::Custom(Box::new(KeyCap {
                        face: state.face(slot, enter),
                        glyph: glyph.clone(),
                        word: word.clone(),
                    })))
                    .border(0, INK, 12)
                    .action(first + slot as ActionId)
                    .feedback(Feedback::Invert),
                    [],
                );
                caps.push(cap);
                let cell = t.add(
                    Node::new(Kind::Padding(crate::geom::Insets::vh(0, GUTTER / 2))).flex(w as u16),
                    [cap],
                );
                cells.push(cell);
                slot += 1;
            }
            if r == 1 {
                cells.push(t.add(Node::new(Kind::Spacer).flex(5), []));
            }
            rows.push(t.add(Node::row(0), cells));
        }
        let col = t.add(Node::column(GUTTER), rows);
        let root = t.pad(crate::geom::Insets::vh(GUTTER, GUTTER / 2), col);
        Keyboard {
            root,
            caps,
            state,
            first,
            enter: enter.to_string(),
        }
    }

    /// The slot of a keyboard action.
    pub fn slot(&self, id: ActionId) -> Option<usize> {
        (self.first..self.first + KEYS as ActionId)
            .contains(&id)
            .then(|| (id - self.first) as usize)
    }

    /// The action node of `slot`, to find its rectangle.
    pub fn cap(&self, slot: usize) -> NodeId {
        self.caps[slot]
    }

    /// Handles a tap or hold on a key. `None` if `ev` is not for the
    /// keyboard. Keys whose face changed are redrawn.
    pub fn handle(&mut self, t: &mut Tree, ev: &UiEvent) -> Option<Edit> {
        let (id, held) = match *ev {
            UiEvent::Action { id, .. } => (id, false),
            UiEvent::LongAction { id, .. } => (id, true),
            _ => return None,
        };
        let slot = self.slot(id)?;
        let edit = if held {
            self.state.hold(slot)
        } else {
            self.state.tap(slot)
        };
        self.relabel(t);
        Some(edit)
    }

    /// Bring every key's face in line with the state; only keys that
    /// change are invalidated.
    pub fn relabel(&mut self, t: &mut Tree) {
        for slot in 0..KEYS {
            let face = self.state.face(slot, &self.enter);
            let node = self.caps[slot];
            let changed = match t.widget_mut::<KeyCap>(node) {
                Some(k) if k.face != face => {
                    k.face = face;
                    true
                }
                _ => false,
            };
            if changed {
                t.invalidate(node, Refresh::Auto);
            }
        }
    }

    /// Centre of `slot` after layout, for scenarios and tests.
    pub fn centre(&self, t: &Tree, slot: usize) -> Point {
        t.rect(self.caps[slot]).center()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot_of(layer: Layer, key: Key) -> usize {
        keys(layer).iter().position(|k| *k == key).unwrap()
    }

    #[test]
    fn rows_add_up_and_every_layer_fills_them() {
        assert_eq!(ROWS.iter().map(|r| r.len()).sum::<usize>(), KEYS);
        for r in [ROWS[0], ROWS[2], ROWS[3]] {
            assert_eq!(r.iter().map(|&w| w as u32).sum::<u32>(), 100);
        }
        // Row 2 plus its two half-key margins.
        assert_eq!(ROWS[1].iter().map(|&w| w as u32).sum::<u32>() + 10, 100);
        for l in [Layer::Letters, Layer::Symbols, Layer::More] {
            assert_eq!(keys(l).len(), KEYS);
        }
    }

    #[test]
    fn every_printable_ascii_character_can_be_typed() {
        let mut all: Vec<char> = Vec::new();
        for l in [Layer::Letters, Layer::Symbols, Layer::More] {
            for k in keys(l) {
                match k {
                    Key::Char(c) => {
                        all.push(c);
                        all.extend(c.to_uppercase());
                    }
                    Key::Space => all.push(' '),
                    _ => {}
                }
            }
        }
        for c in ' '..='~' {
            assert!(all.contains(&c), "{c:?} is on no layer");
        }
        for c in "ąćęłńóśźżĄĆĘŁŃÓŚŹŻ".chars() {
            let base = "acelnoszx".chars().chain("ACELNOSZX".chars());
            assert!(base.clone().any(|b| polish(b) == Some(c)), "{c}");
        }
    }

    #[test]
    fn no_character_is_on_two_keys_of_a_layer() {
        for l in [Layer::Letters, Layer::Symbols, Layer::More] {
            let mut chars: Vec<char> = keys(l)
                .into_iter()
                .filter_map(|k| match k {
                    Key::Char(c) => Some(c),
                    _ => None,
                })
                .collect();
            let n = chars.len();
            chars.sort();
            chars.dedup();
            assert_eq!(chars.len(), n, "{l:?}");
        }
    }

    #[test]
    fn shift_is_one_shot_then_lock_then_off() {
        let mut s = State::default();
        let shift = slot_of(Layer::Letters, Key::Shift);
        let q = slot_of(Layer::Letters, Key::Char('q'));
        assert_eq!(s.tap(q), Edit::Insert('q'));
        s.tap(shift);
        assert_eq!(s.shift, Shift::Once);
        assert_eq!(s.face(q, "").cap, Cap::Text("Q".into()));
        assert_eq!(s.tap(q), Edit::Insert('Q'));
        assert_eq!(s.tap(q), Edit::Insert('q'), "one letter only");
        s.tap(shift);
        s.tap(shift);
        assert_eq!(s.shift, Shift::Lock);
        assert_eq!(s.tap(q), Edit::Insert('Q'));
        assert_eq!(s.tap(q), Edit::Insert('Q'));
        // Space and symbols keep a one-shot shift for the next letter.
        s.tap(shift);
        assert_eq!(s.shift, Shift::Off);
        s.tap(shift);
        assert_eq!(
            s.tap(slot_of(Layer::Letters, Key::Space)),
            Edit::Insert(' ')
        );
        assert_eq!(s.shift, Shift::Once);
    }

    #[test]
    fn polish_layer_for_one_letter() {
        let mut s = State::default();
        let pl = slot_of(Layer::Letters, Key::Polish);
        let a = slot_of(Layer::Letters, Key::Char('a'));
        let x = slot_of(Layer::Letters, Key::Char('x'));
        let q = slot_of(Layer::Letters, Key::Char('q'));
        s.tap(pl);
        assert!(s.face(pl, "").active);
        assert_eq!(s.face(a, "").cap, Cap::Text("ą".into()));
        assert_eq!(s.face(q, "").cap, Cap::Text("q".into()));
        assert_eq!(s.tap(x), Edit::Insert('ź'));
        assert!(!s.polish);
        assert_eq!(s.tap(a), Edit::Insert('a'));
        // With shift: Ą. A letter without a Polish form also ends the layer.
        s.tap(pl);
        s.tap(slot_of(Layer::Letters, Key::Shift));
        assert_eq!(s.tap(a), Edit::Insert('Ą'));
        s.tap(pl);
        assert_eq!(s.tap(q), Edit::Insert('q'));
        assert!(!s.polish);
        // Tapping ąę twice cancels it.
        s.tap(pl);
        s.tap(pl);
        assert_eq!(s.tap(a), Edit::Insert('a'));
    }

    #[test]
    fn holding_types_the_polish_letter() {
        let mut s = State::default();
        let z = slot_of(Layer::Letters, Key::Char('z'));
        let q = slot_of(Layer::Letters, Key::Char('q'));
        let bs = slot_of(Layer::Letters, Key::Backspace);
        assert_eq!(s.hold(z), Edit::Insert('ż'));
        assert_eq!(s.hold(q), Edit::Insert('q'));
        s.tap(slot_of(Layer::Letters, Key::Shift));
        assert_eq!(
            s.hold(slot_of(Layer::Letters, Key::Char('l'))),
            Edit::Insert('Ł')
        );
        assert_eq!(s.shift, Shift::Off);
        assert_eq!(s.hold(bs), Edit::DeleteWord);
    }

    #[test]
    fn layers_switch_and_relabel() {
        let mut s = State::default();
        let to_sym = slot_of(Layer::Letters, Key::ToSymbols);
        assert_eq!(s.tap(to_sym), Edit::None);
        assert_eq!(s.layer, Layer::Symbols);
        assert_eq!(s.face(0, "").cap, Cap::Text("1".into()));
        let more = slot_of(Layer::Symbols, Key::ToMore);
        s.tap(more);
        assert_eq!(s.layer, Layer::More);
        assert_eq!(s.tap(0), Edit::Insert('['));
        // Shift does not apply to symbols.
        s.shift = Shift::Lock;
        assert_eq!(
            s.tap(slot_of(Layer::More, Key::Char('\\'))),
            Edit::Insert('\\')
        );
        s.tap(slot_of(Layer::More, Key::ToLetters));
        assert_eq!(s.layer, Layer::Letters);
        // The enter key shows the caller's word, or an arrow.
        let enter = slot_of(Layer::Letters, Key::Enter);
        assert_eq!(s.face(enter, "Connect").cap, Cap::Text("Connect".into()));
        assert_eq!(s.face(enter, "").cap, Cap::Icon(Icon::Return));
    }

    #[test]
    fn edits() {
        let mut t = String::from("Litwo ojczyzno");
        assert!(apply(&mut t, Edit::Insert('!'), 100));
        assert_eq!(t, "Litwo ojczyzno!");
        assert!(apply(&mut t, Edit::Backspace, 100));
        assert!(apply(&mut t, Edit::DeleteWord, 100));
        assert_eq!(t, "Litwo ");
        assert!(apply(&mut t, Edit::DeleteWord, 100));
        assert_eq!(t, "");
        assert!(!apply(&mut t, Edit::Backspace, 100));
        assert!(!apply(&mut t, Edit::DeleteWord, 100));
        let mut t = String::from("ąę");
        assert!(!apply(&mut t, Edit::Insert('x'), 2), "at the limit");
        assert!(apply(&mut t, Edit::Backspace, 2));
        assert_eq!(t, "ą");
    }

    #[test]
    fn long_text_keeps_its_end_visible() {
        let mut e = TextEngine::new(crate::text::Fonts::embedded());
        let style = TextStyle::new(crate::text::FamilyId(0), 40.0);
        let s = "Litwo! Ojczyzno moja! ty jesteś jak zdrowie";
        let full = e.measure(s, &style);
        assert_eq!(fit_tail(&mut e, s, &style, full + 1.0), s);
        let cut = fit_tail(&mut e, s, &style, full / 2.0);
        assert!(cut.starts_with('…') && s.ends_with(&cut['…'.len_utf8()..]));
        assert!(e.measure(&cut, &style) <= full / 2.0);
        assert!(
            e.covers("•…", &style),
            "Literata has the dot and the ellipsis"
        );
    }
}
