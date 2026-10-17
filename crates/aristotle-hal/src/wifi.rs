// SPDX-License-Identifier: AGPL-3.0-or-later
//! Wi-Fi through the stock `wifid` and `cmd` daemons, over LIPC.
//!
//! Aristotle does not drive `wpa_supplicant` itself. It asks the stock
//! daemons, as KOReader and the stock Wi-Fi wizard do, by running the stock
//! command-line tools:
//!
//! | Tool | Use |
//! | --- | --- |
//! | `lipc-get-prop`, `lipc-set-prop` | ints and strings (`-i`, `-s`) |
//! | `lipc-hash-prop` | hash arrays: `scanList`, `currentEssid`, `createProfile` |
//! | `lipc-wait-event` | events while connecting (`profileUpdateOk`, `connectionAvailable`...) |
//!
//! | Operation | Calls |
//! | --- | --- |
//! | on | `cmd.wirelessEnable=1`, then `wifid.enable=1` (KOReader's order) |
//! | off | `wifid.enable=0`, then `cmd.wirelessEnable=0` |
//! | scan | `wifid.scan ""`, poll `scanState` until `idle`, read `scanList` |
//! | connect | `createProfile` (new networks only), then `cmd.ensureConnection "wifi:ESSID"` |
//! | forget | `wifid.deleteProfile "ESSID"` |
//!
//! Output formats and timings were captured on our PW4 (firmware 5.13.6):
//! see `docs/pw4-reference.md`, "Wi-Fi over LIPC", and the samples in the
//! tests below.
//!
//! # Secrets
//!
//! - The password reaches `wifid` only through `lipc-hash-prop`'s stdin,
//!   never its argv, which any process can read in `/proc/*/cmdline`.
//! - Nothing here reads `profileData`: it returns saved passwords.
//!   `scanList`'s `known` field says whether a network is saved.
//! - Errors from `createProfile` never carry the tool's stderr, whose parse
//!   errors could quote the input.
//! - Nothing raises `wifid`'s log level, whose debug format logs the
//!   password.
//! - How `lipc-hash-prop` escapes `"` and `}` inside a string is unknown, so
//!   names and passwords with `"`, `{`, `}`, `\` or control characters are
//!   refused before anything runs ([`check_text`]).

use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

pub const WIFID: &str = "com.lab126.wifid";
pub const CMD: &str = "com.lab126.cmd";

/// How long one call of a stock tool may take before it is killed.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// A scan took 0.8 s on our PW4; KOReader gives up after 20 s.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(20);
/// After switching on, `wifid` needed 4.3 s to leave `NA` on our PW4.
pub const READY_TIMEOUT: Duration = Duration::from_secs(12);
/// From `ensureConnection` to a verdict.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(40);
/// For `profileUpdateOk` after `createProfile` or `deleteProfile`.
pub const PROFILE_TIMEOUT: Duration = Duration::from_secs(10);

/// What went wrong, in terms the screen can explain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The stock tools are missing: not a Kindle, or the emulator.
    Unavailable,
    /// A tool did not answer in time. The name says which step.
    Timeout { what: &'static str, after: Duration },
    /// A tool failed. `detail` is safe to show and log.
    Failed { what: &'static str, detail: String },
    /// `wifid` rejected the password ("Bad password", "Key too short"...).
    BadPassword { reason: String },
    /// The connection did not come up; `reason` as `wifid` or `cmd` gave it.
    NotConnected { reason: String },
    /// A name or password `lipc-hash-prop` cannot take (see [`check_text`]).
    Unsupported { what: Field, character: char },
}

/// Which text [`Error::Unsupported`] is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Password,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unavailable => write!(f, "the stock LIPC tools are not available"),
            Error::Timeout { what, after } => {
                write!(f, "{what}: no answer in {} s", after.as_secs())
            }
            Error::Failed { what, detail } => write!(f, "{what}: {detail}"),
            Error::BadPassword { reason } => write!(f, "password rejected: {reason}"),
            Error::NotConnected { reason } => write!(f, "not connected: {reason}"),
            Error::Unsupported { what, character } => {
                write!(
                    f,
                    "{what:?} contains {character:?}, which lipc-hash-prop may not take"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

// ----------------------------------------------------------------- transport

/// One event from `lipc-wait-event`: its source, name and arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub source: String,
    pub name: String,
    pub args: Vec<String>,
}

/// Events from a running `lipc-wait-event`. Dropping it stops the tool.
pub struct Listener {
    rx: Receiver<Event>,
    child: Option<Child>,
}

impl Listener {
    /// A listener fed by someone else, for tests.
    pub fn from_channel(rx: Receiver<Event>) -> Listener {
        Listener { rx, child: None }
    }

    /// The next event, or `None` after `timeout` or when the tool ended.
    pub fn next(&self, timeout: Duration) -> Option<Event> {
        self.rx.recv_timeout(timeout).ok()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// The LIPC calls the Wi-Fi code makes. [`Cli`] runs the stock tools; tests
/// substitute a recording fake.
pub trait Lipc {
    fn get_int(&mut self, publisher: &str, prop: &str) -> Result<i64>;
    fn get_str(&mut self, publisher: &str, prop: &str) -> Result<String>;
    fn set_int(&mut self, publisher: &str, prop: &str, value: i64) -> Result<()>;
    fn set_str(&mut self, publisher: &str, prop: &str, value: &str) -> Result<()>;
    /// Access a hash-array property: `input` goes to stdin, `None` sends
    /// NULL (`-n`). Returns the tool's output. `secret` marks input that
    /// must not appear in errors.
    fn hash(
        &mut self,
        publisher: &str,
        prop: &str,
        input: Option<&str>,
        secret: bool,
    ) -> Result<String>;
    /// Start listening to every event of `sources`, for at most `limit`.
    fn listen(&mut self, sources: &[&str], limit: Duration) -> Result<Listener>;
}

/// The stock tools in `/usr/bin`, each call killed after [`CALL_TIMEOUT`].
pub struct Cli {
    pub bin: &'static str,
}

impl Default for Cli {
    fn default() -> Self {
        Cli { bin: "/usr/bin" }
    }
}

/// Output of a finished tool.
struct Run {
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Cli {
    fn tool(&self, name: &str) -> std::path::PathBuf {
        Path::new(self.bin).join(name)
    }

    /// Run `tool args`, feeding `stdin`, and wait at most [`CALL_TIMEOUT`].
    fn run(
        &self,
        what: &'static str,
        tool: &str,
        args: &[&str],
        stdin: Option<&str>,
    ) -> Result<Run> {
        let path = self.tool(tool);
        let mut child = Command::new(&path)
            .args(args)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    Error::Unavailable
                } else {
                    Error::Failed {
                        what,
                        detail: e.to_string(),
                    }
                }
            })?;
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            // Small (a hash array); fits the pipe buffer. Dropping the pipe
            // ends the input.
            let _ = pipe.write_all(input.as_bytes());
        }
        let out = reader(child.stdout.take());
        let err = reader(child.stderr.take());
        let deadline = Instant::now() + CALL_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Error::Timeout {
                        what,
                        after: CALL_TIMEOUT,
                    });
                }
            }
        };
        Ok(Run {
            status: status.code(),
            stdout: out.recv().unwrap_or_default(),
            stderr: err.recv().unwrap_or_default(),
        })
    }

    fn check(what: &'static str, r: Run) -> Result<String> {
        if r.status == Some(0) {
            Ok(r.stdout)
        } else {
            let msg = r.stderr.trim();
            Err(Error::Failed {
                what,
                detail: if msg.is_empty() {
                    format!("exit status {:?}", r.status)
                } else {
                    msg.lines().next().unwrap_or(msg).to_string()
                },
            })
        }
    }
}

/// Reads a pipe to the end on a thread of its own, so a full pipe never
/// stalls the tool.
fn reader<R: Read + Send + 'static>(pipe: Option<R>) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    match pipe {
        Some(mut p) => {
            thread::spawn(move || {
                let mut s = String::new();
                let _ = p.read_to_string(&mut s);
                let _ = tx.send(s);
            });
        }
        None => {
            let _ = tx.send(String::new());
        }
    }
    rx
}

impl Lipc for Cli {
    fn get_int(&mut self, publisher: &str, prop: &str) -> Result<i64> {
        let r = self.run(
            "lipc-get-prop",
            "lipc-get-prop",
            &["-i", publisher, prop],
            None,
        )?;
        let s = Self::check("lipc-get-prop", r)?;
        s.trim().parse().map_err(|_| Error::Failed {
            what: "lipc-get-prop",
            detail: format!("{prop}: not a number: {:?}", s.trim()),
        })
    }

    fn get_str(&mut self, publisher: &str, prop: &str) -> Result<String> {
        let r = self.run(
            "lipc-get-prop",
            "lipc-get-prop",
            &["-s", publisher, prop],
            None,
        )?;
        Ok(Self::check("lipc-get-prop", r)?
            .trim_end_matches('\n')
            .to_string())
    }

    fn set_int(&mut self, publisher: &str, prop: &str, value: i64) -> Result<()> {
        let v = value.to_string();
        let r = self.run(
            "lipc-set-prop",
            "lipc-set-prop",
            &["-i", publisher, prop, &v],
            None,
        )?;
        Self::check("lipc-set-prop", r).map(|_| ())
    }

    fn set_str(&mut self, publisher: &str, prop: &str, value: &str) -> Result<()> {
        let r = self.run(
            "lipc-set-prop",
            "lipc-set-prop",
            &["-s", publisher, prop, value],
            None,
        )?;
        Self::check("lipc-set-prop", r).map(|_| ())
    }

    fn hash(
        &mut self,
        publisher: &str,
        prop: &str,
        input: Option<&str>,
        secret: bool,
    ) -> Result<String> {
        let r = match input {
            None => self.run(
                "lipc-hash-prop",
                "lipc-hash-prop",
                &["-n", publisher, prop],
                None,
            )?,
            Some(i) => self.run(
                "lipc-hash-prop",
                "lipc-hash-prop",
                &[publisher, prop],
                Some(i),
            )?,
        };
        if secret && r.status != Some(0) {
            return Err(Error::Failed {
                what: "lipc-hash-prop",
                detail: format!("{prop}: exit status {:?}", r.status),
            });
        }
        Self::check("lipc-hash-prop", r)
    }

    fn listen(&mut self, sources: &[&str], limit: Duration) -> Result<Listener> {
        let secs = limit.as_secs().max(1).to_string();
        let mut child = Command::new(self.tool("lipc-wait-event"))
            .args(["-m", "-s", &secs, "-l", "*"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    Error::Unavailable
                } else {
                    Error::Failed {
                        what: "lipc-wait-event",
                        detail: e.to_string(),
                    }
                }
            })?;
        if let Some(mut pipe) = child.stdin.take() {
            let _ = pipe.write_all(format!("{}\n", sources.join("\n")).as_bytes());
        }
        let out = child.stdout.take().expect("piped");
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        thread::Builder::new()
            .name("lipc-events".into())
            .spawn(move || {
                let mut ready = Some(ready_tx);
                for line in io::BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    if line.starts_with("Waiting for events") {
                        if let Some(r) = ready.take() {
                            let _ = r.send(());
                        }
                        continue;
                    }
                    if let Some(e) = parse_event(&line) {
                        if tx.send(e).is_err() {
                            break;
                        }
                    }
                }
            })
            .map_err(|e| Error::Failed {
                what: "lipc-wait-event",
                detail: e.to_string(),
            })?;
        // The tool prints "Waiting for events..." once subscribed; an event
        // sent before that would be lost.
        let _ = ready_rx.recv_timeout(Duration::from_secs(2));
        Ok(Listener {
            rx,
            child: Some(child),
        })
    }
}

// ------------------------------------------------------------------- parsing

/// A value in `lipc-hash-prop` output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Int(i64),
    Blob(String),
}

/// `lipc-hash-prop` output: one list of `(key, value)` per hash. Each hash
/// starts with `Hash Index: N`; each field is a line of a tab and
/// `key = "string"` or `key = (int)`. Strings are printed between quotes
/// without escaping, so a value runs to the last quote on its line.
pub fn parse_hashes(s: &str) -> Vec<Vec<(String, Value)>> {
    let mut out: Vec<Vec<(String, Value)>> = Vec::new();
    for line in s.lines() {
        if line.starts_with("Hash Index:") {
            out.push(Vec::new());
            continue;
        }
        let Some(cur) = out.last_mut() else { continue };
        let Some((key, value)) = line.trim_start().split_once(" = ") else {
            continue;
        };
        let value = value.trim_end();
        let v = if let Some(inner) = value.strip_prefix('"') {
            match inner.rfind('"') {
                Some(end) => Value::Str(inner[..end].to_string()),
                None => continue,
            }
        } else if let Some(n) = value.strip_prefix('(').and_then(|v| v.strip_suffix(')')) {
            match n.trim().parse() {
                Ok(n) => Value::Int(n),
                Err(_) => continue,
            }
        } else {
            Value::Blob(value.to_string())
        };
        cur.push((key.trim().to_string(), v));
    }
    out
}

fn get_str<'a>(h: &'a [(String, Value)], key: &str) -> Option<&'a str> {
    h.iter().find_map(|(k, v)| match v {
        Value::Str(s) if k == key => Some(s.as_str()),
        _ => None,
    })
}

fn get_int(h: &[(String, Value)], key: &str) -> Option<i64> {
    h.iter().find_map(|(k, v)| match v {
        Value::Int(n) if k == key => Some(*n),
        _ => None,
    })
}

/// How a network is secured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    Open,
    /// WPA or WPA2 with a password (`key_mgmt` "WPA2 PSK" and the like).
    Psk,
    /// Enterprise (802.1X), WEP or anything else this screen cannot set up.
    Unsupported,
}

/// A network from `scanList`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    pub essid: String,
    /// Signal in steps of `0..=5`, as the stock bars show it.
    pub signal: u8,
    pub security: Security,
    /// `wifid` has a profile for it (`known = "yes"`).
    pub known: bool,
    /// `key_mgmt` as `wifid` reports it ("WPA2 PSK", "WPA2 EAP", "").
    pub key_mgmt: String,
}

/// The networks of a `scanList`, one per name with the strongest signal
/// first, hidden networks (empty names) left out. Fields as captured on our
/// PW4: `bssid`, `essid`, `secured`, `known`, `signal`, `signal_max`,
/// `supported`, `wps`, `enterprise`, `key_mgmt`.
pub fn parse_scan_list(s: &str) -> Vec<Network> {
    let mut nets: Vec<Network> = Vec::new();
    for h in parse_hashes(s) {
        let Some(essid) = get_str(&h, "essid").filter(|e| !e.is_empty()) else {
            continue;
        };
        let max = get_int(&h, "signal_max").filter(|m| *m > 0).unwrap_or(5);
        let signal = get_int(&h, "signal").unwrap_or(0).clamp(0, max);
        let signal = ((signal * 5 + max / 2) / max) as u8;
        let yes = |k| get_str(&h, k) == Some("yes");
        let key_mgmt = get_str(&h, "key_mgmt").unwrap_or("").to_string();
        let security = if !yes("secured") {
            Security::Open
        } else if yes("enterprise")
            || get_str(&h, "supported") == Some("no")
            || !key_mgmt.contains("PSK")
        {
            Security::Unsupported
        } else {
            Security::Psk
        };
        let net = Network {
            essid: essid.to_string(),
            signal,
            security,
            known: yes("known"),
            key_mgmt,
        };
        match nets.iter_mut().find(|n| n.essid == net.essid) {
            Some(n) => {
                n.known |= net.known;
                if net.signal > n.signal {
                    n.signal = net.signal;
                }
            }
            None => nets.push(net),
        }
    }
    // Saved networks first, then by signal; the order is stable for equals.
    nets.sort_by(|a, b| b.known.cmp(&a.known).then(b.signal.cmp(&a.signal)));
    nets
}

/// `currentEssid`: the network `wifid` is on, if any.
pub fn parse_current(s: &str) -> Option<String> {
    let h = parse_hashes(s).into_iter().next()?;
    let essid = get_str(&h, "essid").filter(|e| !e.is_empty())?;
    (get_str(&h, "connected") != Some("no")).then(|| essid.to_string())
}

/// One line of `lipc-wait-event -m` output, with or without `-t`:
///
/// ```text
/// [19:11:12.106780] com.lab126.wifid cmStateChange "PENDING"
/// com.lab126.cmd wirelessEnableChanged 1
/// ```
pub fn parse_event(line: &str) -> Option<Event> {
    let mut rest = line.trim();
    if rest.starts_with('[') {
        rest = rest.split_once(']')?.1.trim_start();
    }
    let (source, rest) = rest.split_once(' ')?;
    if !source.starts_with("com.") {
        return None;
    }
    let rest = rest.trim_start();
    let (name, mut rest) = rest.split_once(' ').unwrap_or((rest, ""));
    if name.is_empty() {
        return None;
    }
    let mut args = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(r) = rest.strip_prefix('"') {
            // A quoted argument ends at the next `" ` or at a final quote.
            let end = r.find("\" ").or_else(|| r.rfind('"')).unwrap_or(r.len());
            args.push(r[..end].to_string());
            rest = r.get(end + 1..).unwrap_or("");
        } else {
            let (a, r) = rest.split_once(' ').unwrap_or((rest, ""));
            args.push(a.to_string());
            rest = r;
        }
    }
    Some(Event {
        source: source.to_string(),
        name: name.to_string(),
        args,
    })
}

/// Refuses text the `lipc-hash-prop` input grammar might misread: `"`,
/// `{`, `}`, `\` and control characters. Returns the first such character.
pub fn check_text(s: &str) -> std::result::Result<(), char> {
    match s
        .chars()
        .find(|c| matches!(c, '"' | '{' | '}' | '\\') || c.is_control())
    {
        Some(c) => Err(c),
        None => Ok(()),
    }
}

/// A WPA password problem, for the keyboard screen to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordProblem {
    /// Fewer than 8 characters.
    TooShort,
    /// More than 63 characters.
    TooLong,
    /// Outside printable ASCII (WPA passphrases are 8 to 63 printable
    /// ASCII characters, IEEE 802.11i), or one of [`check_text`]'s.
    Character(char),
}

/// Checks a WPA passphrase before it is sent.
pub fn check_password(p: &str) -> std::result::Result<(), PasswordProblem> {
    if let Some(c) = p.chars().find(|c| !(' '..='~').contains(c)) {
        return Err(PasswordProblem::Character(c));
    }
    check_text(p).map_err(PasswordProblem::Character)?;
    match p.len() {
        0..=7 => Err(PasswordProblem::TooShort),
        8..=63 => Ok(()),
        _ => Err(PasswordProblem::TooLong),
    }
}

/// The `createProfile` input for `net`, in `lipc-hash-prop`'s grammar:
///
/// ```text
/// { essid = "Home", secured = "yes", psk = "...", store_nw_user_pref = (0) }
/// ```
///
/// `store_nw_user_pref = 0` keeps the password out of Amazon's backup of
/// saved networks (KOReader's comment). The result holds the password: pass
/// it on and drop it, never log it.
pub fn profile_input(essid: &str, psk: Option<&str>) -> Result<String> {
    check_text(essid).map_err(|c| Error::Unsupported {
        what: Field::Name,
        character: c,
    })?;
    Ok(match psk {
        Some(p) => {
            check_text(p).map_err(|c| Error::Unsupported {
                what: Field::Password,
                character: c,
            })?;
            format!(
                "{{ essid = \"{essid}\", secured = \"yes\", psk = \"{p}\", store_nw_user_pref = (0) }}\n"
            )
        }
        None => format!("{{ essid = \"{essid}\", secured = \"no\", store_nw_user_pref = (0) }}\n"),
    })
}

/// Overwrites a string's bytes before freeing it. Best effort: copies made
/// earlier (the keyboard's buffer, a pipe) are not reached.
pub fn wipe(s: String) {
    let mut b = s.into_bytes();
    b.fill(0);
    std::hint::black_box(&b);
}

// --------------------------------------------------------------------- state

/// `wifid`'s connection-manager state (`cmState`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CmState {
    /// `NA`: no interface (Wi-Fi off or starting).
    Na,
    Ready,
    Pending,
    Connected,
    /// `CONNECTED_LOCAL`: associated, but `wifid`'s internet check failed.
    ConnectedLocal,
    Disconnected,
    Other(String),
}

impl CmState {
    pub fn parse(s: &str) -> CmState {
        match s.trim() {
            "NA" | "" => CmState::Na,
            "READY" => CmState::Ready,
            "PENDING" => CmState::Pending,
            "CONNECTED" => CmState::Connected,
            "CONNECTED_LOCAL" => CmState::ConnectedLocal,
            "DISCONNECTED" => CmState::Disconnected,
            other => CmState::Other(other.to_string()),
        }
    }
}

/// What the Wi-Fi screen shows at the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// `cmd.wirelessEnable`.
    pub enabled: bool,
    pub state: CmState,
    /// The network in use, when connected.
    pub essid: Option<String>,
    /// `wlan0`'s IPv4 address, when it has one.
    pub ip: Option<String>,
}

/// Wi-Fi operations over any [`Lipc`].
pub struct Wifi<L: Lipc> {
    pub lipc: L,
    /// Pause between polls of `scanState` and `cmState`.
    pub poll: Duration,
}

impl Wifi<Cli> {
    pub fn stock() -> Self {
        Wifi::new(Cli::default())
    }
}

impl<L: Lipc> Wifi<L> {
    pub fn new(lipc: L) -> Self {
        Wifi {
            lipc,
            poll: Duration::from_millis(250),
        }
    }

    /// Whether the radio is on (`cmd.wirelessEnable`).
    pub fn enabled(&mut self) -> Result<bool> {
        Ok(self.lipc.get_int(CMD, "wirelessEnable")? != 0)
    }

    pub fn status(&mut self) -> Result<Status> {
        let enabled = self.enabled()?;
        if !enabled {
            return Ok(Status {
                enabled,
                state: CmState::Na,
                essid: None,
                ip: None,
            });
        }
        let state = CmState::parse(&self.lipc.get_str(WIFID, "cmState")?);
        let connected = matches!(state, CmState::Connected | CmState::ConnectedLocal);
        let essid = if connected {
            parse_current(&self.lipc.hash(WIFID, "currentEssid", None, false)?)
        } else {
            None
        };
        Ok(Status {
            enabled,
            state,
            essid,
            ip: if connected { ipv4_of("wlan0") } else { None },
        })
    }

    /// Switch the radio, in KOReader's order, then confirm it reads back.
    pub fn set_enabled(&mut self, on: bool) -> Result<()> {
        if on {
            self.lipc.set_int(CMD, "wirelessEnable", 1)?;
            self.lipc.set_int(WIFID, "enable", 1)?;
        } else {
            self.lipc.set_int(WIFID, "enable", 0)?;
            self.lipc.set_int(CMD, "wirelessEnable", 0)?;
        }
        if self.enabled()? != on {
            return Err(Error::Failed {
                what: "wirelessEnable",
                detail: format!("still {}", if on { 0 } else { 1 }),
            });
        }
        Ok(())
    }

    /// After switching on: wait until `wifid` has an interface (`cmState`
    /// leaves `NA`), at most `limit`.
    pub fn wait_ready(&mut self, limit: Duration) -> Result<()> {
        let start = Instant::now();
        loop {
            if CmState::parse(&self.lipc.get_str(WIFID, "cmState")?) != CmState::Na {
                return Ok(());
            }
            if start.elapsed() >= limit {
                return Err(Error::Timeout {
                    what: "wifid",
                    after: limit,
                });
            }
            thread::sleep(self.poll);
        }
    }

    /// Scan and return the networks ([`parse_scan_list`]).
    pub fn scan(&mut self, limit: Duration) -> Result<Vec<Network>> {
        self.lipc.set_str(WIFID, "scan", "")?;
        let start = Instant::now();
        let mut seen_scanning = false;
        loop {
            thread::sleep(self.poll);
            let state = self.lipc.get_str(WIFID, "scanState")?;
            if state.trim() == "idle" {
                // A scan that ended before the first poll never shows
                // "scanning"; 0.8 s is what one took on our PW4.
                if seen_scanning || start.elapsed() >= Duration::from_millis(1500) {
                    break;
                }
            } else {
                seen_scanning = true;
            }
            if start.elapsed() >= limit {
                return Err(Error::Timeout {
                    what: "scan",
                    after: limit,
                });
            }
        }
        Ok(parse_scan_list(
            &self.lipc.hash(WIFID, "scanList", None, false)?,
        ))
    }

    /// Connect to `net`. A network without a profile gets one first
    /// (`createProfile`), with `psk` for a secured one; if that connection
    /// then fails, the new profile is deleted again, so a mistyped password
    /// is not kept. Returns the final state.
    pub fn connect(&mut self, net: &Network, psk: Option<&str>) -> Result<CmState> {
        check_text(&net.essid).map_err(|c| Error::Unsupported {
            what: Field::Name,
            character: c,
        })?;
        let events = self
            .lipc
            .listen(&[WIFID, CMD], CONNECT_TIMEOUT + PROFILE_TIMEOUT)?;
        let mut created = false;
        if !net.known {
            let input = profile_input(&net.essid, psk)?;
            let r = self.lipc.hash(WIFID, "createProfile", Some(&input), true);
            wipe(input);
            r?;
            created = true;
            self.wait_profile(&events, "createProfile")?;
        }
        let r = self.ensure_connection(net, &events);
        if r.is_err() && created {
            // Best effort; the error that matters is the connection's.
            let _ = self.lipc.set_str(WIFID, "deleteProfile", &net.essid);
        }
        r
    }

    fn wait_profile(&mut self, events: &Listener, what: &'static str) -> Result<()> {
        let deadline = Instant::now() + PROFILE_TIMEOUT;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Some(e) = events.next(left) else { break };
            if e.source != WIFID {
                continue;
            }
            match e.name.as_str() {
                "profileUpdateOk" => return Ok(()),
                "profileUpdateFailed" => {
                    let reason = e.args.join(" ");
                    return Err(if is_password_reason(&reason) {
                        Error::BadPassword { reason }
                    } else {
                        Error::Failed {
                            what,
                            detail: reason,
                        }
                    });
                }
                _ => {}
            }
        }
        Err(Error::Timeout {
            what,
            after: PROFILE_TIMEOUT,
        })
    }

    fn ensure_connection(&mut self, net: &Network, events: &Listener) -> Result<CmState> {
        let target = format!("wifi:{}", net.essid);
        self.lipc.set_str(CMD, "ensureConnection", &target)?;
        let start = Instant::now();
        let mut last_poll = start;
        let mut pending = false;
        while start.elapsed() < CONNECT_TIMEOUT {
            if let Some(e) = events.next(self.poll) {
                match (e.source.as_str(), e.name.as_str()) {
                    (CMD, "connectionAvailable") if e.args.first() == Some(&target) => {
                        return Ok(CmState::Connected)
                    }
                    (CMD, "connectionNotAvailable") => {
                        let reason = e.args.get(1).cloned().unwrap_or_default();
                        if e.args.first() == Some(&target) || !reason.is_empty() {
                            return Err(not_connected(reason));
                        }
                    }
                    (WIFID, "cmConnectionFailed") => {
                        return Err(not_connected(e.args.join(" ")));
                    }
                    (WIFID, "cmStateChange") => match e.args.first().map(|s| CmState::parse(s)) {
                        Some(s @ (CmState::Connected | CmState::ConnectedLocal)) => return Ok(s),
                        Some(CmState::Pending) => pending = true,
                        _ => {}
                    },
                    _ => {}
                }
            }
            // The events are the fast path; a poll each second covers a
            // listener that missed them.
            if last_poll.elapsed() >= Duration::from_secs(1) {
                last_poll = Instant::now();
                match CmState::parse(&self.lipc.get_str(WIFID, "cmState")?) {
                    s @ (CmState::Connected | CmState::ConnectedLocal) => {
                        let on = self.lipc.hash(WIFID, "currentEssid", None, false)?;
                        if parse_current(&on).as_deref() == Some(net.essid.as_str()) {
                            return Ok(s);
                        }
                    }
                    CmState::Pending => pending = true,
                    CmState::Ready | CmState::Disconnected if pending => {
                        return Err(not_connected(String::new()))
                    }
                    _ => {}
                }
            }
        }
        Err(Error::Timeout {
            what: "connect",
            after: CONNECT_TIMEOUT,
        })
    }

    /// Delete the saved profile of `essid`. `wifid` (and the stock UI)
    /// refuse this for the network in use, so that one is dropped first by
    /// switching the radio off and on again.
    pub fn forget(&mut self, essid: &str, connected: bool) -> Result<()> {
        check_text(essid).map_err(|c| Error::Unsupported {
            what: Field::Name,
            character: c,
        })?;
        if connected {
            self.set_enabled(false)?;
        }
        let events = self.lipc.listen(&[WIFID], PROFILE_TIMEOUT)?;
        let r = self
            .lipc
            .set_str(WIFID, "deleteProfile", essid)
            .and_then(|_| self.wait_profile(&events, "deleteProfile"));
        if connected {
            self.set_enabled(true)?;
        }
        r
    }
}

fn is_password_reason(r: &str) -> bool {
    let r = r.to_ascii_lowercase();
    r.contains("password") || r.contains("key")
}

fn not_connected(reason: String) -> Error {
    if is_password_reason(&reason) {
        Error::BadPassword { reason }
    } else {
        Error::NotConnected { reason }
    }
}

/// The IPv4 address of `iface`, through `SIOCGIFADDR`.
pub fn ipv4_of(iface: &str) -> Option<String> {
    // SAFETY: a plain datagram socket, closed below; ifreq is plain data
    // that the kernel fills in.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            return None;
        }
        let mut req: libc::ifreq = std::mem::zeroed();
        for (d, s) in req
            .ifr_name
            .iter_mut()
            .zip(iface.bytes().take(libc::IFNAMSIZ - 1))
        {
            *d = s as libc::c_char;
        }
        let ok = libc::ioctl(fd, libc::SIOCGIFADDR as _, &mut req) == 0;
        libc::close(fd);
        if !ok {
            return None;
        }
        let sa = &*(&req.ifr_ifru.ifru_addr as *const libc::sockaddr as *const libc::sockaddr_in);
        let ip = std::net::Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr));
        Some(ip.to_string())
    }
}

// -------------------------------------------------------------------- policy

/// The device agent's Wi-Fi policy (`device/README.md`, "Network").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyMode {
    /// No rules of ours: the stock firewall, which allows all outbound
    /// traffic. Also what applies without the agent.
    Off,
    /// Outbound on `wlan0` only to the allowlist, DHCP and DNS.
    Scoped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// The agent is installed (`/var/local/aristotle/aristotlectl`).
    pub agent: bool,
    pub mode: PolicyMode,
    /// Entries in `net-allow`.
    pub hosts: usize,
    /// `/mnt/us/WIFI_NO_NET_PROBE` exists: `wifid` skips its internet
    /// check, which `scoped` blocks.
    pub no_probe: bool,
}

/// Reads the policy from the agent's state directory `dir`
/// (`/var/local/aristotle`) and the userstore root `us` (`/mnt/us`).
pub fn policy_in(dir: &Path, us: &Path) -> Policy {
    let mode = match fs::read_to_string(dir.join("net-mode"))
        .as_deref()
        .map(str::trim)
    {
        Ok("scoped") => PolicyMode::Scoped,
        _ => PolicyMode::Off,
    };
    Policy {
        agent: dir.join("aristotlectl").exists(),
        mode,
        hosts: fs::read_to_string(dir.join("net-allow"))
            .map(|s| count_allow(&s))
            .unwrap_or(0),
        no_probe: us.join("WIFI_NO_NET_PROBE").exists(),
    }
}

/// The policy on the device.
pub fn policy() -> Policy {
    policy_in(Path::new(crate::system::AGENT_DIR), Path::new("/mnt/us"))
}

/// Entries of a `net-allow` file: lines with something before any `#`.
pub fn count_allow(s: &str) -> usize {
    s.lines()
        .filter(|l| !l.split('#').next().unwrap_or("").trim().is_empty())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::mpsc::Sender;

    /// `lipc-hash-prop -n com.lab126.wifid scanList` on our PW4 with Wi-Fi
    /// just switched on (2026-10-17). Names and BSSIDs replaced; the format,
    /// fields, their order and the other values are as captured. One
    /// network was hidden (empty name).
    pub const SCAN_LIST: &str = "Hash Index: 0
\tbssid = \"02:00:00:00:00:01\"
\tessid = \"Biblioteka\"
\tsecured = \"no\"
\tknown = \"no\"
\tsignal = (5)
\tsignal_max = (5)
\tsupported = \"yes\"
\twps = \"no\"
\tenterprise = \"no\"
\tkey_mgmt = \"\"

Hash Index: 1
\tbssid = \"02:00:00:00:00:02\"
\tessid = \"Biblioteka\"
\tsecured = \"no\"
\tknown = \"no\"
\tsignal = (5)
\tsignal_max = (5)
\tsupported = \"yes\"
\twps = \"no\"
\tenterprise = \"no\"
\tkey_mgmt = \"\"

Hash Index: 2
\tbssid = \"02:00:00:00:00:03\"
\tessid = \"Uczelnia\"
\tsecured = \"yes\"
\tknown = \"no\"
\tsignal = (5)
\tsignal_max = (5)
\tsupported = \"yes\"
\twps = \"no\"
\tenterprise = \"yes\"
\tkey_mgmt = \"WPA2 EAP\"

Hash Index: 3
\tbssid = \"02:00:00:00:00:04\"
\tessid = \"\"
\tsecured = \"yes\"
\tknown = \"no\"
\tsignal = (1)
\tsignal_max = (5)
\tsupported = \"yes\"
\twps = \"no\"
\tenterprise = \"no\"
\tkey_mgmt = \"WPA2 PSK\"

Hash Index: 4
\tbssid = \"02:00:00:00:00:05\"
\tessid = \"Dom Wokulskiego\"
\tsecured = \"yes\"
\tknown = \"no\"
\tsignal = (1)
\tsignal_max = (5)
\tsupported = \"yes\"
\twps = \"yes\"
\tenterprise = \"no\"
\tkey_mgmt = \"WPA2 PSK\"

Hash Index: 5
\tbssid = \"02:00:00:00:00:06\"
\tessid = \"Soplicowo\"
\tsecured = \"yes\"
\tknown = \"yes\"
\tsignal = (1)
\tsignal_max = (5)
\tsupported = \"yes\"
\twps = \"yes\"
\tenterprise = \"no\"
\tkey_mgmt = \"WPA2 PSK\"

";

    /// `currentEssid` with Wi-Fi on and no connection, as captured.
    const CURRENT_NONE: &str = "Hash Index: 0
\tessid = \"\"
\tbssid = \"\"
\tconnected = \"no\"
\tsignal = (0)
\tsignal_max = (5)
\tsecured = \"no\"

";

    /// `lipc-wait-event -m -t -l '*'` (sources wifid and cmd) while Wi-Fi
    /// was switched on, scanned and switched off, as captured.
    const EVENTS: &str = "Event Source: com.lab126.wifid
Event Source: com.lab126.cmd
Waiting for events...
[19:11:08.336880] com.lab126.cmd wirelessEnableChanged 1
[19:11:12.106780] com.lab126.wifid cmStateChange \"PENDING\"
[19:11:12.113546] com.lab126.wifid scanning
[19:11:12.774415] com.lab126.wifid scanComplete
[19:11:15.027145] com.lab126.wifid signalStrength \"0/5\"
[19:11:15.029635] com.lab126.wifid profileUpdateOk
[19:11:15.352744] com.lab126.wifid cmDisconnected
[19:11:15.410050] com.lab126.cmd connectionNotAvailable \"\" \"\"
[19:11:16.390907] com.lab126.wifid cmStateChange \"NA\"
";

    #[test]
    fn scan_list_from_the_device() {
        let nets = parse_scan_list(SCAN_LIST);
        let names: Vec<&str> = nets.iter().map(|n| n.essid.as_str()).collect();
        // Saved first; the open network's two access points are one entry;
        // the hidden network is left out.
        assert_eq!(
            names,
            ["Soplicowo", "Biblioteka", "Uczelnia", "Dom Wokulskiego"]
        );
        assert!(nets[0].known);
        assert_eq!(nets[1].security, Security::Open);
        assert_eq!(nets[1].signal, 5);
        assert_eq!(nets[2].security, Security::Unsupported);
        assert_eq!(nets[2].key_mgmt, "WPA2 EAP");
        assert_eq!(nets[3].security, Security::Psk);
        assert_eq!(nets[3].signal, 1);
    }

    #[test]
    fn hashes_keep_quotes_inside_values_and_skip_junk() {
        let h = parse_hashes(
            "junk before\nHash Index: 0\n\tessid = \"a \"quoted\" name\"\n\tsignal = (3)\n\tblob = 0a0b\n\tbroken = (x)\n",
        );
        assert_eq!(h.len(), 1);
        assert_eq!(
            h[0],
            vec![
                ("essid".into(), Value::Str("a \"quoted\" name".into())),
                ("signal".into(), Value::Int(3)),
                ("blob".into(), Value::Blob("0a0b".into())),
            ]
        );
        assert!(parse_hashes("").is_empty());
    }

    #[test]
    fn signal_scales_to_five_steps() {
        let s = "Hash Index: 0\n\tessid = \"x\"\n\tsignal = (40)\n\tsignal_max = (100)\n";
        assert_eq!(parse_scan_list(s)[0].signal, 2);
    }

    #[test]
    fn current_network() {
        assert_eq!(parse_current(CURRENT_NONE), None);
        let on = "Hash Index: 0\n\tessid = \"Soplicowo\"\n\tconnected = \"yes\"\n";
        assert_eq!(parse_current(on).as_deref(), Some("Soplicowo"));
    }

    #[test]
    fn events_from_the_device() {
        let ev: Vec<Event> = EVENTS.lines().filter_map(parse_event).collect();
        assert_eq!(ev.len(), 9);
        assert_eq!(
            ev[0],
            Event {
                source: CMD.into(),
                name: "wirelessEnableChanged".into(),
                args: vec!["1".into()]
            }
        );
        assert_eq!(ev[1].args, ["PENDING"]);
        assert_eq!(ev[2].name, "scanning");
        assert!(ev[2].args.is_empty());
        assert_eq!(ev[7].name, "connectionNotAvailable");
        assert_eq!(ev[7].args, ["", ""]);
        // Without -t, and a reason with spaces.
        let e = parse_event("com.lab126.cmd connectionNotAvailable \"wifi:Dom\" \"Bad password\"")
            .unwrap();
        assert_eq!(e.args, ["wifi:Dom", "Bad password"]);
    }

    #[test]
    fn text_the_grammar_may_misread_is_refused() {
        assert_eq!(check_text("Soplicowo 5G"), Ok(()));
        assert_eq!(check_text("Pan \"T\""), Err('"'));
        assert_eq!(check_text("a}b"), Err('}'));
        assert_eq!(check_text("a\\b"), Err('\\'));
        assert_eq!(check_text("a\nb"), Err('\n'));
        assert_eq!(check_password("short"), Err(PasswordProblem::TooShort));
        assert_eq!(
            check_password(&"x".repeat(64)),
            Err(PasswordProblem::TooLong)
        );
        assert_eq!(
            check_password("zażółć gęślą"),
            Err(PasswordProblem::Character('ż'))
        );
        assert_eq!(
            check_password("ab{cdefgh"),
            Err(PasswordProblem::Character('{'))
        );
        assert_eq!(check_password("Litwo! Ojczyzno moja"), Ok(()));
    }

    #[test]
    fn profile_input_shape() {
        assert_eq!(
            profile_input("Dom Wokulskiego", Some("Izabela1878")).unwrap(),
            "{ essid = \"Dom Wokulskiego\", secured = \"yes\", psk = \"Izabela1878\", store_nw_user_pref = (0) }\n"
        );
        assert_eq!(
            profile_input("Biblioteka", None).unwrap(),
            "{ essid = \"Biblioteka\", secured = \"no\", store_nw_user_pref = (0) }\n"
        );
        assert!(matches!(
            profile_input("x", Some("pa\"ss")),
            Err(Error::Unsupported {
                what: Field::Password,
                character: '"'
            })
        ));
        // The input parses back as one hash with these fields.
        let back = parse_hashes(&format!(
            "Hash Index: 0\n{}",
            profile_input("a b", Some("p w"))
                .unwrap()
                .trim()
                .trim_matches(|c| c == '{' || c == '}')
                .split(", ")
                .map(|kv| format!("\t{kv}\n"))
                .collect::<String>()
        ));
        assert_eq!(get_str(&back[0], "psk"), Some("p w"));
        assert_eq!(get_int(&back[0], "store_nw_user_pref"), Some(0));
    }

    #[test]
    fn allowlist_and_policy_files() {
        assert_eq!(
            count_allow("# comment\n\ndash.example.org  # HTTPS\n192.168.1.10:8080\n   \n"),
            2
        );
        let dir =
            std::env::temp_dir().join(format!("aristotle-wifi-policy-{}", std::process::id()));
        let us = dir.join("us");
        fs::create_dir_all(&us).unwrap();
        assert_eq!(
            policy_in(&dir, &us),
            Policy {
                agent: false,
                mode: PolicyMode::Off,
                hosts: 0,
                no_probe: false
            }
        );
        fs::write(dir.join("net-mode"), "scoped\n").unwrap();
        fs::write(dir.join("net-allow"), "a.example.org\nb.example.org:8080\n").unwrap();
        fs::write(dir.join("aristotlectl"), "").unwrap();
        fs::write(us.join("WIFI_NO_NET_PROBE"), "").unwrap();
        assert_eq!(
            policy_in(&dir, &us),
            Policy {
                agent: true,
                mode: PolicyMode::Scoped,
                hosts: 2,
                no_probe: true
            }
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A scripted LIPC: answers from queues, records every call.
    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
        /// The stdin of each hash call.
        stdin: Vec<String>,
        strs: VecDeque<(&'static str, String)>,
        ints: VecDeque<(&'static str, i64)>,
        hashes: VecDeque<(&'static str, String)>,
        /// Events sent once a listener starts.
        events: Vec<&'static str>,
        listener: Option<Sender<Event>>,
    }

    impl Fake {
        /// A fake whose daemons send `events` after each string write.
        fn reacting(events: &[&'static str]) -> Fake {
            Fake {
                events: events.to_vec(),
                ..Fake::default()
            }
        }

        fn answer<T>(q: &mut VecDeque<(&'static str, T)>, prop: &str) -> T {
            let i = q
                .iter()
                .position(|(p, _)| *p == prop)
                .unwrap_or_else(|| panic!("no scripted answer for {prop}"));
            q.remove(i).unwrap().1
        }
    }

    impl Lipc for Fake {
        fn get_int(&mut self, p: &str, prop: &str) -> Result<i64> {
            self.calls.push(format!("get -i {p} {prop}"));
            Ok(Self::answer(&mut self.ints, prop))
        }
        fn get_str(&mut self, p: &str, prop: &str) -> Result<String> {
            self.calls.push(format!("get -s {p} {prop}"));
            Ok(Self::answer(&mut self.strs, prop))
        }
        fn set_int(&mut self, p: &str, prop: &str, v: i64) -> Result<()> {
            self.calls.push(format!("set -i {p} {prop} {v}"));
            Ok(())
        }
        fn set_str(&mut self, p: &str, prop: &str, v: &str) -> Result<()> {
            self.calls.push(format!("set -s {p} {prop} {v}"));
            // Events the daemons send in reaction.
            if let Some(tx) = &self.listener {
                for line in &self.events {
                    let _ = tx.send(parse_event(line).unwrap());
                }
            }
            Ok(())
        }
        fn hash(&mut self, p: &str, prop: &str, input: Option<&str>, _: bool) -> Result<String> {
            self.calls.push(format!(
                "hash {} {p} {prop}",
                if input.is_some() { "<stdin>" } else { "-n" }
            ));
            if let Some(i) = input {
                self.stdin.push(i.to_string());
                if let Some(tx) = &self.listener {
                    let _ = tx.send(parse_event("com.lab126.wifid profileUpdateOk").unwrap());
                }
                return Ok(String::new());
            }
            Ok(Self::answer(&mut self.hashes, prop))
        }
        fn listen(&mut self, sources: &[&str], _: Duration) -> Result<Listener> {
            self.calls.push(format!("listen {}", sources.join(",")));
            let (tx, rx) = mpsc::channel();
            self.listener = Some(tx);
            Ok(Listener::from_channel(rx))
        }
    }

    fn wifi(f: Fake) -> Wifi<Fake> {
        let mut w = Wifi::new(f);
        w.poll = Duration::from_millis(1);
        w
    }

    #[test]
    fn switching_follows_koreaders_order() {
        let mut f = Fake::default();
        f.ints.push_back(("wirelessEnable", 1));
        f.ints.push_back(("wirelessEnable", 0));
        let mut w = wifi(f);
        w.set_enabled(true).unwrap();
        w.set_enabled(false).unwrap();
        assert_eq!(
            w.lipc.calls,
            [
                "set -i com.lab126.cmd wirelessEnable 1",
                "set -i com.lab126.wifid enable 1",
                "get -i com.lab126.cmd wirelessEnable",
                "set -i com.lab126.wifid enable 0",
                "set -i com.lab126.cmd wirelessEnable 0",
                "get -i com.lab126.cmd wirelessEnable",
            ]
        );
    }

    #[test]
    fn scan_waits_for_idle_then_reads_the_list() {
        let mut f = Fake::default();
        for s in ["scanning", "scanning", "idle"] {
            f.strs.push_back(("scanState", s.into()));
        }
        f.hashes.push_back(("scanList", SCAN_LIST.into()));
        let mut w = wifi(f);
        assert_eq!(w.scan(SCAN_TIMEOUT).unwrap().len(), 4);
        assert_eq!(w.lipc.calls[0], "set -s com.lab126.wifid scan ");
        assert_eq!(
            w.lipc.calls.last().unwrap(),
            "hash -n com.lab126.wifid scanList"
        );
    }

    #[test]
    fn scan_times_out() {
        let mut f = Fake::default();
        for _ in 0..1000 {
            f.strs.push_back(("scanState", "scanning".into()));
        }
        let mut w = wifi(f);
        assert!(matches!(
            w.scan(Duration::from_millis(20)),
            Err(Error::Timeout { what: "scan", .. })
        ));
    }

    fn net(name: &str, known: bool) -> Network {
        Network {
            essid: name.into(),
            signal: 3,
            security: Security::Psk,
            known,
            key_mgmt: "WPA2 PSK".into(),
        }
    }

    #[test]
    fn connect_new_network_password_only_on_stdin() {
        let mut w = wifi(Fake::reacting(&[
            "com.lab126.wifid cmStateChange \"PENDING\"",
            "com.lab126.cmd connectionAvailable \"wifi:Dom Wokulskiego\"",
        ]));
        let psk = "Izabela1878";
        let s = w
            .connect(&net("Dom Wokulskiego", false), Some(psk))
            .unwrap();
        assert_eq!(s, CmState::Connected);
        assert_eq!(
            w.lipc.calls,
            [
                "listen com.lab126.wifid,com.lab126.cmd",
                "hash <stdin> com.lab126.wifid createProfile",
                "set -s com.lab126.cmd ensureConnection wifi:Dom Wokulskiego",
            ]
        );
        assert!(w.lipc.calls.iter().all(|c| !c.contains(psk)));
        assert_eq!(
            w.lipc.stdin,
            ["{ essid = \"Dom Wokulskiego\", secured = \"yes\", psk = \"Izabela1878\", store_nw_user_pref = (0) }\n"]
        );
    }

    #[test]
    fn a_rejected_password_deletes_the_new_profile() {
        let mut w = wifi(Fake::reacting(&[
            "com.lab126.cmd connectionNotAvailable \"wifi:Dom\" \"Bad password\"",
        ]));
        let e = w
            .connect(&net("Dom", false), Some("wrongpassword"))
            .unwrap_err();
        assert_eq!(
            e,
            Error::BadPassword {
                reason: "Bad password".into()
            }
        );
        assert_eq!(
            w.lipc.calls.last().unwrap(),
            "set -s com.lab126.wifid deleteProfile Dom"
        );
    }

    #[test]
    fn a_known_network_connects_without_a_profile() {
        let mut w = wifi(Fake::reacting(&[
            "com.lab126.wifid cmStateChange \"CONNECTED_LOCAL\"",
        ]));
        let s = w.connect(&net("Soplicowo", true), None).unwrap();
        assert_eq!(s, CmState::ConnectedLocal);
        assert!(w.lipc.calls.iter().all(|c| !c.contains("createProfile")));
        assert!(w.lipc.stdin.is_empty());
    }

    #[test]
    fn unsupported_names_never_reach_lipc() {
        let mut w = wifi(Fake::default());
        assert!(matches!(
            w.connect(&net("a\"b", false), Some("password1")),
            Err(Error::Unsupported {
                what: Field::Name,
                ..
            })
        ));
        assert!(w.lipc.calls.is_empty());
    }

    #[test]
    fn forgetting_the_network_in_use_cycles_the_radio() {
        let mut f = Fake::reacting(&["com.lab126.wifid profileUpdateOk"]);
        f.ints.push_back(("wirelessEnable", 0));
        f.ints.push_back(("wirelessEnable", 1));
        let mut w = wifi(f);
        w.forget("Soplicowo", true).unwrap();
        let calls = &w.lipc.calls;
        let del = calls
            .iter()
            .position(|c| c == "set -s com.lab126.wifid deleteProfile Soplicowo")
            .unwrap();
        assert!(calls[..del].contains(&"set -i com.lab126.cmd wirelessEnable 0".to_string()));
        assert!(calls[del..].contains(&"set -i com.lab126.cmd wirelessEnable 1".to_string()));
    }

    #[test]
    fn status_when_off_reads_one_property() {
        let mut f = Fake::default();
        f.ints.push_back(("wirelessEnable", 0));
        let mut w = wifi(f);
        let s = w.status().unwrap();
        assert!(!s.enabled);
        assert_eq!(w.lipc.calls.len(), 1);
    }

    #[test]
    fn missing_tools_mean_unavailable() {
        let mut cli = Cli {
            bin: "/nonexistent-aristotle-test",
        };
        assert_eq!(cli.get_int(CMD, "wirelessEnable"), Err(Error::Unavailable));
    }
}
