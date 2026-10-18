// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reading-position sync with KOReader's protocol (KOSync), and its
//! settings screen.
//!
//! - **Open**: the position the server has is fetched on a worker. If
//!   another device stored it after this book was last read here, and it is
//!   somewhere else, a popup asks before going there. The page is never
//!   moved without asking, backwards or forwards.
//! - **Close** (back to the library, the screensaver, quitting): the
//!   position is sent, if the reader moved since the book opened and if
//!   KOReader can use it (`aristotle_library::kosync`: EPUB chapters and
//!   PDF/CBZ pages; nothing for MOBI, AZW3 and TXT).
//! - Pulled EPUB positions go to the start of the chapter KOReader names,
//!   then as far into it as KOReader's percentage says, if that falls inside
//!   the chapter ([`within_chapter`]). Element paths after
//!   `DocFragment[N]` are not resolved yet.
//!
//! Failures never interrupt reading: they are logged and shown on the sync
//! screen.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use aristotle_i18n::{tf, tr, Msg};
use aristotle_library::kosync::{self as lk, Fragments};
use aristotle_library::{Format, Locator, Progress};
use aristotle_net::kosync::{self, Kosync, Position};
use aristotle_net::store::SyncField;
use aristotle_render::book::Position as BookPosition;
use aristotle_render::plato_core::anyhow::Error;
use aristotle_ui::screens::network::{self as net_ui, action as na, SyncView};
use aristotle_ui::tree::Refresh;

use crate::net::{describe_sync, Done, Net, TextField, TextRequest};
use crate::reading::Overlay;
use crate::{perf, App, Screen};

/// What we call ourselves on the server (`device`).
pub const DEVICE: &str = "Aristotle";

/// Where a pulled position points, worked out on the worker (the EPUB's
/// fragment map needs its OPF).
#[derive(Clone, Debug, PartialEq)]
pub enum Remote {
    /// Our spine item for KOReader's `DocFragment[N]`.
    Chapter(usize),
    /// A 0-based page of a PDF or CBZ.
    Page(usize),
    /// Only the percentage means anything here.
    Fraction,
}

/// The book open now, as it was when it opened.
pub struct Opened {
    pub id: String,
    pub token: u64,
    pub format: Format,
    pub path: PathBuf,
    /// The initial locator or the last successfully sent locator.
    pub at_open: Option<Locator>,
    acknowledged: u64,
    pending: BTreeMap<u64, Locator>,
}

impl Opened {
    fn needs_push(&self, locator: &Locator) -> bool {
        self.at_open.as_ref() != Some(locator) || self.pending.values().any(|p| p != locator)
    }
}

pub struct PushReceipt {
    pub book: String,
    pub token: u64,
    pub sequence: u64,
    pub locator: Locator,
}

pub enum PushOutcome {
    Unsupported,
    Superseded,
    Finished(Result<(), kosync::Error>),
}

type PushKey = (String, String, String);

#[derive(Clone, Default)]
struct PushGate(Arc<Mutex<BTreeMap<PushKey, u64>>>);

impl PushGate {
    fn run(
        &self,
        key: PushKey,
        sequence: u64,
        send: impl FnOnce() -> Result<(), kosync::Error>,
    ) -> PushOutcome {
        let mut latest = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let previous = latest.entry(key).or_default();
        if sequence <= *previous {
            return PushOutcome::Superseded;
        }
        *previous = sequence;
        PushOutcome::Finished(send())
    }
}

#[derive(Default)]
pub struct SyncState {
    pub opened: Option<Opened>,
    next_token: u64,
    next_push: u64,
    last_push_result: u64,
    push_gate: PushGate,
    /// The progress a book had before this open touched it: what a pulled
    /// position is compared with.
    pub before_open: Option<(String, Option<Progress>)>,
    /// A position to offer once nothing covers the page: book, target
    /// location, the question.
    pub offer: Option<(String, usize, String)>,
    /// What happened last, for the sync screen.
    pub message: String,
    /// A login or registration is running.
    pub checking: bool,
}

/// Whether a pulled position is worth a question: another device's, and
/// somewhere else. Timestamps are not compared: the server stamps with its
/// clock and the device's own may be off. They need not be: a book that
/// moved here is pushed when it closes, so the server keeps another
/// device's record only if that device wrote after our last push.
pub fn worth_asking(remote: &Position, device_id: &str, local: Option<&Progress>) -> bool {
    if remote.device_id == device_id && remote.device == DEVICE {
        return false;
    }
    match local {
        Some(l) => (remote.percentage - l.fraction as f64).abs() >= 0.001,
        None => remote.percentage > 0.0,
    }
}

/// The location `fraction` of the book falls on inside a chapter that
/// spans locations `start..end` and fractions `f0..f1`; the chapter start
/// when the fraction lies outside it (KOReader's percentage and ours are
/// measured differently, so it is only a hint).
pub fn within_chapter(start: usize, end: usize, f0: f32, f1: f32, fraction: f32) -> usize {
    if end <= start || f1 <= f0 || fraction < f0 || fraction >= f1 {
        return start;
    }
    let share = (fraction as f64 - f0 as f64) / (f1 as f64 - f0 as f64);
    (start + (share * (end - start) as f64).round() as usize).min(end - 1)
}

/// Where a pulled `progress` points in `format`, given the book file.
fn remote_target(format: Format, path: &std::path::Path, progress: &str) -> Remote {
    match (format, lk::parse_remote(progress)) {
        (Format::Epub, lk::Remote::Fragment(n)) => Fragments::read(path)
            .ok()
            .and_then(|f| f.item(n))
            .map_or(Remote::Fraction, Remote::Chapter),
        (Format::Pdf | Format::Cbz, lk::Remote::Page(i)) => Remote::Page(i as usize),
        _ => Remote::Fraction,
    }
}

/// The `progress` KOReader can use for a book at `locator`, if any.
fn push_progress(format: Format, path: &std::path::Path, locator: &Locator) -> Option<String> {
    let fragments = (format == Format::Epub)
        .then(|| Fragments::read(path).ok())
        .flatten();
    lk::progress(format, locator, fragments.as_ref())
}

/// What a push needs, owned for the worker.
struct Push {
    account: kosync::Account,
    receipt: PushReceipt,
    gate: PushGate,
    format: Format,
    path: PathBuf,
    fraction: f32,
    device_id: String,
}

impl Push {
    fn run(self) -> Done {
        let Some(progress) = push_progress(self.format, &self.path, &self.receipt.locator) else {
            return Done::Pushed {
                receipt: self.receipt,
                outcome: PushOutcome::Unsupported,
            };
        };
        let p = Position {
            document: lk::document(&self.receipt.book).to_string(),
            progress,
            percentage: kosync::percentage(self.fraction),
            device: DEVICE.into(),
            device_id: self.device_id,
            timestamp: None,
        };
        let key = (
            self.account.server.clone(),
            self.account.username.clone(),
            p.document.clone(),
        );
        let outcome = self.gate.run(key, self.receipt.sequence, || {
            Kosync::new(Net::policy())
                .push(&self.account, &p)
                .map(|_| ())
        });
        perf!(
            "sync: push {} {:?} {:.4}: {}",
            p.document,
            p.progress,
            p.percentage,
            match &outcome {
                PushOutcome::Finished(Ok(())) => "ok".to_string(),
                PushOutcome::Finished(Err(e)) => e.to_string(),
                PushOutcome::Superseded => "superseded".to_string(),
                PushOutcome::Unsupported => unreachable!(),
            }
        );
        Done::Pushed {
            receipt: self.receipt,
            outcome,
        }
    }
}

impl App {
    pub(crate) fn on_pushed(&mut self, receipt: PushReceipt, outcome: PushOutcome) {
        let opened = self
            .sync
            .opened
            .as_mut()
            .filter(|o| o.id == receipt.book && o.token == receipt.token);
        if let Some(o) = opened {
            o.pending.remove(&receipt.sequence);
            if matches!(outcome, PushOutcome::Finished(Ok(()))) && receipt.sequence > o.acknowledged
            {
                o.at_open = Some(receipt.locator);
                o.acknowledged = receipt.sequence;
            }
        }
        if matches!(outcome, PushOutcome::Superseded) {
            return;
        }
        if receipt.sequence <= self.sync.last_push_result {
            return;
        }
        self.sync.last_push_result = receipt.sequence;
        self.sync.message = match outcome {
            PushOutcome::Unsupported => tr(Msg::SyncStateNotSent).to_string(),
            PushOutcome::Finished(Ok(())) => tr(Msg::SyncStateSent).to_string(),
            PushOutcome::Finished(Err(e)) => describe_sync(&e),
            PushOutcome::Superseded => unreachable!(),
        };
    }

    fn sync_account(&self) -> Option<kosync::Account> {
        let s = self.net.accounts.sync.as_ref()?;
        s.auto.then(|| s.account()).flatten()
    }

    /// Before a book opens: keep its progress as it was, for comparing
    /// with the server's.
    pub(crate) fn sync_before_open(&mut self, id: &str) {
        let p = self.lib.book(id).and_then(|b| b.progress.clone());
        self.sync.before_open = Some((id.to_string(), p));
    }

    /// A book is open (its session exists): remember where it started and
    /// ask the server where it is elsewhere.
    pub(crate) fn sync_book_opened(&mut self) {
        let Screen::Reader(r) = &self.screen else {
            return;
        };
        let id = r.session.id.clone();
        if self.sync.opened.as_ref().is_some_and(|o| o.id == id) {
            return;
        }
        let Some(rec) = self.lib.book(&id) else {
            return;
        };
        let Some(path) = self.lib.absolute_path(rec) else {
            return;
        };
        let format = rec.format;
        let at_open = rec.progress.as_ref().map(|p| p.locator.clone());
        self.sync.next_token = self.sync.next_token.wrapping_add(1);
        let token = self.sync.next_token;
        self.sync.opened = Some(Opened {
            id: id.clone(),
            token,
            format,
            path: path.clone(),
            at_open,
            acknowledged: 0,
            pending: BTreeMap::new(),
        });
        self.sync.offer = None;
        let Some(account) = self.sync_account() else {
            return;
        };
        let doc = lk::document(&id).to_string();
        let t0 = Instant::now();
        self.net.spawn("pull", move |_| {
            let result = Kosync::new(Net::policy()).pull(&account, &doc).map(|p| {
                p.map(|p| {
                    let target = remote_target(format, &path, &p.progress);
                    (p, target)
                })
            });
            perf!(
                "sync: pull {doc} in {:.0} ms: {}",
                crate::ms(t0.elapsed()),
                match &result {
                    Ok(Some((p, t))) => format!(
                        "{:?} {:.4} from {} ({t:?})",
                        p.progress, p.percentage, p.device
                    ),
                    Ok(None) => "nothing stored".into(),
                    Err(e) => e.to_string(),
                }
            );
            Done::Pulled {
                book: id,
                token,
                result,
            }
        });
    }

    /// Send changed progress; only successful completion advances the baseline.
    /// `closing` forgets the book; `wait` sends on this thread when quitting.
    pub(crate) fn sync_push(&mut self, closing: bool, wait: bool) {
        let Some(account) = self.sync_account() else {
            if closing {
                self.sync.opened = None;
            }
            return;
        };
        let Some(o) = self.sync.opened.as_mut() else {
            return;
        };
        let Some(p) = self.lib.book(&o.id).and_then(|b| b.progress.clone()) else {
            if closing {
                self.sync.opened = None;
            }
            return;
        };
        let moved = o.needs_push(&p.locator);
        self.sync.next_push += 1;
        if moved {
            o.pending.insert(self.sync.next_push, p.locator.clone());
        }
        let push = Push {
            account,
            receipt: PushReceipt {
                book: o.id.clone(),
                token: o.token,
                sequence: self.sync.next_push,
                locator: p.locator,
            },
            gate: self.sync.push_gate.clone(),
            format: o.format,
            path: o.path.clone(),
            fraction: p.fraction,
            device_id: self.net.accounts.device_id(),
        };
        if closing {
            self.sync.opened = None;
        }
        if !moved {
            return;
        }
        if wait {
            let _ = push.run();
        } else {
            self.net.spawn("push", move |_| push.run());
        }
    }

    /// A pulled position arrived: offer it if it is worth a question and
    /// the book is still open.
    pub(crate) fn on_pulled(
        &mut self,
        book: String,
        token: u64,
        result: Result<Option<(Position, Remote)>, kosync::Error>,
    ) -> Result<(), Error> {
        if !self
            .sync
            .opened
            .as_ref()
            .is_some_and(|o| o.id == book && o.token == token)
        {
            return Ok(());
        }
        let (remote, target) = match result {
            Ok(Some(x)) => x,
            Ok(None) => return Ok(()),
            Err(e) => {
                self.sync.message = describe_sync(&e);
                return Ok(());
            }
        };
        let device_id = self.net.accounts.device_id();
        let local = match &self.sync.before_open {
            Some((id, p)) if *id == book => p.clone(),
            _ => None,
        };
        if !worth_asking(&remote, &device_id, local.as_ref()) {
            return Ok(());
        }
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        if r.session.id != book {
            return Ok(());
        }
        let fraction = remote.percentage as f32;
        let current = r.session.current;
        let book_ref = r.session.book();
        let location = match target {
            Remote::Chapter(item) => {
                let start = book_ref.location_of(&BookPosition::Spine { item, offset: 0 });
                let end = book_ref
                    .location_of(&BookPosition::Spine {
                        item: item + 1,
                        offset: 0,
                    })
                    .unwrap_or(book_ref.len());
                start.map(|s| {
                    within_chapter(
                        s,
                        end,
                        book_ref.fraction(s),
                        book_ref.fraction(end),
                        fraction,
                    )
                })
            }
            Remote::Page(i) => book_ref.location_of(&BookPosition::Page(i)),
            Remote::Fraction => book_ref.location_of(&BookPosition::Fraction(fraction.min(0.9999))),
        };
        let Some(page) = location.and_then(|l| book_ref.page_start(l)) else {
            return Ok(());
        };
        if page == current {
            return Ok(());
        }
        let chapter = book_ref.chapter_title(page).unwrap_or_default();
        let percent = format!("{}%", (remote.percentage * 100.0).floor());
        let device = if remote.device.is_empty() {
            "KOReader".to_string()
        } else {
            remote.device.clone()
        };
        let text = if chapter.is_empty() {
            tf!(Msg::ReaderSyncBody, device = device, percent = percent)
        } else {
            tf!(
                Msg::ReaderSyncBodyChapter,
                device = device,
                percent = percent,
                chapter = chapter
            )
        };
        perf!("sync: offering page at {page} ({percent}) from {device}");
        self.sync.offer = Some((book, page, text));
        self.show_sync_offer()
    }

    /// Show a waiting offer, once nothing covers the page.
    pub(crate) fn show_sync_offer(&mut self) -> Result<(), Error> {
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        let Some((book, _, _)) = &self.sync.offer else {
            return Ok(());
        };
        if *book != r.session.id {
            self.sync.offer = None;
            return Ok(());
        }
        if r.menu_open || !r.overlay.is_none() || r.gestures.holding() {
            return Ok(());
        }
        let Some((_, target, text)) = self.sync.offer.take() else {
            return Ok(());
        };
        r.overlay = Overlay::Sync { target, text };
        self.rebuild_reader(Refresh::Auto)?;
        self.present_full()
    }

    // ------------------------------------------------------------ screen

    pub(crate) fn sync_view_message(&self) -> String {
        if self.sync.checking {
            return tr(Msg::SyncStateChecking).to_string();
        }
        if !self.sync.message.is_empty() {
            return self.sync.message.clone();
        }
        match &self.net.accounts.sync {
            None => tr(Msg::SyncStateOff).to_string(),
            Some(_) => tr(Msg::SyncStateUnchecked).to_string(),
        }
    }

    pub(crate) fn build_sync_screen(
        &mut self,
        status: aristotle_ui::screens::library::StatusLine,
    ) -> (aristotle_ui::tree::Tree, net_ui::Frame) {
        let s = self.net.accounts.sync.clone();
        let message = self.sync_view_message();
        let import = self.net.import_path(crate::net::SYNC_IMPORT);
        let import = import.display().to_string();
        let v = SyncView {
            status,
            server: s
                .as_ref()
                .map(|s| s.server.as_str())
                .filter(|s| !s.is_empty()),
            username: s
                .as_ref()
                .map(|s| s.username.as_str())
                .filter(|s| !s.is_empty()),
            has_key: s.as_ref().is_some_and(|s| !s.key.is_empty()),
            auto: s.as_ref().is_none_or(|s| s.auto),
            message: &message,
            import_path: &import,
        };
        net_ui::build_sync(&mut self.ui.text, &self.styles, &v)
    }

    /// An action on the sync screen. Returns whether the screen needs
    /// building again.
    pub(crate) fn on_sync_action(&mut self, id: aristotle_ui::tree::ActionId) -> bool {
        let s = self.net.accounts.sync.clone();
        match id {
            na::EDIT_SERVER => {
                let v = s.map(|s| s.server).unwrap_or_default();
                self.request_text(TextRequest::new(TextField::SyncServer, &v));
            }
            na::EDIT_USER => {
                let v = s.map(|s| s.username).unwrap_or_default();
                self.request_text(TextRequest::new(TextField::SyncUser, &v));
            }
            na::EDIT_PASSWORD => self.request_text(TextRequest::new(TextField::SyncPassword, "")),
            na::AUTO => {
                if let Some(s) = self.net.accounts.sync.as_mut() {
                    s.auto = !s.auto;
                    self.net.save();
                }
            }
            na::LOG_IN | na::REGISTER => self.sync_login(id == na::REGISTER),
            na::IMPORT => {
                let msgs = self.net.import(true);
                self.sync.message = msgs.join(" ");
            }
            na::FORGET => {
                let before = self.net.accounts.allow_entries();
                self.net.accounts.sync = None;
                self.net.save();
                self.net.update_allowlist(&before);
                self.sync.message.clear();
            }
            _ => return false,
        }
        true
    }

    fn sync_login(&mut self, register: bool) {
        let Some(account) = self.net.accounts.sync.as_ref().and_then(|s| s.account()) else {
            self.sync.message = tr(Msg::SyncStateOff).to_string();
            return;
        };
        self.sync.checking = true;
        self.sync.message.clear();
        self.net.spawn("login", move |_| {
            let k = Kosync::new(Net::policy());
            let result = if register {
                k.register(&account).and_then(|_| k.authorize(&account))
            } else {
                k.authorize(&account)
            };
            perf!(
                "sync: {} {}: {}",
                if register { "register" } else { "login" },
                account.username,
                match &result {
                    Ok(()) => "ok".to_string(),
                    Err(e) => e.to_string(),
                }
            );
            Done::Login { register, result }
        });
    }

    pub(crate) fn on_login(&mut self, register: bool, result: Result<(), kosync::Error>) {
        self.sync.checking = false;
        let user = self
            .net
            .accounts
            .sync
            .as_ref()
            .map(|s| s.username.clone())
            .unwrap_or_default();
        self.sync.message = match result {
            Ok(()) if register => tf!(Msg::SyncStateRegistered, user = user),
            Ok(()) => tf!(Msg::SyncStateOk, user = user),
            Err(e) => describe_sync(&e),
        };
    }

    /// Typed text for a sync setting.
    pub(crate) fn on_sync_text(&mut self, field: SyncField, value: &str) {
        let before = self.net.accounts.allow_entries();
        match self.net.accounts.set_sync_field(field, value) {
            Ok(()) => {
                self.net.save();
                self.net.update_allowlist(&before);
                self.sync.message.clear();
            }
            Err(e) => self.sync.message = e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_requests_cannot_overwrite_newer_progress_on_the_server() {
        let gate = PushGate::default();
        let key = ("server".into(), "user".into(), "book".into());
        assert!(matches!(
            gate.run(key.clone(), 2, || Ok(())),
            PushOutcome::Finished(Ok(()))
        ));
        assert!(matches!(
            gate.run(key, 1, || panic!("obsolete push sent")),
            PushOutcome::Superseded
        ));
        let other = ("server".into(), "user".into(), "other book".into());
        assert!(matches!(
            gate.run(other, 1, || Ok(())),
            PushOutcome::Finished(Ok(()))
        ));
    }

    #[test]
    fn returning_to_the_baseline_still_supersedes_a_pending_different_position() {
        let baseline = Locator::Page { index: 0 };
        let mut opened = Opened {
            id: "book".into(),
            token: 1,
            format: Format::Pdf,
            path: PathBuf::new(),
            at_open: Some(baseline.clone()),
            acknowledged: 0,
            pending: BTreeMap::new(),
        };
        assert!(!opened.needs_push(&baseline));
        opened.pending.insert(1, Locator::Page { index: 5 });
        assert!(opened.needs_push(&baseline));
        opened.pending.clear();
        assert!(!opened.needs_push(&baseline));
    }

    fn remote(device: &str, id: &str, pct: f64, ts: Option<i64>) -> Position {
        Position {
            document: "abc".into(),
            progress: "/body/DocFragment[3]/body".into(),
            percentage: pct,
            device: device.into(),
            device_id: id.into(),
            timestamp: ts,
        }
    }

    fn local(fraction: f32, updated: i64) -> Progress {
        Progress {
            fraction,
            locator: Locator::Epub {
                spine: 2,
                offset: 0,
            },
            updated,
            source: None,
            page: None,
        }
    }

    #[test]
    fn asks_about_other_devices_positions_elsewhere() {
        let me = "0123456789ABCDEF0123456789ABCDEF";
        // Our own position: never.
        assert!(!worth_asking(
            &remote(DEVICE, me, 0.5, Some(200)),
            me,
            Some(&local(0.1, 100))
        ));
        // Another device elsewhere: ask, forwards or backwards, whatever
        // the clocks say.
        assert!(worth_asking(
            &remote("Kobo", "X", 0.05, Some(50)),
            me,
            Some(&local(0.4, 100))
        ));
        assert!(worth_asking(
            &remote("Kobo", "X", 0.9, None),
            me,
            Some(&local(0.4, 100))
        ));
        // Same name, another ID (a second Aristotle): ask.
        assert!(worth_asking(
            &remote(DEVICE, "Y", 0.9, Some(200)),
            me,
            Some(&local(0.4, 100))
        ));
        // The same place: no.
        assert!(!worth_asking(
            &remote("Kobo", "X", 0.4, Some(200)),
            me,
            Some(&local(0.4, 100))
        ));
        // Never read here: anything past the start.
        assert!(worth_asking(&remote("Kobo", "X", 0.3, Some(1)), me, None));
        assert!(!worth_asking(&remote("Kobo", "X", 0.0, Some(1)), me, None));
    }

    #[test]
    fn percentage_inside_the_chapter_only() {
        // Chapter at locations 1000..3000, fractions 0.2..0.4.
        assert_eq!(within_chapter(1000, 3000, 0.2, 0.4, 0.3), 2000);
        assert_eq!(within_chapter(1000, 3000, 0.2, 0.4, 0.2), 1000);
        // Outside: the chapter start, never another chapter.
        assert_eq!(within_chapter(1000, 3000, 0.2, 0.4, 0.45), 1000);
        assert_eq!(within_chapter(1000, 3000, 0.2, 0.4, 0.1), 1000);
        assert_eq!(within_chapter(1000, 3000, 0.2, 0.4, 0.3999), 2999);
        assert_eq!(within_chapter(1000, 1000, 0.2, 0.4, 0.3), 1000);
    }

    #[test]
    fn remote_targets_by_format() {
        let p = std::path::Path::new("/nonexistent.epub");
        assert_eq!(remote_target(Format::Pdf, p, "42"), Remote::Page(41));
        assert_eq!(remote_target(Format::Cbz, p, "1"), Remote::Page(0));
        assert_eq!(
            remote_target(Format::Mobi, p, "/body/DocFragment[3]/body"),
            Remote::Fraction
        );
        // An unreadable EPUB falls back to the percentage.
        assert_eq!(
            remote_target(Format::Epub, p, "/body/DocFragment[3]/body"),
            Remote::Fraction
        );
        assert_eq!(
            push_progress(Format::Pdf, p, &Locator::Page { index: 9 }).as_deref(),
            Some("10")
        );
        assert_eq!(
            push_progress(Format::Mobi, p, &Locator::Kindle { position: 1 }),
            None
        );
    }

    /// A stored (uncompressed) ZIP, enough for an EPUB written for a test.
    fn zip(members: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, data) in members {
            let mut crc = flate2::Crc::new();
            crc.update(data.as_bytes());
            let offset = out.len() as u32;
            let mut h = Vec::new();
            h.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            h.extend_from_slice(&crc.sum().to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&[0, 0]);
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&h);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data.as_bytes());
            cd.extend_from_slice(b"PK\x01\x02\x14\x00");
            cd.extend_from_slice(&h);
            cd.extend_from_slice(&[0; 10]);
            cd.extend_from_slice(&offset.to_le_bytes());
            cd.extend_from_slice(name.as_bytes());
        }
        let at = out.len() as u32;
        out.extend_from_slice(&cd);
        out.extend_from_slice(b"PK\x05\x06\x00\x00\x00\x00");
        out.extend_from_slice(&(members.len() as u16).to_le_bytes());
        out.extend_from_slice(&(members.len() as u16).to_le_bytes());
        out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    /// Our fragment map's Plato side agrees with the renderer's spine (which
    /// a renderer test checks against Plato itself), on an EPUB written for
    /// the test with an item missing from the archive.
    #[test]
    fn fragments_agree_with_the_renderer_spine() {
        let chapter = |t: &str| {
            format!(
                r#"<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>{t}</title></head><body><h1>{t}</h1><p>Litwo! Ojczyzno moja! ty jesteś jak zdrowie.</p></body></html>"#
            )
        };
        let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">test</dc:identifier><dc:title>Próba</dc:title><dc:language>pl</dc:language></metadata>
<manifest>
<item id="a" href="a.xhtml" media-type="application/xhtml+xml"/>
<item id="lost" href="lost.xhtml" media-type="application/xhtml+xml"/>
<item id="b" href="b.xhtml" media-type="application/xhtml+xml"/>
</manifest>
<spine><itemref idref="a"/><itemref idref="lost"/><itemref idref="b"/></spine>
</package>"#;
        let (a, b) = (chapter("Księga pierwsza"), chapter("Księga druga"));
        let bytes = zip(&[
            ("mimetype", "application/epub+zip"),
            (
                "META-INF/container.xml",
                r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
            ),
            ("OEBPS/content.opf", opf),
            ("OEBPS/a.xhtml", &a),
            ("OEBPS/b.xhtml", &b),
        ]);
        let dir = crate::testdir("sync-fragments");
        let book = dir.join("proba.epub");
        std::fs::write(&book, bytes).unwrap();
        let spine = aristotle_render::position::Spine::read(&book).unwrap();
        assert_eq!(spine.sizes.len(), 2, "Plato skips the lost item");
        let f = Fragments::read(&book).unwrap();
        assert_eq!(
            (f.fragment(0), f.fragment(1), f.fragment(2)),
            (Some(1), Some(3), None)
        );
        assert_eq!(
            push_progress(
                Format::Epub,
                &book,
                &Locator::Epub {
                    spine: 1,
                    offset: 5
                }
            )
            .as_deref(),
            Some("/body/DocFragment[3]/body")
        );
        assert_eq!(
            remote_target(Format::Epub, &book, "/body/DocFragment[3]/body/p[1]"),
            Remote::Chapter(1)
        );
        // KOReader's placeholder for the lost item leads to what follows.
        assert_eq!(
            remote_target(Format::Epub, &book, "/body/DocFragment[2]/body"),
            Remote::Chapter(1)
        );
    }
}
