// SPDX-License-Identifier: AGPL-3.0-or-later
//! Saving a downloaded book into the library folder without ever leaving a
//! half-written book behind.
//!
//! The file streams into a hidden `.<name>.part` beside its destination
//! (the library scan skips hidden files), is synced, and is renamed to a
//! name no other file has (" (2)", " (3)" on a clash). Any failure or a
//! cancel removes the part file. Names are made safe for the FAT
//! userstore: no path separators, no `..`, no leading dot, none of
//! `<>:"|?*\` and no control characters.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use crate::http::{Client, Downloaded, Error, Request};

/// The largest book we download.
pub const MAX_BOOK: u64 = 200 * 1024 * 1024;
/// Bytes left free on the userstore after a download.
pub const KEEP_FREE: u64 = 20 * 1024 * 1024;

/// One file or folder name made safe for FAT and for the library: at most
/// `max` bytes, cut at a character boundary. Empty input gives `fallback`.
pub fn safe_name(name: &str, max: usize, fallback: &str) -> String {
    let mut s = String::with_capacity(name.len());
    for c in name.chars() {
        let c = match c {
            '/' | '\\' | ':' | '|' => '-',
            '<' | '>' | '"' | '?' | '*' => continue,
            c if c.is_control() => ' ',
            c => c,
        };
        s.push(c);
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    // No leading dots (hidden, or `..`), no trailing dots or spaces (FAT
    // drops them, so two names could collide).
    let s = s
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' ']);
    let mut out = String::new();
    for c in s.chars() {
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    let out = out.trim_end_matches(['.', ' ']).to_string();
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

/// `dir/stem.ext`, or `dir/stem (2).ext` and so on if taken.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !exists(&first) {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|p| !exists(p))
        .unwrap_or(first)
}

/// FAT compares names without case: so does this, on any filesystem.
fn exists(p: &Path) -> bool {
    if p.exists() {
        return true;
    }
    let (Some(dir), Some(name)) = (p.parent(), p.file_name().and_then(|n| n.to_str())) else {
        return false;
    };
    let lower = name.to_lowercase();
    fs::read_dir(dir).is_ok_and(|mut it| {
        it.any(|e| {
            e.ok()
                .and_then(|e| e.file_name().into_string().ok())
                .is_some_and(|n| n.to_lowercase() == lower)
        })
    })
}

/// What a finished download produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub path: PathBuf,
    pub info: Downloaded,
}

/// Download `req` into `dir` as `stem.ext` (made unique), at most `limit`
/// bytes.
#[allow(clippy::too_many_arguments)]
pub fn save(
    client: &Client,
    req: &Request,
    dir: &Path,
    stem: &str,
    ext: &str,
    limit: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Saved, Error> {
    fs::create_dir_all(dir)?;
    let part = dir.join(format!(".{stem}.{ext}.part"));
    let result = (|| {
        let mut f = File::create(&part)?;
        let info = client.download(req, &mut f, limit, cancel, progress)?;
        f.flush()?;
        f.sync_all()?;
        drop(f);
        let path = unique_path(dir, stem, ext);
        fs::rename(&part, &path)?;
        Ok(Saved { path, info })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Timeouts;
    use crate::policy::Policy;
    use crate::testserver::{Reply, Server};
    use std::time::Duration;

    fn testdir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir()
            .join("aristotle-net-tests")
            .join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names_are_safe_on_fat() {
        assert_eq!(safe_name("Lalka / Tom I", 120, "x"), "Lalka - Tom I");
        assert_eq!(safe_name("../../etc/passwd", 120, "x"), "-..-etc-passwd");
        assert_eq!(safe_name("...", 120, "book"), "book");
        assert_eq!(safe_name(".hidden", 120, "x"), "hidden");
        assert_eq!(safe_name("a\0b\nc", 120, "x"), "a b c");
        assert_eq!(safe_name("What? \"Why\" <not>*", 120, "x"), "What Why not");
        assert_eq!(safe_name("trailing. ", 120, "x"), "trailing");
        assert_eq!(safe_name("Żółć", 5, "x"), "Żó");
        assert!(!safe_name("Pan Tadeusz", 120, "x").contains('/'));
    }

    #[test]
    fn saves_through_a_part_file_and_never_overwrites() {
        let body = "Litwo! Ojczyzno moja! ty jesteś jak zdrowie.".repeat(100);
        let b2 = body.clone();
        let s = Server::start(move |r| match r.path.as_str() {
            "/book.epub" => Reply::text(200, &b2),
            "/short" => Reply::text(200, "x").with_header("X-Note", "fine"),
            _ => Reply::text(404, "gone"),
        });
        let c = Client::new(Policy::open(), Timeouts::quick(Duration::from_secs(5)));
        let dir = testdir("save").join("Calibre");
        let go = |path: &str, limit: u64| {
            save(
                &c,
                &Request::get(s.url(path)),
                &dir,
                "Pan Tadeusz",
                "epub",
                limit,
                &AtomicBool::new(false),
                &mut |_, _| {},
            )
        };
        let a = go("/book.epub", MAX_BOOK).unwrap();
        assert_eq!(a.path, dir.join("Pan Tadeusz.epub"));
        assert_eq!(fs::read_to_string(&a.path).unwrap(), body);
        let b = go("/book.epub", MAX_BOOK).unwrap();
        assert_eq!(b.path, dir.join("Pan Tadeusz (2).epub"));
        // Failures leave nothing behind: a 404, and a body over the limit.
        assert!(matches!(
            go("/missing", MAX_BOOK),
            Err(Error::Status(404, _))
        ));
        assert!(matches!(go("/book.epub", 100), Err(Error::TooLarge(100))));
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["Pan Tadeusz (2).epub", "Pan Tadeusz.epub"]);
        // Case-insensitive clash, as on FAT.
        fs::write(dir.join("LALKA.epub"), "x").unwrap();
        assert_eq!(
            unique_path(&dir, "Lalka", "epub"),
            dir.join("Lalka (2).epub")
        );
    }
}
