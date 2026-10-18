// SPDX-License-Identifier: AGPL-3.0-or-later
//! Highlights and bookmarks: how they are stored in the library index, and
//! exported for note-taking tools.
//!
//! An annotation made here is an [`Annotation`] with source "aristotle":
//!
//! - `position`: the start and end as renderer positions that survive a
//!   new font size (see [`aristotle_render::position`]), written as
//!   `epub:<spine item>:<offset>`, `kindle:<position>`, `page:<index>` or
//!   `fraction:<f>`;
//! - `location`: Kindle locations (150 bytes of text each, counted from 1),
//!   what `My Clippings.txt` and the Markdown notes show;
//! - `chapter`, `created` (local time), `text`.
//!
//! Every new highlight is appended to `My Clippings.txt` at once (as the
//! stock reader does, unless the setting "Save highlights to My
//! Clippings.txt" is off) and the book's Markdown file is rewritten
//! ([`aristotle_library::export`]).

use std::path::{Path, PathBuf};

use aristotle_library::export;
use aristotle_library::index::{Annotation, AnnotationKind, BookRecord};
use aristotle_render::book::Position;

pub const SOURCE: &str = "aristotle";

/// A navigation target only when the source provides an exact position.
pub fn target(a: &Annotation, format: aristotle_library::Format) -> Option<Position> {
    let start = &a.position.as_ref()?.0;
    match a.source.as_str() {
        SOURCE => decode(start),
        "kindle-sidecar"
            if matches!(
                format,
                aristotle_library::Format::Mobi | aristotle_library::Format::Azw3
            ) =>
        {
            aristotle_library::sidecar::kindle_position(start).map(Position::Kindle)
        }
        _ => None,
    }
}

/// The stock Kindle's bytes per location (calibre's constant).
const BYTES_PER_LOCATION: u64 = 150;

pub fn encode(p: &Position) -> String {
    match p {
        Position::Spine { item, offset } => format!("epub:{item}:{offset}"),
        Position::Kindle(k) => format!("kindle:{k}"),
        Position::Page(i) => format!("page:{i}"),
        Position::Fraction(f) => format!("fraction:{f}"),
    }
}

pub fn decode(s: &str) -> Option<Position> {
    let (kind, rest) = s.split_once(':')?;
    match kind {
        "epub" => {
            let (a, b) = rest.split_once(':')?;
            Some(Position::Spine {
                item: a.parse().ok()?,
                offset: b.parse().ok()?,
            })
        }
        "kindle" => rest.parse().ok().map(Position::Kindle),
        "page" => rest.parse().ok().map(Position::Page),
        "fraction" => rest.parse().ok().map(Position::Fraction),
        _ => None,
    }
}

/// Kindle location of a Plato location (or a MOBI position): 1-based, 150
/// bytes each. For EPUB it counts bytes of the book's XHTML, which is what
/// a stock-converted book would count too, give or take markup.
pub fn kindle_location(bytes: u64) -> u32 {
    (bytes / BYTES_PER_LOCATION + 1) as u32
}

/// Local time "YYYY-MM-DD HH:MM:SS" (the clippings' and notes' format).
pub fn local_time(unix: i64) -> String {
    // SAFETY: localtime_r only writes into the tm passed.
    unsafe {
        let t = unix as _;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return String::new();
        }
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

/// What the reader knows about a new highlight.
pub struct NewHighlight {
    pub text: String,
    pub start: Position,
    pub end: Position,
    /// Byte counts for Kindle locations: the start and end in the text.
    pub start_bytes: u64,
    pub end_bytes: u64,
    /// 1-based page for fixed-layout books.
    pub page: Option<usize>,
    pub chapter: Option<String>,
    pub created: String,
}

pub fn highlight(h: NewHighlight) -> Annotation {
    Annotation {
        kind: AnnotationKind::Highlight,
        text: Some(h.text.split_whitespace().collect::<Vec<_>>().join(" ")),
        page: h.page.map(|p| p.to_string()),
        location: if h.page.is_some() {
            None
        } else {
            Some((kindle_location(h.start_bytes), kindle_location(h.end_bytes)))
        },
        position: Some((encode(&h.start), encode(&h.end))),
        created: Some(h.created),
        source: SOURCE.into(),
        chapter: h.chapter,
        note: None,
    }
}

pub fn bookmark(at: &Position, bytes: u64, chapter: Option<String>, created: String) -> Annotation {
    Annotation {
        kind: AnnotationKind::Bookmark,
        text: None,
        page: None,
        location: Some((kindle_location(bytes), kindle_location(bytes))),
        position: Some((encode(at), encode(at))),
        created: Some(created),
        source: SOURCE.into(),
        chapter,
        note: None,
    }
}

/// The book's annotations made here, of one kind, with decoded positions.
pub fn ours(book: &BookRecord, kind: AnnotationKind) -> Vec<(usize, Position, Position)> {
    book.annotations
        .iter()
        .enumerate()
        .filter(|(_, a)| a.kind == kind && a.source == SOURCE)
        .filter_map(|(i, a)| {
            let (s, e) = a.position.as_ref()?;
            Some((i, decode(s)?, decode(e)?))
        })
        .collect()
}

/// Where exports go: `My Clippings.txt` in the library root (on the device
/// `/mnt/us/documents`) and `aristotle-notes/` next to the root
/// (`/mnt/us/aristotle-notes`).
#[derive(Clone, Debug)]
pub struct ExportPaths {
    pub clippings: PathBuf,
    pub notes: PathBuf,
    /// `clippings` is a test file (`--test-clippings`, the preview), not
    /// the user's: it is written whatever the setting says.
    pub test: bool,
}

impl ExportPaths {
    pub fn for_root(root: &Path) -> ExportPaths {
        ExportPaths {
            clippings: root.join("My Clippings.txt"),
            notes: root.parent().unwrap_or(root).join("aristotle-notes"),
            test: false,
        }
    }
}

/// Export the book's highlights; errors are logged, never fatal.
/// `clippings` is the setting "Save highlights to My Clippings.txt": off,
/// only the Markdown notes are rewritten and the user's `My Clippings.txt`
/// is not touched (a test file still gets the clippings).
pub fn export(paths: &ExportPaths, book: &BookRecord, clippings: bool) {
    if clippings || paths.test {
        match export::append_clippings(&paths.clippings, book) {
            Ok(n) if n > 0 => crate::perf!("export: {n} clippings appended"),
            Ok(_) => {}
            Err(e) => eprintln!("aristotle: {}: {e}", paths.clippings.display()),
        }
    }
    if let Err(e) = export::write_markdown(&paths.notes, book) {
        eprintln!("aristotle: {}: {e}", paths.notes.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aristotle_library::index::{FileStamp, Status};
    use aristotle_library::metadata::{Author, Metadata};
    use aristotle_library::Format;
    use regex::Regex;

    #[test]
    fn positions_round_trip() {
        for p in [
            Position::Spine {
                item: 3,
                offset: 1200,
            },
            Position::Kindle(84521),
            Position::Page(7),
            Position::Fraction(0.25),
        ] {
            assert_eq!(decode(&encode(&p)), Some(p.clone()));
        }
        assert_eq!(decode("xpointer:/body"), None);
        assert_eq!(kindle_location(0), 1);
        assert_eq!(kindle_location(7510), 51);
    }

    #[test]
    fn imported_targets_require_exact_positions_in_a_matching_format() {
        let mut a = bookmark(&Position::Kindle(7510), 7510, None, String::new());
        a.source = "kindle-sidecar".into();
        a.position = Some(("7510:7510:11:state".into(), "8000:8000:11:state".into()));
        assert_eq!(target(&a, Format::Mobi), Some(Position::Kindle(7510)));
        assert_eq!(target(&a, Format::Azw3), Some(Position::Kindle(7510)));
        assert_eq!(target(&a, Format::Epub), None);
        assert_eq!(target(&a, Format::Pdf), None);
        a.position = Some(("not a position".into(), "8000".into()));
        assert_eq!(target(&a, Format::Mobi), None);
        a.source = "kindle-clippings".into();
        a.position = None;
        a.location = Some((51, 54));
        assert_eq!(target(&a, Format::Mobi), None);
    }

    #[test]
    fn imported_annotations_never_become_editable_page_marks() {
        let mut b = book();
        let a = bookmark(
            &Position::Spine {
                item: 0,
                offset: 100,
            },
            100,
            None,
            String::new(),
        );
        let mut imported = a.clone();
        imported.source = "kindle-sidecar".into();
        b.annotations.extend([imported, a]);
        let positions = ours(&b, AnnotationKind::Bookmark);
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].0, 1);
    }

    /// A clippings-to-notes pipeline (a Python script that turns
    /// `My Clippings.txt` into Obsidian notes), ported line by line: how it
    /// splits entries, title and author, location and date, and attaches
    /// notes to highlights. Kept faithful, quirks included, so the export is
    /// tested against what such a pipeline reads.
    #[derive(Debug, PartialEq)]
    struct ParsedItem {
        title: String,
        author: String,
        text: String,
        r#where: String,
        date: String,
        note: String,
    }

    fn pipeline_parse(raw: &str) -> Vec<ParsedItem> {
        let months = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let raw = raw.trim_start_matches('\u{FEFF}').replace('\r', "");
        let head_re = Regex::new(r"^(.*?)\s*\(((?:[^()]|\([^()]*\))*)\)\s*$").unwrap();
        let loc_re = Regex::new(r"(location|page) ([\d-]+)").unwrap();
        let date_re = Regex::new(r"(\d{1,2}) (\w+) (\d{4}) (\d\d:\d\d)").unwrap();
        let mut pending: Vec<((String, String), String)> = Vec::new();
        let mut out = Vec::new();
        for chunk in raw.split("==========") {
            let lines: Vec<String> = chunk
                .trim()
                .split('\n')
                .map(|l| {
                    l.trim_matches(|c| c == '\u{FEFF}' || c == ' ')
                        .trim_end()
                        .to_string()
                })
                .collect();
            if lines.len() < 3 {
                continue;
            }
            let (head, meta) = (&lines[0], &lines[1]);
            let text = norm(&lines[2..].join(" "));
            let (title, author) = match head_re.captures(head) {
                Some(c) => (c[1].to_string(), c[2].replace(';', ", ")),
                None => (head.clone(), String::new()),
            };
            let r#where = loc_re
                .captures(meta)
                .map(|c| {
                    format!(
                        "{} {}",
                        if &c[1] == "location" { "lok." } else { "s." },
                        &c[2]
                    )
                })
                .unwrap_or_default();
            let date = date_re
                .captures(meta)
                .map(|c| {
                    let m = months.iter().position(|m| *m == &c[2]).map_or(1, |i| i + 1);
                    format!("{}-{:02}-{:02}", &c[3], m, c[1].parse::<u32>().unwrap())
                })
                .unwrap_or_default();
            let first = r#where.split('-').next().unwrap_or("").to_string();
            let last = r#where.split('-').next_back().unwrap_or("").to_string();
            if meta.contains("Your Note") {
                pending.push(((title.clone(), first), text));
                continue;
            }
            if !meta.contains("Highlight") || text.is_empty() {
                continue;
            }
            let mut take = |k: &(String, String)| {
                pending
                    .iter()
                    .position(|(pk, _)| pk == k)
                    .map(|i| pending.remove(i).1)
            };
            let note = take(&(title.clone(), last))
                .or_else(|| take(&(title.clone(), first)))
                .unwrap_or_default();
            out.push(ParsedItem {
                title: norm(&title),
                author: norm(&author),
                text,
                r#where,
                date,
                note,
            });
        }
        out
    }

    fn book() -> BookRecord {
        BookRecord {
            id: "x".into(),
            file: FileStamp {
                path: "Platon/fedon.epub".into(),
                size: 1,
                mtime: 1,
                mtime_ns: 0,
            },
            format: Format::Epub,
            meta: Metadata {
                title: "Fedon (Dialogi)".into(),
                authors: vec![
                    Author {
                        name: "Platon".into(),
                        sort: "Platon".into(),
                    },
                    Author {
                        name: "Władysław Witwicki".into(),
                        sort: "Witwicki, Władysław".into(),
                    },
                ],
                ..Metadata::default()
            },
            cover: None,
            added: 0,
            last_opened: None,
            status: Status::Reading,
            progress: None,
            settings: Default::default(),
            annotations: Vec::new(),
        }
    }

    #[test]
    fn export_round_trips_through_a_clippings_parser() {
        let mut b = book();
        let mut h = highlight(NewHighlight {
            text: "Kto się naprawdę\n oddaje filozofii, ten się sposobi do śmierci.".into(),
            start: Position::Spine {
                item: 2,
                offset: 9000,
            },
            end: Position::Spine {
                item: 2,
                offset: 9070,
            },
            start_bytes: 45_000,
            end_bytes: 45_070,
            page: None,
            chapter: Some("Fedon".into()),
            created: "2026-10-17 21:05:09".into(),
        });
        b.annotations.push(h.clone());
        h.text = Some("ψυχή — dusza".into());
        h.location = Some((900, 900));
        h.note = Some("por. Fajdros 245c".into());
        b.annotations.push(h);
        // A bookmark and a stock clipping are not exported.
        b.annotations.push(bookmark(
            &Position::Spine { item: 1, offset: 0 },
            10,
            None,
            "2026-10-17 21:00:00".into(),
        ));

        let dir = crate::testdir("marks-export");
        let paths = ExportPaths::for_root(&dir.join("documents"));
        std::fs::create_dir_all(dir.join("documents")).unwrap();
        export(&paths, &b, true);
        export(&paths, &b, true); // idempotent
        let raw = std::fs::read_to_string(&paths.clippings).unwrap();
        let items = pipeline_parse(&raw);
        assert_eq!(
            items,
            vec![
                ParsedItem {
                    title: "Fedon (Dialogi)".into(),
                    author: "Platon, Władysław Witwicki".into(),
                    text: "Kto się naprawdę oddaje filozofii, ten się sposobi do śmierci.".into(),
                    r#where: "lok. 301".into(),
                    date: "2026-10-17".into(),
                    note: String::new(),
                },
                ParsedItem {
                    title: "Fedon (Dialogi)".into(),
                    author: "Platon, Władysław Witwicki".into(),
                    text: "ψυχή — dusza".into(),
                    r#where: "lok. 900".into(),
                    date: "2026-10-17".into(),
                    note: "por. Fajdros 245c".into(),
                },
            ]
        );
        let md = std::fs::read_to_string(
            paths
                .notes
                .join("Fedon (Dialogi) — Platon, Władysław Witwicki.md"),
        )
        .unwrap();
        assert!(md.contains("> Kto się naprawdę oddaje filozofii, ten się sposobi do śmierci.\n\n— Fedon · loc. 301 · 2026-10-17\n"), "{md}");
    }

    #[test]
    fn clippings_setting_off_leaves_my_clippings_alone() {
        let mut b = book();
        b.annotations.push(highlight(NewHighlight {
            text: "Dusza jest nieśmiertelna.".into(),
            start: Position::Spine { item: 2, offset: 1 },
            end: Position::Spine {
                item: 2,
                offset: 20,
            },
            start_bytes: 1500,
            end_bytes: 1520,
            page: None,
            chapter: None,
            created: "2026-10-17 09:00:00".into(),
        }));
        let dir = crate::testdir("marks-no-clippings");
        std::fs::create_dir_all(dir.join("documents")).unwrap();
        let paths = ExportPaths::for_root(&dir.join("documents"));
        export(&paths, &b, false);
        assert!(!paths.clippings.exists(), "My Clippings.txt was written");
        // The highlight still reaches Aristotle's own notes.
        let md = std::fs::read_dir(&paths.notes).unwrap().count();
        assert_eq!(md, 1);
        // A test file (`--test-clippings`) is written whatever the setting.
        let test = ExportPaths {
            clippings: dir.join("test-clippings.txt"),
            notes: dir.join("test-notes"),
            test: true,
        };
        export(&test, &b, false);
        let raw = std::fs::read_to_string(&test.clippings).unwrap();
        assert!(raw.contains("Dusza jest nieśmiertelna."), "{raw}");
    }

    /// A real pipeline script, when given: run its parser on our export.
    /// The script is a Python module with `clippings(path)`:
    /// `ARISTOTLE_CLIPPINGS_SCRIPT=path cargo test ... clippings_script -- --ignored`.
    #[test]
    #[ignore]
    fn clippings_script() {
        let Some(script) = std::env::var_os("ARISTOTLE_CLIPPINGS_SCRIPT").map(PathBuf::from) else {
            return;
        };
        let mut b = book();
        b.annotations.push(highlight(NewHighlight {
            text: "Dusza jest nieśmiertelna.".into(),
            start: Position::Spine { item: 2, offset: 1 },
            end: Position::Spine {
                item: 2,
                offset: 20,
            },
            start_bytes: 1500,
            end_bytes: 1520,
            page: None,
            chapter: None,
            created: "2026-10-17 09:00:00".into(),
        }));
        let dir = crate::testdir("marks-script");
        let paths = ExportPaths::for_root(&dir);
        export(&paths, &b, true);
        let code = "import importlib.machinery as m, importlib.util as u, sys, json\n\
            from pathlib import Path\n\
            l = m.SourceFileLoader('kz', sys.argv[1]); s = u.spec_from_loader('kz', l)\n\
            kz = u.module_from_spec(s); l.exec_module(kz)\n\
            print(json.dumps(list(kz.clippings(Path(sys.argv[2]))), ensure_ascii=False))";
        let out = std::process::Command::new("python3")
            .args(["-I", "-c", code])
            .arg(&script)
            .arg(&paths.clippings)
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.contains("\"text\": \"Dusza jest nieśmiertelna.\""), "{s}");
        assert!(s.contains("\"where\": \"lok. 11\""), "{s}");
        assert!(s.contains("\"date\": \"2026-10-17\""), "{s}");
    }
}
