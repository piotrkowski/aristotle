// SPDX-License-Identifier: AGPL-3.0-or-later
//! Text input: a title, an optional hint, one text field and the
//! on-screen keyboard (`crate::keyboard`). Any feature that needs a line of
//! text (a Wi-Fi password, a server URL, a user name) builds this screen
//! from a [`Spec`] and feeds it the screen's [`UiEvent`]s until
//! [`TextInput::handle`] returns [`Outcome::Submit`] or
//! [`Outcome::Cancel`]. `docs/ui-framework.md`, "Text input", shows the
//! app side.
//!
//! Every key acknowledges on touch-down with an A2 inversion of that key
//! only; the character and the restored key go out together as one DU
//! update of the pixels that changed. Nothing on this screen flashes after
//! it opened.

use super::library::{status_line, StatusLine};
use super::{icon_button, text_button};
use crate::geom::Insets;
use crate::icons::Icon;
use crate::keyboard::{apply, Edit, Keyboard, TextField};
use crate::text::{Align, Paragraph, TextEngine, TextStyle};
use crate::theme::{margin, IconSize, Space, Styles};
use crate::tree::{ActionId, Node, NodeId, Tree};
use crate::ui::UiEvent;
use aristotle_i18n::{tr, trn, Msg, Plural};

pub mod action {
    use crate::tree::ActionId;
    /// The back arrow: cancel.
    pub const BACK: ActionId = 1;
    /// "Show"/"Hide" next to a password.
    pub const REVEAL: ActionId = 2;
    /// `KEY + slot` for each key of the keyboard.
    pub const KEY: ActionId = 100;
}

/// A check run on submit. `Err` keeps the keyboard open and shows the
/// message under the field.
pub type Validate = fn(&str) -> Result<(), String>;

/// What to ask for.
pub struct Spec<'a> {
    pub status: StatusLine<'a>,
    /// The screen's title, e.g. "Password for Home". Two lines at most.
    pub title: &'a str,
    /// A line under the title, e.g. where the text goes.
    pub hint: Option<&'a str>,
    /// The text the field starts with.
    pub initial: &'a str,
    /// Dots instead of characters, with a "Show" button.
    pub password: bool,
    /// The word on the enter key ("Connect", "Save"); empty for an arrow.
    pub submit: &'a str,
    /// At most this many characters.
    pub max_chars: usize,
    pub validate: Option<Validate>,
}

impl Spec<'_> {
    /// A plain text spec with the defaults: no hint, empty, "Done",
    /// 256 characters, no check.
    pub fn new<'a>(status: StatusLine<'a>, title: &'a str) -> Spec<'a> {
        Spec {
            status,
            title,
            hint: None,
            initial: "",
            password: false,
            submit: tr(Msg::KeyboardDone),
            max_chars: 256,
            validate: None,
        }
    }
}

/// What a [`TextInput::handle`] call ended in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Still typing; render the tree.
    Editing,
    /// Enter, with text that passed the check.
    Submit(String),
    /// The back arrow.
    Cancel,
}

/// The screen's state and the nodes it updates in place.
pub struct TextInput {
    pub field: NodeId,
    /// The message line under the field.
    pub message: NodeId,
    /// The "Show"/"Hide" label, for passwords.
    pub reveal: Option<NodeId>,
    pub keyboard: Keyboard,
    /// The busy mark in the status line, hidden.
    pub busy: NodeId,
    max_chars: usize,
    validate: Option<Validate>,
    has_message: bool,
}

impl TextInput {
    pub fn build(_text: &mut TextEngine, s: &Styles, spec: &Spec) -> (Tree, TextInput) {
        let mut t = Tree::new();
        let (status, busy) = status_line(&mut t, s, &spec.status);
        let back = icon_button(&mut t, Icon::ArrowLeft, IconSize::STANDARD, action::BACK);
        let mut title = Paragraph::plain(spec.title, s.headline(), Align::Left);
        title.style.max_lines = Some(2);
        let title = t.text(title);
        let title = t.add(Node::column(0).flex(1), [title]);
        let g = t.gap(Space::XS, 1);
        let header = t.add(Node::row(0), [back, g, title]);
        let header = t.pad(Insets::new(0, 0, 0, -26), header);
        let mut top = vec![status, header];
        if let Some(h) = spec.hint {
            let mut p = Paragraph::plain(h, s.small(), Align::Left);
            p.style.max_lines = Some(3);
            let p = t.text(p);
            top.push(t.pad(Insets::new(Space::XS, 0, 0, 0), p));
        }
        let field = t.custom(TextField {
            text: spec.initial.chars().take(spec.max_chars).collect(),
            password: spec.password,
            reveal: false,
            style: TextStyle::new(s.family, crate::geom::pt(12.0)),
        });
        let field_box = t.add(Node::column(0).flex(1), [field]);
        let mut row = vec![field_box];
        let mut reveal = None;
        if spec.password {
            let b = text_button(&mut t, tr(Msg::KeyboardShow), s.ui_bold(), action::REVEAL);
            // button → padding → label
            let label = t.node(t.node(b).children[0]).children[0];
            reveal = Some(label);
            row.push(b);
        }
        let row = t.add(Node::row(Space::S), row);
        top.push(t.pad(Insets::new(Space::L, 0, 0, 0), row));
        let mut msg = Paragraph::plain("", s.ui(), Align::Left);
        msg.style.max_lines = Some(2);
        let message = t.text(msg);
        let message = t.add(Node::column(0), [message]);
        let line = s.ui().size * s.ui().line_height;
        let message_box = t.fixed(None, Some((2.0 * line).ceil() as i32), Some(message));
        top.push(t.pad(Insets::new(Space::S, Space::XS, 0, Space::XS), message_box));
        let top = t.add(Node::column(0), top);
        let top = t.pad(Insets::new(0, margin(), 0, margin()), top);
        let fill = t.spacer(1);
        let keyboard = Keyboard::build(&mut t, s, action::KEY, spec.submit);
        let kb = t.add(Node::column(0), [keyboard.root]);
        let kb = t.pad(Insets::new(0, 0, crate::geom::mm(1.0), 0), kb);
        let col = t.add(Node::column(0), [top, fill, kb]);
        t.set_root(col);
        let message = t.node(message).children[0];
        (
            t,
            TextInput {
                field,
                message,
                reveal,
                keyboard,
                busy,
                max_chars: spec.max_chars,
                validate: spec.validate,
                has_message: false,
            },
        )
    }

    /// The text typed so far.
    pub fn value<'t>(&self, t: &'t Tree) -> &'t str {
        t.widget::<TextField>(self.field)
            .map_or("", |f| f.text.as_str())
    }

    /// Shows `msg` under the field, or clears it.
    pub fn set_message(&mut self, t: &mut Tree, msg: Option<&str>) {
        if msg.is_none() && !self.has_message {
            return;
        }
        self.has_message = msg.is_some();
        t.set_text(self.message, msg.unwrap_or(""));
    }

    /// Handles one event of the screen. Call `ui.render()` afterwards, once.
    pub fn handle(&mut self, t: &mut Tree, ev: &UiEvent) -> Outcome {
        if let UiEvent::Action { id, .. } = ev {
            match *id {
                action::BACK => return Outcome::Cancel,
                action::REVEAL => {
                    self.toggle_reveal(t);
                    return Outcome::Editing;
                }
                _ => {}
            }
        }
        let Some(edit) = self.keyboard.handle(t, ev) else {
            return Outcome::Editing;
        };
        match edit {
            Edit::Enter => {
                let text = self.value(t).to_string();
                if let Some(check) = self.validate {
                    if let Err(msg) = check(&text) {
                        self.set_message(t, Some(&msg));
                        return Outcome::Editing;
                    }
                }
                Outcome::Submit(text)
            }
            Edit::None => Outcome::Editing,
            edit => {
                let max = self.max_chars;
                let changed = t
                    .widget_mut::<TextField>(self.field)
                    .is_some_and(|f| apply(&mut f.text, edit, max));
                if changed {
                    t.invalidate(self.field, crate::tree::Refresh::Auto);
                    self.set_message(t, None);
                } else if matches!(edit, Edit::Insert(_)) {
                    let msg = trn(Plural::KeyboardTooLong, max);
                    self.set_message(t, Some(&msg));
                }
                Outcome::Editing
            }
        }
    }

    fn toggle_reveal(&mut self, t: &mut Tree) {
        let Some(label) = self.reveal else { return };
        let shown = t.widget_mut::<TextField>(self.field).map(|f| {
            f.reveal = !f.reveal;
            f.reveal
        });
        t.invalidate(self.field, crate::tree::Refresh::Auto);
        let word = if shown == Some(true) {
            Msg::KeyboardHide
        } else {
            Msg::KeyboardShow
        };
        t.set_text(label, tr(word));
    }

    /// The slot of the key at `id`, for tests and scenarios.
    pub fn key_action(&self, slot: usize) -> ActionId {
        action::KEY + slot as ActionId
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{MIN_TARGET, SCREEN_HEIGHT, SCREEN_WIDTH};
    use crate::input::{Phase, TouchEvent};
    use crate::keyboard::{keys, Key, Layer};
    use crate::text::{FamilyId, Fonts};
    use crate::ui::Ui;
    use crate::Waveform;

    const STATUS: StatusLine<'static> = StatusLine {
        clock: "21:38",
        battery: Some(0.8),
        charging: false,
    };

    fn screen(spec: &Spec) -> (Ui, TextInput) {
        let mut text = TextEngine::new(Fonts::embedded());
        let (tree, input) = TextInput::build(&mut text, &Styles::new(FamilyId(0)), spec);
        let mut ui = Ui::new(tree, text, SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
        ui.layout();
        ui.render();
        (ui, input)
    }

    fn slot(c: char) -> usize {
        keys(Layer::Letters)
            .iter()
            .position(|k| *k == Key::Char(c))
            .unwrap()
    }

    fn type_key(ui: &mut Ui, input: &mut TextInput, slot: usize, at_ms: u64) -> Outcome {
        let p = input.keyboard.centre(&ui.tree, slot);
        let mut out = Outcome::Editing;
        for (phase, ms) in [(Phase::Down, at_ms), (Phase::Up, at_ms + 60)] {
            for ev in ui.handle_touch(TouchEvent {
                phase,
                id: 0,
                x: p.x,
                y: p.y,
                time_us: ms * 1000,
            }) {
                out = input.handle(&mut ui.tree, &ev);
            }
            if phase == Phase::Down {
                ui.render();
            }
        }
        out
    }

    fn check_len(s: &str) -> Result<(), String> {
        if s.len() >= 3 {
            Ok(())
        } else {
            Err("too short".into())
        }
    }

    #[test]
    fn typing_submits_the_text() {
        let spec = Spec {
            validate: Some(check_len),
            ..Spec::new(STATUS, "Name")
        };
        let (mut ui, mut input) = screen(&spec);
        let enter = keys(Layer::Letters)
            .iter()
            .position(|k| *k == Key::Enter)
            .unwrap();
        type_key(&mut ui, &mut input, slot('o'), 0);
        type_key(&mut ui, &mut input, slot('k'), 1000);
        ui.render();
        assert_eq!(input.value(&ui.tree), "ok");
        assert_eq!(type_key(&mut ui, &mut input, enter, 2000), Outcome::Editing);
        assert_eq!(
            input.value(&ui.tree),
            "ok",
            "the check failed: still typing"
        );
        type_key(&mut ui, &mut input, slot('o'), 3000);
        assert_eq!(
            type_key(&mut ui, &mut input, enter, 4000),
            Outcome::Submit("oko".into())
        );
    }

    #[test]
    fn a_key_is_a2_on_the_key_then_one_small_du_update() {
        let spec = Spec {
            password: true,
            ..Spec::new(STATUS, "Password for Soplicowo")
        };
        let (mut ui, mut input) = screen(&spec);
        for (i, c) in "tadeusz".chars().enumerate() {
            let s = slot(c);
            let key = ui.tree.rect(input.keyboard.cap(s));
            let p = key.center();
            let t0 = i as u64 * 1000;
            let down = TouchEvent {
                phase: Phase::Down,
                id: 0,
                x: p.x,
                y: p.y,
                time_us: t0 * 1000,
            };
            assert!(ui.handle_touch(down).is_empty());
            let d = ui.render();
            assert_eq!(d.len(), 1, "{c}: {d:?}");
            assert_eq!(d[0].waveform, Waveform::A2);
            assert!(key.intersect(&d[0].rect) == Some(d[0].rect), "only the key");
            let up = TouchEvent {
                phase: Phase::Up,
                time_us: (t0 + 60) * 1000,
                ..down
            };
            for ev in ui.handle_touch(up) {
                input.handle(&mut ui.tree, &ev);
            }
            let d = ui.render();
            assert_eq!(d.len(), 1, "{c}: {d:?}");
            assert_eq!(d[0].waveform, Waveform::DU, "{c}: no grays, no flash");
            // Minimal damage: the key and a dot's worth of the field.
            let field = ui.tree.rect(input.field);
            for m in ui.minimal_damage() {
                assert!(
                    m.rect == key || field.intersect(&m.rect) == Some(m.rect),
                    "{c}: {m:?}"
                );
                if field.intersects(&m.rect) {
                    assert!(m.rect.w < 120 && m.rect.h < field.h, "{c}: {m:?}");
                }
            }
        }
        assert_eq!(input.value(&ui.tree), "tadeusz");
    }

    #[test]
    fn shift_relabels_only_letters_and_reveal_shows_the_text() {
        let spec = Spec {
            password: true,
            ..Spec::new(STATUS, "Password")
        };
        let (mut ui, mut input) = screen(&spec);
        let shift = keys(Layer::Letters)
            .iter()
            .position(|k| *k == Key::Shift)
            .unwrap();
        type_key(&mut ui, &mut input, shift, 0);
        let d = ui.render();
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].waveform, Waveform::DU);
        type_key(&mut ui, &mut input, slot('j'), 1000);
        ui.render();
        assert_eq!(input.value(&ui.tree), "J");
        let field = ui.tree.widget::<TextField>(input.field).unwrap();
        assert_eq!(field.shown(), "•");
        let ev = UiEvent::Action {
            id: action::REVEAL,
            node: input.field,
            at: crate::geom::Point::default(),
        };
        input.handle(&mut ui.tree, &ev);
        assert_eq!(
            ui.tree.widget::<TextField>(input.field).unwrap().shown(),
            "J"
        );
        let d = ui.render();
        assert_eq!(d.len(), 1, "field and label in one update");
    }

    #[test]
    fn back_cancels_and_the_limit_is_kept() {
        let spec = Spec {
            max_chars: 2,
            initial: "abc",
            ..Spec::new(STATUS, "Code")
        };
        let (mut ui, mut input) = screen(&spec);
        assert_eq!(input.value(&ui.tree), "ab");
        type_key(&mut ui, &mut input, slot('q'), 0);
        assert_eq!(input.value(&ui.tree), "ab");
        let ev = UiEvent::Action {
            id: action::BACK,
            node: input.field,
            at: crate::geom::Point::default(),
        };
        assert_eq!(input.handle(&mut ui.tree, &ev), Outcome::Cancel);
    }

    #[test]
    fn keys_are_targets_and_fit_the_screen() {
        let (mut ui, input) = screen(&Spec::new(STATUS, "Title"));
        ui.layout();
        for s in 0..crate::keyboard::KEYS {
            let r = ui.tree.rect(input.keyboard.cap(s));
            assert!(r.h >= MIN_TARGET, "{s}: {r:?}");
            assert!(r.w >= 96, "{s}: {r:?}");
            assert!(r.x >= 0 && r.right() <= SCREEN_WIDTH && r.bottom() <= SCREEN_HEIGHT);
            assert_eq!(
                ui.tree.hit(r.center()),
                crate::tree::Hit::Target(input.keyboard.cap(s))
            );
        }
        let field = ui.tree.rect(input.field);
        let first_key = ui.tree.rect(input.keyboard.cap(0));
        assert!(field.bottom() < first_key.y);
    }
}
