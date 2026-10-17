// SPDX-License-Identifier: AGPL-3.0-or-later
//! The device's Wi-Fi policy as the app sees it (`device/README.md`,
//! "Network"): the mode in `net-mode` and the hosts in `net-allow`, both in
//! the agent's directory (`/var/local/aristotle`).
//!
//! In `scoped` mode the firewall drops traffic to every host not listed. The
//! client checks each request and each redirect against the same list
//! first, so a missing entry fails at once with "<host> is not in the
//! allowed hosts" instead of a connection timeout. In `off` mode, and when
//! the agent is not installed, the stock firewall applies and every host
//! may be reached.
//!
//! The entry grammar follows `parse()` in `device/extension/agent/net.sh`:
//! `host[:port][/udp|/tcp]`, port 443 by default, `#` starts a comment. Lines
//! net.sh rejects are skipped here too. UDP entries never allow HTTP.

use std::path::Path;

use crate::url::Url;

/// One TCP entry of `net-allow`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allowed {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    /// `scoped` mode: only `allowed` may be reached.
    pub scoped: bool,
    pub allowed: Vec<Allowed>,
}

impl Policy {
    /// No restriction: the stock firewall, or tests.
    pub fn open() -> Policy {
        Policy::default()
    }

    /// The policy the agent applies, from its directory. Unreadable files
    /// count as missing: no `net-mode` is `off`, no `net-allow` is empty.
    pub fn load(agent_dir: &Path) -> Policy {
        let mode = std::fs::read_to_string(agent_dir.join("net-mode")).unwrap_or_default();
        let allow = std::fs::read_to_string(agent_dir.join("net-allow")).unwrap_or_default();
        Policy {
            scoped: mode.trim() == "scoped",
            allowed: parse_allow(&allow),
        }
    }

    pub fn allows(&self, url: &Url) -> bool {
        !self.scoped
            || self
                .allowed
                .iter()
                .any(|a| a.port == url.port() && a.host.eq_ignore_ascii_case(url.host()))
    }
}

/// The TCP entries of a `net-allow` file, as net.sh reads them.
pub fn parse_allow(text: &str) -> Vec<Allowed> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        let Some(entry) = line.split_whitespace().next() else {
            continue;
        };
        if let Some(a) = parse_entry(entry) {
            out.push(a);
        }
    }
    out
}

/// One `host[:port][/tcp]` word; `None` for UDP and for what net.sh
/// rejects.
pub fn parse_entry(entry: &str) -> Option<Allowed> {
    if entry.ends_with("/udp") {
        return None;
    }
    let entry = entry.strip_suffix("/tcp").unwrap_or(entry);
    let (host, port) = match entry.split_once(':') {
        Some((h, p)) => (h, p),
        None => (entry, "443"),
    };
    if host.is_empty()
        || host.starts_with('-')
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let port: u16 = port.parse().ok().filter(|&p| p >= 1)?;
    Some(Allowed {
        host: host.to_ascii_lowercase(),
        port,
    })
}

/// The `net-allow` entry that lets the device reach `url`, if net.sh can
/// express it (an IPv6 literal cannot be).
pub fn entry_for(url: &Url) -> Option<String> {
    let e = url.host_port();
    parse_entry(&e).map(|_| e)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixture of device/tests/net.sh: the same lines are accepted.
    #[test]
    fn reads_net_allow_like_net_sh() {
        let fixture = include_str!("../../../device/tests/net/net-allow");
        let got: Vec<String> = parse_allow(fixture)
            .iter()
            .map(|a| format!("{} {}", a.host, a.port))
            .collect();
        let want: Vec<String> = include_str!("../../../device/tests/net/expected-parse")
            .lines()
            .filter(|l| l.ends_with(" tcp"))
            .map(|l| l.trim_end_matches(" tcp").to_string())
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn scoped_mode_checks_host_and_port() {
        let p = Policy {
            scoped: true,
            allowed: parse_allow("Calibre.lan:8080 # aristotle:opds\nsync.example.org\n"),
        };
        let ok = |s: &str| p.allows(&Url::parse(s).unwrap());
        assert!(ok("http://calibre.lan:8080/opds"));
        assert!(!ok("http://calibre.lan/opds"));
        assert!(ok("https://sync.example.org/users/auth"));
        assert!(!ok("http://sync.example.org/users/auth"));
        assert!(!ok("https://elsewhere.example.org/"));
        assert!(Policy::open().allows(&Url::parse("http://anything/").unwrap()));
        assert_eq!(
            entry_for(&Url::parse("http://192.168.15.201:17200/").unwrap()).as_deref(),
            Some("192.168.15.201:17200")
        );
        assert_eq!(entry_for(&Url::parse("http://[::1]/").unwrap()), None);
    }
}
