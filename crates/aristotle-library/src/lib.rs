//! Book library for Aristotle.
//!
//! Finds books under a root folder (on the device `/mnt/us/documents`),
//! identifies them by content, extracts metadata and covers, and keeps a
//! small JSON index with reading state. Pure Rust with no C dependencies, so
//! it builds statically for `armv7-unknown-linux-musleabihf`.
//!
//! - [`library::Library`]: the entry point (open, scan, query, save).
//! - [`format`]: detection by magic bytes.
//! - [`metadata`] with [`epub`], [`mobi`], [`pdf`]: per-format parsers.
//! - [`cover`]: grayscale, 16-level dithered thumbnails for e-ink.
//! - [`index`]: the persistent records, progress and settings.
//! - [`query`]: sorting, filters, diacritic-insensitive search.
//! - [`clippings`], [`sidecar`]: importing state left by the stock Kindle
//!   reader and KOReader.
//!
//! See `docs/library.md` for the design and measurements.

pub mod clippings;
pub mod cover;
pub mod encoding;
pub mod epub;
pub mod error;
pub mod format;
pub mod id;
pub mod index;
pub mod library;
pub mod metadata;
pub mod mobi;
pub mod pdf;
pub mod query;
pub mod sidecar;
pub mod text;
pub mod zip;

pub use error::{Error, Result};
pub use format::Format;
pub use index::{BookRecord, Locator, Progress, Status};
pub use library::{Library, ScanOptions, ScanReport};
pub use metadata::Metadata;
pub use query::{Query, SortKey, StatusFilter};

/// A fresh, empty directory for one test, under `$TMPDIR` (which the build
/// instructions point at `~/.cache/aristotle/tmp`, not the RAM-backed /tmp).
#[cfg(test)]
pub(crate) fn testdir(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let dir = std::env::temp_dir()
        .join("aristotle-library-tests")
        .join(format!("{name}-{}-{n}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
