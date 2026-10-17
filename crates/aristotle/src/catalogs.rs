// SPDX-License-Identifier: AGPL-3.0-or-later
//! The network screens in the app: progress sync (its logic is in
//! `sync.rs`), the OPDS catalogs, a catalog's feeds and one book.
//!
//! Browsing keeps the feeds already seen on a stack, so going back costs no
//! request. A feed's next and previous pages replace the page on screen.
//! A download goes to `<books>/<catalog name>/<title> - <author>.<ext>`
//! through a hidden part file (`aristotle_net::download`), then the library
//! is scanned so the book appears with its ID, the partial MD5 KOSync uses.
//!
//! Screen switches flash once; a feed page, a message or a download's
//! progress changes in place ([`App::show_network`] with `Refresh::Auto`).

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use aristotle_i18n::{fmt_bytes, tf, tr, Msg};
use aristotle_net::download::{self, KEEP_FREE, MAX_BOOK};
use aristotle_net::opds::{self, Catalog, Entry, Feed, Search};
use aristotle_net::store::SyncField;
use aristotle_net::{http, Client, Request, Url};
use aristotle_render::plato_core::anyhow::Error;
use aristotle_ui::screens::network::{
    self as net_ui, action as na, CatalogRow, CatalogsView, EntryView, FeedRow, FeedView, Paged,
};
use aristotle_ui::tree::Refresh;
use aristotle_ui::ui::UiEvent;

use crate::device::Facts;
use crate::net::{describe, describe_opds, Done, Net, TextField, TextRequest};
use crate::{perf, App, Flow, Screen};

/// Which network screen is up.
pub enum NetView {
    Sync,
    Catalogs,
    Feed(Browse),
    /// A book of the feed on top of `browse`.
    Entry(Browse, usize),
}

/// The screen and the nodes it updates.
pub struct NetScreen {
    pub view: NetView,
    pub frame: Option<net_ui::Frame>,
    /// The settings page to return to.
    pub settings_page: usize,
}

/// One feed page as loaded.
pub struct Page {
    pub url: Url,
    pub feed: Feed,
    /// The list page shown, to come back to it.
    pub list_page: usize,
    /// A title instead of the feed's (search results).
    pub title: Option<String>,
}

/// Browsing one catalog.
pub struct Browse {
    pub catalog: usize,
    /// Where Back goes, last on top.
    pub stack: Vec<Page>,
    pub page: Page,
}

/// What the catalog screens wait for.
#[derive(Default)]
pub struct CatalogState {
    /// The answer to this request is the one the screen wants; older ones
    /// are dropped (the reader went elsewhere meanwhile).
    pub token: u64,
    pub pending: Option<Pending>,
    pub message: String,
    /// The book just downloaded, to offer "Read now".
    pub saved: Option<PathBuf>,
    /// Set to stop the download under way.
    pub cancel: Option<Arc<AtomicBool>>,
}

/// What the request `token` is for.
pub enum Pending {
    /// Open a catalog's root feed.
    Open {
        catalog: usize,
    },
    /// Into an entry's feed, or search results: pushed on the stack.
    Push {
        url: Url,
        title: Option<String>,
    },
    /// The server's next or previous page: replaces the one shown.
    Turn {
        url: Url,
        next: bool,
    },
    Download,
}

fn catalog_client(net: &Net, i: usize) -> Option<(Catalog, Url)> {
    let c = net.accounts.catalogs.get(i)?;
    let url = Url::parse(&c.url).ok()?;
    let http = Client::new(Net::policy(), Catalog::timeouts());
    Some((Catalog::new(http, c.credentials()), url))
}

/// "Bolesław Prus · EPUB 1.2 MB, PDF".
fn entry_detail(e: &Entry) -> String {
    let mut parts = Vec::new();
    if !e.authors.is_empty() {
        parts.push(e.authors.join(", "));
    } else if e.acquisitions.is_empty() && !e.summary.is_empty() {
        parts.push(e.summary.clone());
    }
    let formats: Vec<String> = e.acquisitions.iter().map(format_label).collect();
    if !formats.is_empty() {
        parts.push(formats.join(", "));
    }
    parts.join(" · ")
}

fn format_label(a: &opds::Acquisition) -> String {
    match a.size {
        Some(n) => format!("{} · {}", a.kind.label(), fmt_bytes(n)),
        None => a.kind.label().to_string(),
    }
}

impl App {
    /// Switch to a network screen.
    pub(crate) fn open_network(
        &mut self,
        view: NetView,
        settings_page: usize,
    ) -> Result<(), Error> {
        if matches!(view, NetView::Catalogs | NetView::Sync) {
            self.catalogs.message.clear();
        }
        self.screen = Screen::Network(Box::new(NetScreen {
            view,
            frame: None,
            settings_page,
        }));
        self.show_network(Refresh::Full)
    }

    /// Build the network screen shown, as a switch (`Full`) or in place.
    pub(crate) fn show_network(&mut self, refresh: Refresh) -> Result<(), Error> {
        let facts = Facts::status(&self.battery);
        self.clock = facts.clock.clone();
        let status = facts.status_line();
        let Screen::Network(ns) = &self.screen else {
            return Ok(());
        };
        let message = self.catalogs.message.clone();
        let sync = matches!(ns.view, NetView::Sync);
        let (tree, frame) = if sync {
            self.build_sync_screen(status)
        } else {
            let Screen::Network(ns) = &self.screen else {
                return Ok(());
            };
            match &ns.view {
                NetView::Sync => unreachable!(),
                NetView::Catalogs => {
                    let cats = &self.net.accounts.catalogs;
                    let rows: Vec<CatalogRow> = cats
                        .iter()
                        .map(|c| CatalogRow {
                            name: &c.name,
                            url: &c.url,
                        })
                        .collect();
                    let import = self.net.import_path(crate::net::CATALOGS_IMPORT);
                    let import = import.display().to_string();
                    let v = CatalogsView {
                        status,
                        catalogs: &rows,
                        message: &message,
                        import_path: &import,
                    };
                    net_ui::build_catalogs(&mut self.ui.text, &self.styles, &v)
                }
                NetView::Feed(b) => {
                    let details: Vec<String> =
                        b.page.feed.entries.iter().map(entry_detail).collect();
                    let rows: Vec<FeedRow> = b
                        .page
                        .feed
                        .entries
                        .iter()
                        .zip(&details)
                        .map(|(e, d)| FeedRow {
                            title: if e.title.is_empty() { "—" } else { &e.title },
                            detail: d,
                            navigation: e.acquisitions.is_empty(),
                        })
                        .collect();
                    let title = b.page.title.as_deref().unwrap_or(&b.page.feed.title);
                    let v = FeedView {
                        status,
                        title,
                        rows: &rows,
                        searchable: b.page.feed.search.is_some(),
                        more_before: b.page.feed.previous.is_some(),
                        more_after: b.page.feed.next.is_some(),
                        message: &message,
                    };
                    net_ui::build_feed(&mut self.ui.text, &self.styles, &v)
                }
                NetView::Entry(b, i) => {
                    let e = &b.page.feed.entries[*i];
                    let formats: Vec<String> = e.acquisitions.iter().map(format_label).collect();
                    let authors = e.authors.join(", ");
                    let v = EntryView {
                        status,
                        title: &e.title,
                        authors: &authors,
                        summary: &e.summary,
                        formats: &formats,
                        downloaded: self.catalogs.saved.is_some(),
                        message: &message,
                    };
                    net_ui::build_entry(&mut self.ui.text, &self.styles, &v)
                }
            }
        };
        let Screen::Network(ns) = &self.screen else {
            return Ok(());
        };
        let list_page = match &ns.view {
            NetView::Feed(b) => b.page.list_page,
            _ => 0,
        };
        self.ui.set_tree_with(tree, refresh);
        self.ui.layout();
        if list_page > 0 {
            self.ui.tree.set_page(frame.list, list_page);
            self.ui.layout();
        }
        frame.sync(&mut self.ui.tree);
        let busy = self.net.busy() && (self.catalogs.pending.is_some() || self.sync.checking);
        self.ui.tree.set_hidden(frame.busy, !busy);
        self.ui.gesture_config_mut().long_press_us = 500_000;
        if let Screen::Network(ns) = &mut self.screen {
            ns.frame = Some(frame);
        }
        self.render_ui()
    }

    /// Put `text` in the message line now (one small update).
    fn net_message(&mut self, text: String, busy: bool) -> Result<(), Error> {
        self.catalogs.message = text.clone();
        if let Screen::Network(ns) = &self.screen {
            if let Some(f) = &ns.frame {
                f.set_message(&mut self.ui.tree, &text);
                self.ui.tree.set_hidden(f.busy, !busy);
            }
        }
        self.render_ui()
    }

    pub(crate) fn on_network_event(&mut self, ev: UiEvent) -> Result<Flow, Error> {
        let UiEvent::Action { id, .. } = ev else {
            return Ok(Flow::Continue);
        };
        let Screen::Network(ns) = &mut self.screen else {
            return Ok(Flow::Continue);
        };
        if let Some(f) = &ns.frame {
            match f.handle(&mut self.ui.tree, id) {
                Paged::Turned => {
                    self.ui.layout();
                    f.sync(&mut self.ui.tree);
                    let now = self.ui.tree.page(f.list).0;
                    if let NetView::Feed(b) = &mut ns.view {
                        b.page.list_page = now;
                    }
                    return Ok(Flow::Continue);
                }
                Paged::Fetch { next } => {
                    if let NetView::Feed(b) = &ns.view {
                        let url = if next {
                            b.page.feed.next.clone()
                        } else {
                            b.page.feed.previous.clone()
                        };
                        if let Some(url) = url {
                            let catalog = b.catalog;
                            self.fetch_feed(catalog, url.clone(), Pending::Turn { url, next })?;
                        }
                    }
                    return Ok(Flow::Continue);
                }
                Paged::No => {}
            }
        }
        if id == na::BACK {
            return self.network_back().map(|_| Flow::Continue);
        }
        let view = match &ns.view {
            NetView::Sync => 0,
            NetView::Catalogs => 1,
            NetView::Feed(_) => 2,
            NetView::Entry(..) => 3,
        };
        match view {
            0 => {
                if self.on_sync_action(id) {
                    self.show_network(Refresh::Auto)?;
                }
            }
            1 => self.on_catalogs_action(id)?,
            2 => self.on_feed_action(id)?,
            _ => self.on_entry_action(id)?,
        }
        Ok(Flow::Continue)
    }

    fn network_back(&mut self) -> Result<(), Error> {
        let Screen::Network(ns) = &mut self.screen else {
            return Ok(());
        };
        // A request for the screen being left is no longer wanted.
        self.catalogs.token += 1;
        self.catalogs.pending = None;
        self.catalogs.message.clear();
        let page = ns.settings_page;
        let view = std::mem::replace(&mut ns.view, NetView::Catalogs);
        ns.view = match view {
            NetView::Sync | NetView::Catalogs => return self.show_settings(Refresh::Full, page),
            NetView::Feed(mut b) => match b.stack.pop() {
                Some(prev) => {
                    b.page = prev;
                    NetView::Feed(b)
                }
                None => NetView::Catalogs,
            },
            NetView::Entry(b, _) => {
                if let Some(c) = self.catalogs.cancel.take() {
                    c.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.catalogs.saved = None;
                NetView::Feed(b)
            }
        };
        self.show_network(Refresh::Full)
    }

    fn on_catalogs_action(&mut self, id: aristotle_ui::tree::ActionId) -> Result<(), Error> {
        let n = self.net.accounts.catalogs.len();
        match id {
            na::ADD => {
                self.request_text(TextRequest::new(TextField::CatalogUrl, "http://"));
                Ok(())
            }
            na::IMPORT => {
                let msgs = self.net.import(true);
                self.catalogs.message = msgs.join(" ");
                self.show_network(Refresh::Auto)
            }
            id if (na::ITEM..na::ITEM + n as u32).contains(&id) => {
                let catalog = (id - na::ITEM) as usize;
                let Some((_, url)) = catalog_client(&self.net, catalog) else {
                    return Ok(());
                };
                self.fetch_feed(catalog, url, Pending::Open { catalog })
            }
            id if (na::REMOVE..na::REMOVE + n as u32).contains(&id) => {
                let before = self.net.accounts.allow_entries();
                let c = self
                    .net
                    .accounts
                    .catalogs
                    .remove((id - na::REMOVE) as usize);
                self.net.save();
                self.net.update_allowlist(&before);
                self.catalogs.message = tf!(Msg::CatalogsRemoved, name = c.name);
                self.show_network(Refresh::Auto)
            }
            _ => Ok(()),
        }
    }

    fn on_feed_action(&mut self, id: aristotle_ui::tree::ActionId) -> Result<(), Error> {
        let Screen::Network(ns) = &mut self.screen else {
            return Ok(());
        };
        let NetView::Feed(b) = &mut ns.view else {
            return Ok(());
        };
        if id == na::SEARCH {
            self.request_text(TextRequest::new(TextField::Search, ""));
            return Ok(());
        }
        let Some(i) = id.checked_sub(na::ITEM).map(|i| i as usize) else {
            return Ok(());
        };
        let Some(e) = b.page.feed.entries.get(i) else {
            return Ok(());
        };
        if !e.acquisitions.is_empty() {
            let view = std::mem::replace(&mut ns.view, NetView::Catalogs);
            if let NetView::Feed(b) = view {
                ns.view = NetView::Entry(b, i);
            }
            self.catalogs.message.clear();
            self.catalogs.saved = None;
            return self.show_network(Refresh::Full);
        }
        if let Some(url) = e.navigation.clone() {
            let catalog = b.catalog;
            return self.fetch_feed(catalog, url.clone(), Pending::Push { url, title: None });
        }
        Ok(())
    }

    /// Fetch a feed on a worker; the screen shows "Loading…" meanwhile.
    fn fetch_feed(&mut self, catalog: usize, url: Url, pending: Pending) -> Result<(), Error> {
        let Some((client, _)) = catalog_client(&self.net, catalog) else {
            return Ok(());
        };
        self.catalogs.token += 1;
        let token = self.catalogs.token;
        self.catalogs.pending = Some(pending);
        self.net.spawn("feed", move |_| {
            let t0 = std::time::Instant::now();
            let result = client.feed(&url);
            perf!(
                "opds: {url} in {:.0} ms: {}",
                crate::ms(t0.elapsed()),
                match &result {
                    Ok(f) => format!("{} entries", f.entries.len()),
                    Err(e) => e.to_string(),
                }
            );
            Done::Feed { token, result }
        });
        self.net_message(tr(Msg::CatalogsLoading).to_string(), true)
    }

    /// Search the open catalog for `terms`.
    fn search(&mut self, terms: &str) -> Result<(), Error> {
        let Screen::Network(ns) = &self.screen else {
            return Ok(());
        };
        let NetView::Feed(b) = &ns.view else {
            return Ok(());
        };
        let Some(search) = b.page.feed.search.clone() else {
            return Ok(());
        };
        let catalog = b.catalog;
        let Some((client, _)) = catalog_client(&self.net, catalog) else {
            return Ok(());
        };
        self.catalogs.token += 1;
        let token = self.catalogs.token;
        let title = tf!(Msg::CatalogsResults, terms = terms);
        // The URL is known only once the description is read; the stack
        // entry takes the template's address meanwhile.
        let placeholder = match &search {
            Search::Description(u) => u.clone(),
            Search::Template(t) => {
                opds::search_url(t, terms).unwrap_or_else(|_| b.page.url.clone())
            }
        };
        self.catalogs.pending = Some(Pending::Push {
            url: placeholder,
            title: Some(title),
        });
        let terms = terms.to_string();
        self.net.spawn("search", move |_| {
            let result = client
                .search_url(&search, &terms)
                .and_then(|url| client.feed(&url));
            Done::Feed { token, result }
        });
        self.net_message(tr(Msg::CatalogsLoading).to_string(), true)
    }

    /// A feed arrived.
    pub(crate) fn on_feed(
        &mut self,
        token: u64,
        result: Result<Feed, opds::Error>,
    ) -> Result<(), Error> {
        if token != self.catalogs.token {
            return Ok(());
        }
        let Some(pending) = self.catalogs.pending.take() else {
            return Ok(());
        };
        let feed = match result {
            Ok(f) => f,
            Err(e) => {
                let msg = describe_opds(&e);
                return if matches!(self.screen, Screen::Network(_)) {
                    self.net_message(msg, false)
                } else {
                    Ok(())
                };
            }
        };
        let Screen::Network(ns) = &mut self.screen else {
            return Ok(());
        };
        self.catalogs.message.clear();
        let refresh = match pending {
            Pending::Open { catalog } => {
                // A catalog added by address is named by its first feed.
                if let Some(c) = self.net.accounts.catalogs.get_mut(catalog) {
                    let host_only = Url::parse(&c.url).is_ok_and(|u| u.host() == c.name);
                    if host_only && !feed.title.is_empty() {
                        c.name = feed.title.clone();
                        self.net.save();
                    }
                }
                let url = Url::parse(&self.net.accounts.catalogs[catalog].url)
                    .unwrap_or_else(|_| Url::parse("http://invalid/").unwrap());
                ns.view = NetView::Feed(Browse {
                    catalog,
                    stack: Vec::new(),
                    page: Page {
                        url,
                        feed,
                        list_page: 0,
                        title: None,
                    },
                });
                Refresh::Full
            }
            Pending::Push { url, title } => {
                let NetView::Feed(b) = &mut ns.view else {
                    return Ok(());
                };
                let page = Page {
                    url,
                    feed,
                    list_page: 0,
                    title,
                };
                let old = std::mem::replace(&mut b.page, page);
                b.stack.push(old);
                Refresh::Full
            }
            Pending::Turn { url, next } => {
                let NetView::Feed(b) = &mut ns.view else {
                    return Ok(());
                };
                b.page.url = url;
                b.page.feed = feed;
                // Coming back from the next page, show its last list page.
                b.page.list_page = if next { 0 } else { usize::MAX / 2 };
                Refresh::Auto
            }
            Pending::Download => return Ok(()),
        };
        self.show_network(refresh)?;
        // Clamp a "last page" request to the real last page.
        if let Screen::Network(ns) = &mut self.screen {
            if let (NetView::Feed(b), Some(f)) = (&mut ns.view, &ns.frame) {
                b.page.list_page = self.ui.tree.page(f.list).0;
            }
        }
        Ok(())
    }

    fn on_entry_action(&mut self, id: aristotle_ui::tree::ActionId) -> Result<(), Error> {
        if id == na::READ {
            let Some(path) = self.catalogs.saved.clone() else {
                return Ok(());
            };
            let rel = path
                .strip_prefix(&self.root)
                .ok()
                .map(|p| p.to_string_lossy().into_owned());
            let book = self
                .lib
                .books()
                .iter()
                .find(|b| Some(&b.file.path) == rel.as_ref())
                .map(|b| b.id.clone());
            if let Some(id) = book {
                return self.open_book(&id, None);
            }
            return Ok(());
        }
        let Some(i) = id
            .checked_sub(na::DOWNLOAD)
            .map(|i| i as usize)
            .filter(|&i| i < 20)
        else {
            return Ok(());
        };
        if self.catalogs.pending.is_some() {
            return Ok(());
        }
        let Screen::Network(ns) = &self.screen else {
            return Ok(());
        };
        let NetView::Entry(b, e) = &ns.view else {
            return Ok(());
        };
        let entry = &b.page.feed.entries[*e];
        let Some(acq) = entry.acquisitions.get(i).cloned() else {
            return Ok(());
        };
        let Some((client, _)) = catalog_client(&self.net, b.catalog) else {
            return Ok(());
        };
        let catalog_name = self.net.accounts.catalogs[b.catalog].name.clone();
        let dir = self
            .root
            .join(download::safe_name(&catalog_name, 80, "Catalog"));
        let stem = match entry.authors.first() {
            Some(a) => format!("{} - {a}", entry.title),
            None => entry.title.clone(),
        };
        let stem = download::safe_name(&stem, 120, "book");
        let free = aristotle_hal::system::storage(&self.root)
            .map(|s| s.free.saturating_sub(KEEP_FREE))
            .unwrap_or(MAX_BOOK);
        let limit = MAX_BOOK.min(free);
        if acq.size.is_some_and(|s| s > limit) {
            let msg = if limit < MAX_BOOK {
                tr(Msg::CatalogsNoSpace).to_string()
            } else {
                tf!(Msg::CatalogsTooLarge, size = fmt_bytes(MAX_BOOK))
            };
            return self.net_message(msg, false);
        }
        self.catalogs.token += 1;
        let token = self.catalogs.token;
        self.catalogs.pending = Some(Pending::Download);
        let cancel = Arc::new(AtomicBool::new(false));
        self.catalogs.cancel = Some(cancel.clone());
        let ext = acq.kind.extension();
        self.net.spawn("download", move |tx| {
            let req = Request::get(acq.url.clone()).credentials(client.credentials().cloned());
            let mut last = 0u64;
            let t0 = std::time::Instant::now();
            let result = download::save(
                client.client(),
                &req,
                &dir,
                &stem,
                ext,
                limit,
                &cancel,
                &mut |bytes, total| {
                    // A report per 5 % (or 1 MB without a total): each is
                    // a small update on the panel.
                    let step = total.map_or(1 << 20, |t| (t / 20).max(1));
                    if bytes >= last + step {
                        last = bytes;
                        let _ = tx.send(Done::Progress {
                            token,
                            bytes,
                            total,
                        });
                    }
                },
            );
            perf!(
                "opds: download {} in {:.0} ms: {}",
                acq.url,
                crate::ms(t0.elapsed()),
                match &result {
                    Ok(s) => format!("{} bytes to {}", s.info.bytes, s.path.display()),
                    Err(e) => e.to_string(),
                }
            );
            Done::Downloaded {
                token,
                result: result.map(|s| s.path),
            }
        });
        self.net_message(tf!(Msg::CatalogsDownloading, percent = ""), true)
    }

    pub(crate) fn on_download_progress(
        &mut self,
        token: u64,
        bytes: u64,
        total: Option<u64>,
    ) -> Result<(), Error> {
        if token != self.catalogs.token || !matches!(self.screen, Screen::Network(_)) {
            return Ok(());
        }
        let shown = match total {
            Some(t) if t > 0 => format!("{}%", bytes * 100 / t),
            _ => fmt_bytes(bytes),
        };
        self.net_message(tf!(Msg::CatalogsDownloading, percent = shown), true)
    }

    pub(crate) fn on_downloaded(
        &mut self,
        token: u64,
        result: Result<PathBuf, http::Error>,
    ) -> Result<(), Error> {
        self.catalogs.cancel = None;
        if token == self.catalogs.token {
            self.catalogs.pending = None;
        }
        match result {
            Ok(path) => {
                // The library picks the book up: a scan of the new file
                // (and anything else that changed) gives it its ID.
                let root = self.root.clone();
                self.scan(&root)?;
                let shown = path
                    .strip_prefix(&self.root)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                self.catalogs.saved = Some(path);
                self.catalogs.message = tf!(Msg::CatalogsDownloaded, path = shown);
                if matches!(self.screen, Screen::Network(_)) {
                    self.show_network(Refresh::Auto)?;
                }
            }
            Err(http::Error::Cancelled) => {}
            Err(e) => {
                if token == self.catalogs.token && matches!(self.screen, Screen::Network(_)) {
                    self.net_message(describe(&e), false)?;
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ typed text

    /// Ask for a line of text. The on-screen keyboard's `TextInput` will
    /// answer through [`App::on_text`]; until it lands, the screen says
    /// where to put the setting instead.
    pub(crate) fn request_text(&mut self, req: TextRequest) {
        perf!(
            "text: {:?} requested ({}), no keyboard yet",
            req.field,
            req.title
        );
        let file = match req.field {
            TextField::SyncServer | TextField::SyncUser | TextField::SyncPassword => {
                crate::net::SYNC_IMPORT
            }
            TextField::CatalogUrl | TextField::Search => crate::net::CATALOGS_IMPORT,
        };
        let path = self.net.import_path(file).display().to_string();
        let msg = tf!(Msg::SyncTypeLater, path = path);
        if matches!(
            req.field,
            TextField::SyncServer | TextField::SyncUser | TextField::SyncPassword
        ) {
            self.sync.message = msg.clone();
        }
        let _ = self.net_message(msg, false);
    }

    /// Typed text arrived for `field`.
    #[allow(dead_code)] // called by the on-screen keyboard once it lands
    pub(crate) fn on_text(&mut self, field: TextField, value: &str) -> Result<(), Error> {
        match field {
            TextField::SyncServer => self.on_sync_text(SyncField::Server, value),
            TextField::SyncUser => self.on_sync_text(SyncField::Username, value),
            TextField::SyncPassword => self.on_sync_text(SyncField::Password, value),
            TextField::CatalogUrl => {
                let before = self.net.accounts.allow_entries();
                match self.net.accounts.add_catalog(value) {
                    Ok(_) => {
                        self.net.save();
                        self.net.update_allowlist(&before);
                        self.catalogs.message.clear();
                    }
                    Err(e) => self.catalogs.message = e.to_string(),
                }
            }
            TextField::Search => return self.search(value),
        }
        self.show_network(Refresh::Auto)
    }

    /// Results of network workers, between events.
    pub(crate) fn poll_net(&mut self) -> Result<(), Error> {
        for done in self.net.poll() {
            match done {
                Done::Login { register, result } => {
                    self.on_login(register, result);
                    if matches!(&self.screen, Screen::Network(ns) if matches!(ns.view, NetView::Sync))
                    {
                        self.show_network(Refresh::Auto)?;
                    }
                }
                Done::Pulled { book, result } => self.on_pulled(book, result)?,
                Done::Pushed { book, result } => match result {
                    None => perf!("sync: nothing KOReader could use for {book}; not sent"),
                    Some(Ok(())) => {}
                    Some(Err(e)) => self.sync.message = crate::net::describe_sync(&e),
                },
                Done::Feed { token, result } => self.on_feed(token, result)?,
                Done::Progress {
                    token,
                    bytes,
                    total,
                } => self.on_download_progress(token, bytes, total)?,
                Done::Downloaded { token, result } => self.on_downloaded(token, result)?,
            }
        }
        if self.sync.offer.is_some() {
            self.show_sync_offer()?;
        }
        Ok(())
    }
}
