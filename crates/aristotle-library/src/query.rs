// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sorting, filtering and search over the index.
//!
//! The whole library is a few dozen records, so queries are a linear filter
//! and a sort over borrowed records; there is no secondary index to keep in
//! sync. Text comparisons use [`text::fold`] keys, so "Łódź" sorts with "L"
//! and a search for "zolc" finds "żółć". That is not Polish collation (where
//! "ł" follows "l"), but it is predictable and needs no locale tables.

use std::cmp::Ordering;

use crate::format::Format;
use crate::index::{BookRecord, Status};
use crate::text;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    /// Last opened first; never-opened books follow, newest additions first.
    #[default]
    Recent,
    Title,
    /// First author's sort name, then series order, then title.
    Author,
    /// Series name and index; books outside a series come last, by title.
    Series,
    /// Newest additions first.
    Added,
}

impl SortKey {
    pub fn parse(s: &str) -> Option<SortKey> {
        Some(match s {
            "recent" => SortKey::Recent,
            "title" => SortKey::Title,
            "author" => SortKey::Author,
            "series" => SortKey::Series,
            "added" => SortKey::Added,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusFilter {
    Unread,
    Reading,
    Finished,
    /// Unread or reading.
    Unfinished,
}

impl StatusFilter {
    pub fn parse(s: &str) -> Option<StatusFilter> {
        Some(match s {
            "unread" => StatusFilter::Unread,
            "reading" => StatusFilter::Reading,
            "finished" => StatusFilter::Finished,
            "unfinished" => StatusFilter::Unfinished,
            _ => return None,
        })
    }

    fn accepts(self, s: Status) -> bool {
        match self {
            StatusFilter::Unread => s == Status::Unread,
            StatusFilter::Reading => s == Status::Reading,
            StatusFilter::Finished => s == Status::Finished,
            StatusFilter::Unfinished => s != Status::Finished,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    pub sort: SortKey,
    pub reverse: bool,
    /// Empty means all formats.
    pub formats: Vec<Format>,
    pub status: Option<StatusFilter>,
    /// Diacritic- and case-insensitive; every word must occur in the title,
    /// an author name or the series name.
    pub search: Option<String>,
}

pub fn run<'a>(books: &'a [BookRecord], q: &Query) -> Vec<&'a BookRecord> {
    let needle = q
        .search
        .as_deref()
        .map(text::fold)
        .filter(|s| !s.is_empty());
    let filtered = books
        .iter()
        .filter(|b| q.formats.is_empty() || q.formats.contains(&b.format))
        .filter(|b| q.status.is_none_or(|s| s.accepts(b.status)))
        .filter(|b| {
            needle
                .as_deref()
                .is_none_or(|n| text::matches_folded(&search_text(b), n))
        });
    // Precompute keys: folding inside the comparator would redo it n log n times.
    let mut keyed: Vec<(SortKeyValue, &BookRecord)> =
        filtered.map(|b| (key(b, q.sort), b)).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    if q.reverse {
        keyed.reverse();
    }
    keyed.into_iter().map(|(_, b)| b).collect()
}

fn search_text(b: &BookRecord) -> String {
    let mut s = b.meta.title.clone();
    for a in &b.meta.authors {
        s.push(' ');
        s.push_str(&a.name);
    }
    if let Some(se) = &b.meta.series {
        s.push(' ');
        s.push_str(&se.name);
    }
    text::fold(&s)
}

/// A sort key compared part by part. [`key`] builds every key for one
/// [`SortKey`] in the same shape, so parts at one position share a variant.
struct SortKeyValue(Vec<KeyPart>);

enum KeyPart {
    Int(i64),
    Num(f64),
    Str(String),
}

impl SortKeyValue {
    fn cmp(&self, other: &Self) -> Ordering {
        for (a, b) in self.0.iter().zip(&other.0) {
            let o = match (a, b) {
                (KeyPart::Int(a), KeyPart::Int(b)) => a.cmp(b),
                (KeyPart::Num(a), KeyPart::Num(b)) => a.total_cmp(b),
                (KeyPart::Str(a), KeyPart::Str(b)) => a.cmp(b),
                _ => Ordering::Equal,
            };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    }
}

fn key(b: &BookRecord, sort: SortKey) -> SortKeyValue {
    use KeyPart::*;
    let title = Str(text::fold(&b.meta.title));
    let author = Str(b
        .meta
        .authors
        .first()
        .map(|a| text::fold(&a.sort))
        .unwrap_or_else(|| "\u{10FFFF}".into()));
    let (series, index) = match &b.meta.series {
        Some(s) => (Str(text::fold(&s.name)), Num(s.index.unwrap_or(f64::MAX))),
        None => (Str("\u{10FFFF}".into()), Num(f64::MAX)),
    };
    let path = Str(b.file.path.clone());
    SortKeyValue(match sort {
        SortKey::Recent => vec![
            Int(-b.last_opened.unwrap_or(i64::MIN + 1)),
            Int(-b.added),
            title,
            path,
        ],
        SortKey::Title => vec![title, author, path],
        SortKey::Author => vec![author, series, index, title, path],
        SortKey::Series => vec![series, index, title, path],
        SortKey::Added => vec![Int(-b.added), title, path],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::FileStamp;
    use crate::metadata::{Author, Metadata, Series};

    fn book(
        title: &str,
        author: &str,
        series: Option<(&str, f64)>,
        added: i64,
        opened: Option<i64>,
        format: Format,
        status: Status,
    ) -> BookRecord {
        BookRecord {
            id: title.into(),
            file: FileStamp {
                path: format!("{author}/{title}"),
                size: 0,
                mtime: 0,
                mtime_ns: 0,
            },
            format,
            meta: Metadata {
                title: title.into(),
                authors: vec![Author::new(author, None)],
                series: series.map(|(n, i)| Series {
                    name: n.into(),
                    index: Some(i),
                }),
                ..Metadata::default()
            },
            cover: None,
            added,
            last_opened: opened,
            status,
            progress: None,
            settings: Default::default(),
            annotations: vec![],
        }
    }

    fn sample() -> Vec<BookRecord> {
        vec![
            book(
                "Uczta",
                "Platon",
                Some(("Dialogi", 2.0)),
                10,
                Some(500),
                Format::Epub,
                Status::Reading,
            ),
            book(
                "Łąki",
                "Zofia Żak",
                None,
                30,
                None,
                Format::Pdf,
                Status::Unread,
            ),
            book(
                "Fedon",
                "Platon",
                Some(("Dialogi", 1.0)),
                20,
                Some(900),
                Format::Epub,
                Status::Finished,
            ),
            book(
                "Lalka",
                "Bolesław Prus",
                None,
                40,
                None,
                Format::Mobi,
                Status::Unread,
            ),
        ]
    }

    fn titles(v: Vec<&BookRecord>) -> Vec<&str> {
        v.into_iter().map(|b| b.meta.title.as_str()).collect()
    }

    #[test]
    fn sorts() {
        let b = sample();
        let q = |sort| Query {
            sort,
            ..Query::default()
        };
        assert_eq!(
            titles(run(&b, &q(SortKey::Recent))),
            ["Fedon", "Uczta", "Lalka", "Łąki"]
        );
        assert_eq!(
            titles(run(&b, &q(SortKey::Title))),
            ["Fedon", "Łąki", "Lalka", "Uczta"]
        );
        assert_eq!(
            titles(run(&b, &q(SortKey::Author))),
            ["Fedon", "Uczta", "Lalka", "Łąki"]
        );
        assert_eq!(
            titles(run(&b, &q(SortKey::Series))),
            ["Fedon", "Uczta", "Łąki", "Lalka"]
        );
        assert_eq!(
            titles(run(
                &b,
                &Query {
                    sort: SortKey::Added,
                    reverse: true,
                    ..Query::default()
                }
            )),
            ["Uczta", "Fedon", "Łąki", "Lalka"]
        );
    }

    #[test]
    fn filters_and_search() {
        let b = sample();
        let q = Query {
            formats: vec![Format::Epub],
            status: Some(StatusFilter::Unfinished),
            ..Query::default()
        };
        assert_eq!(titles(run(&b, &q)), ["Uczta"]);
        let q = Query {
            status: Some(StatusFilter::Unread),
            sort: SortKey::Title,
            ..Query::default()
        };
        assert_eq!(titles(run(&b, &q)), ["Łąki", "Lalka"]);
        let q = Query {
            search: Some("LAKI zak".into()),
            ..Query::default()
        };
        assert_eq!(titles(run(&b, &q)), ["Łąki"]);
        let q = Query {
            search: Some("bolesław".into()),
            ..Query::default()
        };
        assert_eq!(titles(run(&b, &q)), ["Lalka"]);
        let q = Query {
            search: Some("dialogi".into()),
            sort: SortKey::Series,
            ..Query::default()
        };
        assert_eq!(titles(run(&b, &q)), ["Fedon", "Uczta"]);
    }
}
