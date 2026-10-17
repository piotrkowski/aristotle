// SPDX-License-Identifier: AGPL-3.0-or-later
//! A tiny HTTP/1.1 server on 127.0.0.1 for the tests: one thread per
//! connection, one request per connection (`Connection: close`), replies
//! from a closure. It counts requests per path, and 401s under "401".

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use crate::url::Url;

#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    /// Path and query.
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Req {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn text(status: u16, body: &str) -> Reply {
        Reply {
            status,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn typed(status: u16, content_type: &str, body: &[u8]) -> Reply {
        Reply {
            status,
            headers: vec![("Content-Type".into(), content_type.into())],
            body: body.to_vec(),
        }
    }

    pub fn redirect(to: &str) -> Reply {
        Reply::text(302, "").with_header("Location", to)
    }

    pub fn basic_challenge(realm: &str) -> Reply {
        Reply::text(401, "").with_header("WWW-Authenticate", &format!("Basic realm=\"{realm}\""))
    }

    pub fn with_header(mut self, k: &str, v: &str) -> Reply {
        self.headers.push((k.into(), v.into()));
        self
    }
}

type Handler = dyn Fn(&Req) -> Reply + Send + Sync;

pub struct Server {
    port: u16,
    hits: Arc<Mutex<HashMap<String, usize>>>,
    pub requests: Arc<Mutex<Vec<Req>>>,
}

impl Server {
    pub fn start(handler: impl Fn(&Req) -> Reply + Send + Sync + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(Mutex::new(HashMap::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let (h2, r2) = (hits.clone(), requests.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let (handler, hits, requests) = (handler.clone(), h2.clone(), r2.clone());
                std::thread::spawn(move || serve(stream, &*handler, &hits, &requests));
            }
        });
        Server {
            port,
            hits,
            requests,
        }
    }

    /// A server that wants Digest (MD5, `qop=auth`) with these credentials,
    /// as calibre's content server does over plain HTTP, and answers
    /// "ok PATH".
    pub fn digest_protected(realm: &str, user: &str, password: &str) -> Server {
        let (realm, user, password) = (realm.to_string(), user.to_string(), password.to_string());
        Server::start(move |r| {
            let nonce = "6f4b1c2d";
            let ok = r.header("authorization").is_some_and(|a| {
                let field = |name: &str| {
                    let key = format!("{name}=");
                    let at = a.find(&key)? + key.len();
                    let rest = &a[at..];
                    Some(if let Some(q) = rest.strip_prefix('"') {
                        q[..q.find('"')?].to_string()
                    } else {
                        rest.split(',').next()?.trim().to_string()
                    })
                };
                let (Some(u), Some(uri), Some(nc), Some(cnonce), Some(resp)) = (
                    field("username"),
                    field("uri"),
                    field("nc"),
                    field("cnonce"),
                    field("response"),
                ) else {
                    return false;
                };
                let ha1 = crate::auth::md5_hex(format!("{u}:{realm}:{password}").as_bytes());
                let ha2 = crate::auth::md5_hex(format!("{}:{uri}", r.method).as_bytes());
                let want = crate::auth::md5_hex(
                    format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}").as_bytes(),
                );
                u == user && uri == r.path && resp == want
            });
            if ok {
                Reply::text(200, &format!("ok {}", r.path))
            } else {
                Reply::text(401, "").with_header(
                    "WWW-Authenticate",
                    &format!("Digest realm=\"{realm}\", nonce=\"{nonce}\", algorithm=\"MD5\", qop=\"auth\""),
                )
            }
        })
    }

    pub fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://127.0.0.1:{}{path}", self.port)).unwrap()
    }

    pub fn hits(&self, key: &str) -> usize {
        self.hits.lock().unwrap().get(key).copied().unwrap_or(0)
    }
}

fn serve(
    stream: TcpStream,
    handler: &Handler,
    hits: &Mutex<HashMap<String, usize>>,
    requests: &Mutex<Vec<Req>>,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).unwrap_or(0) == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let len = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0; len];
    let _ = reader.read_exact(&mut body);
    let req = Req {
        method,
        path: path.clone(),
        headers,
        body,
    };
    let reply = handler(&req);
    {
        let mut h = hits.lock().unwrap();
        *h.entry(path).or_default() += 1;
        if reply.status == 401 {
            *h.entry("401".into()).or_default() += 1;
        }
    }
    requests.lock().unwrap().push(req);
    let mut out = stream;
    let mut head = format!("HTTP/1.1 {} X\r\n", reply.status);
    for (k, v) in &reply.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        reply.body.len()
    ));
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(&reply.body);
    let _ = out.flush();
}
