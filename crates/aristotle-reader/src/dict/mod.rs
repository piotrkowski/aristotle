//! Dictionary lookup: StarDict dictionaries, chosen by the language of the
//! word and the book.
//!
//! - [`stardict`] reads the files, [`fold`] makes lookups accent- and
//!   case-insensitive through an index built once per dictionary, and
//!   [`lemma`] guesses base forms when a form is not listed.
//! - [`Dictionaries`] finds the dictionaries on the device, orders them for
//!   a word (Greek script goes to Greek dictionaries, Greek-Polish first;
//!   other words to the book's language, then the other Latin-script
//!   dictionaries), remembers the reader's last choice per language, and
//!   builds missing indexes in the background.
//!
//! Dictionaries are found as `*.ifo` files (one folder level deep) under
//! the search folders, by default `/mnt/us/dictionaries` and KOReader's
//! `/mnt/us/koreader/data/dict`, so dictionaries installed for KOReader
//! work as they are. The language pair comes from the file name: `grc-pl.ifo` is
//! Ancient Greek to Polish.

pub mod fold;
pub mod lemma;
pub mod service;
pub mod stardict;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use aristotle_render::richtext::{self, Block, RichText, Run};

use self::fold::{FoldIndex, SYN};
use self::stardict::{Data, IdxEntry, Index, Info, Paths};

/// Default dictionary folders on the device.
pub const SEARCH_DIRS: [&str; 2] = ["/mnt/us/dictionaries", "/mnt/us/koreader/data/dict"];

/// At most this many entries in a popup: a word rarely has more meanings
/// worth a page, and every entry costs a decompression.
const MAX_ENTRIES: usize = 6;

/// One article of a lookup result.
#[derive(Clone, Debug)]
pub struct Entry {
    pub headword: String,
    /// The form that matched, when it is not the headword (an inflected
    /// form listed as a synonym, or a folded spelling).
    pub form: Option<String>,
    pub definition: RichText,
}

#[derive(Clone, Debug)]
pub struct Found {
    pub entries: Vec<Entry>,
    /// Found through a guessed base form ([`lemma`]), not the word itself.
    pub approximate: bool,
}

/// A dictionary on disk.
pub struct Dictionary {
    /// File stem, e.g. "grc-pl".
    pub id: String,
    /// The `bookname` of the `.ifo`.
    pub name: String,
    /// Source and target language from the file name, if it has the
    /// `xx-yy` shape.
    pub source: Option<String>,
    pub target: Option<String>,
    paths: Paths,
    info: Info,
    index: Index,
    data: Data,
    fold: Option<FoldIndex>,
    fold_path: PathBuf,
}

/// Language pair from a name like "grc-pl" or "x-grc-pl".
fn langs_of(stem: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = stem.split(['-', '_']).collect();
    let is_lang = |s: &str| (2..=3).contains(&s.len()) && s.chars().all(|c| c.is_ascii_lowercase());
    match parts.as_slice() {
        [.., a, b] if is_lang(a) && is_lang(b) => (Some(a.to_string()), Some(b.to_string())),
        _ => (None, None),
    }
}

/// Where the fold index of a dictionary lives: keyed by name and file
/// sizes, so a replaced dictionary gets a new index, and a copy of the
/// same files (from the host) finds the existing one.
pub fn fold_path(cache: &Path, paths: &Paths) -> PathBuf {
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let syn = paths.syn.as_deref().map(size).unwrap_or(0);
    cache.join(format!(
        "{}-{}-{}.fold",
        paths.stem(),
        size(&paths.idx),
        syn
    ))
}

impl Dictionary {
    pub fn open(ifo: &Path, cache: &Path) -> io::Result<Dictionary> {
        let paths = Paths::from_ifo(ifo)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "incomplete dictionary"))?;
        let info = Info::parse(&std::fs::read_to_string(ifo)?);
        let index = Index::open(&paths, &info)?;
        let data = Data::open(&paths.dict)?;
        let id = paths.stem();
        let (source, target) = langs_of(&id);
        let fold_path = fold_path(cache, &paths);
        let fold = FoldIndex::open(&fold_path).ok();
        Ok(Dictionary {
            name: if info.bookname.is_empty() {
                id.clone()
            } else {
                info.bookname.clone()
            },
            id,
            source,
            target,
            paths,
            info,
            index,
            data,
            fold,
            fold_path,
        })
    }

    /// Label for the switcher: "grc–pl" from the file name, or the
    /// `.ifo`'s `bookname` (the popup shortens a long one to fit, see
    /// `overlay::fit_label`).
    pub fn label(&self) -> String {
        match (&self.source, &self.target) {
            (Some(s), Some(t)) => format!("{s}–{t}"),
            _ => self.name.clone(),
        }
    }

    /// Whether the fold index is there (load it if it appeared).
    pub fn ready(&mut self) -> bool {
        if self.fold.is_none() && self.fold_path.is_file() {
            self.fold = FoldIndex::open(&self.fold_path).ok();
        }
        self.fold.is_some()
    }

    pub fn ifo(&self) -> &Path {
        &self.paths.ifo
    }

    pub fn fold_file(&self) -> &Path {
        &self.fold_path
    }

    /// The entry and the matched surface form behind a fold-index ref.
    fn resolve(&self, r: u32) -> io::Result<(IdxEntry, String)> {
        let fold = self.fold.as_ref().expect("ready");
        if r & SYN != 0 {
            let (form, n) = self.index.syn_at((r & !SYN) as u64)?;
            let e = self.index.idx_at(fold.idx_offset(n)?)?;
            Ok((e, form))
        } else {
            let e = self.index.idx_at(r as u64)?;
            let form = e.word.clone();
            Ok((e, form))
        }
    }

    /// Articles whose headword or listed form folds to `key`, best match
    /// for `query` first.
    fn find_key(&mut self, key: &str, query: &str) -> io::Result<Vec<Entry>> {
        if !self.ready() {
            return Ok(Vec::new());
        }
        let refs = self.fold.as_ref().expect("ready").find(key)?;
        let lower = query.to_lowercase();
        let unaccented = grave_to_acute(&lower);
        let mut hits: Vec<(u8, IdxEntry, String)> = Vec::new();
        for r in refs {
            let (e, form) = self.resolve(r)?;
            let fl = form.to_lowercase();
            let rank = if form == query {
                0
            } else if fl == lower || grave_to_acute(&fl) == unaccented {
                1
            } else {
                2
            };
            match hits.iter_mut().find(|h| h.1 == e) {
                Some(h) if rank < h.0 => {
                    h.0 = rank;
                    h.2 = form;
                }
                Some(_) => {}
                None => hits.push((rank, e, form)),
            }
        }
        hits.sort_by_key(|h| h.0);
        // A word spelled as the book spells it beats words that only fold
        // to the same letters (ψυχή the noun, not a form of ψύχω).
        if hits.first().is_some_and(|h| h.0 <= 1) {
            hits.retain(|h| h.0 <= 1);
        }
        hits.truncate(MAX_ENTRIES);
        let mut out = Vec::new();
        for (_, e, _) in hits {
            let definition = self.definition(&e)?;
            // Show what was tapped when it is not the headword itself.
            let form =
                Some(query.to_string()).filter(|f| f.to_lowercase() != e.word.to_lowercase());
            out.push(Entry {
                headword: e.word,
                form,
                definition,
            });
        }
        Ok(out)
    }

    fn definition(&mut self, e: &IdxEntry) -> io::Result<RichText> {
        let raw = self.data.read(e.offset, e.size)?;
        let seq = self.info.sametypesequence.clone();
        let (kind, body): (char, &[u8]) = match seq.chars().next() {
            Some(k) => {
                // With several types per entry, the first field ends at a
                // NUL; a single type takes the whole entry.
                let end = if seq.len() > 1 {
                    raw.iter().position(|&b| b == 0).unwrap_or(raw.len())
                } else {
                    raw.len()
                };
                (k, &raw[..end])
            }
            None => {
                let k = raw.first().map(|&b| b as char).unwrap_or('m');
                let rest = raw.get(1..).unwrap_or(&[]);
                let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
                (k, &rest[..end])
            }
        };
        let text = String::from_utf8_lossy(body);
        Ok(match kind {
            'h' | 'g' | 'x' | 'w' => richtext::from_html(&text),
            _ => RichText::plain(&text),
        })
    }

    /// Look `word` up: its folded form, then (if `lang` has rules) guessed
    /// base forms.
    pub fn lookup(&mut self, word: &str, lang: &str) -> io::Result<Option<Found>> {
        if let Some(f) = self.exact(word)? {
            return Ok(Some(f));
        }
        self.guess(word, lang)
    }

    /// The word itself (folded), or a form the dictionary lists.
    pub fn exact(&mut self, word: &str) -> io::Result<Option<Found>> {
        let key = fold::fold(word);
        if key.is_empty() {
            return Ok(None);
        }
        let entries = self.find_key(&key, word)?;
        Ok((!entries.is_empty()).then_some(Found {
            entries,
            approximate: false,
        }))
    }

    /// Base forms guessed by [`lemma`] rules.
    pub fn guess(&mut self, word: &str, lang: &str) -> io::Result<Option<Found>> {
        let key = fold::fold(word);
        let lang = self.source.clone().unwrap_or_else(|| lang.to_string());
        for cand in lemma::candidates(&key, &lang) {
            let entries = self.find_key(&cand, word)?;
            if !entries.is_empty() {
                return Ok(Some(Found {
                    entries,
                    approximate: true,
                }));
            }
        }
        Ok(None)
    }

    /// Headwords near `word` in folded order, the ones sharing the longest
    /// beginning first.
    pub fn suggestions(&mut self, word: &str, n: usize) -> Vec<String> {
        if !self.ready() {
            return Vec::new();
        }
        let key = fold::fold(word);
        let Ok(near) = self.fold.as_ref().expect("ready").around(&key, 24) else {
            return Vec::new();
        };
        let common = |a: &str| {
            a.chars()
                .zip(key.chars())
                .take_while(|(x, y)| x == y)
                .count()
        };
        let mut scored: Vec<(usize, String)> = Vec::new();
        for (k, r) in near {
            let Ok((e, _)) = self.resolve(r) else {
                continue;
            };
            if scored.iter().any(|(_, w)| *w == e.word) {
                continue;
            }
            scored.push((common(&k), e.word));
        }
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        scored
            .into_iter()
            .filter(|(c, _)| *c >= 2)
            .take(n)
            .map(|(_, w)| w)
            .collect()
    }
}

/// Greek oxytone words take a grave accent inside a sentence; dictionaries
/// list the acute.
///
/// Also drops vowel length marks (macron, breve): the Greek-English
/// dictionary writes them in headwords (ψῡχή), books do not. So this key
/// tells ψυχή from ψῡ́χη (a form of ψύχω) where the fold key cannot.
fn grave_to_acute(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfd()
        .filter(|&c| c != '\u{0304}' && c != '\u{0306}')
        .map(|c| if c == '\u{0300}' { '\u{0301}' } else { c })
        .nfc()
        .collect()
}

/// The language a word's script implies: Greek letters mean Ancient Greek
/// (modern Greek is not handled yet), Cyrillic Russian.
pub fn script_lang(word: &str) -> Option<&'static str> {
    for c in word.chars() {
        match c as u32 {
            0x0370..=0x03FF | 0x1F00..=0x1FFF => return Some("grc"),
            0x0400..=0x04FF => return Some("ru"),
            _ => {}
        }
    }
    None
}

/// "pl-PL", "PL", "pol" to "pl"; "grc", "el" stay.
pub fn base_lang(tag: &str) -> String {
    let t = tag
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match t.as_str() {
        "pol" => "pl".into(),
        "eng" => "en".into(),
        "lat" => "la".into(),
        "ger" | "deu" => "de".into(),
        _ => t,
    }
}

/// A word as tapped, ready to look up: surrounding punctuation, soft
/// hyphens and quotes removed.
pub fn clean_word(raw: &str) -> String {
    let w: String = raw.chars().filter(|&c| c != '\u{00AD}').collect();
    w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’')
        .trim_matches(|c: char| c == '\'' || c == '’')
        .to_string()
}

/// The result of looking a word up across dictionaries.
#[derive(Clone, Debug)]
pub struct Lookup {
    pub word: String,
    pub lang: String,
    /// Dictionaries offered in the switcher, best first (indexes into
    /// [`Dictionaries::dicts`]).
    pub order: Vec<usize>,
    /// The dictionary shown.
    pub shown: Option<usize>,
    pub found: Option<Found>,
    pub suggestions: Vec<String>,
    /// Some dictionary in the order still has its index being built.
    pub indexing: bool,
    pub millis: f64,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Remembered {
    /// Language to the dictionary id last chosen for it.
    #[serde(default)]
    last: BTreeMap<String, String>,
}

/// The dictionaries on the device.
pub struct Dictionaries {
    pub dicts: Vec<Dictionary>,
    remembered: Remembered,
    state_file: Option<PathBuf>,
}

impl Dictionaries {
    /// Find dictionaries under `dirs` (and one folder level below), with
    /// fold indexes in `cache`; remember choices in `state_file`.
    pub fn discover(dirs: &[PathBuf], cache: &Path, state_file: Option<PathBuf>) -> Dictionaries {
        let mut ifos = Vec::new();
        for d in dirs {
            collect_ifos(d, 2, &mut ifos);
        }
        ifos.sort();
        let mut dicts: Vec<Dictionary> = Vec::new();
        for ifo in ifos {
            match Dictionary::open(&ifo, cache) {
                Ok(d) if !dicts.iter().any(|x| x.id == d.id) => dicts.push(d),
                Ok(_) => {}
                Err(e) => eprintln!("aristotle: dictionary {}: {e}", ifo.display()),
            }
        }
        let remembered = state_file
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default();
        Dictionaries {
            dicts,
            remembered,
            state_file,
        }
    }

    /// Dictionaries without a fold index yet: (`.ifo`, index file).
    pub fn missing_indexes(&mut self) -> Vec<(PathBuf, PathBuf)> {
        self.dicts
            .iter_mut()
            .filter_map(|d| {
                (!d.ready()).then(|| (d.ifo().to_path_buf(), d.fold_file().to_path_buf()))
            })
            .collect()
    }

    /// The dictionaries to try for a word in `lang`, best first.
    pub fn order(&self, lang: &str) -> Vec<usize> {
        let latin = |l: &str| !matches!(l, "grc" | "el" | "ru");
        let pref_target = |d: &Dictionary| match d.target.as_deref() {
            Some("pl") => 0,
            Some(_) => 1,
            None => 2,
        };
        let mut own: Vec<usize> = (0..self.dicts.len())
            .filter(|&i| self.dicts[i].source.as_deref() == Some(lang))
            .collect();
        own.sort_by_key(|&i| (pref_target(&self.dicts[i]), self.dicts[i].id.clone()));
        if let Some(last) = self.remembered.last.get(lang) {
            if let Some(p) = own.iter().position(|&i| &self.dicts[i].id == last) {
                let i = own.remove(p);
                own.insert(0, i);
            }
        }
        if latin(lang) {
            // Latin and English phrases turn up in Polish books and the
            // other way round: the other Latin-script dictionaries follow,
            // Polish, Latin and English first.
            let rank = |s: Option<&str>| match s {
                Some("pl") => 0,
                Some("la") => 1,
                Some("en") => 2,
                Some(_) => 3,
                None => 4,
            };
            let mut rest: Vec<usize> = (0..self.dicts.len())
                .filter(|&i| !own.contains(&i) && self.dicts[i].source.as_deref().is_none_or(latin))
                .collect();
            rest.sort_by_key(|&i| {
                (
                    rank(self.dicts[i].source.as_deref()),
                    pref_target(&self.dicts[i]),
                    self.dicts[i].id.clone(),
                )
            });
            own.extend(rest);
        }
        own
    }

    /// Look `raw` (a word as tapped) up for a book in `book_lang`.
    pub fn lookup(&mut self, raw: &str, book_lang: Option<&str>) -> Lookup {
        let t = Instant::now();
        let word = clean_word(raw);
        let lang = script_lang(&word)
            .map(String::from)
            .or_else(|| book_lang.map(base_lang))
            .unwrap_or_else(|| "pl".into());
        let order = self.order(&lang);
        let mut l = Lookup {
            word,
            lang,
            order,
            shown: None,
            found: None,
            suggestions: Vec::new(),
            indexing: false,
            millis: 0.0,
        };
        let order = l.order.clone();
        let report = |d: &Dictionary, e: io::Error| eprintln!("aristotle: {}: {e}", d.id);
        // 1. The word or a listed form, in order.
        let mut exact: Option<(usize, Found)> = None;
        for (k, &i) in order.iter().enumerate() {
            if !self.dicts[i].ready() {
                l.indexing = true;
                continue;
            }
            match self.dicts[i].exact(&l.word) {
                Ok(Some(f)) => {
                    exact = Some((k, f));
                    break;
                }
                Ok(None) => {}
                Err(e) => report(&self.dicts[i], e),
            }
        }
        // 2. A later dictionary knew the form (the Greek-English one lists
        // 1.8 million forms, the Greek-Polish one almost none): look its
        // headword up in the preferred dictionaries of the same language.
        if let Some((k, f)) = &exact {
            let src = self.dicts[order[*k]].source.clone();
            'transfer: for &j in &order[..*k] {
                if self.dicts[j].source != src || src.is_none() || !self.dicts[j].ready() {
                    continue;
                }
                for e in &f.entries {
                    if let Ok(Some(mut g)) = self.dicts[j].exact(&e.headword) {
                        for x in &mut g.entries {
                            x.form = Some(l.word.clone())
                                .filter(|w| w.to_lowercase() != x.headword.to_lowercase());
                        }
                        exact = Some((order.iter().position(|&x| x == j).unwrap_or(0), g));
                        break 'transfer;
                    }
                }
            }
        }
        if let Some((k, f)) = exact {
            l.shown = Some(order[k]);
            l.found = Some(f);
        } else {
            // 3. Guessed base forms, in order.
            for &i in &order {
                if !self.dicts[i].ready() {
                    continue;
                }
                match self.dicts[i].guess(&l.word, &l.lang) {
                    Ok(Some(f)) => {
                        l.shown = Some(i);
                        l.found = Some(f);
                        break;
                    }
                    Ok(None) => {}
                    Err(e) => report(&self.dicts[i], e),
                }
            }
        }
        if l.found.is_none() {
            if let Some(&first) = l.order.first() {
                l.shown = Some(first);
                l.suggestions = self.dicts[first].suggestions(&l.word, 6);
            }
        }
        l.millis = t.elapsed().as_secs_f64() * 1000.0;
        l
    }

    /// Show `l` from dictionary `i` instead (the switcher), and remember
    /// the choice for the word's language.
    pub fn switch(&mut self, l: &mut Lookup, i: usize) {
        let t = Instant::now();
        l.shown = Some(i);
        l.found = None;
        l.suggestions.clear();
        let d = &mut self.dicts[i];
        if d.ready() {
            match d.lookup(&l.word, &l.lang) {
                Ok(f) => l.found = f,
                Err(e) => eprintln!("aristotle: {}: {e}", d.id),
            }
            if l.found.is_none() {
                l.suggestions = d.suggestions(&l.word, 6);
            }
        }
        l.millis = t.elapsed().as_secs_f64() * 1000.0;
        let id = self.dicts[i].id.clone();
        if self.remembered.last.get(&l.lang) != Some(&id) {
            self.remembered.last.insert(l.lang.clone(), id);
            if let Some(p) = &self.state_file {
                if let Ok(data) = serde_json::to_vec_pretty(&self.remembered) {
                    let _ = aristotle_library::index::write_atomic(p, &data);
                }
            }
        }
    }
}

fn collect_ifos(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth > 1 {
            collect_ifos(&p, depth - 1, out);
        } else if p.extension().is_some_and(|x| x == "ifo") {
            out.push(p);
        }
    }
}

/// Build fold indexes for `missing` (from [`Dictionaries::missing_indexes`])
/// one after another; meant for a background thread at idle priority.
pub fn build_indexes(missing: &[(PathBuf, PathBuf)]) {
    for (ifo, out) in missing {
        let t = Instant::now();
        let built = (|| -> io::Result<fold::BuildStats> {
            let paths = Paths::from_ifo(ifo)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "incomplete"))?;
            let info = Info::parse(&std::fs::read_to_string(ifo)?);
            let index = Index::open(&paths, &info)?;
            fold::build(&index, out)
        })();
        match built {
            Ok(s) => crate::perf!(
                "dict: indexed {} ({} words, {} forms, {} keys) in {:.0} ms",
                ifo.display(),
                s.idx_entries,
                s.syn_entries,
                s.records,
                crate::ms(t.elapsed())
            ),
            Err(e) => eprintln!("aristotle: indexing {}: {e}", ifo.display()),
        }
    }
}

/// Section headings of Wiktionary entries that are not senses: examples,
/// syntax, related words, etymology, translations, inflection, references
/// (Polish and English Wiktionary, lower case, without the colon).
const NOT_SENSES: &[&str] = &[
    // Polish Wiktionary.
    "przykłady",
    "składnia",
    "kolokacje",
    "synonimy",
    "antonimy",
    "hiperonimy",
    "hiponimy",
    "holonimy",
    "meronimy",
    "wyrazy pokrewne",
    "związki frazeologiczne",
    "etymologia",
    "uwagi",
    "tłumaczenia",
    "źródła",
    "odmiana",
    "wymowa",
    "zobacz też",
    "kognaty",
    "przysłowia",
    "pochodne",
    "zapożyczenia",
    "warianty",
    "skrót",
    "temat",
    // English Wiktionary.
    "etymology",
    "pronunciation",
    "declension",
    "inflection",
    "conjugation",
    "derived terms",
    "related terms",
    "descendants",
    "references",
    "further reading",
    "synonyms",
    "antonyms",
    "see also",
    "usage notes",
    "quotations",
    "alternative forms",
    "anagrams",
    "coordinate terms",
    "hypernyms",
    "hyponyms",
    "translations",
];

fn not_senses(heading: &str) -> bool {
    let h = heading.trim().trim_end_matches(':').trim().to_lowercase();
    // "Etymology 2", "Declension 1".
    let h = h.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ');
    NOT_SENSES.contains(&h)
}

/// The senses of a definition: what comes before and between the
/// sections [`NOT_SENSES`] names, without pronunciation lines.
pub fn senses(def: &RichText) -> RichText {
    let mut out = RichText::default();
    let mut keep = true;
    for b in &def.blocks {
        let text = b.text();
        if b.heading {
            keep = !not_senses(&text);
            if !keep {
                continue;
            }
        }
        if keep && !text.trim_start().starts_with("IPA:") {
            out.blocks.push(b.clone());
        }
    }
    out
}

/// A result as rich text for the popup: each article's headword (with the
/// matched form) when there are several, then its senses, or with `full`
/// the whole entry.
pub fn as_rich_text(found: &Found, full: bool) -> RichText {
    let mut out = RichText::default();
    // One article: the popup's title already names it.
    let headings = found.entries.len() > 1;
    for e in &found.entries {
        let defs = if full {
            // Pronunciation lines are IPA the reading fonts lack.
            RichText {
                blocks: e
                    .definition
                    .blocks
                    .iter()
                    .filter(|b| !b.text().trim_start().starts_with("IPA:"))
                    .cloned()
                    .collect(),
            }
        } else {
            senses(&e.definition)
        };
        if headings {
            let mut head = vec![Run {
                text: e.headword.clone(),
                bold: true,
                italic: false,
            }];
            if let Some(f) = &e.form {
                head.push(Run {
                    text: format!("  ({f})"),
                    bold: false,
                    italic: true,
                });
            }
            out.blocks.push(Block {
                runs: head,
                heading: true,
            });
        }
        out.blocks.extend(defs.blocks);
    }
    out
}

/// Whether the full entry says more than its senses ("Więcej").
pub fn has_more(found: &Found) -> bool {
    as_rich_text(found, true).blocks.len() > as_rich_text(found, false).blocks.len()
}

/// The first sense of the first article, short: for a word-by-word gloss.
/// Skips part-of-speech lines (all italic) and sense numbers like "(1.1)".
pub fn gloss(found: &Found) -> String {
    let Some(e) = found.entries.first() else {
        return String::new();
    };
    let line = e
        .definition
        .blocks
        .iter()
        .find(|b| {
            !b.heading
                && !b.runs.iter().all(|r| r.italic)
                && !b.text().trim_start().starts_with("IPA:")
        })
        .map(Block::text)
        .unwrap_or_default();
    let line = line.trim();
    let line = match line.strip_prefix('(') {
        Some(rest)
            if rest.split(')').next().is_some_and(|n| {
                n.chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
            }) =>
        {
            rest.split_once(')').map_or(rest, |(_, r)| r).trim()
        }
        _ => line,
    };
    let mut g: String = line.chars().take(60).collect();
    if line.chars().count() > 60 {
        g.push('…');
    }
    if found.approximate {
        g = format!("≈ {g}");
    }
    format!("{} — {g}", e.headword)
}

#[cfg(test)]
mod tests;
