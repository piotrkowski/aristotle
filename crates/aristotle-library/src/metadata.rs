//! Book metadata and the per-format dispatch.
//!
//! Precedence, highest first:
//!
//! 1. Metadata embedded in the file (OPF, EXTH, PDF Info dictionary).
//! 2. The folder layout: on the device books sit in `documents/<Author>/`,
//!    so a book without an embedded author takes the name of its folder.
//! 3. The file name, for the title (always the case for TXT).
//!
//! Every parser returns what it found and leaves gaps empty; [`extract`]
//! fills the gaps from 2 and 3 so that every book has a title.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::format::Format;
use crate::text;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<Series>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// Publication date as written in the book (often just a year or an ISO date).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub identifiers: Vec<Identifier>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
    /// Page count for fixed-layout formats (PDF pages, CBZ images).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<u32>,
    /// Where the title came from, so the UI can mark guessed titles.
    #[serde(default)]
    pub title_source: TitleSource,
    /// The file claims DRM (MOBI encryption, PDF /Encrypt). Such books are
    /// listed but the reader will not open them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub drm: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    /// Sort name ("Mickiewicz, Adam"): from `file-as` when present, else derived.
    pub sort: String,
}

impl Author {
    pub fn new(name: &str, file_as: Option<&str>) -> Author {
        let name = text::clean(name);
        let sort = match file_as.map(text::clean) {
            Some(f) if !f.is_empty() => f,
            _ => text::author_sort_name(&name),
        };
        Author { name, sort }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identifier {
    /// Lowercase scheme: `isbn`, `uuid`, `asin`, `calibre`, `doi`, or empty.
    pub scheme: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleSource {
    #[default]
    Embedded,
    Filename,
}

/// Everything a parser found: metadata plus the raw cover image bytes, which
/// the scanner turns into a thumbnail and then drops.
#[derive(Debug, Default)]
pub struct Extracted {
    pub meta: Metadata,
    pub cover: Option<Vec<u8>>,
    /// A Kindle dictionary (EXTH 531), which belongs in the dictionary
    /// lookup, not in the library.
    pub dictionary: bool,
}

/// Extracts metadata (and the cover, if `want_cover`) from a book, then
/// applies the folder and filename fallbacks. `author_dir` is the name of the
/// book's folder below the library root, if any.
pub fn extract(
    path: &Path,
    format: Format,
    author_dir: Option<&str>,
    want_cover: bool,
) -> Result<Extracted> {
    let mut ex = match format {
        Format::Epub => crate::epub::extract(path, want_cover)?,
        Format::Mobi | Format::Azw3 => crate::mobi::extract(path, want_cover)?,
        Format::Pdf => crate::pdf::extract(path)?,
        Format::Cbz => crate::epub::extract_cbz(path, want_cover)?,
        Format::Txt => Extracted::default(),
    };
    finish(&mut ex.meta, path, author_dir);
    Ok(ex)
}

fn finish(meta: &mut Metadata, path: &Path, author_dir: Option<&str>) {
    meta.title = text::clean(&meta.title);
    if meta.title.is_empty() {
        meta.title = title_from_filename(path);
        meta.title_source = TitleSource::Filename;
    }
    meta.authors.retain(|a| !a.name.is_empty());
    if meta.authors.is_empty() {
        if let Some(dir) = author_dir {
            meta.authors.push(Author::new(dir, None));
        }
    }
    let mut seen = Vec::new();
    meta.authors.retain(|a| {
        let k = text::fold(&a.name);
        let fresh = !seen.contains(&k);
        seen.push(k);
        fresh
    });
    if let Some(l) = &meta.language {
        let l = l.trim().to_lowercase();
        meta.language = if l.is_empty() || l == "und" {
            None
        } else {
            Some(l)
        };
    }
    for field in [
        &mut meta.description,
        &mut meta.publisher,
        &mut meta.published,
    ] {
        if field.as_deref().is_some_and(|s| s.trim().is_empty()) {
            *field = None;
        }
    }
}

/// "Obrona_Sokratesa__p1234567890.epub" → "Obrona Sokratesa". Underscores
/// become spaces (shops use them instead of spaces) and a trailing store
/// product code after a double underscore is dropped.
pub fn title_from_filename(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = match stem.find("__") {
        Some(i) if i > 0 => stem[..i].to_owned(),
        _ => stem,
    };
    text::clean(&stem.replace('_', " "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_titles() {
        assert_eq!(
            title_from_filename(Path::new("a/Obrona_Sokratesa__p1234567890.epub")),
            "Obrona Sokratesa"
        );
        assert_eq!(title_from_filename(Path::new("Lalka.mobi")), "Lalka");
    }

    #[test]
    fn folder_supplies_missing_author() {
        let mut m = Metadata::default();
        finish(
            &mut m,
            Path::new("documents/Bolesław Prus/Lalka.txt"),
            Some("Bolesław Prus"),
        );
        assert_eq!(m.title, "Lalka");
        assert_eq!(m.title_source, TitleSource::Filename);
        assert_eq!(
            m.authors,
            vec![Author {
                name: "Bolesław Prus".into(),
                sort: "Prus, Bolesław".into()
            }]
        );
    }
}
