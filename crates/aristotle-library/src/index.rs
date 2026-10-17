//! The persistent library index.
//!
//! One JSON file, `library.json`, in the data directory. JSON over a binary
//! format such as postcard because:
//!
//! - fields can be added later with `#[serde(default)]` and old files still
//!   load; postcard has no field names, so any schema change breaks them;
//! - it can be read and repaired with a text editor over SSH;
//! - size and speed do not matter at this scale: 31 books are about 17 KB
//!   and load in well under a millisecond on the host.
//!
//! Writes are atomic (temporary file, fsync, rename), so a crash or a
//! battery cut never leaves a half-written index; the stock reader does the
//! same with its sidecars.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::format::Format;
use crate::metadata::Metadata;

pub const INDEX_VERSION: u32 = 1;
pub const INDEX_FILE: &str = "library.json";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub version: u32,
    /// Library root the paths are relative to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default)]
    pub books: Vec<BookRecord>,
    /// Files that were sniffed and found not to be books, so that the next
    /// scan does not open them again while they are unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignored: Vec<FileStamp>,
    /// Books whose file disappeared. Their reading state is kept so that a
    /// book deleted and copied back over USB resumes where it was.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<BookRecord>,
}

/// Size and modification time: the cheap "has this file changed" test. Both
/// are compared because FAT timestamps have 2-second resolution, so a quick
/// rewrite can keep the mtime but rarely keeps the size too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStamp {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    #[serde(default)]
    pub mtime_ns: u32,
}

impl FileStamp {
    pub fn same_file(&self, other: &FileStamp) -> bool {
        self.size == other.size && self.mtime == other.mtime && self.mtime_ns == other.mtime_ns
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BookRecord {
    /// KOReader-compatible partial MD5 of the content (see [`crate::id`]).
    pub id: String,
    #[serde(flatten)]
    pub file: FileStamp,
    pub format: Format,
    pub meta: Metadata,
    /// Thumbnail cache key (see [`crate::cover::CoverCache`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// Unix seconds when the book first appeared in the library.
    pub added: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened: Option<i64>,
    #[serde(default)]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    #[serde(default, skip_serializing_if = "ReaderSettings::is_empty")]
    pub settings: ReaderSettings,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
}

impl BookRecord {
    /// Copies everything the reader owns (as opposed to what the scanner
    /// derives from the file) from `old`. Used when a file is replaced,
    /// moved, or comes back after removal.
    pub fn inherit_state(&mut self, old: &BookRecord) {
        self.added = old.added;
        self.last_opened = old.last_opened;
        self.status = old.status;
        self.progress = old.progress.clone();
        self.settings = old.settings.clone();
        self.annotations = old.annotations.clone();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Unread,
    Reading,
    Finished,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// Position in the book from 0.0 to 1.0, for progress bars and sorting.
    /// Format-independent but approximate; the locator is authoritative.
    pub fraction: f32,
    pub locator: Locator,
    /// Unix seconds of the last update, to pick the newer of two sources.
    pub updated: i64,
    /// Where this progress came from: "aristotle", "kindle", "koreader".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Printed page label at the position, when a page map is known
    /// (Kindle `.apnx`), for display such as "page 4 of 180".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

/// A format-specific position. Each variant is what the corresponding
/// renderer can seek to exactly; `fraction` alone drifts when fonts change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Locator {
    /// Reflowable EPUB: spine item and character offset in its text.
    Epub { spine: u32, offset: u32 },
    /// MOBI/AZW3: the Kindle "position", a byte offset into the
    /// uncompressed text, which is what the stock reader stores.
    Kindle { position: u64 },
    /// Fixed layout (PDF, CBZ): zero-based page index.
    Page { index: u32 },
    /// Plain text: byte offset.
    Text { offset: u64 },
    /// KOReader's CRE XPointer, kept verbatim so it can be converted once
    /// the EPUB renderer exists.
    XPointer { xpointer: String },
}

/// Per-book overrides of the global reader settings. Every field is
/// optional: `None` means "use the global default".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReaderSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub margin: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justify: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hyphenation: Option<bool>,
    /// Fixed layout zoom (1.0 = fit width).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<u16>,
    /// Forward-compatible bag for settings not modelled yet.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub extra: std::collections::BTreeMap<String, String>,
}

impl ReaderSettings {
    pub fn is_empty(&self) -> bool {
        *self == ReaderSettings::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnnotationKind {
    Highlight,
    Note,
    Bookmark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub kind: AnnotationKind,
    /// Highlighted text, or the note's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Printed page label ("12", "xii") when the book has page numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// Kindle location range (1 location = 150 bytes of MOBI text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<(u32, u32)>,
    /// Format-specific position when known exactly (from sidecars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<(String, String)>,
    /// Local time as written by the source, "YYYY-MM-DD HH:MM:SS".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// "kindle-clippings", "kindle-sidecar", "koreader", "aristotle".
    pub source: String,
}

impl Index {
    pub fn load(path: &Path) -> Result<Index> {
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Index {
                    version: INDEX_VERSION,
                    ..Index::default()
                });
            }
            Err(e) => return Err(e.into()),
        };
        let index: Index =
            serde_json::from_slice(&data).map_err(|e| Error::Index(e.to_string()))?;
        if index.version > INDEX_VERSION {
            return Err(Error::Index(format!(
                "version {} is newer than this build ({INDEX_VERSION})",
                index.version
            )));
        }
        Ok(index)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let data = serde_json::to_vec(self).map_err(|e| Error::Index(e.to_string()))?;
        write_atomic(path, &data)
    }
}

/// Writes `data` to `path` via a temporary file and a rename, with an fsync
/// in between so the rename never exposes an empty file after power loss.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_empty_index() {
        let dir = crate::testdir("index");
        let idx = Index::load(&dir.join("nope.json")).unwrap();
        assert_eq!(idx.version, INDEX_VERSION);
        assert!(idx.books.is_empty());
    }

    #[test]
    fn roundtrip_and_unknown_fields() {
        let dir = crate::testdir("index");
        let p = dir.join("library.json");
        let mut idx = Index {
            version: 1,
            ..Index::default()
        };
        idx.books.push(BookRecord {
            id: "abc".into(),
            file: FileStamp {
                path: "Platon/Uczta.epub".into(),
                size: 10,
                mtime: 5,
                mtime_ns: 0,
            },
            format: Format::Epub,
            meta: Metadata {
                title: "Uczta".into(),
                ..Metadata::default()
            },
            cover: None,
            added: 1,
            last_opened: None,
            status: Status::Reading,
            progress: Some(Progress {
                fraction: 0.5,
                locator: Locator::Epub {
                    spine: 3,
                    offset: 120,
                },
                updated: 2,
                source: None,
                page: None,
            }),
            settings: ReaderSettings {
                font_size: Some(11.5),
                ..Default::default()
            },
            annotations: vec![],
        });
        idx.save(&p).unwrap();
        let back = Index::load(&p).unwrap();
        assert_eq!(back.books, idx.books);

        // Files written by a future minor version with extra fields still load.
        let json = String::from_utf8(fs::read(&p).unwrap()).unwrap().replacen(
            "\"added\":1",
            "\"added\":1,\"future\":true",
            1,
        );
        fs::write(&p, json).unwrap();
        assert_eq!(Index::load(&p).unwrap().books.len(), 1);
    }
}
