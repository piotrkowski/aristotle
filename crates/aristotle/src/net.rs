// SPDX-License-Identifier: AGPL-3.0-or-later
//! The app's side of the network features: the settings on disk, worker
//! threads for requests, the device's allowlist, typed text, and errors in
//! the reader's language.
//!
//! - Settings, passwords included, live in `/var/local/aristotle/accounts.json`
//!   (root's folder on the ext3 partition, never the FAT userstore;
//!   `aristotle_net::store`). `sync.json` and `catalogs.json` next to the
//!   binary are read at start and on request, until the on-screen keyboard
//!   can type them, and lose their passwords once read.
//! - Every request runs on a thread of its own and reports a [`Done`]
//!   through a channel the main loop polls ([`Net::poll`]); the UI never
//!   waits for the network.
//! - Each request reads the agent's policy afresh (`net-mode`,
//!   `net-allow`), so a host the settings just allowed is reachable at once.
//! - Saving a server or catalog adds `host:port # aristotle:sync|opds` to
//!   the allowlist through `aristotlectl net allow`, and deleting it removes
//!   that line ([`Net::update_allowlist`]).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use aristotle_hal::system;
use aristotle_i18n::{fmt_bytes, tf, tr, Msg};
use aristotle_net::http::Error as HttpError;
use aristotle_net::store::{Accounts, ImportError, Imported};
use aristotle_net::{kosync, opds, Policy};

use crate::perf;

/// File names of the settings the reader writes by hand, next to the app.
pub const SYNC_IMPORT: &str = "sync.json";
pub const CATALOGS_IMPORT: &str = "catalogs.json";

/// What a worker thread finished, for the main loop.
pub enum Done {
    /// Log in or register.
    Login {
        register: bool,
        result: Result<(), kosync::Error>,
    },
    /// The position the server has for a book just opened.
    Pulled {
        book: String,
        result: Result<Option<(kosync::Position, crate::sync::Remote)>, kosync::Error>,
    },
    Pushed {
        book: String,
        /// `None` when there was nothing KOReader could use.
        result: Option<Result<(), kosync::Error>>,
    },
    /// A feed for the catalog screens (`token` says which request).
    Feed {
        token: u64,
        result: Result<opds::Feed, opds::Error>,
    },
    /// Bytes so far and the total, if known.
    Progress {
        token: u64,
        bytes: u64,
        total: Option<u64>,
    },
    Downloaded {
        token: u64,
        result: Result<PathBuf, HttpError>,
    },
}

/// Where `accounts.json` goes: `/var/local/aristotle`, the agent's folder,
/// or `$ARISTOTLE_ACCOUNTS_DIR` (development). It must be a folder only
/// this user owns and writes (`aristotle_net::store::private_dir`).
pub fn accounts_dir() -> PathBuf {
    std::env::var_os("ARISTOTLE_ACCOUNTS_DIR")
        .map_or_else(|| PathBuf::from(system::AGENT_DIR), PathBuf::from)
}

/// Where the network features keep and find things.
pub struct Paths {
    /// The private folder for `accounts.json` ([`accounts_dir`]).
    pub state: PathBuf,
    /// Where `sync.json` and `catalogs.json` are read from.
    pub import: PathBuf,
}

pub struct Net {
    pub paths: Paths,
    pub accounts: Accounts,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    running: usize,
}

impl Net {
    /// The settings, with any changed import file read in.
    pub fn new(paths: Paths) -> Net {
        let (tx, rx) = mpsc::channel();
        // A folder that could leak passwords is not read either: settings
        // then last for this run only, and saving reports why.
        let accounts = match aristotle_net::store::private_dir(&paths.state) {
            Ok(()) => Accounts::load(&paths.state),
            Err(e) => {
                eprintln!("aristotle: {e}");
                Accounts::default()
            }
        };
        let mut net = Net {
            paths,
            accounts,
            tx,
            rx,
            running: 0,
        };
        for msg in net.import(false) {
            perf!("net: {msg}");
        }
        net
    }

    pub fn import_path(&self, file: &str) -> PathBuf {
        self.paths.import.join(file)
    }

    /// Read `sync.json` and `catalogs.json` (only if they changed, unless
    /// `force`), save, and bring the allowlist along. Returns a message per
    /// file read or failed.
    pub fn import(&mut self, force: bool) -> Vec<String> {
        let before = self.accounts.allow_entries();
        let mut out = Vec::new();
        let mut changed = false;
        for file in [SYNC_IMPORT, CATALOGS_IMPORT] {
            let path = self.import_path(file);
            let r = if file == SYNC_IMPORT {
                self.accounts.import_sync(&path, force)
            } else {
                self.accounts.import_catalogs(&path, force)
            };
            match r {
                Ok(Imported::Loaded) => {
                    changed = true;
                    out.push(tf!(Msg::SyncImported, path = path.display()));
                }
                Ok(Imported::Missing) if force => {
                    out.push(tf!(Msg::SyncNothingToImport, path = path.display()));
                }
                Ok(_) => {}
                Err(e) => out.push(import_error(&path, &e)),
            }
        }
        if changed {
            self.save();
            self.update_allowlist(&before);
        }
        out
    }

    pub fn save(&mut self) {
        if let Err(e) = self.accounts.save(&self.paths.state) {
            eprintln!("aristotle: can't save the network settings: {e}");
        }
    }

    /// Add the allowlist entries the settings gained since `before` and
    /// remove those they lost. Runs `aristotlectl`, which applies the
    /// policy at once in `scoped` mode (a second or two); settings change
    /// rarely, so this blocks.
    pub fn update_allowlist(&self, before: &BTreeSet<(String, &'static str)>) {
        let after = self.accounts.allow_entries();
        for (add, (entry, feature)) in after
            .difference(before)
            .map(|e| (true, e))
            .chain(before.difference(&after).map(|e| (false, e)))
        {
            match system::net_allow(add, entry, feature) {
                Ok(msg) => perf!("net: allowlist: {msg}"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    perf!("net: no agent, no allowlist to change for {entry}")
                }
                Err(e) => eprintln!("aristotle: allowlist {entry}: {e}"),
            }
        }
    }

    /// The device's policy now.
    pub fn policy() -> Policy {
        Policy::load(&system::agent_dir())
    }

    /// Run `job` on a worker thread; its result arrives through
    /// [`Net::poll`].
    pub fn spawn(&mut self, name: &str, job: impl FnOnce(&Sender<Done>) -> Done + Send + 'static) {
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("net-{name}"))
            .spawn(move || {
                let done = job(&tx);
                let _ = tx.send(done);
            });
        match spawned {
            Ok(_) => self.running += 1,
            Err(e) => eprintln!("aristotle: can't start the {name} thread: {e}"),
        }
    }

    /// Results that arrived since the last call. Progress reports do not
    /// end a job.
    pub fn poll(&mut self) -> Vec<Done> {
        let mut out = Vec::new();
        while let Ok(d) = self.rx.try_recv() {
            if !matches!(d, Done::Progress { .. }) {
                self.running = self.running.saturating_sub(1);
            }
            out.push(d);
        }
        out
    }

    /// Whether a request is under way (the loop polls more often then).
    pub fn busy(&self) -> bool {
        self.running > 0
    }
}

fn import_error(path: &Path, e: &ImportError) -> String {
    tf!(
        Msg::SyncErrorFile,
        path = path.display(),
        error = e.to_string()
    )
}

/// An HTTP failure in the reader's words.
pub fn describe(e: &HttpError) -> String {
    match e {
        HttpError::NotAllowed(host) => tf!(Msg::NetNotAllowed, host = host),
        HttpError::Connect(_) => tr(Msg::NetUnreachable).to_string(),
        HttpError::Timeout => tr(Msg::NetTimeout).to_string(),
        HttpError::NoTls => tr(Msg::NetNoHttps).to_string(),
        HttpError::Tls { clock: true, .. } => tr(Msg::NetClock).to_string(),
        HttpError::Tls { detail, .. } => tf!(Msg::NetTls, detail = detail),
        HttpError::Unauthorized => tr(Msg::NetRefused).to_string(),
        HttpError::TooLarge(n) => tf!(Msg::CatalogsTooLarge, size = fmt_bytes(*n)),
        e => tf!(Msg::NetOther, detail = e.to_string()),
    }
}

pub fn describe_sync(e: &kosync::Error) -> String {
    match e {
        kosync::Error::Http(e) => describe(e),
        kosync::Error::Unauthorized => tr(Msg::NetRefused).to_string(),
        kosync::Error::UserExists => tr(Msg::SyncErrorTaken).to_string(),
        kosync::Error::RegistrationClosed => tr(Msg::SyncErrorClosed).to_string(),
        e => tf!(Msg::NetOther, detail = e.to_string()),
    }
}

pub fn describe_opds(e: &opds::Error) -> String {
    match e {
        opds::Error::Http(e) => describe(e),
        opds::Error::NotAFeed(_) => tr(Msg::CatalogsNotFeed).to_string(),
    }
}

// ------------------------------------------------------------ typed text

/// What a line of typed text is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextField {
    SyncServer,
    SyncUser,
    SyncPassword,
    CatalogUrl,
    Search,
}

/// A request for one line of text: what the on-screen keyboard's
/// `TextInput` needs to ask for it. The answer comes back through
/// `App::on_text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRequest {
    pub field: TextField,
    /// The title over the keyboard ("Server").
    pub title: String,
    pub initial: String,
    /// Hidden while typed (passwords).
    pub secret: bool,
}

impl TextRequest {
    pub fn new(field: TextField, initial: &str) -> TextRequest {
        let title = tr(match field {
            TextField::SyncServer => Msg::SyncServer,
            TextField::SyncUser => Msg::SyncUser,
            TextField::SyncPassword => Msg::SyncPassword,
            TextField::CatalogUrl => Msg::CatalogsAdd,
            TextField::Search => Msg::CatalogsSearch,
        });
        TextRequest {
            field,
            title: title.to_string(),
            initial: initial.to_string(),
            secret: field == TextField::SyncPassword,
        }
    }
}
