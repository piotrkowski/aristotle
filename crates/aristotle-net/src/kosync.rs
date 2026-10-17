// SPDX-License-Identifier: AGPL-3.0-or-later
//! The KOReader progress sync protocol (KOSync), client side.
//!
//! What the server expects, from KOReader's `plugins/kosync.koplugin`
//! (`api.json`, `KOSyncClient.lua`) and the reference server
//! (`koreader-sync-server`, `app/controllers/1/syncs_controller.lua`):
//!
//! - Every request sends `Accept: application/vnd.koreader.v1+json`.
//! - Authenticated calls send `x-auth-user` and `x-auth-key`, the
//!   lowercase hex MD5 of the password, computed on the client. Only that
//!   key is stored, never the password.
//! - `POST /users/create` `{username, password: key}` → 201; error code
//!   2002 means the name is taken, 2005 that registration is closed.
//! - `GET /users/auth` → 200, or 401 (code 2001).
//! - `PUT /syncs/progress` `{document, progress, percentage, device,
//!   device_id}` → 200 `{document, timestamp}`. The server stamps the time;
//!   the last write wins.
//! - `GET /syncs/progress/<document>` → the same fields plus `timestamp`,
//!   or `{}` when nothing was stored.
//!
//! What `progress` holds for each format, and when not to send anything at
//! all, is the caller's business (`aristotle_library::kosync`).

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::auth::md5_hex;
use crate::http::{self, Client, Request, Timeouts};
use crate::policy::Policy;
use crate::url::{encode_component, Url};

const ACCEPT: &str = "application/vnd.koreader.v1+json";
/// Responses are a few hundred bytes.
const LIMIT: u64 = 64 * 1024;

/// The account the settings hold: the server and the key, not the
/// password.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub server: String,
    pub username: String,
    /// `md5(password)`, lowercase hex.
    pub key: String,
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("server", &self.server)
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// The key KOSync servers compare: what KOReader sends as `x-auth-key`.
pub fn key(password: &str) -> String {
    md5_hex(password.as_bytes())
}

/// A position as the server stores it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub document: String,
    pub progress: String,
    pub percentage: f64,
    pub device: String,
    pub device_id: String,
    /// Set by the server; absent when sending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

/// `percentage` as KOReader sends it: floored to four decimals.
pub fn percentage(fraction: f32) -> f64 {
    ((fraction.clamp(0.0, 1.0) as f64) * 10_000.0).floor() / 10_000.0
}

/// Whether a document ID is one the reference server accepts.
pub fn valid_document(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[derive(Debug)]
pub enum Error {
    Http(http::Error),
    /// 401, code 2001: wrong user name or key.
    Unauthorized,
    /// 2002 on register.
    UserExists,
    /// 2005 on register.
    RegistrationClosed,
    /// Another `{code, message}` error, or an unexpected status.
    Server {
        status: u16,
        message: String,
    },
    /// A body that is not the JSON the protocol describes.
    BadResponse(String),
    /// A field the server would reject (an empty progress, a bad ID).
    Invalid(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(e) => e.fmt(f),
            Error::Unauthorized => f.write_str("the server refused the user name or password"),
            Error::UserExists => f.write_str("this user name is taken"),
            Error::RegistrationClosed => f.write_str("the server does not take new users"),
            Error::Server { status, message } => write!(f, "HTTP {status}: {message}"),
            Error::BadResponse(s) => write!(f, "unexpected answer: {s}"),
            Error::Invalid(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for Error {}

impl From<http::Error> for Error {
    fn from(e: http::Error) -> Self {
        match e {
            http::Error::Unauthorized => Error::Unauthorized,
            e => Error::Http(e),
        }
    }
}

impl Error {
    /// Worth trying again later: the network or the server, not the
    /// account or the request.
    pub fn transient(&self) -> bool {
        match self {
            Error::Http(e) => !matches!(
                e,
                http::Error::NotAllowed(_) | http::Error::NoTls | http::Error::Url(_)
            ),
            Error::Server { status, .. } => *status >= 500,
            _ => false,
        }
    }
}

#[derive(Deserialize)]
struct ServerError {
    code: Option<i64>,
    message: Option<String>,
}

pub struct Kosync {
    http: Client,
}

impl Kosync {
    /// A client with KOReader's patience for sync calls (5 s).
    pub fn new(policy: Policy) -> Kosync {
        Kosync::with_timeout(policy, Duration::from_secs(5))
    }

    pub fn with_timeout(policy: Policy, t: Duration) -> Kosync {
        Kosync {
            http: Client::new(policy, Timeouts::quick(t)),
        }
    }

    fn base(account: &Account) -> Result<Url, Error> {
        Url::parse(&account.server).map_err(|e| Error::Http(e.into()))
    }

    fn signed(req: Request, a: &Account) -> Request {
        req.header("accept", ACCEPT)
            .header("x-auth-user", &a.username)
            .header("x-auth-key", &a.key)
    }

    /// `POST /users/create`.
    pub fn register(&self, a: &Account) -> Result<(), Error> {
        let body = serde_json::json!({ "username": a.username, "password": a.key });
        let req = Request::post(
            Self::base(a)?.with_path("users/create"),
            body.to_string().into_bytes(),
        )
        .header("accept", ACCEPT)
        .header("content-type", "application/json");
        let r = self.http.send(&req, LIMIT)?;
        match r.status {
            200 | 201 => Ok(()),
            _ => Err(server_error(r.status, &r.body)),
        }
    }

    /// `GET /users/auth`.
    pub fn authorize(&self, a: &Account) -> Result<(), Error> {
        let req = Self::signed(Request::get(Self::base(a)?.with_path("users/auth")), a);
        let r = self.http.send(&req, LIMIT)?;
        match r.status {
            200 => Ok(()),
            _ => Err(server_error(r.status, &r.body)),
        }
    }

    /// `GET /syncs/progress/<document>`: `None` when the server has no
    /// position, or one without a progress string.
    pub fn pull(&self, a: &Account, document: &str) -> Result<Option<Position>, Error> {
        if !valid_document(document) {
            return Err(Error::Invalid("bad document ID"));
        }
        let url =
            Self::base(a)?.with_path(&format!("syncs/progress/{}", encode_component(document)));
        let r = self.http.send(&Self::signed(Request::get(url), a), LIMIT)?;
        if r.status != 200 {
            return Err(server_error(r.status, &r.body));
        }
        let v: serde_json::Value =
            serde_json::from_slice(&r.body).map_err(|e| Error::BadResponse(e.to_string()))?;
        let Some(obj) = v.as_object() else {
            return Err(Error::BadResponse("not an object".into()));
        };
        let progress = match obj.get("progress") {
            Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(),
            _ => return Ok(None),
        };
        let num = |k: &str| {
            obj.get(k)
                .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
        };
        let text = |k: &str| {
            obj.get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        Ok(Some(Position {
            document: text("document"),
            progress,
            percentage: num("percentage").unwrap_or(0.0).clamp(0.0, 1.0),
            device: text("device"),
            device_id: text("device_id"),
            timestamp: num("timestamp").map(|t| t as i64),
        }))
    }

    /// `PUT /syncs/progress`. Returns the server's timestamp, if it sent
    /// one. Refuses an empty progress: KOReader would take the reader to
    /// the first page.
    pub fn push(&self, a: &Account, p: &Position) -> Result<Option<i64>, Error> {
        if p.progress.trim().is_empty() {
            return Err(Error::Invalid("empty progress"));
        }
        if !valid_document(&p.document) {
            return Err(Error::Invalid("bad document ID"));
        }
        let mut body = p.clone();
        body.timestamp = None;
        let json = serde_json::to_vec(&body).map_err(|e| Error::BadResponse(e.to_string()))?;
        let req = Self::signed(
            Request::put(Self::base(a)?.with_path("syncs/progress"), json),
            a,
        )
        .header("content-type", "application/json");
        let r = self.http.send(&req, LIMIT)?;
        if r.status != 200 {
            return Err(server_error(r.status, &r.body));
        }
        let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap_or_default();
        Ok(v.get("timestamp").and_then(|t| t.as_i64()))
    }
}

fn server_error(status: u16, body: &[u8]) -> Error {
    let e: Option<ServerError> = serde_json::from_slice(body).ok();
    let code = e.as_ref().and_then(|e| e.code);
    let message =
        e.and_then(|e| e.message)
            .unwrap_or_else(|| match http::status_error(status, body) {
                http::Error::Status(_, text) if !text.is_empty() => text,
                _ => "no details".into(),
            });
    match (status, code) {
        (401, _) | (_, Some(2001)) => Error::Unauthorized,
        (_, Some(2002)) => Error::UserExists,
        (_, Some(2005)) => Error::RegistrationClosed,
        _ => Error::Server { status, message },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testserver::{Reply, Req, Server};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// A KOSync server after the reference one: users, keys, positions,
    /// its status codes and error bodies.
    fn mock() -> (Server, Arc<Mutex<HashMap<String, serde_json::Value>>>) {
        let users: Arc<Mutex<HashMap<String, String>>> = Arc::default();
        let docs: Arc<Mutex<HashMap<String, serde_json::Value>>> = Arc::default();
        let d2 = docs.clone();
        let s = Server::start(move |r: &Req| {
            if r.header("accept") != Some(ACCEPT) {
                return Reply::text(412, "");
            }
            let err = |status, code: i64, msg: &str| {
                Reply::typed(
                    status,
                    "application/json",
                    serde_json::json!({"code": code, "message": msg})
                        .to_string()
                        .as_bytes(),
                )
            };
            let json = |status, v: serde_json::Value| {
                Reply::typed(status, "application/json", v.to_string().as_bytes())
            };
            let mut users = users.lock().unwrap();
            if r.method == "POST" && r.path == "/users/create" {
                let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
                let name = v["username"].as_str().unwrap().to_string();
                if name == "closed" {
                    return err(402, 2005, "User registration is disabled.");
                }
                if users.contains_key(&name) {
                    return err(402, 2002, "Username is already registered.");
                }
                users.insert(name.clone(), v["password"].as_str().unwrap().to_string());
                return json(201, serde_json::json!({ "username": name }));
            }
            let authed = matches!(
                (r.header("x-auth-user"), r.header("x-auth-key")),
                (Some(u), Some(k)) if users.get(u).is_some_and(|key| key == k)
            );
            if !authed {
                return err(401, 2001, "Unauthorized");
            }
            let mut docs = d2.lock().unwrap();
            match (r.method.as_str(), r.path.as_str()) {
                ("GET", "/users/auth") => json(200, serde_json::json!({"authorized": "OK"})),
                ("PUT", "/syncs/progress") => {
                    let mut v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
                    v["timestamp"] = 1_792_000_000.into();
                    let doc = v["document"].as_str().unwrap().to_string();
                    docs.insert(doc.clone(), v);
                    json(
                        200,
                        serde_json::json!({"document": doc, "timestamp": 1_792_000_000}),
                    )
                }
                ("GET", p) if p.starts_with("/syncs/progress/") => {
                    let doc = &p["/syncs/progress/".len()..];
                    json(200, docs.get(doc).cloned().unwrap_or(serde_json::json!({})))
                }
                _ => Reply::text(404, ""),
            }
        });
        (s, docs)
    }

    fn account(s: &Server, user: &str, password: &str) -> Account {
        Account {
            server: s.url("/").to_string(),
            username: user.into(),
            key: key(password),
        }
    }

    #[test]
    fn register_login_push_pull() {
        let (s, docs) = mock();
        let k = Kosync::new(Policy::open());
        let a = account(&s, "anna", "test");
        assert_eq!(a.key, "098f6bcd4621d373cade4e832627b4f6");
        assert!(!format!("{a:?}").contains(&a.key));
        assert!(matches!(k.authorize(&a), Err(Error::Unauthorized)));
        k.register(&a).unwrap();
        assert!(matches!(k.register(&a), Err(Error::UserExists)));
        assert!(matches!(
            k.register(&account(&s, "closed", "x")),
            Err(Error::RegistrationClosed)
        ));
        k.authorize(&a).unwrap();
        assert!(matches!(
            k.authorize(&account(&s, "anna", "wrong")),
            Err(Error::Unauthorized)
        ));

        let doc = "0123456789abcdef0123456789abcdef";
        assert_eq!(k.pull(&a, doc).unwrap(), None);
        let p = Position {
            document: doc.into(),
            progress: "/body/DocFragment[7]/body".into(),
            percentage: percentage(0.43219),
            device: "Aristotle".into(),
            device_id: "0123456789ABCDEF0123456789ABCDEF".into(),
            timestamp: None,
        };
        assert_eq!(p.percentage, 0.4321);
        assert_eq!(k.push(&a, &p).unwrap(), Some(1_792_000_000));
        // The body is what KOReader sends: no timestamp, the five fields.
        let sent = docs.lock().unwrap()[doc].clone();
        assert_eq!(sent["progress"], "/body/DocFragment[7]/body");
        assert_eq!(sent["device"], "Aristotle");
        let back = k.pull(&a, doc).unwrap().unwrap();
        assert_eq!(back.timestamp, Some(1_792_000_000));
        assert_eq!(
            Position {
                timestamp: None,
                ..back
            },
            p
        );
        let headers = &s.requests.lock().unwrap()[0].headers;
        assert!(headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("accept") && v == ACCEPT));
    }

    #[test]
    fn never_pushes_an_empty_position_or_a_bad_id() {
        let (s, _) = mock();
        let k = Kosync::new(Policy::open());
        let a = account(&s, "anna", "test");
        let mut p = Position {
            document: "abc".into(),
            progress: " ".into(),
            percentage: 0.1,
            device: "Aristotle".into(),
            device_id: "X".into(),
            timestamp: None,
        };
        assert!(matches!(k.push(&a, &p), Err(Error::Invalid(_))));
        p.progress = "12".into();
        p.document = "abc-2".into();
        assert!(matches!(k.push(&a, &p), Err(Error::Invalid(_))));
        assert!(s.requests.lock().unwrap().is_empty(), "nothing was sent");
    }

    #[test]
    fn errors_say_what_to_do() {
        let s = Server::start(|r| match r.path.as_str() {
            "/users/auth" => Reply::text(502, "Bad Gateway"),
            _ => Reply::typed(
                200,
                "application/json",
                br#"{"progress": "", "percentage": 0.5}"#,
            ),
        });
        let k = Kosync::new(Policy::open());
        let a = account(&s, "anna", "test");
        let e = k.authorize(&a).unwrap_err();
        assert!(e.transient(), "{e}");
        assert_eq!(e.to_string(), "HTTP 502: Bad Gateway");
        // A record with an empty progress is no position.
        assert_eq!(k.pull(&a, "abc").unwrap(), None);
        let scoped = Policy {
            scoped: true,
            allowed: Vec::new(),
        };
        let e = Kosync::new(scoped).authorize(&a).unwrap_err();
        assert!(!e.transient());
        assert!(
            e.to_string().ends_with("is not in the allowed hosts"),
            "{e}"
        );
    }
}
