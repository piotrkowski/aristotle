// SPDX-License-Identifier: AGPL-3.0-or-later
//! Format detection by content.
//!
//! Extensions on a Kindle lie often enough to matter: calibre and Send to
//! Kindle produce `.azw3` files that are joint MOBI/KF8 containers, `.azw`
//! may be either, `.prc` and `.pdb` are PalmDB, and a renamed or truncated
//! download may not be a book at all. So we read the first bytes and decide
//! from magic numbers; the extension only decides between formats that have
//! no magic (TXT) or share one (CBZ versus a random ZIP).

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::zip::ZipArchive;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Epub,
    /// Mobipocket (KF7), including joint KF7/KF8 files.
    Mobi,
    /// KF8-only files (MOBI header version 8), usually `.azw3`.
    Azw3,
    Pdf,
    Txt,
    Cbz,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Epub => "epub",
            Format::Mobi => "mobi",
            Format::Azw3 => "azw3",
            Format::Pdf => "pdf",
            Format::Txt => "txt",
            Format::Cbz => "cbz",
        }
    }

    pub fn parse(s: &str) -> Option<Format> {
        Some(match s.to_ascii_lowercase().as_str() {
            "epub" => Format::Epub,
            "mobi" => Format::Mobi,
            "azw3" | "azw" => Format::Azw3,
            "pdf" => Format::Pdf,
            "txt" => Format::Txt,
            "cbz" => Format::Cbz,
            _ => return None,
        })
    }
}

/// Extensions worth opening at all. Everything else (`.jpg`, `.apnx`,
/// `.lua`, `.json`, ...) is skipped without a read, which keeps a scan of a
/// cluttered documents folder cheap.
pub const CANDIDATE_EXTENSIONS: &[&str] = &[
    "epub", "kepub", "mobi", "azw", "azw3", "prc", "pdb", "pdf", "txt", "cbz", "zip",
];

/// Files the stock Kindle keeps in `documents` that are not books.
const NOT_BOOKS: &[&str] = &["my clippings.txt"];

pub fn is_candidate(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    if NOT_BOOKS.contains(&name.as_str()) {
        return false;
    }
    match extension(path) {
        Some(e) => CANDIDATE_EXTENSIONS.contains(&e.as_str()),
        // Extensionless files get one sniff; the scanner remembers non-books.
        None => true,
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

/// Detects the format of `path` from its content. `Ok(None)` means "not a
/// book we handle", which includes encrypted KFX and plain ZIP archives.
pub fn sniff(path: &Path) -> Result<Option<Format>> {
    let mut head = [0u8; 1024];
    let n = read_head(path, &mut head)?;
    let head = &head[..n];
    let ext = extension(path).unwrap_or_default();

    // PDF allows junk before the header; Acrobat looks in the first 1 KiB.
    if head.windows(5).any(|w| w == b"%PDF-") {
        return Ok(Some(Format::Pdf));
    }
    // PalmDB: type and creator at offset 60. BOOKMOBI is Mobipocket/KF8;
    // TEXtREAd is plain PalmDOC, which the MOBI reader also handles.
    if n >= 68 && (&head[60..68] == b"BOOKMOBI" || &head[60..68] == b"TEXtREAd") {
        return Ok(Some(mobi_flavour(path)?));
    }
    if head.starts_with(b"PK\x03\x04") {
        return sniff_zip(path, head, &ext);
    }
    if ext == "txt" && looks_like_text(head) {
        return Ok(Some(Format::Txt));
    }
    Ok(None)
}

fn read_head(path: &Path, buf: &mut [u8]) -> Result<usize> {
    let mut f = File::open(path)?;
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

fn mobi_flavour(path: &Path) -> Result<Format> {
    Ok(match crate::mobi::header_version(path) {
        Ok(8) => Format::Azw3,
        _ => Format::Mobi,
    })
}

fn sniff_zip(path: &Path, head: &[u8], ext: &str) -> Result<Option<Format>> {
    // OCF: the first member must be an uncompressed "mimetype" holding
    // "application/epub+zip", so a 60-byte check usually settles it.
    if head.len() >= 58
        && &head[30..38] == b"mimetype"
        && head[38..].starts_with(b"application/epub+zip")
    {
        return Ok(Some(Format::Epub));
    }
    // Plenty of tools get the mimetype rule wrong; fall back to the directory.
    let zip = match ZipArchive::open(path) {
        Ok(z) => z,
        // A damaged file that claims to be a book should be reported, not silently ignored.
        Err(e) if matches!(ext, "epub" | "kepub" | "cbz") => return Err(e),
        Err(_) => return Ok(None),
    };
    if zip.find("META-INF/container.xml").is_some() {
        return Ok(Some(Format::Epub));
    }
    let images = zip
        .entries()
        .iter()
        .filter(|e| crate::cover::is_image_name(&e.name))
        .count();
    if (ext == "cbz" || ext == "zip") && images > 0 && images * 2 >= zip.entries().len() {
        return Ok(Some(Format::Cbz));
    }
    Ok(None)
}

/// No NUL bytes and mostly printable: good enough to reject binaries that
/// were given a `.txt` name. UTF-16 text has NULs, so accept a BOM first.
fn looks_like_text(head: &[u8]) -> bool {
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    let control = head
        .iter()
        .filter(|&&b| b < 0x20 && !matches!(b, b'\n' | b'\r' | b'\t' | 0x0C))
        .count();
    !head.contains(&0) && control * 50 <= head.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_content_not_extension() {
        let dir = crate::testdir("format");
        let pdf = dir.join("book.epub");
        std::fs::write(&pdf, b"%PDF-1.4\n%junk").unwrap();
        assert_eq!(sniff(&pdf).unwrap(), Some(Format::Pdf));

        let epub = dir.join("renamed.zip");
        std::fs::write(
            &epub,
            crate::zip::testutil::build(&[("mimetype", b"application/epub+zip", false)]),
        )
        .unwrap();
        assert_eq!(sniff(&epub).unwrap(), Some(Format::Epub));

        let cbz = dir.join("comic.cbz");
        std::fs::write(
            &cbz,
            crate::zip::testutil::build(&[("01.png", b"x", false), ("02.png", b"y", false)]),
        )
        .unwrap();
        assert_eq!(sniff(&cbz).unwrap(), Some(Format::Cbz));

        let txt = dir.join("notes.txt");
        std::fs::write(&txt, "Zażółć gęślą jaźń\n".as_bytes()).unwrap();
        assert_eq!(sniff(&txt).unwrap(), Some(Format::Txt));

        let bin = dir.join("binary.txt");
        std::fs::write(&bin, [0u8, 1, 2, 3, 0, 0]).unwrap();
        assert_eq!(sniff(&bin).unwrap(), None);
    }

    #[test]
    fn clippings_file_is_not_a_book() {
        assert!(!is_candidate(Path::new(
            "/mnt/us/documents/My Clippings.txt"
        )));
        assert!(is_candidate(Path::new(
            "/mnt/us/documents/Platon/Uczta.EPUB"
        )));
        assert!(!is_candidate(Path::new("/mnt/us/documents/x.apnx")));
    }
}
