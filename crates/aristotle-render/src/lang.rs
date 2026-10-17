// SPDX-License-Identifier: AGPL-3.0-or-later
//! Choosing the language a book is hyphenated in.
//!
//! Plato hyphenates EPUB text in the book's first `dc:language`. Most books
//! declare exactly one, but some declare several in arbitrary order: in a
//! test library a Polish translation lists `de-DE`, `en-US`, `pl-PL`, and
//! got German patterns, which break Polish words in wrong places
//! ("zobac-zymy" instead of "zo-ba-czy-my"). When a book declares more than
//! one language, we look at its text and pick the one whose characteristic
//! letters occur most.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use plato_core::anyhow::{format_err, Error};
use zip::ZipArchive;

/// Letters that mark a language's text, by primary subtag. Languages
/// without an entry (English, for one) are what remains when none of these
/// letters is frequent.
const MARKERS: &[(&str, &str)] = &[
    ("pl", "ąćęłńśźżĄĆĘŁŃŚŹŻ"),
    ("cs", "ěščřžůĚŠČŘŽŮ"),
    ("sk", "ľĺŕôäĽĹŔÔ"),
    ("de", "äöüßÄÖÜ"),
    ("fr", "éèêàçùâîôûœÉÈÊÀÇ"),
    ("es", "ñáíóú¿¡Ñ"),
    ("it", "àèìòùÀÈÌÒÙ"),
    ("pt", "ãõçâêôÃÕ"),
];

/// Bytes of markup to read for the guess: enough text for a stable count,
/// small enough to keep opening a book fast on the device.
const SAMPLE_BYTES: u64 = 192 * 1024;

/// Markers per thousand letters below which a language's letters are
/// treated as incidental (a name or a quotation).
const MIN_RATE: f64 = 2.0;

/// The language to hyphenate the EPUB at `path` in, when it is ambiguous;
/// `None` when the book declares at most one language, so Plato's own
/// choice stands.
pub fn epub_language(path: &Path) -> Result<Option<String>, Error> {
    let mut zip = ZipArchive::new(File::open(path)?)?;
    let container = read_entry(&mut zip, "META-INF/container.xml", u64::MAX)?;
    let opf_path = attribute(&container, "full-path")
        .ok_or_else(|| format_err!("container.xml names no OPF"))?;
    let opf = read_entry(&mut zip, &opf_path, u64::MAX)?;
    let declared = declared_languages(&opf);
    if declared.len() < 2 {
        return Ok(None);
    }

    let mut sample = String::new();
    for i in 0..zip.len() {
        if sample.len() as u64 >= SAMPLE_BYTES {
            break;
        }
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_ascii_lowercase();
        if name.ends_with(".xhtml") || name.ends_with(".html") || name.ends_with(".htm") {
            let mut buf = String::new();
            (&mut entry)
                .take(SAMPLE_BYTES - sample.len() as u64)
                .read_to_string(&mut buf)
                .ok();
            sample.push_str(&buf);
        }
    }
    Ok(Some(guess(&strip_tags(&sample), &declared)))
}

fn read_entry(zip: &mut ZipArchive<File>, name: &str, limit: u64) -> Result<String, Error> {
    let mut text = String::new();
    zip.by_name(name)?.take(limit).read_to_string(&mut text)?;
    Ok(text)
}

/// The value of the first `name="..."` attribute in `xml`.
fn attribute(xml: &str, name: &str) -> Option<String> {
    let start = xml.find(&format!("{name}=\""))? + name.len() + 2;
    let len = xml[start..].find('"')?;
    Some(xml[start..start + len].to_string())
}

/// Every `<dc:language>` value in an OPF document, in order.
pub fn declared_languages(opf: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = opf;
    while let Some(i) = rest.find("<dc:language") {
        rest = &rest[i..];
        let Some(open_end) = rest.find('>') else {
            break;
        };
        let Some(close) = rest.find("</dc:language>") else {
            break;
        };
        if open_end < close {
            let value = rest[open_end + 1..close].trim();
            if !value.is_empty() {
                out.push(value.to_string());
            }
        }
        rest = &rest[close..];
        rest = &rest[1..];
    }
    out
}

/// Drop markup, keep text. Crude, but only letter frequencies matter.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn primary(tag: &str) -> String {
    tag.split(['-', '_']).next().unwrap_or(tag).to_lowercase()
}

/// Pick the candidate whose marker letters are most frequent in `text`. If
/// none is frequent enough, prefer a candidate without markers (such as
/// English), then the first declared one.
pub fn guess(text: &str, candidates: &[String]) -> String {
    let letters = text.chars().filter(|c| c.is_alphabetic()).count().max(1) as f64;
    let rate = |markers: &str| {
        text.chars().filter(|c| markers.contains(*c)).count() as f64 * 1000.0 / letters
    };
    let mut best: Option<(&String, f64)> = None;
    for tag in candidates {
        let p = primary(tag);
        if let Some((_, markers)) = MARKERS.iter().find(|(l, _)| *l == p) {
            let r = rate(markers);
            if r >= MIN_RATE && best.is_none_or(|(_, b)| r > b) {
                best = Some((tag, r));
            }
        }
    }
    if let Some((tag, _)) = best {
        return tag.clone();
    }
    candidates
        .iter()
        .find(|t| !MARKERS.iter().any(|(l, _)| *l == primary(t)))
        .unwrap_or(&candidates[0])
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_all_declared_languages() {
        let opf = r#"<metadata><dc:language>de-DE</dc:language>
            <dc:language xsi:type="dcterms:RFC3066">en-US</dc:language>
            <dc:language>pl-PL</dc:language></metadata>"#;
        assert_eq!(declared_languages(opf), ["de-DE", "en-US", "pl-PL"]);
    }

    #[test]
    fn polish_text_wins_over_a_german_first_declaration() {
        let langs = ["de-DE".to_string(), "en-US".into(), "pl-PL".into()];
        let text = "W początkach roku 1878, kiedy świat polityczny zajmował się \
                    pokojem san-stefańskim i wyborem nowego papieża.";
        assert_eq!(guess(text, &langs), "pl-PL");
    }

    #[test]
    fn plain_latin_text_falls_back_to_english() {
        let langs = ["pl".to_string(), "en".into()];
        assert_eq!(
            guess("The quick brown fox jumps over the lazy dog.", &langs),
            "en"
        );
    }
}
