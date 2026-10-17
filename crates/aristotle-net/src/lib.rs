// SPDX-License-Identifier: AGPL-3.0-or-later
//! Aristotle's network client.
//!
//! - [`http`]: a blocking HTTP client over `ureq` with timeouts, size
//!   limits, redirects checked against the device's allowlist ([`policy`])
//!   and Basic or Digest auth ([`auth`]).
//! - [`kosync`]: KOReader's progress sync protocol.
//! - [`opds`]: OPDS 1.2 catalogs, and [`download`] to save their books.
//!
//! Pure Rust by default, plain HTTP only. The `tls` feature adds HTTPS
//! through rustls and ring, which compiles C (see `Cargo.toml`). Nothing
//! here runs on its own: the app calls it from worker threads, and only
//! for servers and catalogs the reader has set up (`docs/library.md`,
//! "Progress sync" and "Catalogs").

pub mod auth;
pub mod download;
pub mod http;
pub mod kosync;
pub mod opds;
pub mod policy;
pub mod store;
pub mod url;

#[cfg(test)]
mod testserver;

pub use auth::Credentials;
pub use http::{Client, Error, Request, Timeouts};
pub use policy::Policy;
pub use url::Url;

/// Whether this build speaks HTTPS.
pub const HTTPS: bool = cfg!(feature = "tls");
