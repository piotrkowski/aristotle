// SPDX-License-Identifier: AGPL-3.0-or-later
//! Absolute `http` and `https` URLs: parsing, reference resolution (RFC 3986
//! section 5.2, for links in feeds and `Location` headers) and the
//! `host:port` the allowlist speaks.
//!
//! Only what the clients here need. Bytes that may not appear in a request
//! target (spaces, non-ASCII, `"<>\^`{|}`) are percent-encoded when parsing,
//! so a sloppy `href` from a feed still makes a valid request.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Url {
    https: bool,
    /// Lower case; IPv6 literals keep their brackets.
    host: String,
    port: u16,
    /// Path and query, starting with `/`; no fragment.
    target: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlError(pub String);

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bad address {:?}", self.0)
    }
}

impl std::error::Error for UrlError {}

impl Url {
    /// An absolute `http://` or `https://` URL. Credentials in the
    /// authority (`user:pass@`) are refused: they belong in the settings,
    /// not in an address that gets logged.
    pub fn parse(s: &str) -> Result<Url, UrlError> {
        let bad = || UrlError(s.to_string());
        let s = s.trim();
        let (scheme, rest) = s.split_once("://").ok_or_else(bad)?;
        let https = match scheme.to_ascii_lowercase().as_str() {
            "http" => false,
            "https" => true,
            _ => return Err(bad()),
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);
        if authority.contains('@') || authority.is_empty() {
            return Err(bad());
        }
        let (host, port) = split_host_port(authority).ok_or_else(bad)?;
        let port = match port {
            Some(p) => p,
            None if https => 443,
            None => 80,
        };
        let tail = tail.split('#').next().unwrap_or("");
        let target = if tail.is_empty() || tail.starts_with('?') {
            format!("/{tail}")
        } else {
            tail.to_string()
        };
        Ok(Url {
            https,
            host,
            port,
            target: encode_target(&target),
        })
    }

    /// `reference` resolved against this URL, as a browser would follow a
    /// link on this page.
    pub fn join(&self, reference: &str) -> Result<Url, UrlError> {
        let r = reference.trim();
        let r = r.split('#').next().unwrap_or("");
        if let Some(i) = r.find(':') {
            let scheme = &r[..i];
            if !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
                && !r[..i].contains('/')
            {
                return Url::parse(r);
            }
        }
        if let Some(rest) = r.strip_prefix("//") {
            let scheme = if self.https { "https" } else { "http" };
            return Url::parse(&format!("{scheme}://{rest}"));
        }
        let (base_path, base_query) = split_query(&self.target);
        let target = if r.is_empty() {
            match base_query {
                Some(q) => format!("{base_path}?{q}"),
                None => base_path.to_string(),
            }
        } else if r.starts_with('?') {
            format!("{base_path}{r}")
        } else {
            let (path, query) = split_query(r);
            let merged = if path.starts_with('/') {
                path.to_string()
            } else {
                let dir = &base_path[..base_path.rfind('/').map_or(0, |i| i + 1)];
                format!("{dir}{path}")
            };
            let mut t = remove_dot_segments(&merged);
            if t.is_empty() {
                t.push('/');
            }
            match query {
                Some(q) => format!("{t}?{q}"),
                None => t,
            }
        };
        Ok(Url {
            https: self.https,
            host: self.host.clone(),
            port: self.port,
            target: encode_target(&target),
        })
    }

    pub fn https(&self) -> bool {
        self.https
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Path and query.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// The path without the query.
    pub fn path(&self) -> &str {
        split_query(&self.target).0
    }

    /// `host:port`, the form of an allowlist entry.
    pub fn host_port(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Scheme, host and port: credentials go only to the origin they were
    /// given for.
    pub fn same_origin(&self, other: &Url) -> bool {
        self.https == other.https && self.host == other.host && self.port == other.port
    }

    /// This URL with `segment` appended to its path (a server base URL
    /// plus an API route), any query dropped.
    pub fn with_path(&self, segment: &str) -> Url {
        let base = self.path().trim_end_matches('/');
        Url {
            target: encode_target(&format!("{base}/{}", segment.trim_start_matches('/'))),
            ..self.clone()
        }
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.https { "https" } else { "http" };
        let default = if self.https { 443 } else { 80 };
        write!(f, "{scheme}://{}", self.host)?;
        if self.port != default {
            write!(f, ":{}", self.port)?;
        }
        f.write_str(&self.target)
    }
}

fn split_query(s: &str) -> (&str, Option<&str>) {
    match s.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (s, None),
    }
}

fn split_host_port(authority: &str) -> Option<(String, Option<u16>)> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let close = rest.find(']')?;
        let host = &authority[..close + 2];
        let after = &rest[close + 1..];
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p),
            None if after.is_empty() => None,
            None => return None,
        };
        (host, port)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    if host.is_empty()
        || !host.starts_with('[')
            && !host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return None;
    }
    let port = match port {
        Some(p) => Some(p.parse::<u16>().ok().filter(|&p| p > 0)?),
        None => None,
    };
    Some((host.to_ascii_lowercase(), port))
}

/// RFC 3986 section 5.2.4.
fn remove_dot_segments(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let absolute = path.starts_with('/');
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len() - 1;
    for (i, seg) in segments.iter().enumerate() {
        match *seg {
            "." => {
                if i == last {
                    out.push("");
                }
            }
            ".." => {
                if out.len() > 1 || (!absolute && !out.is_empty()) {
                    out.pop();
                }
                if i == last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    let mut s = out.join("/");
    if absolute && !s.starts_with('/') {
        s.insert(0, '/');
    }
    s
}

/// Percent-encodes what may not stand in a request target; existing escapes
/// stay as they are.
fn encode_target(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_graphic() && !b"\"<>\\^`{|}".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Percent-encodes a query value (`application/x-www-form-urlencoded`
/// rules aside: spaces become `%20`, which every server reads).
pub fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints() {
        let u = Url::parse("HTTP://Example.org:8080/opds?x=1#top").unwrap();
        assert_eq!(
            (u.host(), u.port(), u.target()),
            ("example.org", 8080, "/opds?x=1")
        );
        assert_eq!(u.to_string(), "http://example.org:8080/opds?x=1");
        let u = Url::parse("https://sync.example.org").unwrap();
        assert_eq!((u.port(), u.target()), (443, "/"));
        assert_eq!(u.host_port(), "sync.example.org:443");
        assert_eq!(u.to_string(), "https://sync.example.org/");
        assert_eq!(
            Url::parse("http://[fe80::1]:81/a").unwrap().host_port(),
            "[fe80::1]:81"
        );
        for bad in [
            "ftp://x/",
            "example.org",
            "http://user:pw@x/",
            "http://x:0/",
            "http://x:70000/",
            "http://a b/",
            "http:///path",
        ] {
            assert!(Url::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn resolves_references_like_rfc_3986() {
        // RFC 3986 section 5.4.1, with http instead of the opaque base.
        let base = Url::parse("http://a/b/c/d;p?q").unwrap();
        let cases = [
            ("g", "http://a/b/c/g"),
            ("./g", "http://a/b/c/g"),
            ("g/", "http://a/b/c/g/"),
            ("/g", "http://a/g"),
            ("//g", "http://g/"),
            ("?y", "http://a/b/c/d;p?y"),
            ("g?y", "http://a/b/c/g?y"),
            ("#s", "http://a/b/c/d;p?q"),
            ("", "http://a/b/c/d;p?q"),
            (".", "http://a/b/c/"),
            ("./", "http://a/b/c/"),
            ("..", "http://a/b/"),
            ("../g", "http://a/b/g"),
            ("../..", "http://a/"),
            ("../../g", "http://a/g"),
            ("../../../g", "http://a/g"),
            ("/./g", "http://a/g"),
            ("g.", "http://a/b/c/g."),
            ("g/../h", "http://a/b/c/h"),
            ("https://other:8443/x", "https://other:8443/x"),
        ];
        for (r, want) in cases {
            assert_eq!(base.join(r).unwrap().to_string(), want, "{r}");
        }
    }

    #[test]
    fn encodes_what_a_target_cannot_hold() {
        let base = Url::parse("http://h/opds/").unwrap();
        assert_eq!(
            base.join("get/Pan Tadeusz.epub").unwrap().target(),
            "/opds/get/Pan%20Tadeusz.epub"
        );
        assert_eq!(
            base.join("a%20b?q=ł").unwrap().target(),
            "/opds/a%20b?q=%C5%82"
        );
        assert_eq!(encode_component("Lalka Prus"), "Lalka%20Prus");
        assert_eq!(encode_component("a&b=c/ż"), "a%26b%3Dc%2F%C5%BC");
    }

    #[test]
    fn api_routes_join_onto_a_base() {
        let u = Url::parse("http://h:7200").unwrap();
        assert_eq!(u.with_path("users/auth").target(), "/users/auth");
        let u = Url::parse("https://h/kosync/").unwrap();
        assert_eq!(
            u.with_path("/syncs/progress/abc").target(),
            "/kosync/syncs/progress/abc"
        );
        assert!(u.same_origin(&Url::parse("https://H:443/x").unwrap()));
        assert!(!u.same_origin(&Url::parse("http://h/x").unwrap()));
    }
}
