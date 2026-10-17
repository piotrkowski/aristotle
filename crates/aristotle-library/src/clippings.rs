//! Parser for the stock Kindle's `My Clippings.txt`.
//!
//! The format, read from firmware 5.13.6 (`ReaderSDK-impl.jar`, class
//! `Clipping`, bundles `ClippingsResources*`): UTF-8, a BOM only when the
//! file is created, CRLF line ends, and per entry
//!
//! ```text
//! {title} ({author})
//! - {label} | Added on {date, FULL} {time, MEDIUM}
//!
//! {text}
//! ==========
//! ```
//!
//! The label is localised and has three shapes: "Your Highlight at location
//! 60-63", "Your Highlight on page 12 | location 180-181" (books with page
//! numbers) and "Your Highlight on page 12" (PDF). Ranges are `a-b` (Dutch
//! `a t/m b`). Russian uses an en dash instead of the leading hyphen and puts
//! "в" between date and time.
//!
//! Firmware 5.13.6 has no Polish bundle; newer firmware writes Polish
//! labels we have not seen yet. So this parser does not match whole
//! templates. It finds the kind by keyword stems in many languages, tells
//! page from location by keyword (falling back to "location"), and takes the
//! date from the last `|` segment with a month-name table. The Polish date
//! form ("czwartek, 14 grudnia 2023 20:27:45", ICU `EEEE, d MMMM y
//! HH:mm:ss`) was checked against the firmware's ICU 58 data; the Polish
//! label words in the tests are our best guess.

use crate::encoding::decode_guess;
use crate::index::{Annotation, AnnotationKind};
use crate::text;

#[derive(Clone, Debug, PartialEq)]
pub struct Clipping {
    pub title: String,
    pub author: Option<String>,
    pub kind: AnnotationKind,
    /// Page label; front matter uses Roman numerals, so it is a string.
    pub page: Option<String>,
    pub location: Option<(u32, u32)>,
    /// "YYYY-MM-DD HH:MM:SS", local time of the device.
    pub added: Option<String>,
    /// Highlighted text or note text; empty for bookmarks.
    pub text: String,
}

impl Clipping {
    pub fn to_annotation(&self) -> Annotation {
        Annotation {
            kind: self.kind.clone(),
            text: Some(self.text.clone()).filter(|t| !t.is_empty()),
            page: self.page.clone(),
            location: self.location,
            position: None,
            created: self.added.clone(),
            source: "kindle-clippings".into(),
            chapter: None,
            note: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct ParseResult {
    pub clippings: Vec<Clipping>,
    /// Entries that had a separator but no recognisable header.
    pub skipped: usize,
}

const HIGHLIGHT: &[&str] = &[
    "highlight",
    "zakreśl",
    "zaznacz",
    "podkreśl",
    "markierung",
    "surlignement",
    "subrayado",
    "evidenziazione",
    "destaque",
    "выделен",
    "マーカー",
    "标注",
];
const NOTE: &[&str] = &["note", "notatk", "notiz", "nota", "заметк", "メモ", "笔记"];
const BOOKMARK: &[&str] = &[
    "bookmark",
    "zakładk",
    "lesezeichen",
    "signet",
    "marcador",
    "segnalibro",
    "bladwijzer",
    "закладк",
    "ブックマーク",
    "书签",
];
const PAGE: &[&str] = &[
    "page",
    "stron",
    "str.",
    "seite",
    "página",
    "pagina",
    "страниц",
    "ページ",
    "页",
];
const LOCATION: &[&str] = &[
    "location",
    "lokalizac",
    "pozycj",
    "poz.",
    "position",
    "emplacement",
    "posición",
    "posizione",
    "locatie",
    "posição",
    "позици",
    "位置",
];

pub fn parse(bytes: &[u8]) -> ParseResult {
    let text = decode_guess(bytes);
    let mut res = ParseResult::default();
    for raw in text.split("==========") {
        let lines: Vec<&str> = raw
            .lines()
            .map(|l| l.trim_start_matches('\u{FEFF}').trim_end_matches('\r'))
            .collect();
        let mut it = lines.iter().skip_while(|l| l.trim().is_empty());
        let (Some(title_line), Some(meta_line)) = (it.next(), it.next()) else {
            if raw.trim().is_empty() {
                continue;
            }
            res.skipped += 1;
            continue;
        };
        let body: Vec<&str> = it.copied().collect();
        match parse_entry(title_line, meta_line, &body) {
            Some(c) => res.clippings.push(c),
            None => res.skipped += 1,
        }
    }
    res
}

fn parse_entry(title_line: &str, meta_line: &str, body: &[&str]) -> Option<Clipping> {
    let meta = meta_line.trim();
    let meta = meta
        .strip_prefix('-')
        .or_else(|| meta.strip_prefix('–'))
        .unwrap_or(meta)
        .trim();
    let (title, author) = split_title_author(title_line.trim());
    let segments: Vec<&str> = meta.split('|').map(str::trim).collect();
    let first = segments.first()?.to_lowercase();
    let kind = if BOOKMARK.iter().any(|k| first.contains(k)) {
        AnnotationKind::Bookmark
    } else if HIGHLIGHT.iter().any(|k| first.contains(k)) {
        AnnotationKind::Highlight
    } else if NOTE.iter().any(|k| first.contains(k)) {
        AnnotationKind::Note
    } else {
        return None;
    };
    let mut page = None;
    let mut location = None;
    let mut added = None;
    for (i, seg) in segments.iter().enumerate() {
        let lower = seg.to_lowercase();
        let is_last = i + 1 == segments.len() && i > 0;
        if is_last {
            if let Some(d) = parse_date(seg) {
                added = Some(d);
                continue;
            }
        }
        let is_page =
            PAGE.iter().any(|k| lower.contains(k)) && !LOCATION.iter().any(|k| lower.contains(k));
        let Some(num) = number_part(seg, is_page) else {
            continue;
        };
        if is_page {
            page = Some(
                num.split(['-', '–'])
                    .next()
                    .unwrap_or(&num)
                    .trim()
                    .to_owned(),
            );
        } else if let Some(r) = parse_range(&num) {
            location = Some(r);
        }
    }
    let text = body.join("\n").trim().to_owned();
    Some(Clipping {
        title,
        author,
        kind,
        page,
        location,
        added,
        text,
    })
}

/// "Title (Author)" → title and author. The author is the last parenthesised
/// group, which keeps parentheses inside titles intact. No group means no
/// author (the stock reader then writes trailing spaces).
fn split_title_author(line: &str) -> (String, Option<String>) {
    let line = line.trim_end();
    if line.ends_with(')') {
        let mut depth = 0;
        for (i, c) in line.char_indices().rev() {
            match c {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        let title = line[..i].trim();
                        let author = line[i + 1..line.len() - 1].trim();
                        if !title.is_empty() && !author.is_empty() {
                            return (title.to_owned(), Some(author.to_owned()));
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    (line.to_owned(), None)
}

/// The number-ish tail of a segment: "location 180-181" → "180-181",
/// "page xii" → "xii", "lokalizacji 60 t/m 63" → "60 t/m 63".
fn number_part(seg: &str, allow_roman: bool) -> Option<String> {
    let words: Vec<&str> = seg.split_whitespace().collect();
    let start = words.iter().position(|w| {
        w.chars().next().is_some_and(|c| c.is_ascii_digit()) || allow_roman && is_roman(w)
    })?;
    Some(words[start..].join(" "))
}

fn is_roman(w: &str) -> bool {
    !w.is_empty() && w.len() <= 8 && w.chars().all(|c| "ivxlcdm".contains(c))
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    let s = s.replace("t/m", "-").replace('–', "-");
    let mut parts = s.split('-').map(|p| p.trim().parse::<u32>());
    let a = parts.next()?.ok()?;
    let b = parts.next().and_then(|r| r.ok()).unwrap_or(a);
    Some((a, b.max(a)))
}

/// Months in the forms clippings use: English, Polish nominative and
/// genitive, German, French, Spanish, Italian, Dutch, Portuguese, Russian
/// genitive. Matched by prefix so "września" and "wrzesień" both work.
const MONTHS: &[(&[&str], u32)] = &[
    (
        &[
            "jan",
            "styczeń",
            "styczn",
            "januar",
            "janvier",
            "enero",
            "gennaio",
            "janeiro",
            "январ",
        ],
        1,
    ),
    (
        &[
            "feb",
            "luty",
            "lutego",
            "februar",
            "février",
            "febrero",
            "febbraio",
            "februari",
            "fevereiro",
            "феврал",
        ],
        2,
    ),
    (
        &[
            "mar", "marzec", "marca", "märz", "mars", "marzo", "maart", "março", "март",
        ],
        3,
    ),
    (
        &[
            "apr",
            "kwiecień",
            "kwietni",
            "avril",
            "abril",
            "aprile",
            "апрел",
        ],
        4,
    ),
    (
        &[
            "may", "maj", "mai", "mayo", "maggio", "mei", "maio", "мая", "май",
        ],
        5,
    ),
    (
        &[
            "jun", "czerwiec", "czerwc", "juni", "juin", "junio", "giugno", "junho", "июн",
        ],
        6,
    ),
    (
        &[
            "jul", "lipiec", "lipca", "juli", "juillet", "julio", "luglio", "julho", "июл",
        ],
        7,
    ),
    (
        &[
            "aug",
            "sierpień",
            "sierpni",
            "august",
            "août",
            "agosto",
            "augustus",
            "август",
        ],
        8,
    ),
    (
        &[
            "sep",
            "wrzesień",
            "wrześni",
            "septemb",
            "septiembre",
            "settembre",
            "setembro",
            "сентябр",
        ],
        9,
    ),
    (
        &[
            "oct",
            "październik",
            "październik",
            "paźdz",
            "oktober",
            "octobre",
            "octubre",
            "ottobre",
            "outubro",
            "октябр",
        ],
        10,
    ),
    (
        &[
            "nov",
            "listopad",
            "novemb",
            "noviembre",
            "novembre",
            "novembro",
            "ноябр",
        ],
        11,
    ),
    (
        &[
            "dec",
            "grudzień",
            "grudni",
            "dezember",
            "décembre",
            "diciembre",
            "dicembre",
            "december",
            "dezembro",
            "декабр",
        ],
        12,
    ),
];

fn month_of(word: &str) -> Option<u32> {
    let w = word
        .trim_matches(|c: char| !c.is_alphabetic())
        .to_lowercase();
    if w.len() < 3 {
        return None;
    }
    MONTHS
        .iter()
        .find(|(names, _)| names.iter().any(|n| w.starts_with(n)))
        .map(|(_, m)| *m)
}

/// Parses the "Added on ..." segment in any of the supported locales:
/// finds a 4-digit year, a month name, a day number and `h:mm[:ss]` with an
/// optional AM/PM, regardless of order.
pub fn parse_date(seg: &str) -> Option<String> {
    let mut year = None;
    let mut month = None;
    let mut day = None;
    let mut time = None;
    let mut pm = None;
    let tokens: Vec<&str> = seg
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .collect();
    for t in &tokens {
        let tl = t.to_lowercase();
        if tl == "pm" || tl == "p.m." {
            pm = Some(true);
            continue;
        }
        if tl == "am" || tl == "a.m." {
            pm = Some(false);
            continue;
        }
        if t.contains(':') {
            let parts: Vec<u32> = t
                .split(':')
                .filter_map(|p| p.trim_matches(|c: char| !c.is_ascii_digit()).parse().ok())
                .collect();
            if parts.len() >= 2 {
                time = Some((parts[0], parts[1], parts.get(2).copied().unwrap_or(0)));
            }
            continue;
        }
        let digits = t.trim_end_matches(['.', 'г']).trim_end_matches('.');
        if let Ok(n) = digits.parse::<u32>() {
            if digits.len() == 4 {
                year = Some(n);
            } else if day.is_none() && (1..=31).contains(&n) {
                day = Some(n);
            }
            continue;
        }
        // Weekdays come before the month in every locale and some look like
        // month prefixes (French "mardi"), so the last match wins.
        if let Some(m) = month_of(t) {
            month = Some(m);
        }
    }
    let (mut h, mi, s) = time.unwrap_or((0, 0, 0));
    match pm {
        Some(true) if h < 12 => h += 12,
        Some(false) if h == 12 => h = 0,
        _ => {}
    }
    let (y, m, d) = (year?, month?, day?);
    if h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some(format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}"))
}

/// Normalised key for matching a clipping title to a library book.
pub fn title_key(title: &str) -> String {
    text::fold(title)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EN_GB: &str = "\u{FEFF}Pan Tadeusz. Historia szlachecka z roku 1811 i 1812 (Adam Mickiewicz)\r\n\
- Your Highlight at location 12-14 | Added on Thursday, 14 December 2023 20:27:45\r\n\r\n\
Litwo! Ojczyzno moja! ty jesteś jak zdrowie.\r\n==========\r\n\
\u{FEFF}Lalka (Prus, Bolesław)\r\n\
- Your Highlight on page 12 | location 180-181 | Added on Saturday, 16 December 2023 09:12:40\r\n\r\n\
Wokulski (nie bez racji) myślał...\r\n==========\r\n\
Lalka (Prus, Bolesław)\r\n\
- Your Bookmark on page xii | location 145 | Added on Saturday, 16 December 2023 09:13:05\r\n\r\n\r\n==========\r\n";

    #[test]
    fn english_entries() {
        let r = parse(EN_GB.as_bytes());
        assert_eq!(r.skipped, 0);
        assert_eq!(r.clippings.len(), 3);
        let c = &r.clippings[0];
        assert_eq!(
            c.title,
            "Pan Tadeusz. Historia szlachecka z roku 1811 i 1812"
        );
        assert_eq!(c.author.as_deref(), Some("Adam Mickiewicz"));
        assert_eq!(c.kind, AnnotationKind::Highlight);
        assert_eq!(c.location, Some((12, 14)));
        assert_eq!(c.page, None);
        assert_eq!(c.added.as_deref(), Some("2023-12-14 20:27:45"));
        assert_eq!(c.text, "Litwo! Ojczyzno moja! ty jesteś jak zdrowie.");
        let c = &r.clippings[1];
        assert_eq!(c.author.as_deref(), Some("Prus, Bolesław"));
        assert_eq!(
            (c.page.as_deref(), c.location),
            (Some("12"), Some((180, 181)))
        );
        let c = &r.clippings[2];
        assert_eq!(c.kind, AnnotationKind::Bookmark);
        assert_eq!(
            (c.page.as_deref(), c.location),
            (Some("xii"), Some((145, 145)))
        );
        assert_eq!(c.text, "");
    }

    #[test]
    fn us_english_date() {
        assert_eq!(
            parse_date("Added on Thursday, December 14, 2023 8:27:45 PM").as_deref(),
            Some("2023-12-14 20:27:45")
        );
        assert_eq!(
            parse_date("Added on Monday, January 2, 2023 12:05:00 AM").as_deref(),
            Some("2023-01-02 00:05:00")
        );
    }

    #[test]
    fn polish_entries() {
        let pl = "Państwo (Platon)\r\n\
- Twoje zakreślenie w lokalizacji 1406-1408 | Dodano: czwartek, 14 grudnia 2023 20:27:45\r\n\r\n\
Sprawiedliwość jest cnotą duszy.\r\n==========\r\n\
Państwo (Platon)\r\n\
- Twoja notatka na stronie 7 | lokalizacja 1408 | Dodano: piątek, 3 września 2021 07:05:09\r\n\r\n\
Por. Gorgiasz\r\n==========\r\n\
Państwo (Platon)\r\n\
- Twoja zakładka w lokalizacji 2000 | Dodano: niedziela, 1 maja 2022 0:00:01\r\n\r\n\r\n==========\r\n";
        let r = parse(pl.as_bytes());
        assert_eq!(r.clippings.len(), 3);
        let [a, b, c] = &r.clippings[..] else {
            panic!()
        };
        assert_eq!(
            (a.kind.clone(), a.location, a.added.as_deref()),
            (
                AnnotationKind::Highlight,
                Some((1406, 1408)),
                Some("2023-12-14 20:27:45")
            )
        );
        assert_eq!(
            (b.kind.clone(), b.page.as_deref(), b.location),
            (AnnotationKind::Note, Some("7"), Some((1408, 1408)))
        );
        assert_eq!(b.added.as_deref(), Some("2021-09-03 07:05:09"));
        assert_eq!(b.text, "Por. Gorgiasz");
        assert_eq!(
            (c.kind.clone(), c.added.as_deref()),
            (AnnotationKind::Bookmark, Some("2022-05-01 00:00:01"))
        );
    }

    #[test]
    fn other_locales_from_firmware() {
        // Templates from the 5.13.6 de, fr, nl and ru bundles.
        let de = "Buch (Autor)\n- Ihre Markierung auf Seite 3 | bei Position 40-41 | Hinzugefügt am Donnerstag, 14. Dezember 2023 20:27:45\n\nText\n==========\n";
        let c = &parse(de.as_bytes()).clippings[0];
        assert_eq!(
            (c.page.as_deref(), c.location, c.added.as_deref()),
            (Some("3"), Some((40, 41)), Some("2023-12-14 20:27:45"))
        );
        let fr = "Livre (Auteur)\n- Votre signet à l\u{2bc}emplacement 77 | Ajouté le jeudi 14 décembre 2023 20:27:45\n\n\n==========\n";
        let c = &parse(fr.as_bytes()).clippings[0];
        assert_eq!(
            (c.kind.clone(), c.location),
            (AnnotationKind::Bookmark, Some((77, 77)))
        );
        let nl = "Boek (Auteur)\n- Je highlight op locatie 10 t/m 12 | Toegevoegd op donderdag 14 december 2023 20:27:45\n\nx\n==========\n";
        assert_eq!(parse(nl.as_bytes()).clippings[0].location, Some((10, 12)));
        let ru = "Книга (Автор)\n\u{2013} Ваша заметка на странице 5 | Добавлено: четверг, 14 декабря 2023 г. в 20:27:45\n\nтекст\n==========\n";
        let c = &parse(ru.as_bytes()).clippings[0];
        assert_eq!(
            (c.kind.clone(), c.page.as_deref(), c.added.as_deref()),
            (AnnotationKind::Note, Some("5"), Some("2023-12-14 20:27:45"))
        );
    }

    #[test]
    fn titles_with_parentheses_and_no_author() {
        assert_eq!(
            split_title_author("Dzieła (tom 2) (Platon)"),
            ("Dzieła (tom 2)".into(), Some("Platon".into()))
        );
        assert_eq!(split_title_author("Notatki  "), ("Notatki".into(), None));
    }

    #[test]
    fn garbage_is_skipped() {
        let r = parse(b"just text\n==========\nTitle\n- something odd\n\nx\n==========\n");
        assert_eq!(r.clippings.len(), 0);
        assert_eq!(r.skipped, 2);
    }
}
