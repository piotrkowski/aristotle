// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Wi-Fi screen's model and its worker thread.
//!
//! Every LIPC call (`aristotle_hal::wifi`) runs on the worker: switching
//! takes seconds (4.3 s from on to an interface on our PW4), a scan about
//! 0.8 s, a connection up to 40 s. The input loop sends [`Job`]s, polls
//! [`Done`]s between events, updates the [`Model`] and rebuilds the screen,
//! so the panel shows each step as one partial update.
//!
//! `--wifi-read-only` refuses connecting and forgetting before any LIPC
//! call, for device tests on a Kindle whose networks are not ours.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use aristotle_hal::wifi::{self as hal, CmState, Field, Network, Security, Status, Wifi};
use aristotle_i18n::{tf, tr, Msg};
use aristotle_ui::screens::wifi::{ListState, NetworkRow, Policy};

/// Work for the worker.
pub enum Job {
    Status,
    /// Switch the radio. On is followed by a scan.
    Enable(bool),
    Scan,
    Connect {
        net: Network,
        /// The password of a new secured network. Wiped after use.
        psk: Option<String>,
    },
    Forget {
        essid: String,
        connected: bool,
    },
}

/// What the worker finished.
pub enum Done {
    Status(hal::Result<Status>),
    Enabled {
        on: bool,
        result: hal::Result<()>,
    },
    Scanned(hal::Result<Vec<Network>>),
    Connected {
        essid: String,
        result: hal::Result<CmState>,
    },
    Forgot {
        essid: String,
        result: hal::Result<()>,
    },
}

pub struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    /// Jobs sent and not finished.
    pending: usize,
    last_status: Instant,
}

/// How often the screen refreshes the state while it is open and idle.
pub const STATUS_EVERY: Duration = Duration::from_secs(5);

impl Worker {
    pub fn start(read_only: bool) -> Worker {
        let (jobs, rx) = mpsc::channel::<Job>();
        let (tx, done) = mpsc::channel();
        let spawned = thread::Builder::new().name("wifi".into()).spawn(move || {
            let mut w = Wifi::stock();
            for job in rx {
                let mut out = Vec::new();
                run(&mut w, job, read_only, &mut out);
                for d in out {
                    if tx.send(d).is_err() {
                        return;
                    }
                }
            }
        });
        if let Err(e) = spawned {
            eprintln!("aristotle: wifi: can't start the worker: {e}");
        }
        Worker {
            jobs,
            done,
            pending: 0,
            last_status: Instant::now(),
        }
    }

    pub fn send(&mut self, job: Job) {
        if matches!(job, Job::Status) {
            self.last_status = Instant::now();
        }
        // A job is answered by one Done; switching on by two.
        self.pending += if matches!(job, Job::Enable(true)) {
            2
        } else {
            1
        };
        if self.jobs.send(job).is_err() {
            self.pending = 0;
        }
    }

    pub fn poll(&mut self) -> Option<Done> {
        let d = self.done.try_recv().ok()?;
        self.pending = self.pending.saturating_sub(1);
        Some(d)
    }

    pub fn busy(&self) -> bool {
        self.pending > 0
    }

    /// Whether a periodic status refresh is due.
    pub fn status_due(&self) -> bool {
        !self.busy() && self.last_status.elapsed() >= STATUS_EVERY
    }
}

fn read_only_error() -> hal::Error {
    hal::Error::Failed {
        what: "read-only",
        detail: "--wifi-read-only".into(),
    }
}

fn run(w: &mut Wifi<hal::Cli>, job: Job, read_only: bool, out: &mut Vec<Done>) {
    let t = Instant::now();
    match job {
        Job::Status => out.push(Done::Status(w.status())),
        Job::Enable(on) => {
            let mut result = w.set_enabled(on);
            if on && result.is_ok() {
                result = w.wait_ready(hal::READY_TIMEOUT);
            }
            crate::perf!(
                "wifi: {} in {:.0} ms: {result:?}",
                if on { "on" } else { "off" },
                crate::ms(t.elapsed())
            );
            let ok = result.is_ok();
            out.push(Done::Enabled { on, result });
            if on {
                let t = Instant::now();
                let scanned = if ok {
                    w.scan(hal::SCAN_TIMEOUT)
                } else {
                    Ok(Vec::new())
                };
                log_scan(&scanned, t);
                out.push(Done::Scanned(scanned));
            }
        }
        Job::Scan => {
            let r = w.scan(hal::SCAN_TIMEOUT);
            log_scan(&r, t);
            out.push(Done::Scanned(r));
        }
        Job::Connect { net, psk } => {
            let result = if read_only {
                Err(read_only_error())
            } else {
                w.connect(&net, psk.as_deref())
            };
            if let Some(p) = psk {
                hal::wipe(p);
            }
            // The name is the reader's own data: not logged.
            crate::perf!(
                "wifi: connect finished in {:.0} ms: {}",
                crate::ms(t.elapsed()),
                match &result {
                    Ok(s) => format!("{s:?}"),
                    Err(e) => error_kind(e),
                }
            );
            out.push(Done::Connected {
                essid: net.essid,
                result,
            });
        }
        Job::Forget { essid, connected } => {
            let result = if read_only {
                Err(read_only_error())
            } else {
                w.forget(&essid, connected)
            };
            crate::perf!(
                "wifi: forget finished in {:.0} ms: {}",
                crate::ms(t.elapsed()),
                result.as_ref().map_or_else(error_kind, |_| "ok".into())
            );
            out.push(Done::Forgot { essid, result });
        }
    }
}

fn log_scan(r: &hal::Result<Vec<Network>>, t: Instant) {
    crate::perf!(
        "wifi: scan in {:.0} ms: {}",
        crate::ms(t.elapsed()),
        match r {
            Ok(n) => format!("{} networks", n.len()),
            Err(e) => error_kind(e),
        }
    );
}

/// An error for the log, without names (they are the reader's).
fn error_kind(e: &hal::Error) -> String {
    match e {
        hal::Error::BadPassword { .. } => "password rejected".into(),
        hal::Error::NotConnected { .. } => "not connected".into(),
        hal::Error::Unsupported { what, .. } => format!("unsupported character in {what:?}"),
        other => other.to_string(),
    }
}

/// What the Wi-Fi screen is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    TurningOn,
    TurningOff,
    Scanning,
    Connecting(String),
    Forgetting(String),
}

/// The screen's state, kept while other screens are shown so a connection
/// can finish meanwhile.
pub struct Model {
    pub status: Option<Status>,
    /// The stock tools are missing.
    pub unavailable: bool,
    pub networks: Vec<Network>,
    pub phase: Phase,
    /// The last error, for the state line; cleared by the next action.
    pub error: Option<String>,
    /// The saved network whose "Forget" was tapped once.
    pub armed: Option<String>,
    pub policy: Policy,
    /// The settings page to go back to.
    pub settings_page: usize,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            status: None,
            unavailable: false,
            networks: Vec::new(),
            phase: Phase::Idle,
            error: None,
            armed: None,
            policy: Policy::Stock,
            settings_page: 0,
        }
    }
}

/// The message for an error about network `name`.
pub fn message(e: &hal::Error, name: &str) -> String {
    match e {
        hal::Error::Unavailable => tr(Msg::WifiUnavailable).to_string(),
        hal::Error::Timeout { after, .. } => tf!(Msg::WifiErrorTimeout, s = after.as_secs()),
        hal::Error::Failed { what, detail } => {
            tf!(Msg::WifiErrorFailed, detail = format!("{what}: {detail}"))
        }
        hal::Error::BadPassword { .. } => tf!(Msg::WifiErrorPassword, name = name),
        hal::Error::NotConnected { reason } if reason.is_empty() => {
            tf!(Msg::WifiErrorConnect, name = name)
        }
        hal::Error::NotConnected { reason } => {
            tf!(Msg::WifiErrorConnectReason, name = name, reason = reason)
        }
        hal::Error::Unsupported {
            what: Field::Name,
            character,
        } => tf!(Msg::WifiNameChar, c = character),
        hal::Error::Unsupported {
            what: Field::Password,
            character,
        } => tf!(Msg::WifiPasswordChar, c = character),
    }
}

/// The keyboard's check for a WPA password.
pub fn check_password(p: &str) -> Result<(), String> {
    hal::check_password(p).map_err(|e| match e {
        hal::PasswordProblem::TooShort => tr(Msg::WifiPasswordShort).to_string(),
        hal::PasswordProblem::TooLong => tr(Msg::WifiPasswordLong).to_string(),
        hal::PasswordProblem::Character(c) => tf!(Msg::WifiPasswordChar, c = c),
    })
}

impl Model {
    pub fn enabled(&self) -> bool {
        match self.phase {
            Phase::TurningOn => true,
            Phase::TurningOff => false,
            _ => self.status.as_ref().is_some_and(|s| s.enabled),
        }
    }

    pub fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// The network in use.
    pub fn connected(&self) -> Option<&str> {
        let s = self.status.as_ref()?;
        matches!(s.state, CmState::Connected | CmState::ConnectedLocal)
            .then_some(s.essid.as_deref())
            .flatten()
    }

    /// The line under the switch.
    pub fn state_line(&self) -> String {
        match &self.phase {
            Phase::TurningOn => return tr(Msg::WifiTurningOn).into(),
            Phase::TurningOff => return tr(Msg::WifiTurningOff).into(),
            Phase::Connecting(n) => return tf!(Msg::WifiConnecting, name = n),
            Phase::Forgetting(n) => return tf!(Msg::WifiForgetting, name = n),
            Phase::Idle | Phase::Scanning => {}
        }
        if let Some(e) = &self.error {
            return e.clone();
        }
        self.summary()
    }

    /// The state alone, for the settings row.
    pub fn summary(&self) -> String {
        if self.unavailable {
            return tr(Msg::WifiUnavailable).into();
        }
        let Some(s) = &self.status else {
            return String::new();
        };
        if !s.enabled {
            return tr(Msg::WifiOff).into();
        }
        match (&s.state, s.essid.as_deref(), s.ip.as_deref()) {
            (CmState::Connected, Some(n), Some(ip)) => tf!(Msg::WifiConnectedIp, name = n, ip = ip),
            (CmState::Connected, Some(n), None) => tf!(Msg::WifiConnected, name = n),
            (CmState::ConnectedLocal, Some(n), _) => tf!(Msg::WifiConnectedLocal, name = n),
            _ => tr(Msg::WifiOn).into(),
        }
    }

    pub fn list_state(&self) -> ListState {
        if !self.enabled() || self.unavailable {
            ListState::Off
        } else if self.networks.is_empty()
            && matches!(self.phase, Phase::Scanning | Phase::TurningOn)
        {
            ListState::Scanning
        } else {
            ListState::Networks
        }
    }

    pub fn rows(&self) -> Vec<NetworkRow<'_>> {
        let on = self.connected();
        self.networks
            .iter()
            .map(|n| NetworkRow {
                name: &n.essid,
                signal: n.signal,
                secured: n.security != Security::Open,
                saved: n.known,
                connected: on == Some(n.essid.as_str()),
                supported: n.security != Security::Unsupported,
                forget_armed: self.armed.as_deref() == Some(n.essid.as_str()),
            })
            .collect()
    }

    /// Takes in what the worker finished. Returns jobs to send next.
    pub fn apply(&mut self, d: Done) -> Vec<Job> {
        match d {
            Done::Status(Ok(s)) => {
                self.unavailable = false;
                if !s.enabled {
                    self.networks.clear();
                }
                self.status = Some(s);
                Vec::new()
            }
            Done::Status(Err(e)) => {
                self.unavailable = e == hal::Error::Unavailable;
                if !self.unavailable {
                    self.error = Some(message(&e, ""));
                }
                Vec::new()
            }
            Done::Enabled { on, result } => {
                match result {
                    Ok(()) => {
                        self.phase = if on { Phase::Scanning } else { Phase::Idle };
                        if !on {
                            self.networks.clear();
                        }
                    }
                    Err(e) => {
                        self.phase = if on { Phase::Scanning } else { Phase::Idle };
                        self.error = Some(message(&e, ""));
                    }
                }
                vec![Job::Status]
            }
            Done::Scanned(r) => {
                if self.phase == Phase::Scanning {
                    self.phase = Phase::Idle;
                }
                match r {
                    Ok(n) => self.networks = n,
                    Err(e) => self.error = Some(message(&e, "")),
                }
                Vec::new()
            }
            Done::Connected { essid, result } => {
                self.phase = Phase::Idle;
                if let Err(e) = result {
                    self.error = Some(message(&e, &essid));
                }
                vec![Job::Status, Job::Scan]
            }
            Done::Forgot { essid, result } => {
                self.phase = Phase::Idle;
                if let Err(e) = result {
                    self.error = Some(message(&e, &essid));
                }
                vec![Job::Status, Job::Scan]
            }
        }
    }

    /// The policy as the device agent keeps it.
    pub fn read_policy(&mut self) {
        let p = hal::policy();
        self.policy = match p.mode {
            hal::PolicyMode::Off => Policy::Stock,
            hal::PolicyMode::Scoped => Policy::Scoped {
                hosts: p.hosts,
                no_probe: p.no_probe,
            },
        };
    }
}

/// What the screen shows, to tell whether a worker result changed it.
fn signature(m: &Model) -> String {
    format!(
        "{}|{:?}|{}|{:?}",
        m.state_line(),
        m.list_state(),
        m.busy(),
        m.rows()
    )
}

impl crate::App {
    /// The worker, started on first use.
    fn wifi_worker(&mut self) -> &mut Worker {
        let read_only = self.wifi_read_only;
        self.wifi.get_or_insert_with(|| Worker::start(read_only))
    }

    /// Read the state now, on this thread (a few LIPC calls of about
    /// 10 ms each), so a screen opens with it.
    pub(crate) fn refresh_wifi_status(&mut self) {
        let t = Instant::now();
        let r = Wifi::stock().status();
        crate::perf!("wifi: status read in {:.0} ms", crate::ms(t.elapsed()));
        self.wifi_model.apply(Done::Status(r));
    }

    /// Open the Wi-Fi screen from the settings, which were on `page`.
    pub(crate) fn open_wifi(&mut self, settings_page: usize) -> Result<(), crate::Error> {
        self.wifi_model.settings_page = settings_page;
        self.wifi_model.read_policy();
        self.wifi_model.error = None;
        self.wifi_model.armed = None;
        self.refresh_wifi_status();
        if self.wifi_model.enabled() && !self.wifi_model.busy() {
            self.wifi_model.phase = Phase::Scanning;
            self.wifi_worker().send(Job::Scan);
        }
        self.show_wifi(aristotle_ui::tree::Refresh::Full)
    }

    /// Build the Wi-Fi screen from the model: `Full` for a screen switch,
    /// `Auto` for a change of the screen already shown.
    pub(crate) fn show_wifi(
        &mut self,
        refresh: aristotle_ui::tree::Refresh,
    ) -> Result<(), crate::Error> {
        use aristotle_ui::screens::wifi::{WifiScreen, WifiView};
        let page = match &self.screen {
            crate::Screen::Wifi { view } => self.ui.tree.page(view.list).0,
            _ => 0,
        };
        let facts = crate::device::Facts::status(&self.battery);
        self.clock = facts.clock.clone();
        let m = &self.wifi_model;
        let state = m.state_line();
        let rows = m.rows();
        let v = WifiView {
            status: facts.status_line(),
            on: m.enabled(),
            state: &state,
            busy: m.busy(),
            list: m.list_state(),
            networks: &rows,
            policy: m.policy,
        };
        let (tree, view) = WifiScreen::build(&mut self.ui.text, &self.styles, &v);
        self.ui.set_tree_with(tree, refresh);
        self.ui.layout();
        if page > 0 {
            self.ui.tree.set_page(view.list, page);
            self.ui.layout();
        }
        view.sync(&mut self.ui.tree);
        self.ui.gesture_config_mut().long_press_us = 500_000;
        self.screen = crate::Screen::Wifi { view };
        self.render_ui()
    }

    pub(crate) fn on_wifi_event(
        &mut self,
        ev: aristotle_ui::ui::UiEvent,
    ) -> Result<crate::Flow, crate::Error> {
        use aristotle_ui::screens::wifi::action as a;
        use aristotle_ui::tree::Refresh;
        let aristotle_ui::ui::UiEvent::Action { id, .. } = ev else {
            return Ok(crate::Flow::Continue);
        };
        let crate::Screen::Wifi { view } = &self.screen else {
            return Ok(crate::Flow::Continue);
        };
        if view.handle(&mut self.ui.tree, id) {
            self.ui.layout();
            view.sync(&mut self.ui.tree);
            return Ok(crate::Flow::Continue);
        }
        let busy = self.wifi_model.busy();
        let m = &mut self.wifi_model;
        match id {
            a::BACK => {
                let page = m.settings_page;
                return self
                    .show_settings(Refresh::Full, page)
                    .map(|_| crate::Flow::Continue);
            }
            a::TOGGLE if !busy && !m.unavailable => {
                let on = !m.enabled();
                m.error = None;
                m.armed = None;
                m.phase = if on {
                    Phase::TurningOn
                } else {
                    Phase::TurningOff
                };
                self.wifi_worker().send(Job::Enable(on));
            }
            a::SCAN if !busy => {
                m.error = None;
                m.armed = None;
                m.phase = Phase::Scanning;
                self.wifi_worker().send(Job::Scan);
            }
            id if id >= a::FORGET && !busy => {
                let Some(net) = m.networks.get((id - a::FORGET) as usize) else {
                    return Ok(crate::Flow::Continue);
                };
                let essid = net.essid.clone();
                m.error = None;
                if m.armed.as_deref() == Some(essid.as_str()) {
                    m.armed = None;
                    let connected = m.connected() == Some(essid.as_str());
                    m.phase = Phase::Forgetting(essid.clone());
                    self.wifi_worker().send(Job::Forget { essid, connected });
                } else {
                    m.armed = Some(essid);
                }
            }
            id if id >= a::NETWORK && !busy => {
                let Some(net) = m.networks.get((id - a::NETWORK) as usize).cloned() else {
                    return Ok(crate::Flow::Continue);
                };
                m.armed = None;
                m.error = None;
                if let Err(c) = hal::check_text(&net.essid) {
                    m.error = Some(message(
                        &hal::Error::Unsupported {
                            what: Field::Name,
                            character: c,
                        },
                        "",
                    ));
                } else if net.security == Security::Psk && !net.known {
                    return self.ask_wifi_password(net).map(|_| crate::Flow::Continue);
                } else {
                    return self
                        .wifi_connect(net, None, Refresh::Auto)
                        .map(|_| crate::Flow::Continue);
                }
            }
            _ => return Ok(crate::Flow::Continue),
        }
        self.show_wifi(Refresh::Auto)?;
        Ok(crate::Flow::Continue)
    }

    fn ask_wifi_password(&mut self, net: Network) -> Result<(), crate::Error> {
        use crate::ask::TextRequest;
        use aristotle_ui::tree::Refresh;
        let title = tf!(Msg::WifiPasswordTitle, name = &net.essid);
        self.ask_text(TextRequest {
            hint: Some(tr(Msg::WifiPasswordHint).into()),
            password: true,
            submit: tr(Msg::WifiConnect).into(),
            max_chars: 63,
            validate: Some(check_password),
            ..TextRequest::new(
                title,
                Box::new(move |app, text| match text {
                    Some(psk) => app.wifi_connect(net, Some(psk), Refresh::Full),
                    None => app.show_wifi(Refresh::Full),
                }),
            )
        })
    }

    /// Start connecting to `net` and show the Wi-Fi screen saying so.
    pub(crate) fn wifi_connect(
        &mut self,
        net: Network,
        psk: Option<String>,
        refresh: aristotle_ui::tree::Refresh,
    ) -> Result<(), crate::Error> {
        self.wifi_model.phase = Phase::Connecting(net.essid.clone());
        self.wifi_model.error = None;
        self.wifi_worker().send(Job::Connect { net, psk });
        self.show_wifi(refresh)
    }

    /// Take in the worker's results; redraw the Wi-Fi screen if they
    /// changed it. Called between events.
    pub(crate) fn poll_wifi(&mut self) -> Result<(), crate::Error> {
        let on_screen = matches!(self.screen, crate::Screen::Wifi { .. });
        let Some(w) = self.wifi.as_mut() else {
            return Ok(());
        };
        let before = signature(&self.wifi_model);
        while let Some(d) = w.poll() {
            for job in self.wifi_model.apply(d) {
                w.send(job);
            }
        }
        if on_screen && w.status_due() && !self.wifi_model.busy() {
            w.send(Job::Status);
        }
        if on_screen && signature(&self.wifi_model) != before {
            self.show_wifi(aristotle_ui::tree::Refresh::Auto)?;
        }
        Ok(())
    }

    /// Whether the loop should poll the worker soon.
    pub(crate) fn wifi_busy(&self) -> bool {
        self.wifi.as_ref().is_some_and(|w| w.busy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(enabled: bool, state: CmState, essid: Option<&str>) -> Status {
        Status {
            enabled,
            state,
            essid: essid.map(String::from),
            ip: None,
        }
    }

    #[test]
    fn state_lines() {
        aristotle_i18n::with_language("en", || {
            let mut m = Model::default();
            assert_eq!(m.state_line(), "");
            m.apply(Done::Status(Ok(status(false, CmState::Na, None))));
            assert_eq!(m.state_line(), "Off");
            assert_eq!(m.list_state(), ListState::Off);
            m.phase = Phase::TurningOn;
            assert_eq!(m.state_line(), "Turning on…");
            assert_eq!(m.list_state(), ListState::Scanning);
            let next = m.apply(Done::Enabled {
                on: true,
                result: Ok(()),
            });
            assert!(matches!(next[..], [Job::Status]));
            assert_eq!(m.phase, Phase::Scanning);
            m.apply(Done::Status(Ok(status(true, CmState::Ready, None))));
            m.apply(Done::Scanned(Ok(hal::parse_scan_list(SCAN))));
            assert_eq!(m.phase, Phase::Idle);
            assert_eq!(m.state_line(), "On, not connected");
            assert_eq!(m.list_state(), ListState::Networks);
            m.apply(Done::Status(Ok(status(
                true,
                CmState::Connected,
                Some("Soplicowo"),
            ))));
            assert_eq!(m.state_line(), "Connected to Soplicowo");
            assert!(m.rows()[0].connected);
        });
    }

    const SCAN: &str = "Hash Index: 0\n\tessid = \"Soplicowo\"\n\tsecured = \"yes\"\n\tknown = \"yes\"\n\tsignal = (4)\n\tsignal_max = (5)\n\tkey_mgmt = \"WPA2 PSK\"\n";

    #[test]
    fn errors_name_the_network() {
        aristotle_i18n::with_language("pl", || {
            let mut m = Model::default();
            m.phase = Phase::Connecting("Dom".into());
            let next = m.apply(Done::Connected {
                essid: "Dom".into(),
                result: Err(hal::Error::BadPassword {
                    reason: "Bad password".into(),
                }),
            });
            assert_eq!(next.len(), 2);
            assert_eq!(m.state_line(), "Złe hasło do sieci Dom");
            assert_eq!(
                message(
                    &hal::Error::Timeout {
                        what: "scan",
                        after: Duration::from_secs(20)
                    },
                    ""
                ),
                "Usługa Wi-Fi nie odpowiedziała w ciągu 20 s"
            );
        });
    }

    #[test]
    fn password_check_messages() {
        aristotle_i18n::with_language("en", || {
            assert_eq!(check_password("Izabela1878"), Ok(()));
            assert_eq!(check_password("abc").unwrap_err(), "At least 8 characters");
            assert_eq!(
                check_password("zażółć gęślą").unwrap_err(),
                "Wi-Fi passwords can't contain “ż”"
            );
        });
    }
}
