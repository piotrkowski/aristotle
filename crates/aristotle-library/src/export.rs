//! Exporting highlights: the stock Kindle's `My Clippings.txt` and a clean
//! Markdown file per book.
//!
//! The owner's notes pipeline (a host tool that turns `My Clippings.txt`
//! into Obsidian notes) must keep working unchanged, so the clippings are
//! written exactly as the stock reader writes them in its en_GB locale
//! ([`crate::clippings`] documents the template, read from firmware
//! 5.13.6):
//!
//! ```text
//! {title} ({author})
//! - Your Highlight at location 60-63 | Added on Thursday, 14 December 2023 20:27:45
//!
//! {text}
//! ==========
//! ```
//!
//! with CRLF line ends and a BOM only when the file is created. The labels
//! stay English on purpose: the pipeline matches "Highlight", "Your Note",
//! "location" and English month names.
//!
//! Appending is idempotent: entries whose title and text are already in the
//! file are not written again, so exporting after every new highlight, or
//! twice, never duplicates anything. Deleting a highlight in the reader
//! does not remove it from the file, as on the stock Kindle.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::clippings::{self, title_key};
use crate::index::{Annotation, AnnotationKind, BookRecord};

/// Where the stock reader keeps its clippings on the device.
pub const CLIPPINGS_PATH: &str = "/mnt/us/documents/My Clippings.txt";
/// Where the Markdown notes go on the device.
pub const NOTES_DIR: &str = "/mnt/us/aristotle-notes";

const SEPARATOR: &str = "==========";
const MONTHS: [&str; 12] = [
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
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// The book's title line: "Title (Author One; Author Two)", as the stock
/// reader writes it.
pub fn title_line(book: &BookRecord) -> String {
    let authors: Vec<&str> = book.meta.authors.iter().map(|a| a.name.as_str()).collect();
    if authors.is_empty() {
        book.meta.title.clone()
    } else {
        format!("{} ({})", book.meta.title, authors.join("; "))
    }
}

/// Text on one line with single spaces: what the stock reader stores and
/// what line-based parsers expect.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Day of the week (0 = Monday) of a proleptic Gregorian date.
fn weekday(y: i32, m: u32, d: u32) -> usize {
    // Days since 1970-01-01 (a Thursday), Howard Hinnant's algorithm.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe as u32 * 365 + yoe as u32 / 4 - yoe as u32 / 100 + doy;
    let days = era as i64 * 146_097 + doe as i64 - 719_468;
    (days + 3).rem_euclid(7) as usize
}

/// "YYYY-MM-DD HH:MM:SS" as the en_GB clippings date, "Thursday, 14
/// December 2023 20:27:45".
pub fn kindle_date(created: &str) -> Option<String> {
    let (date, time) = created.trim().split_once(' ')?;
    let mut parts = date.split('-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(format!(
        "{}, {} {} {} {}",
        WEEKDAYS[weekday(y, m, d)],
        d,
        MONTHS[m as usize - 1],
        y,
        time.trim()
    ))
}

fn location_label(loc: Option<(u32, u32)>) -> String {
    match loc {
        Some((a, b)) if b > a => format!(" at location {a}-{b}"),
        Some((a, _)) => format!(" at location {a}"),
        None => String::new(),
    }
}

/// One clippings entry with CRLF line ends, separator included. A note is
/// a "Your Note" entry at its highlight's first location: the owner's
/// parser attaches a pending note to the next highlight whose range starts
/// there.
pub fn entry(title_line: &str, kind: &AnnotationKind, a: &Annotation, text: &str) -> String {
    let (label, loc) = match kind {
        AnnotationKind::Highlight => ("Your Highlight", a.location),
        AnnotationKind::Note => ("Your Note", a.location.map(|(s, _)| (s, s))),
        AnnotationKind::Bookmark => ("Your Bookmark", a.location.map(|(s, _)| (s, s))),
    };
    let mut meta = format!("- {label}{}", location_label(loc));
    if let Some(date) = a.created.as_deref().and_then(kindle_date) {
        meta.push_str(" | Added on ");
        meta.push_str(&date);
    }
    format!(
        "{title_line}\r\n{meta}\r\n\r\n{}\r\n{SEPARATOR}\r\n",
        one_line(text)
    )
}

/// The clippings entries for a book's highlights made in Aristotle (and
/// their notes). Bookmarks are left out: they are reading aids, and the
/// pipeline ignores them anyway.
pub fn entries(book: &BookRecord) -> Vec<(String, String)> {
    let head = title_line(book);
    let mut out = Vec::new();
    for a in highlights(book) {
        let Some(text) = a.text.as_deref().filter(|t| !t.trim().is_empty()) else {
            continue;
        };
        // A note goes first: the stock reader writes it right before or
        // after its highlight, and parsers that attach notes to the next
        // highlight (the owner's does) need it before.
        if let Some(note) = a.note.as_deref().filter(|n| !n.trim().is_empty()) {
            out.push((one_line(note), entry(&head, &AnnotationKind::Note, a, note)));
        }
        out.push((
            one_line(text),
            entry(&head, &AnnotationKind::Highlight, a, text),
        ));
    }
    out
}

/// Aristotle's highlights of a book, in reading order.
pub fn highlights(book: &BookRecord) -> Vec<&Annotation> {
    let mut v: Vec<&Annotation> = book
        .annotations
        .iter()
        .filter(|a| a.kind == AnnotationKind::Highlight && a.source == "aristotle")
        .collect();
    v.sort_by_key(|a| a.location.map(|l| l.0).unwrap_or(u32::MAX));
    v
}

/// Appends the book's highlights that `path` does not hold yet. Creates the
/// file (with a BOM, like the stock reader) if needed. Returns how many
/// entries were written.
pub fn append_clippings(path: &Path, book: &BookRecord) -> io::Result<usize> {
    let existing = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    let key = title_key(&book.meta.title);
    let have: Vec<String> = clippings::parse(&existing)
        .clippings
        .into_iter()
        .filter(|c| title_key(&c.title) == key)
        .map(|c| one_line(&c.text))
        .collect();
    let mut out = String::new();
    let mut n = 0;
    for (text, e) in entries(book) {
        if have.contains(&text) {
            continue;
        }
        out.push_str(&e);
        n += 1;
    }
    if n == 0 {
        return Ok(0);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    if existing.is_empty() {
        f.write_all("\u{FEFF}".as_bytes())?;
    } else if !existing.ends_with(b"\n") {
        f.write_all(b"\r\n")?;
    }
    f.write_all(out.as_bytes())?;
    f.sync_all()?;
    Ok(n)
}

/// File name of a book's Markdown notes: "Title — Author.md", with the
/// characters Obsidian and FAT reject removed (the owner's pipeline names
/// its notes the same way).
pub fn markdown_name(book: &BookRecord) -> String {
    let author = book
        .meta
        .authors
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let base = if author.is_empty() {
        book.meta.title.clone()
    } else {
        format!("{} — {}", book.meta.title, author)
    };
    let clean: String = base
        .chars()
        .filter(|c| !"\\/:*?\"<>|#^[]".contains(*c) && !c.is_control())
        .take(150)
        .collect();
    format!("{}.md", clean.trim())
}

fn yaml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// "lok. 60-63", the location as the owner's notes write it.
pub fn location_pl(loc: Option<(u32, u32)>) -> Option<String> {
    match loc? {
        (a, b) if b > a => Some(format!("lok. {a}-{b}")),
        (a, _) => Some(format!("lok. {a}")),
    }
}

/// A book's highlights as Markdown in the shape of the owner's Obsidian
/// notes: front matter, title, author, then one quote per highlight with
/// its chapter, location and date, and the note if there is one.
pub fn markdown(book: &BookRecord) -> String {
    let author = book
        .meta
        .authors
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut s = format!(
        "---\ntytuł: {}\nautor: {}\nźródło: Aristotle\ntags: [kindle, zakreślenia]\n---\n\n# {}\n",
        yaml_string(&book.meta.title),
        yaml_string(&author),
        book.meta.title
    );
    if !author.is_empty() {
        s.push_str(&format!("\n*{author}*\n"));
    }
    for a in highlights(book) {
        let Some(text) = a.text.as_deref() else {
            continue;
        };
        let date = a.created.as_deref().map(|c| c.get(..10).unwrap_or(c));
        let meta: Vec<String> = [a.chapter.clone(), location_pl(a.location)]
            .into_iter()
            .flatten()
            .chain(date.map(String::from))
            .filter(|x| !x.trim().is_empty())
            .collect();
        s.push_str(&format!("\n> {}\n", one_line(text)));
        if !meta.is_empty() {
            s.push_str(&format!("\n— {}\n", meta.join(" · ")));
        }
        if let Some(note) = a.note.as_deref().filter(|n| !n.trim().is_empty()) {
            s.push_str(&format!("\n**Notatka:** {}\n", one_line(note)));
        }
    }
    s
}

/// Writes (replaces) the book's Markdown file in `dir`. A book without
/// highlights gets no file, and an existing one is left alone.
pub fn write_markdown(dir: &Path, book: &BookRecord) -> io::Result<Option<PathBuf>> {
    if highlights(book).is_empty() {
        return Ok(None);
    }
    let path = dir.join(markdown_name(book));
    crate::index::write_atomic(&path, markdown(book).as_bytes())
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{FileStamp, Status};
    use crate::metadata::{Author, Metadata};
    use crate::Format;

    fn book() -> BookRecord {
        BookRecord {
            id: "x".into(),
            file: FileStamp {
                path: "Platon/Fedon.epub".into(),
                size: 1,
                mtime: 1,
                mtime_ns: 0,
            },
            format: Format::Epub,
            meta: Metadata {
                title: "Fedon".into(),
                authors: vec![Author {
                    name: "Platon".into(),
                    sort: "Platon".into(),
                }],
                ..Metadata::default()
            },
            cover: None,
            added: 0,
            last_opened: None,
            status: Status::Reading,
            progress: None,
            settings: Default::default(),
            annotations: vec![
                highlight("Dusza jest nieśmiertelna.", (120, 121), None),
                highlight(
                    "Filozofowanie to\n przygotowanie do śmierci.",
                    (60, 63),
                    Some("por. Fajdros"),
                ),
            ],
        }
    }

    pub(crate) fn highlight(text: &str, loc: (u32, u32), note: Option<&str>) -> Annotation {
        Annotation {
            kind: AnnotationKind::Highlight,
            text: Some(text.into()),
            page: None,
            location: Some(loc),
            position: Some(("epub:3:100".into(), "epub:3:180".into())),
            created: Some("2026-10-17 21:05:09".into()),
            source: "aristotle".into(),
            chapter: Some("Rozmowa w więzieniu".into()),
            note: note.map(String::from),
        }
    }

    #[test]
    fn dates_in_the_en_gb_form() {
        assert_eq!(
            kindle_date("2023-12-14 20:27:45").as_deref(),
            Some("Thursday, 14 December 2023 20:27:45")
        );
        assert_eq!(
            kindle_date("2026-10-17 08:00:00").as_deref(),
            Some("Saturday, 17 October 2026 08:00:00")
        );
        assert_eq!(
            kindle_date("2024-02-29 12:00:00").as_deref(),
            Some("Thursday, 29 February 2024 12:00:00")
        );
        assert_eq!(kindle_date("garbage"), None);
    }

    #[test]
    fn entries_match_the_stock_template() {
        let b = book();
        let e = entries(&b);
        // Reading order; the note comes right before its highlight.
        assert_eq!(e.len(), 3);
        assert_eq!(
            e[0].1,
            "Fedon (Platon)\r\n- Your Note at location 60 | Added on Saturday, 17 October 2026 21:05:09\r\n\r\npor. Fajdros\r\n==========\r\n"
        );
        assert_eq!(
            e[1].1,
            "Fedon (Platon)\r\n- Your Highlight at location 60-63 | Added on Saturday, 17 October 2026 21:05:09\r\n\r\nFilozofowanie to przygotowanie do śmierci.\r\n==========\r\n"
        );
    }

    #[test]
    fn append_is_idempotent_and_round_trips() {
        let dir = crate::testdir("export");
        let path = dir.join("My Clippings.txt");
        // A stock entry is already there.
        fs::write(
            &path,
            "\u{FEFF}Lalka (Prus, Bolesław)\r\n- Your Highlight at location 1-2 | Added on Saturday, 16 December 2023 09:12:40\r\n\r\nWokulski.\r\n==========\r\n",
        )
        .unwrap();
        let mut b = book();
        assert_eq!(append_clippings(&path, &b).unwrap(), 3);
        assert_eq!(append_clippings(&path, &b).unwrap(), 0);
        b.annotations.push(highlight("Trzecie.", (200, 200), None));
        assert_eq!(append_clippings(&path, &b).unwrap(), 1);

        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.iter().filter(|&&c| c == 0xEF).count(), 1, "one BOM");
        let parsed = clippings::parse(&bytes);
        assert_eq!(parsed.skipped, 0);
        let fedon: Vec<_> = parsed
            .clippings
            .iter()
            .filter(|c| c.title == "Fedon")
            .collect();
        assert_eq!(fedon.len(), 4);
        assert_eq!(fedon[1].kind, AnnotationKind::Highlight);
        assert_eq!(fedon[1].author.as_deref(), Some("Platon"));
        assert_eq!(fedon[1].location, Some((60, 63)));
        assert_eq!(fedon[1].added.as_deref(), Some("2026-10-17 21:05:09"));
        assert_eq!(fedon[1].text, "Filozofowanie to przygotowanie do śmierci.");
        assert_eq!(fedon[0].kind, AnnotationKind::Note);
        assert_eq!(fedon[3].location, Some((200, 200)));
    }

    #[test]
    fn markdown_in_the_owners_shape() {
        let md = markdown(&book());
        assert!(md.starts_with(
            "---\ntytuł: \"Fedon\"\nautor: \"Platon\"\nźródło: Aristotle\ntags: [kindle, zakreślenia]\n---\n\n# Fedon\n\n*Platon*\n"
        ));
        assert!(md.contains(
            "\n> Filozofowanie to przygotowanie do śmierci.\n\n— Rozmowa w więzieniu · lok. 60-63 · 2026-10-17\n\n**Notatka:** por. Fajdros\n"
        ));
        let first = md.find("Filozofowanie").unwrap();
        let second = md.find("Dusza").unwrap();
        assert!(first < second, "reading order");
        assert_eq!(markdown_name(&book()), "Fedon — Platon.md");
    }

    #[test]
    fn markdown_file_names_lose_forbidden_characters() {
        let mut b = book();
        b.meta.title = "Obrona: \"Sokratesa\" [wyd. 2]?".into();
        assert_eq!(markdown_name(&b), "Obrona Sokratesa wyd. 2 — Platon.md");
        let dir = crate::testdir("export-md");
        let p = write_markdown(&dir, &b).unwrap().unwrap();
        assert!(fs::read_to_string(p)
            .unwrap()
            .contains("tytuł: \"Obrona: \\\"Sokratesa\\\""));
    }
}
