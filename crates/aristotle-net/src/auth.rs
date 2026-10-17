// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP authentication: Basic (RFC 7617) and Digest (RFC 7616 with the MD5
//! and MD5-sess algorithms, `qop=auth` or none).
//!
//! calibre's content server asks for Digest over plain HTTP and Basic over
//! HTTPS (`auth_mode = auto` in `calibre/srv/opts.py`); Calibre-Web, COPS
//! and most proxies ask for Basic. SHA-256 Digest is not offered by any of
//! them and is not implemented.

use md5::{Digest as _, Md5};

/// A user name and password for one catalog or server.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// Lowercase hex MD5.
pub fn md5_hex(data: &[u8]) -> String {
    hex(&Md5::digest(data))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `Authorization: Basic ...`.
pub fn basic(c: &Credentials) -> String {
    format!(
        "Basic {}",
        base64(format!("{}:{}", c.username, c.password).as_bytes())
    )
}

fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A Digest challenge from `WWW-Authenticate`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    /// `MD5` (the default) or `MD5-sess`.
    pub algorithm: Option<String>,
    /// Whether the server offers `qop=auth`.
    pub qop_auth: bool,
}

/// What a 401 asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scheme {
    Basic,
    Digest(Challenge),
}

/// The scheme to answer, from all `WWW-Authenticate` values: Digest with a
/// supported algorithm first, then Basic.
pub fn parse_challenges<'a>(values: impl IntoIterator<Item = &'a str>) -> Option<Scheme> {
    let mut basic = false;
    for v in values {
        // One header may hold several challenges; a new one starts with a
        // scheme token followed by a space.
        for (scheme, params) in split_challenges(v) {
            match scheme.to_ascii_lowercase().as_str() {
                "digest" => {
                    let mut c = Challenge::default();
                    for (k, val) in params {
                        match k.to_ascii_lowercase().as_str() {
                            "realm" => c.realm = val,
                            "nonce" => c.nonce = val,
                            "opaque" => c.opaque = Some(val),
                            "algorithm" => c.algorithm = Some(val),
                            "qop" => {
                                c.qop_auth = val
                                    .split(',')
                                    .any(|q| q.trim().eq_ignore_ascii_case("auth"))
                            }
                            _ => {}
                        }
                    }
                    let alg = c.algorithm.as_deref().unwrap_or("MD5");
                    if !c.nonce.is_empty()
                        && (alg.eq_ignore_ascii_case("MD5") || alg.eq_ignore_ascii_case("MD5-sess"))
                    {
                        return Some(Scheme::Digest(c));
                    }
                }
                "basic" => basic = true,
                _ => {}
            }
        }
    }
    basic.then_some(Scheme::Basic)
}

type Params = Vec<(String, String)>;

fn split_challenges(s: &str) -> Vec<(String, Params)> {
    let mut out: Vec<(String, Params)> = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    let skip = |i: &mut usize| {
        while *i < b.len() && (b[*i].is_ascii_whitespace() || b[*i] == b',') {
            *i += 1;
        }
    };
    loop {
        skip(&mut i);
        if i >= b.len() {
            break;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b',') {
            i += 1;
        }
        let token = &s[start..i];
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if j < b.len() && b[j] == b'=' {
            // A parameter of the current challenge.
            i = j + 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            let value = if i < b.len() && b[i] == b'"' {
                i += 1;
                let mut v = String::new();
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        i += 1;
                    }
                    let ch = s[i..].chars().next().unwrap_or(' ');
                    v.push(ch);
                    i += ch.len_utf8();
                }
                i += 1;
                v
            } else {
                let st = i;
                while i < b.len() && b[i] != b',' {
                    i += 1;
                }
                s[st..i].trim().to_string()
            };
            if let Some(last) = out.last_mut() {
                last.1.push((token.to_string(), value));
            }
        } else {
            out.push((token.to_string(), Vec::new()));
            i = j;
        }
    }
    out
}

/// The `Authorization: Digest ...` value for one request. `nc` counts the
/// requests made with this nonce, from 1; `cnonce` is the client's random
/// nonce.
pub fn digest(
    c: &Credentials,
    ch: &Challenge,
    method: &str,
    uri: &str,
    nc: u32,
    cnonce: &str,
) -> String {
    let mut ha1 = md5_hex(format!("{}:{}:{}", c.username, ch.realm, c.password).as_bytes());
    let sess = ch
        .algorithm
        .as_deref()
        .is_some_and(|a| a.eq_ignore_ascii_case("MD5-sess"));
    if sess {
        ha1 = md5_hex(format!("{ha1}:{}:{cnonce}", ch.nonce).as_bytes());
    }
    let ha2 = md5_hex(format!("{method}:{uri}").as_bytes());
    let nc = format!("{nc:08x}");
    let response = if ch.qop_auth {
        md5_hex(format!("{ha1}:{}:{nc}:{cnonce}:auth:{ha2}", ch.nonce).as_bytes())
    } else {
        md5_hex(format!("{ha1}:{}:{ha2}", ch.nonce).as_bytes())
    };
    let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut h = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{response}\"",
        q(&c.username),
        q(&ch.realm),
        q(&ch.nonce),
        q(uri)
    );
    if let Some(a) = &ch.algorithm {
        h.push_str(&format!(", algorithm={a}"));
    }
    if ch.qop_auth {
        h.push_str(&format!(", qop=auth, nc={nc}, cnonce=\"{cnonce}\""));
    }
    if let Some(o) = &ch.opaque {
        h.push_str(&format!(", opaque=\"{}\"", q(o)));
    }
    h
}

/// 16 random bytes as hex: `/dev/urandom`, else the clock and the process.
pub fn random_hex() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_ok();
    if !ok {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let seed = format!("{t}:{}:{:p}", std::process::id(), &b);
        b.copy_from_slice(&Md5::digest(seed.as_bytes()));
    }
    hex(&b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mufasa() -> Credentials {
        Credentials {
            username: "Mufasa".into(),
            password: "Circle Of Life".into(),
        }
    }

    #[test]
    fn md5_and_base64_vectors() {
        // The KOSync key of "test", as the design note pins it.
        assert_eq!(md5_hex(b"test"), "098f6bcd4621d373cade4e832627b4f6");
        // RFC 4648 section 10.
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
        // RFC 7617 section 2.
        let c = Credentials {
            username: "Aladdin".into(),
            password: "open sesame".into(),
        };
        assert_eq!(basic(&c), "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
        assert!(!format!("{c:?}").contains("sesame"));
    }

    /// RFC 2617 section 3.5 (the MD5 example RFC 7616 keeps).
    #[test]
    fn digest_rfc_2617_example() {
        let header = r#"Digest
                 realm="testrealm@host.com",
                 qop="auth,auth-int",
                 nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093",
                 opaque="5ccc069c403ebaf9f0171e9517f40e41""#;
        let Some(Scheme::Digest(ch)) = parse_challenges([header]) else {
            panic!("no digest challenge");
        };
        assert_eq!(ch.realm, "testrealm@host.com");
        assert!(ch.qop_auth);
        let h = digest(&mufasa(), &ch, "GET", "/dir/index.html", 1, "0a4f113b");
        assert!(
            h.contains(r#"response="6629fae49393a05397450978507c4ef1""#),
            "{h}"
        );
        assert!(h.contains("nc=00000001"), "{h}");
        assert!(h.contains(r#"opaque="5ccc069c403ebaf9f0171e9517f40e41""#));
    }

    /// RFC 7616 section 3.9.1 with MD5: same inputs as its SHA-256 example.
    #[test]
    fn digest_rfc_7616_md5_example() {
        let header = r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=MD5, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#;
        let Some(Scheme::Digest(ch)) = parse_challenges([header]) else {
            panic!();
        };
        let c = Credentials {
            username: "Mufasa".into(),
            password: "Circle of Life".into(),
        };
        let h = digest(
            &c,
            &ch,
            "GET",
            "/dir/index.html",
            1,
            "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ",
        );
        assert!(
            h.contains(r#"response="8ca523f5e9506fed4657c9700eebdbec""#),
            "{h}"
        );
    }

    #[test]
    fn picks_digest_over_basic_and_skips_unknown_algorithms() {
        let calibre = r#"Digest realm="calibre", nonce="abc", algorithm="MD5", qop="auth""#;
        let s = parse_challenges([r#"Basic realm="x""#, calibre]);
        assert!(
            matches!(s, Some(Scheme::Digest(ref c)) if c.realm == "calibre" && c.nonce == "abc")
        );
        let s = parse_challenges([
            r#"Digest realm="x", nonce="n", algorithm=SHA-256, Basic realm="x""#,
        ]);
        assert_eq!(s, Some(Scheme::Basic));
        assert_eq!(parse_challenges([r#"Bearer realm="x""#]), None);
        // No qop: the RFC 2069 form.
        let Some(Scheme::Digest(ch)) = parse_challenges([r#"Digest realm="r", nonce="n""#]) else {
            panic!();
        };
        let h = digest(&mufasa(), &ch, "GET", "/", 1, "c");
        assert!(!h.contains("qop"));
        assert_eq!(random_hex().len(), 32);
    }
}
