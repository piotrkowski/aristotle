// SPDX-License-Identifier: AGPL-3.0-or-later
//! Asking the reader for a line of text: the app side of
//! `aristotle_ui::screens::text_input`.
//!
//! ```ignore
//! self.ask_text(TextRequest {
//!     password: true,
//!     submit: tr(Msg::WifiConnect).into(),
//!     validate: Some(wifi::check_password),
//!     ..TextRequest::new(title, Box::new(move |app, text| match text {
//!         Some(psk) => app.wifi_connect(net, Some(psk)),
//!         None => app.show_wifi(Refresh::Full),
//!     }))
//! })?;
//! ```
//!
//! The keyboard replaces the current screen with one flash. Typing is
//! handled here; enter (with text that passed `validate`) or the back arrow
//! calls `done` once, with the text or `None`. `done` must show the next
//! screen. The screensaver cancels the request: a half-typed password is
//! not kept across it.

use aristotle_i18n::{tr, Msg};
use aristotle_ui::screens::text_input::{Outcome, Spec, TextInput, Validate};
use aristotle_ui::tree::Refresh;
use aristotle_ui::ui::UiEvent;

use crate::device::Facts;
use crate::{App, Error, Flow, Screen};

/// What happens with the text: `Some` after enter, `None` after the back
/// arrow or the screensaver. Shows the next screen.
pub type TextDone = Box<dyn FnOnce(&mut App, Option<String>) -> Result<(), Error>>;

/// A request for one line of text.
pub struct TextRequest {
    pub title: String,
    /// A line under the title.
    pub hint: Option<String>,
    pub initial: String,
    /// Dots instead of characters, with "Show".
    pub password: bool,
    /// The word on the enter key; empty shows an arrow.
    pub submit: String,
    pub max_chars: usize,
    /// Checked on enter; an `Err` message keeps the keyboard open.
    pub validate: Option<Validate>,
    pub done: TextDone,
}

impl TextRequest {
    /// Plain text: no hint, empty, "Done", 256 characters, no check.
    pub fn new(title: impl Into<String>, done: TextDone) -> TextRequest {
        TextRequest {
            title: title.into(),
            hint: None,
            initial: String::new(),
            password: false,
            submit: tr(Msg::KeyboardDone).into(),
            max_chars: 256,
            validate: None,
            done,
        }
    }
}

impl App {
    /// Show the keyboard for `req` (one flash).
    pub(crate) fn ask_text(&mut self, req: TextRequest) -> Result<(), Error> {
        let facts = Facts::status(&self.battery);
        self.clock = facts.clock.clone();
        let spec = Spec {
            status: facts.status_line(),
            title: &req.title,
            hint: req.hint.as_deref(),
            initial: &req.initial,
            password: req.password,
            submit: &req.submit,
            max_chars: req.max_chars,
            validate: req.validate,
        };
        let (tree, input) = TextInput::build(&mut self.ui.text, &self.styles, &spec);
        self.ui.set_tree_with(tree, Refresh::Full);
        self.ui.gesture_config_mut().long_press_us = 500_000;
        self.screen = Screen::TextInput {
            input: Box::new(input),
            done: Some(req.done),
        };
        self.render_ui()
    }

    pub(crate) fn on_text_event(&mut self, ev: UiEvent) -> Result<Flow, Error> {
        let Screen::TextInput { input, done } = &mut self.screen else {
            return Ok(Flow::Continue);
        };
        let text = match input.handle(&mut self.ui.tree, &ev) {
            Outcome::Editing => return Ok(Flow::Continue),
            Outcome::Submit(s) => Some(s),
            Outcome::Cancel => None,
        };
        if let Some(done) = done.take() {
            done(self, text)?;
        }
        Ok(Flow::Continue)
    }

    /// The screensaver came while the keyboard was up: give up the request.
    pub(crate) fn cancel_text(&mut self) -> Result<(), Error> {
        if let Screen::TextInput { done, .. } = &mut self.screen {
            if let Some(done) = done.take() {
                return done(self, None);
            }
        }
        Ok(())
    }
}
