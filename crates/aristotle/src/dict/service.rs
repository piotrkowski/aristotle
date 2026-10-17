// SPDX-License-Identifier: AGPL-3.0-or-later
//! Lookups on a thread of their own.
//!
//! A lookup is normally a few milliseconds, but the first one after a
//! while may wait for flash reads, and the reader must give feedback when
//! a long press fires, not when the dictionary answers. So the reader asks
//! here and waits a short while ([`Service::wait`]); if the answer is late
//! it shows the selected word first and the popup when the answer comes.
//!
//! The thread owns the [`Dictionaries`]. Missing fold indexes are built by
//! another thread at idle priority, so they never delay a lookup.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use super::{build_indexes, Dictionaries, Lookup};

/// A dictionary as the switcher shows it.
#[derive(Clone, Debug)]
pub struct DictLabel {
    pub id: String,
    pub label: String,
}

pub enum Request {
    Lookup {
        word: String,
        book_lang: Option<String>,
    },
    /// Show the lookup from another dictionary (index into the labels).
    Switch { lookup: Lookup, dict: usize },
    /// A word-by-word gloss of a phrase.
    Gloss {
        words: Vec<String>,
        book_lang: Option<String>,
    },
}

pub enum Reply {
    Lookup(Lookup),
    /// (word, short gloss or "—").
    Gloss(Vec<(String, String)>),
}

pub struct Service {
    tx: Sender<Request>,
    rx: Receiver<Reply>,
    pub dicts: Vec<DictLabel>,
}

impl Service {
    /// Find the dictionaries (cheap: a few file opens) and start the
    /// lookup thread and, if needed, the index builder.
    pub fn start(dirs: Vec<PathBuf>, cache: PathBuf, state: PathBuf) -> Service {
        let mut d = Dictionaries::discover(&dirs, &cache, Some(state));
        let dicts = d
            .dicts
            .iter()
            .map(|x| DictLabel {
                id: x.id.clone(),
                label: x.label(),
            })
            .collect();
        let missing = d.missing_indexes();
        if !missing.is_empty() {
            let _ = thread::Builder::new()
                .name("dict-index".into())
                .spawn(move || {
                    crate::pagecache::lower_priority();
                    build_indexes(&missing);
                });
        }
        let (tx, requests) = mpsc::channel::<Request>();
        let (answers, rx) = mpsc::channel();
        let _ = thread::Builder::new().name("dict".into()).spawn(move || {
            for r in requests {
                let reply = match r {
                    Request::Lookup { word, book_lang } => {
                        Reply::Lookup(d.lookup(&word, book_lang.as_deref()))
                    }
                    Request::Switch { mut lookup, dict } => {
                        d.switch(&mut lookup, dict);
                        Reply::Lookup(lookup)
                    }
                    Request::Gloss { words, book_lang } => Reply::Gloss(
                        words
                            .iter()
                            .map(|w| {
                                let l = d.lookup(w, book_lang.as_deref());
                                let g = l.found.as_ref().map(super::gloss).unwrap_or_default();
                                // "mori  morior — umierać", but "memento
                                // przypomnienie", not "memento — memento — …".
                                let same = format!("{} — ", l.word.to_lowercase());
                                let g = if g.to_lowercase().starts_with(&same) {
                                    g.chars().skip(same.chars().count()).collect()
                                } else {
                                    g
                                };
                                (l.word, if g.is_empty() { "—".into() } else { g })
                            })
                            .collect(),
                    ),
                };
                if answers.send(reply).is_err() {
                    break;
                }
            }
        });
        Service { tx, rx, dicts }
    }

    pub fn is_empty(&self) -> bool {
        self.dicts.is_empty()
    }

    pub fn ask(&self, r: Request) {
        // Drop answers nobody waited for.
        while self.rx.try_recv().is_ok() {}
        let _ = self.tx.send(r);
    }

    /// The answer, if it comes within `timeout`.
    pub fn wait(&self, timeout: Duration) -> Option<Reply> {
        self.rx.recv_timeout(timeout).ok()
    }
}
