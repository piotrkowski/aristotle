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
    // MOBI and PDF have no sort names, and their author fields are often
    // inverted or several names run together; EPUB has file-as instead.
    if matches!(format, Format::Mobi | Format::Azw3 | Format::Pdf) {
        if let Some(fixed) = author_dir.and_then(|d| authors_from_folder(&ex.meta.authors, d)) {
            ex.meta.authors = fixed;
        }
    }
    finish(&mut ex.meta, path, author_dir);
    Ok(ex)
}

/// Common given names (folded), used only to tell which end of a two-part
/// name is the given name. Polish first, then frequent foreign ones in
/// Polish libraries. A name missing here only makes the heuristic abstain.
const GIVEN_NAMES: &[&str] = &[
    "adam",
    "adrianna",
    "agata",
    "agnieszka",
    "aleksander",
    "aleksandra",
    "alicja",
    "andrzej",
    "anna",
    "antoni",
    "artur",
    "barbara",
    "bartosz",
    "beata",
    "boleslaw",
    "bozena",
    "cezary",
    "czeslaw",
    "damian",
    "danuta",
    "dariusz",
    "dawid",
    "dorota",
    "edward",
    "edyta",
    "elzbieta",
    "emilia",
    "ewa",
    "ewelina",
    "filip",
    "gabriela",
    "grazyna",
    "grzegorz",
    "halina",
    "hanna",
    "helena",
    "henryk",
    "ignacy",
    "igor",
    "ilona",
    "irena",
    "iwona",
    "izabela",
    "jacek",
    "jadwiga",
    "jakub",
    "jan",
    "janina",
    "janusz",
    "jaroslaw",
    "jerzy",
    "joanna",
    "jolanta",
    "jozef",
    "julia",
    "juliusz",
    "justyna",
    "kacper",
    "kamil",
    "kamila",
    "karol",
    "karolina",
    "katarzyna",
    "kazimierz",
    "kinga",
    "klaudia",
    "konrad",
    "krystyna",
    "krzysztof",
    "leszek",
    "lucyna",
    "lukasz",
    "maciej",
    "magdalena",
    "maja",
    "malgorzata",
    "marcin",
    "marek",
    "maria",
    "marian",
    "marta",
    "martyna",
    "mateusz",
    "michal",
    "mieczyslaw",
    "miroslaw",
    "monika",
    "natalia",
    "olga",
    "oliwia",
    "patrycja",
    "paulina",
    "pawel",
    "piotr",
    "przemyslaw",
    "rafal",
    "renata",
    "robert",
    "roman",
    "ryszard",
    "sebastian",
    "slawomir",
    "stanislaw",
    "stanislawa",
    "sylwia",
    "szymon",
    "tadeusz",
    "teresa",
    "tomasz",
    "urszula",
    "waldemar",
    "weronika",
    "wieslaw",
    "wiktoria",
    "witold",
    "wladyslaw",
    "wojciech",
    "zbigniew",
    "zofia",
    "zuzanna",
    "zygmunt",
    "albert",
    "arthur",
    "carl",
    "charles",
    "christopher",
    "daniel",
    "david",
    "elizabeth",
    "emily",
    "ernest",
    "franz",
    "friedrich",
    "fyodor",
    "george",
    "haruki",
    "isaac",
    "james",
    "jane",
    "john",
    "joseph",
    "karl",
    "kenneth",
    "klaus",
    "leo",
    "margaret",
    "mark",
    "mary",
    "matthew",
    "michael",
    "milan",
    "niccolo",
    "noah",
    "paul",
    "peter",
    "richard",
    "sarah",
    "simone",
    "stephen",
    "steven",
    "susan",
    "thomas",
    "umberto",
    "ursula",
    "viktor",
    "virginia",
    "william",
];

fn is_given(word: &str) -> bool {
    GIVEN_NAMES.contains(&text::fold(word).as_str())
}

/// Rebuilds the authors from the folder name when that is clearly better
/// than the embedded field, else `None`.
///
/// Device folders such as "Lecki Tomasz, Zolkiewska Jadwiga" are sort
/// names, ASCII-folded, comma-separated, while a MOBI may say "Żółkiewska
/// Jadwiga Łęcki Tomasz" in one author field. The folder is used only if:
///
/// - its words are exactly the embedded words (ignoring case, diacritics and
///   punctuation), so it describes the same people, and
/// - it fixes something: it splits one field into several authors, or it
///   reveals an inverted "Surname Given" field.
///
/// A "Last, First" folder ("Mickiewicz, Adam") names one person with a known
/// surname. Otherwise, which word is the given name is decided by
/// [`GIVEN_NAMES`]. When neither
/// end is a known given name, a comma-separated folder with several authors
/// is taken as surname-first (a sort list); a single ambiguous name is left
/// alone. Spelling with diacritics always comes from the embedded field.
pub fn authors_from_folder(embedded: &[Author], folder: &str) -> Option<Vec<Author>> {
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect()
    };
    // Each embedded word with the index of the author it belongs to.
    let emb: Vec<(String, usize)> = embedded
        .iter()
        .enumerate()
        .flat_map(|(n, a)| words(&a.name).into_iter().map(move |w| (w, n)))
        .collect();
    // "&" and ";" always separate people. A comma is ambiguous: in
    // "Zulawski Jerzy, Konopnicka Maria" it separates two people, in
    // "Mickiewicz, Adam" it separates surname from given name. A side with
    // a single word means the latter; then the surname length is known.
    let mut groups: Vec<(Vec<String>, Option<usize>)> = Vec::new();
    for part in folder.split(['&', ';']) {
        let pieces: Vec<Vec<String>> = part
            .split(',')
            .map(words)
            .filter(|g| !g.is_empty())
            .collect();
        if pieces.len() == 2 && pieces.iter().any(|p| p.len() == 1) {
            groups.push((pieces.concat(), Some(pieces[0].len())));
        } else {
            groups.extend(pieces.into_iter().map(|p| (p, None)));
        }
    }
    if emb.is_empty() || groups.is_empty() {
        return None;
    }
    let mut a: Vec<String> = emb.iter().map(|(w, _)| text::fold(w)).collect();
    let mut b: Vec<String> = groups
        .iter()
        .flat_map(|(g, _)| g)
        .map(|w| text::fold(w))
        .collect();
    a.sort();
    b.sort();
    if a != b {
        return None;
    }
    let several = groups.len() > 1;
    let mut used = vec![false; emb.len()];
    let mut out: Vec<(usize, Author)> = Vec::new();
    for (g, surname_len) in &groups {
        // Restore the embedded spelling (diacritics) word by word.
        let mut orig = Vec::new();
        let mut owners = Vec::new();
        let mut first_at = usize::MAX;
        for w in g {
            let f = text::fold(w);
            let i = (0..emb.len()).find(|&i| !used[i] && text::fold(&emb[i].0) == f)?;
            used[i] = true;
            first_at = first_at.min(i);
            orig.push(emb[i].0.clone());
            owners.push(emb[i].1);
        }
        let author = if let Some(n) = *surname_len {
            // "Last, First" names one person; never merge two embedded authors.
            if owners.iter().any(|&o| o != owners[0]) {
                return None;
            }
            let (surname, given) = (orig[..n].join(" "), orig[n..].join(" "));
            Author::new(
                &format!("{given} {surname}"),
                Some(&format!("{surname}, {given}")),
            )
        } else if orig.len() == 1 {
            Author::new(&orig[0], None)
        } else {
            let (first, last) = (&orig[0], &orig[orig.len() - 1]);
            let surname_first = match (is_given(first), is_given(last)) {
                (false, true) => true,
                (true, false) => false,
                _ if several => true,
                _ => return None,
            };
            if surname_first {
                let given = orig[1..].join(" ");
                Author::new(
                    &format!("{given} {first}"),
                    Some(&format!("{first}, {given}")),
                )
            } else {
                let given = orig[..orig.len() - 1].join(" ");
                Author::new(
                    &format!("{given} {last}"),
                    Some(&format!("{last}, {given}")),
                )
            }
        };
        out.push((first_at, author));
    }
    // Keep the order in which the book names its authors.
    out.sort_by_key(|(i, _)| *i);
    let fixed: Vec<Author> = out.into_iter().map(|(_, a)| a).collect();
    let same =
        fixed.len() == embedded.len() && fixed.iter().zip(embedded).all(|(x, y)| x.name == y.name);
    if same {
        None
    } else {
        Some(fixed)
    }
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

    fn names(v: &[Author]) -> Vec<(&str, &str)> {
        v.iter()
            .map(|a| (a.name.as_str(), a.sort.as_str()))
            .collect()
    }

    #[test]
    fn folder_fixes_run_together_inverted_authors() {
        let emb = [Author::new("Żółkiewska Jadwiga Łęcki Tomasz", None)];
        let fixed = authors_from_folder(&emb, "Lecki Tomasz, Zolkiewska Jadwiga").unwrap();
        assert_eq!(
            names(&fixed),
            [
                ("Jadwiga Żółkiewska", "Żółkiewska, Jadwiga"),
                ("Tomasz Łęcki", "Łęcki, Tomasz")
            ]
        );
    }

    #[test]
    fn folder_fixes_single_inverted_author() {
        let emb = [Author::new("Schopenhauer Arthur", None)];
        let fixed = authors_from_folder(&emb, "Arthur Schopenhauer").unwrap();
        assert_eq!(
            names(&fixed),
            [("Arthur Schopenhauer", "Schopenhauer, Arthur")]
        );
        // Folder in sort order works too.
        assert_eq!(
            names(&authors_from_folder(&emb, "Schopenhauer Arthur").unwrap()),
            names(&fixed)
        );
    }

    #[test]
    fn folder_is_ignored_unless_clearly_better() {
        // Already right.
        assert!(
            authors_from_folder(&[Author::new("Adam Mickiewicz", None)], "Mickiewicz Adam")
                .is_none()
        );
        // Different people.
        assert!(authors_from_folder(&[Author::new("Bolesław Prus", None)], "Różni").is_none());
        // Ambiguous single name: neither word is a known given name.
        assert!(authors_from_folder(&[Author::new("Xyz Qwe", None)], "Qwe Xyz").is_none());
        assert!(authors_from_folder(&[Author::new("Platon", None)], "Platon").is_none());
        // A "Last, First" folder over an already correct name is a no-op
        // (regression: it once became "Adam, Mickiewicz").
        assert!(
            authors_from_folder(&[Author::new("Adam Mickiewicz", None)], "Mickiewicz, Adam")
                .is_none()
        );
        // Two one-word authors are not one "Last, First" person.
        let two = [
            Author::new("Platon", None),
            Author::new("Arystoteles", None),
        ];
        assert!(authors_from_folder(&two, "Platon, Arystoteles").is_none());
    }

    #[test]
    fn last_first_folder_fixes_inverted_and_multiword_surnames() {
        let fixed = authors_from_folder(
            &[Author::new("Conan Doyle Arthur", None)],
            "Conan Doyle, Arthur",
        )
        .unwrap();
        assert_eq!(
            names(&fixed),
            [("Arthur Conan Doyle", "Conan Doyle, Arthur")]
        );
    }

    /// The two device cases end to end, through `extract` and the real
    /// folder paths.
    #[test]
    fn device_author_folders() {
        let dir = crate::testdir("authors");
        let check = |folder: &str, file: &str, embedded: &str| {
            let p = dir.join(folder).join(file);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(
                &p,
                crate::mobi::testutil::build("T", &[(100, embedded.as_bytes())], &[], 6),
            )
            .unwrap();
            extract(&p, Format::Mobi, Some(folder), false)
                .unwrap()
                .meta
                .authors
        };
        let a = check(
            "Mickiewicz, Adam",
            "Pan Tadeusz - Adam Mickiewicz.mobi",
            "Adam Mickiewicz",
        );
        assert_eq!(names(&a), [("Adam Mickiewicz", "Mickiewicz, Adam")]);
        let a = check(
            "Zulawski Jerzy, Konopnicka Maria",
            "Opowiadania.mobi",
            "Konopnicka Maria Żuławski Jerzy",
        );
        assert_eq!(
            names(&a),
            [
                ("Maria Konopnicka", "Konopnicka, Maria"),
                ("Jerzy Żuławski", "Żuławski, Jerzy")
            ]
        );
    }

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
