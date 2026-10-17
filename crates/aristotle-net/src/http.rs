// SPDX-License-Identifier: AGPL-3.0-or-later
//! A small blocking HTTP client over `ureq`: timeouts, a body size limit,
//! the device's host allowlist on every hop, and Basic or Digest auth sent
//! only to the origin the credentials belong to.
//!
//! Redirects are followed here, not by `ureq`, so each target is checked
//! against the [`Policy`] before any connection: at most
//! [`MAX_REDIRECTS`], for `GET` only. No proxy is used, whatever the
//! environment says, because a proxy would carry traffic to hosts the
//! allowlist does not name.
//!
//! Every call blocks; the app runs them on worker threads.

use std::fmt;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::auth::{self, Credentials, Scheme};
use crate::policy::Policy;
use crate::url::{Url, UrlError};

pub const MAX_REDIRECTS: usize = 5;
const USER_AGENT: &str = concat!("Aristotle/", env!("CARGO_PKG_VERSION"));

#[derive(Debug)]
pub enum Error {
    Url(UrlError),
    /// The device's policy does not allow this `host:port`.
    NotAllowed(String),
    /// `https://` in a build without the `tls` feature.
    NoTls,
    /// The server could not be reached: DNS, refused, unreachable.
    Connect(String),
    Timeout,
    /// The body is larger than the caller allows.
    TooLarge(u64),
    TooManyRedirects,
    /// The server asked for credentials we don't have, or refused them.
    Unauthorized,
    /// A status the caller did not expect, with the start of the body.
    Status(u16, String),
    /// TLS failed; `clock` when the certificate is not valid at the
    /// device's time, which usually means a wrong clock.
    Tls {
        clock: bool,
        detail: String,
    },
    Cancelled,
    Io(std::io::Error),
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Url(e) => e.fmt(f),
            Error::NotAllowed(h) => write!(f, "{h} is not in the allowed hosts"),
            Error::NoTls => f.write_str("this build has no HTTPS"),
            Error::Connect(d) => write!(f, "can't connect: {d}"),
            Error::Timeout => f.write_str("the server did not answer in time"),
            Error::TooLarge(n) => write!(f, "larger than {n} bytes"),
            Error::TooManyRedirects => f.write_str("too many redirects"),
            Error::Unauthorized => f.write_str("the user name or password was refused"),
            Error::Status(s, body) if body.is_empty() => write!(f, "HTTP {s}"),
            Error::Status(s, body) => write!(f, "HTTP {s}: {body}"),
            Error::Tls { detail, .. } => write!(f, "TLS: {detail}"),
            Error::Cancelled => f.write_str("cancelled"),
            Error::Io(e) => e.fmt(f),
            Error::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

impl From<UrlError> for Error {
    fn from(e: UrlError) -> Self {
        Error::Url(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

fn from_ureq(e: ureq::Error) -> Error {
    use ureq::Error as U;
    match e {
        U::Timeout(_) => Error::Timeout,
        U::HostNotFound => Error::Connect("host not found".into()),
        U::ConnectionFailed => Error::Connect("connection failed".into()),
        U::BodyExceedsLimit(n) => Error::TooLarge(n),
        U::TooManyRedirects => Error::TooManyRedirects,
        U::Io(io) => match io.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => Error::Timeout,
            std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::HostUnreachable
            | std::io::ErrorKind::NetworkUnreachable
            | std::io::ErrorKind::AddrNotAvailable => Error::Connect(io.to_string()),
            _ => tls_or_io(io),
        },
        U::Tls(d) => Error::Tls {
            clock: false,
            detail: d.into(),
        },
        other => {
            let s = other.to_string();
            if s.contains("https") && s.to_ascii_lowercase().contains("tls") {
                Error::NoTls
            } else {
                Error::Other(s)
            }
        }
    }
}

/// rustls reports certificate problems as I/O errors wrapping its own;
/// an expired or not-yet-valid certificate on a device with a good
/// certificate store is nearly always a wrong clock.
fn tls_or_io(io: std::io::Error) -> Error {
    let s = io.to_string();
    let lower = s.to_ascii_lowercase();
    if lower.contains("certificate") || lower.contains("tls") {
        let clock = lower.contains("expired") || lower.contains("notvalidyet");
        Error::Tls { clock, detail: s }
    } else {
        Error::Io(io)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
        }
    }
}

/// One request. Build with [`Request::get`] and friends.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: Method,
    pub url: Url,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// Sent to `url`'s origin only, never across a redirect to another.
    pub credentials: Option<Credentials>,
}

impl Request {
    pub fn get(url: Url) -> Request {
        Request {
            method: Method::Get,
            url,
            headers: Vec::new(),
            body: None,
            credentials: None,
        }
    }

    pub fn post(url: Url, body: Vec<u8>) -> Request {
        Request {
            method: Method::Post,
            body: Some(body),
            ..Request::get(url)
        }
    }

    pub fn put(url: Url, body: Vec<u8>) -> Request {
        Request {
            method: Method::Put,
            body: Some(body),
            ..Request::get(url)
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Request {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn credentials(mut self, c: Option<Credentials>) -> Request {
        self.credentials = c;
        self
    }
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    /// Where the body came from, after redirects.
    pub url: Url,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Timeouts for one kind of work.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    /// Connecting, including DNS.
    pub connect: Duration,
    /// From sending the request to the end of the response headers.
    pub response: Duration,
    /// Reading the whole body.
    pub body: Duration,
}

impl Timeouts {
    /// Small API calls: everything within a few seconds.
    pub fn quick(total: Duration) -> Timeouts {
        Timeouts {
            connect: total,
            response: total,
            body: total,
        }
    }
}

pub struct Client {
    agent: ureq::Agent,
    policy: Policy,
    /// The scheme the last challenged origin asked for (Digest with its
    /// nonce count), so later requests authenticate without a 401 first.
    auth: Mutex<Option<(Url, AuthState)>>,
}

enum AuthState {
    Basic,
    Digest(auth::Challenge, u32),
}

/// What a download wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Downloaded {
    pub bytes: u64,
    pub url: Url,
    pub content_type: Option<String>,
    /// The file name the server suggests (`Content-Disposition`).
    pub filename: Option<String>,
}

impl Client {
    pub fn new(policy: Policy, t: Timeouts) -> Client {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .proxy(None)
            .user_agent(USER_AGENT)
            .timeout_resolve(Some(t.connect))
            .timeout_connect(Some(t.connect))
            .timeout_send_request(Some(t.response))
            .timeout_send_body(Some(t.response))
            .timeout_recv_response(Some(t.response))
            .timeout_recv_body(Some(t.body))
            .build();
        Client {
            agent: ureq::Agent::new_with_config(config),
            policy,
            auth: Mutex::new(None),
        }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Send `req` and read the body, at most `limit` bytes. Any status
    /// comes back as a response except 401 with credentials refused, which
    /// is [`Error::Unauthorized`].
    pub fn send(&self, req: &Request, limit: u64) -> Result<Response, Error> {
        let (mut resp, url) = self.exchange(req)?;
        let content_type = header(&resp, "content-type");
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(from_ureq)?;
        Ok(Response {
            status,
            url,
            content_type,
            body,
        })
    }

    /// Stream a `GET` into `out`: at most `limit` bytes, checking `cancel`
    /// between reads and reporting `(bytes so far, total if known)`.
    pub fn download(
        &self,
        req: &Request,
        out: &mut dyn Write,
        limit: u64,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<Downloaded, Error> {
        let (mut resp, url) = self.exchange(req)?;
        let status = resp.status().as_u16();
        if status != 200 {
            let mut body = Vec::new();
            let _ = resp
                .body_mut()
                .with_config()
                .limit(4096)
                .reader()
                .read_to_end(&mut body);
            return Err(status_error(status, &body));
        }
        let total = resp.body().content_length();
        if total.is_some_and(|t| t > limit) {
            return Err(Error::TooLarge(limit));
        }
        let content_type = header(&resp, "content-type");
        let filename = header(&resp, "content-disposition").and_then(|d| disposition_name(&d));
        let mut reader = resp.body_mut().with_config().limit(limit).reader();
        let mut buf = vec![0u8; 64 * 1024];
        let mut bytes = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(io_from_body(e, limit)),
            };
            out.write_all(&buf[..n])?;
            bytes += n as u64;
            progress(bytes, total);
        }
        if total.is_some_and(|t| t != bytes) {
            return Err(Error::Other(format!(
                "the download stopped after {bytes} of {} bytes",
                total.unwrap_or(0)
            )));
        }
        Ok(Downloaded {
            bytes,
            url,
            content_type,
            filename,
        })
    }

    /// Send, following redirects and answering one auth challenge. The
    /// response's body is still unread.
    fn exchange(&self, req: &Request) -> Result<(ureq::http::Response<ureq::Body>, Url), Error> {
        let mut url = req.url.clone();
        let mut redirects = 0;
        let mut challenged = false;
        loop {
            if !self.policy.allows(&url) {
                return Err(Error::NotAllowed(url.host_port()));
            }
            #[cfg(not(feature = "tls"))]
            if url.https() {
                return Err(Error::NoTls);
            }
            let creds = req
                .credentials
                .as_ref()
                .filter(|_| url.same_origin(&req.url));
            let authorization = creds.and_then(|c| self.authorization(c, req.method, &url));
            let resp = self.once(req, &url, authorization.as_deref())?;
            let status = resp.status().as_u16();
            if status == 401 {
                // Answer one challenge per exchange: the first one, or a
                // fresh nonce after a cached one went stale.
                if let (Some(_), false) = (creds, challenged) {
                    let values: Vec<String> = resp
                        .headers()
                        .get_all("www-authenticate")
                        .iter()
                        .filter_map(|v| v.to_str().ok().map(String::from))
                        .collect();
                    if let Some(scheme) = auth::parse_challenges(values.iter().map(String::as_str))
                    {
                        let state = match scheme {
                            Scheme::Digest(ch) => AuthState::Digest(ch, 0),
                            Scheme::Basic => AuthState::Basic,
                        };
                        *self.auth.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some((url.clone(), state));
                        challenged = true;
                        continue;
                    }
                }
                if creds.is_some() {
                    return Err(Error::Unauthorized);
                }
                return Ok((resp, url));
            }
            if matches!(status, 301 | 302 | 303 | 307 | 308) && req.method == Method::Get {
                let Some(location) = header(&resp, "location") else {
                    return Ok((resp, url));
                };
                redirects += 1;
                if redirects > MAX_REDIRECTS {
                    return Err(Error::TooManyRedirects);
                }
                url = url.join(&location)?;
                continue;
            }
            return Ok((resp, url));
        }
    }

    /// The `Authorization` header for `url`, once its origin has asked
    /// for a scheme. Nothing goes out before the first 401: a password is
    /// never sent in the clear to a server that would have asked for
    /// Digest.
    fn authorization(&self, c: &Credentials, method: Method, url: &Url) -> Option<String> {
        let mut st = self.auth.lock().unwrap_or_else(|e| e.into_inner());
        match st.as_mut() {
            Some((origin, AuthState::Digest(ch, nc))) if origin.same_origin(url) => {
                *nc += 1;
                Some(auth::digest(
                    c,
                    ch,
                    method.as_str(),
                    url.target(),
                    *nc,
                    &auth::random_hex(),
                ))
            }
            Some((origin, AuthState::Basic)) if origin.same_origin(url) => Some(auth::basic(c)),
            _ => None,
        }
    }

    fn once(
        &self,
        req: &Request,
        url: &Url,
        authorization: Option<&str>,
    ) -> Result<ureq::http::Response<ureq::Body>, Error> {
        let mut b = ureq::http::Request::builder()
            .method(req.method.as_str())
            .uri(url.to_string());
        for (k, v) in &req.headers {
            b = b.header(k.as_str(), v.as_str());
        }
        if let Some(a) = authorization {
            b = b.header("authorization", a);
        }
        let result = match &req.body {
            Some(body) => {
                let r = b
                    .body(body.clone())
                    .map_err(|e| Error::Other(e.to_string()))?;
                self.agent.run(r)
            }
            None => {
                let r = b.body(()).map_err(|e| Error::Other(e.to_string()))?;
                self.agent.run(r)
            }
        };
        result.map_err(from_ureq)
    }
}

fn header(resp: &ureq::http::Response<ureq::Body>, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

fn io_from_body(e: std::io::Error, limit: u64) -> Error {
    // ureq wraps its own errors in the reader's io::Error.
    match e.into_inner() {
        Some(inner) => match inner.downcast::<ureq::Error>() {
            Ok(u) => match *u {
                ureq::Error::BodyExceedsLimit(_) => Error::TooLarge(limit),
                u => from_ureq(u),
            },
            Err(other) => Error::Other(other.to_string()),
        },
        None => Error::Other("read failed".into()),
    }
}

/// An error for an unexpected status, with the start of a text body.
pub fn status_error(status: u16, body: &[u8]) -> Error {
    if status == 401 {
        return Error::Unauthorized;
    }
    let text: String = String::from_utf8_lossy(body)
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    Error::Status(status, text.trim().to_string())
}

/// `filename="..."` or `filename*=UTF-8''...` of a `Content-Disposition`.
fn disposition_name(d: &str) -> Option<String> {
    let mut plain = None;
    for part in d.split(';') {
        let Some((k, v)) = part.trim().split_once('=') else {
            continue;
        };
        match k.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                let v = v.trim();
                let enc = v.split("''").nth(1)?;
                return Some(percent_decode(enc));
            }
            "filename" => plain = Some(v.trim().trim_matches('"').to_string()),
            _ => {}
        }
    }
    plain
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testserver::{Reply, Server};

    fn client(policy: Policy) -> Client {
        Client::new(policy, Timeouts::quick(Duration::from_secs(5)))
    }

    #[test]
    fn follows_redirects_only_to_allowed_hosts() {
        let s = Server::start(|r| match r.path.as_str() {
            "/a" => Reply::redirect("/b"),
            "/b" => Reply::text(200, "here"),
            "/away" => Reply::redirect("http://elsewhere.invalid:9/x"),
            "/loop" => Reply::redirect("/loop"),
            _ => Reply::text(404, "no"),
        });
        let c = client(Policy::open());
        let r = c.send(&Request::get(s.url("/a")), 1000).unwrap();
        assert_eq!((r.status, r.text().as_str()), (200, "here"));
        assert_eq!(r.url.path(), "/b");
        assert!(matches!(
            c.send(&Request::get(s.url("/loop")), 1000),
            Err(Error::TooManyRedirects)
        ));
        // Scoped: the test server is allowed, the redirect target is not.
        let scoped = Policy {
            scoped: true,
            allowed: crate::policy::parse_allow(&s.url("/").host_port()),
        };
        let c = client(scoped);
        let e = c.send(&Request::get(s.url("/away")), 1000).unwrap_err();
        assert_eq!(
            e.to_string(),
            "elsewhere.invalid:9 is not in the allowed hosts"
        );
        let e = c
            .send(
                &Request::get(Url::parse("http://other.invalid/").unwrap()),
                10,
            )
            .unwrap_err();
        assert!(matches!(e, Error::NotAllowed(ref h) if h == "other.invalid:80"));
        assert_eq!(s.hits("/away"), 1);
    }

    #[test]
    fn limits_the_body() {
        let s = Server::start(|_| Reply::text(200, &"x".repeat(5000)));
        let c = client(Policy::open());
        assert!(matches!(
            c.send(&Request::get(s.url("/")), 100),
            Err(Error::TooLarge(_))
        ));
        let mut out = Vec::new();
        let e = c
            .download(
                &Request::get(s.url("/")),
                &mut out,
                1000,
                &AtomicBool::new(false),
                &mut |_, _| {},
            )
            .unwrap_err();
        assert!(matches!(e, Error::TooLarge(1000)), "{e}");
    }

    #[test]
    fn answers_digest_like_calibre() {
        let s = Server::digest_protected("calibre", "anna", "lalka 1890");
        let good = Credentials {
            username: "anna".into(),
            password: "lalka 1890".into(),
        };
        let c = client(Policy::open());
        let r = c
            .send(
                &Request::get(s.url("/opds")).credentials(Some(good.clone())),
                1000,
            )
            .unwrap();
        assert_eq!((r.status, r.text().as_str()), (200, "ok /opds"));
        // The next request reuses the challenge: no second 401.
        let r = c
            .send(
                &Request::get(s.url("/opds/2")).credentials(Some(good)),
                1000,
            )
            .unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(s.hits("401"), 1);
        let bad = Credentials {
            username: "anna".into(),
            password: "wrong".into(),
        };
        let c = client(Policy::open());
        assert!(matches!(
            c.send(&Request::get(s.url("/opds")).credentials(Some(bad)), 1000),
            Err(Error::Unauthorized)
        ));
    }

    #[test]
    fn sends_basic_and_never_to_another_origin() {
        let other =
            Server::start(|r| Reply::text(200, r.header("authorization").unwrap_or("none")));
        let target = other.url("/file");
        let s = Server::start(move |r| match r.header("authorization") {
            Some("Basic YW5uYTpwdw==") => {
                if r.path == "/hop" {
                    Reply::redirect(&target.to_string())
                } else {
                    Reply::text(200, "basic ok")
                }
            }
            _ => Reply::basic_challenge("cw"),
        });
        let creds = Credentials {
            username: "anna".into(),
            password: "pw".into(),
        };
        let c = client(Policy::open());
        let r = c
            .send(
                &Request::get(s.url("/")).credentials(Some(creds.clone())),
                100,
            )
            .unwrap();
        assert_eq!(r.text(), "basic ok");
        let r = c
            .send(&Request::get(s.url("/hop")).credentials(Some(creds)), 100)
            .unwrap();
        assert_eq!(r.text(), "none", "credentials leaked to another origin");
        // Without credentials a 401 is just a response.
        let r = c.send(&Request::get(s.url("/")), 100).unwrap();
        assert_eq!(r.status, 401);
    }

    #[test]
    fn downloads_with_progress_and_cancel() {
        let s = Server::start(|_| {
            Reply::text(200, &"y".repeat(200_000)).with_header(
                "content-disposition",
                "attachment; filename*=UTF-8''Lalka%20-%20Prus.epub",
            )
        });
        let c = client(Policy::open());
        let mut out = Vec::new();
        let mut seen = 0;
        let d = c
            .download(
                &Request::get(s.url("/get/epub/1")),
                &mut out,
                1 << 20,
                &AtomicBool::new(false),
                &mut |n, total| {
                    seen = n;
                    assert_eq!(total, Some(200_000));
                },
            )
            .unwrap();
        assert_eq!((d.bytes, seen, out.len()), (200_000, 200_000, 200_000));
        assert_eq!(d.filename.as_deref(), Some("Lalka - Prus.epub"));
        let e = c
            .download(
                &Request::get(s.url("/")),
                &mut Vec::new(),
                1 << 20,
                &AtomicBool::new(true),
                &mut |_, _| {},
            )
            .unwrap_err();
        assert!(matches!(e, Error::Cancelled));
    }

    #[test]
    fn refused_connections_and_https_without_tls() {
        let c = client(Policy::open());
        // A port nothing listens on (bound, then closed).
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        assert!(matches!(
            c.send(&Request::get(url), 10),
            Err(Error::Connect(_))
        ));
        #[cfg(not(feature = "tls"))]
        assert!(matches!(
            c.send(
                &Request::get(Url::parse("https://127.0.0.1:1/").unwrap()),
                10
            ),
            Err(Error::NoTls)
        ));
        assert_eq!(
            disposition_name(r#"attachment; filename="Pan Tadeusz.mobi""#).as_deref(),
            Some("Pan Tadeusz.mobi")
        );
    }
}
