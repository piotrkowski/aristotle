// SPDX-License-Identifier: AGPL-3.0-or-later
//! Dictionary tests on small StarDict dictionaries generated here, in the
//! shape of the Wiktionary exports (HTML entries, inflected forms as
//! synonyms, dictzip). The real dictionaries are not committed; an ignored
//! test measures lookups on them when they are present.

use std::io::Write;
use std::path::{Path, PathBuf};

use flate2::{Compress, Compression, FlushCompress};

use super::*;

/// Write a StarDict dictionary `stem` into `dir`: entries (headword, HTML)
/// and synonyms (form, headword). Sorted the way the Wiktionary
/// dictionaries are (by lower case), definitions compressed as dictzip
/// with small chunks so lookups cross chunk boundaries.
pub fn write_dict(
    dir: &Path,
    stem: &str,
    entries: &[(&str, &str)],
    syns: &[(&str, &str)],
) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let mut entries: Vec<(&str, &str)> = entries.to_vec();
    entries.sort_by_key(|(w, _)| (w.to_lowercase(), w.to_string()));
    let mut text = Vec::new();
    let mut idx = Vec::new();
    for (w, def) in &entries {
        idx.extend_from_slice(w.as_bytes());
        idx.push(0);
        idx.extend_from_slice(&(text.len() as u32).to_be_bytes());
        idx.extend_from_slice(&(def.len() as u32).to_be_bytes());
        text.extend_from_slice(def.as_bytes());
    }
    let mut syns: Vec<(&str, &str)> = syns.to_vec();
    syns.sort_by_key(|(w, _)| (w.to_lowercase(), w.to_string()));
    let mut syn = Vec::new();
    for (form, head) in &syns {
        let n = entries.iter().position(|(w, _)| w == head).unwrap() as u32;
        syn.extend_from_slice(form.as_bytes());
        syn.push(0);
        syn.extend_from_slice(&n.to_be_bytes());
    }
    // dictzip: 64-byte chunks, each deflated with a full flush.
    let chunk = 64usize;
    let mut packed = Vec::new();
    let mut sizes = Vec::new();
    let mut c = Compress::new(Compression::default(), false);
    for part in text.chunks(chunk) {
        let mut out = Vec::with_capacity(part.len() * 2 + 64);
        let before = c.total_out();
        c.compress_vec(part, &mut out, FlushCompress::Full).unwrap();
        sizes.push((c.total_out() - before) as u16);
        packed.extend_from_slice(&out);
    }
    let mut out = Vec::new();
    c.compress_vec(&[], &mut out, FlushCompress::Finish)
        .unwrap();
    packed.extend_from_slice(&out);
    let mut ra = Vec::new();
    ra.extend_from_slice(&1u16.to_le_bytes());
    ra.extend_from_slice(&(chunk as u16).to_le_bytes());
    ra.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    for s in &sizes {
        ra.extend_from_slice(&s.to_le_bytes());
    }
    let mut extra = b"RA".to_vec();
    extra.extend_from_slice(&(ra.len() as u16).to_le_bytes());
    extra.extend_from_slice(&ra);
    let mut dz = vec![0x1f, 0x8b, 8, 4, 0, 0, 0, 0, 0, 3];
    dz.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    dz.extend_from_slice(&extra);
    dz.extend_from_slice(&packed);
    dz.extend_from_slice(&[0u8; 8]); // CRC and size: not read
    let p = |ext: &str| dir.join(format!("{stem}{ext}"));
    std::fs::write(p(".dict.dz"), dz).unwrap();
    std::fs::write(p(".idx"), &idx).unwrap();
    std::fs::write(p(".syn"), &syn).unwrap();
    let mut ifo = std::fs::File::create(p(".ifo")).unwrap();
    write!(
        ifo,
        "StarDict's dict ifo file\nversion=3.0.0\nbookname=Test {stem}\nwordcount={}\nsynwordcount={}\nidxfilesize={}\nidxoffsetbits=32\nsametypesequence=h\n",
        entries.len(),
        syns.len(),
        idx.len()
    )
    .unwrap();
    p(".ifo")
}

fn fixtures(root: &Path) -> (PathBuf, PathBuf) {
    let dicts = root.join("dict");
    write_dict(
        &dicts.join("grc-pl"),
        "grc-pl",
        &[
            (
                "ψυχή",
                "<p><i>rzeczownik</i></p><dl><dd>(1.1) dusza</dd><dd>(1.2) oddech</dd></dl>",
            ),
            ("λόγος", "<dl><dd>(1.1) słowo</dd><dd>(1.2) rozum</dd></dl>"),
            ("ἀρετή", "<dl><dd>(1.1) cnota, dzielność</dd></dl>"),
            ("Σωκράτης", "<dl><dd>(1.1) Sokrates</dd></dl>"),
        ],
        &[("ψυχή#gr", "ψυχή")],
    );
    write_dict(
        &dicts.join("grc-en"),
        "grc-en",
        &[
            ("ψῡχή", "<p>soul, breath</p>"),
            ("λέγω", "<p>to say, to speak</p>"),
            ("εὐδαιμονῐ́ᾱ", "<p>happiness</p>"),
        ],
        &[
            ("λέγουσι", "λέγω"),
            ("ψυχῆς", "ψῡχή"),
            ("εὐδαιμονία", "εὐδαιμονῐ́ᾱ"),
        ],
    );
    write_dict(
        &dicts.join("pl-pl"),
        "pl-pl",
        &[
            (
                "dusza",
                "<dl><dd>(1.1) niematerialna część człowieka</dd></dl>",
            ),
            (
                "rzeczywistość",
                "<dl><dd>(1.1) to, co istnieje naprawdę</dd></dl>",
            ),
            ("łaska", "<dl><dd>(1.1) życzliwość</dd></dl>"),
            ("laska", "<dl><dd>(1.1) kij do podpierania się</dd></dl>"),
            ("filozof", "<dl><dd>(1.1) myśliciel</dd></dl>"),
        ],
        &[
            ("duszy", "dusza"),
            ("duszą", "dusza"),
            ("dusze", "dusza"),
            ("łasce", "łaska"),
            ("dzień_świąteczny#pl", "dusza"),
        ],
    );
    write_dict(
        &dicts.join("la-pl"),
        "la-pl",
        &[
            ("anima", "<dl><dd>(1.1) dusza</dd></dl>"),
            ("animus", "<dl><dd>(1.1) umysł</dd></dl>"),
        ],
        &[("animae", "anima")],
    );
    (dicts, root.join("cache"))
}

fn open(root: &Path) -> Dictionaries {
    let (dicts, cache) = fixtures(root);
    let mut d = Dictionaries::discover(&[dicts], &cache, Some(root.join("dict.json")));
    for m in d.missing_indexes() {
        build_indexes(&[m]);
    }
    d
}

fn headwords(l: &Lookup) -> Vec<String> {
    l.found
        .as_ref()
        .map(|f| f.entries.iter().map(|e| e.headword.clone()).collect())
        .unwrap_or_default()
}

fn shown<'a>(d: &'a Dictionaries, l: &Lookup) -> &'a str {
    &d.dicts[l.shown.unwrap()].id
}

#[test]
fn senses_only_by_default() {
    use aristotle_render::richtext::from_html;
    // Polish Wiktionary: senses, then labelled sections.
    let pl = from_html(
        "<p><i>rzeczownik</i></p><dl><dd>IPA: [ˈduʃa]</dd></dl><dl><dd>(1.1) dusza</dd><dd>(1.2) duch</dd></dl>\
         <dl><dt>przykłady<span>:</span></dt><dd>(1.1) Przykład.</dd></dl>\
         <dl><dt>etymologia:</dt><dd>prasł.</dd></dl>",
    );
    assert_eq!(senses(&pl).text(), "rzeczownik\n(1.1) dusza\n(1.2) duch");
    // English Wiktionary: part of speech headings stay, sections go.
    let en = from_html(
        "<h4>Noun</h4><ol><li>word</li><li>reason</li></ol><section><h4>Etymology 1</h4><p>From PIE.</p></section>\
         <h4>Verb</h4><ol><li>to say</li></ol><h4>Declension</h4><p>table</p>",
    );
    assert_eq!(senses(&en).text(), "Noun\nword\nreason\nVerb\nto say");
    let found = Found {
        entries: vec![Entry {
            headword: "dusza".into(),
            form: None,
            definition: pl,
        }],
        approximate: false,
    };
    assert!(has_more(&found));
    assert!(as_rich_text(&found, true).text().contains("Przykład."));
}

#[test]
fn languages_from_names() {
    assert_eq!(langs_of("grc-pl"), (Some("grc".into()), Some("pl".into())));
    assert_eq!(langs_of("x-la-pl"), (Some("la".into()), Some("pl".into())));
    assert_eq!(
        langs_of("filozofia-pojecia-i-postacie-wikipedia-pl"),
        (None, None)
    );
    assert_eq!(base_lang("pl-PL"), "pl");
    assert_eq!(script_lang("ψυχή"), Some("grc"));
    assert_eq!(script_lang("dusza"), None);
    assert_eq!(clean_word("„duszy,”"), "duszy");
    assert_eq!(clean_word("rzeczy\u{AD}wistość"), "rzeczywistość");
}

#[test]
fn dictzip_reads_across_chunks() {
    let root = crate::testdir("dict-dz");
    let (dicts, _) = fixtures(&root);
    let paths = Paths::from_ifo(&dicts.join("pl-pl/pl-pl.ifo")).unwrap();
    let info = Info::parse(&std::fs::read_to_string(&paths.ifo).unwrap());
    let index = Index::open(&paths, &info).unwrap();
    let mut data = Data::open(&paths.dict).unwrap();
    let mut seen = Vec::new();
    index
        .walk_idx(|pos, w| seen.push((pos, w.to_string())))
        .unwrap();
    assert_eq!(seen.len(), 5);
    for (pos, w) in seen {
        let e = index.idx_at(pos).unwrap();
        assert_eq!(e.word, w);
        let def = String::from_utf8(data.read(e.offset, e.size).unwrap()).unwrap();
        assert!(
            def.starts_with("<dl><dd>(1.1)") && def.ends_with("</dd></dl>"),
            "{def}"
        );
    }
}

#[test]
fn greek_goes_to_greek_polish_first_accent_insensitive() {
    aristotle_i18n::with_language("pl", greek_in_a_polish_ui);
}

fn greek_in_a_polish_ui() {
    let root = crate::testdir("dict-grc");
    let mut d = open(&root);
    // In a Polish book, a Greek word goes to Greek dictionaries only.
    for w in ["ψυχή", "ψυχὴ", "ΨΥΧΗ", "ψυχη", "ψυχή,"] {
        let l = d.lookup(w, Some("pl"));
        assert_eq!(shown(&d, &l), "grc-pl", "{w}");
        assert_eq!(headwords(&l), ["ψυχή"], "{w}");
        assert!(!l.found.as_ref().unwrap().approximate);
        assert_eq!(l.order.len(), 2, "only Greek dictionaries");
    }
    // Breathing and grave accent: ἀρετὴ.
    assert_eq!(headwords(&d.lookup("ἀρετὴ", Some("pl"))), ["ἀρετή"]);
    // Not in grc-pl: falls through to grc-en, where the headword carries
    // length marks and the form is a listed synonym.
    let l = d.lookup("εὐδαιμονία", Some("pl"));
    assert_eq!(shown(&d, &l), "grc-en");
    assert_eq!(headwords(&l), ["εὐδαιμονῐ́ᾱ"]);
    // An inflected form grc-pl lacks: the genitive is found by a guessed
    // nominative there, marked approximate, before grc-en's exact form.
    let l = d.lookup("λόγου", Some("pl"));
    assert_eq!(shown(&d, &l), "grc-pl");
    assert_eq!(headwords(&l), ["λόγος"]);
    assert!(l.found.as_ref().unwrap().approximate);
    // A form only grc-en lists: its headword is looked up in grc-pl, so
    // the Polish article shows, exact, with the tapped form.
    let l = d.lookup("ψυχῆς", Some("pl"));
    assert_eq!(shown(&d, &l), "grc-pl");
    assert_eq!(headwords(&l), ["ψυχή"]);
    let f = l.found.as_ref().unwrap();
    assert!(!f.approximate);
    assert_eq!(f.entries[0].form.as_deref(), Some("ψυχῆς"));
    // Definition text.
    let rt = as_rich_text(l.found.as_ref().unwrap(), false);
    assert!(
        rt.text().starts_with("rzeczownik\n(1.1) dusza"),
        "{}",
        rt.text()
    );
}

#[test]
fn the_ui_language_picks_the_target_and_a_choice_wins() {
    let root = crate::testdir("dict-ui");
    let mut d = open(&root);
    let id = |d: &Dictionaries, o: Vec<usize>| -> Vec<String> {
        o.iter().map(|&i| d.dicts[i].id.clone()).collect()
    };
    assert_eq!(id(&d, d.order_for("grc", "en")), ["grc-en", "grc-pl"]);
    assert_eq!(id(&d, d.order_for("grc", "pl")), ["grc-pl", "grc-en"]);
    let l = aristotle_i18n::with_language("en", || d.lookup("ψυχή", Some("pl")));
    assert_eq!(shown(&d, &l), "grc-en");
    let l = aristotle_i18n::with_language("pl", || d.lookup("ψυχή", Some("pl")));
    assert_eq!(shown(&d, &l), "grc-pl");
    // A choice made in the switcher wins over the UI language.
    let mut l = aristotle_i18n::with_language("en", || d.lookup("ψυχή", Some("pl")));
    let pl = d.dicts.iter().position(|x| x.id == "grc-pl").unwrap();
    d.switch(&mut l, pl);
    assert_eq!(id(&d, d.order_for("grc", "en")), ["grc-pl", "grc-en"]);
    let l = aristotle_i18n::with_language("en", || d.lookup("ψυχή", None));
    assert_eq!(shown(&d, &l), "grc-pl");
}

#[test]
fn switching_is_remembered_per_language() {
    aristotle_i18n::with_language("pl", switching_in_a_polish_ui);
}

fn switching_in_a_polish_ui() {
    let root = crate::testdir("dict-switch");
    let mut d = open(&root);
    let mut l = d.lookup("ψυχή", Some("pl"));
    assert_eq!(shown(&d, &l), "grc-pl");
    let en = d.dicts.iter().position(|x| x.id == "grc-en").unwrap();
    d.switch(&mut l, en);
    assert_eq!(headwords(&l), ["ψῡχή"]);
    // Next Greek lookup starts with grc-en; reopening keeps the choice.
    let l = d.lookup("ψυχή", Some("pl"));
    assert_eq!(shown(&d, &l), "grc-en");
    let mut again = Dictionaries::discover(
        &[root.join("dict")],
        &root.join("cache"),
        Some(root.join("dict.json")),
    );
    let l = again.lookup("ψυχῆς", None);
    assert_eq!(shown(&again, &l), "grc-en");
    // Polish words are not affected.
    let l = again.lookup("duszy", Some("pl"));
    assert_eq!(shown(&again, &l), "pl-pl");
}

#[test]
fn polish_forms_diacritics_and_fallbacks() {
    let root = crate::testdir("dict-pl");
    let mut d = open(&root);
    let l = d.lookup("Duszą", Some("pl-PL"));
    assert_eq!(headwords(&l), ["dusza"]);
    let f = &l.found.as_ref().unwrap().entries[0];
    assert_eq!(f.form.as_deref(), Some("Duszą"));
    // ł is a letter: łaska and laska stay apart, the exact one first.
    assert_eq!(headwords(&d.lookup("łaska", Some("pl")))[0], "łaska");
    assert_eq!(headwords(&d.lookup("laska", Some("pl")))[0], "laska");
    // A form the dictionary does not list: stripped ending.
    let l = d.lookup("rzeczywistości", Some("pl"));
    assert_eq!(headwords(&l), ["rzeczywistość"]);
    assert!(l.found.as_ref().unwrap().approximate);
    // Missing diacritics.
    assert_eq!(
        headwords(&d.lookup("rzeczywistosc", Some("pl"))),
        ["rzeczywistość"]
    );
    // Latin in a Polish book: pl-pl misses, la-pl answers.
    let l = d.lookup("animae", Some("pl"));
    assert_eq!(shown(&d, &l), "la-pl");
    assert_eq!(headwords(&l), ["anima"]);
    // Link targets from the synonym file are not indexed.
    assert!(d.lookup("dzień_świąteczny", Some("pl")).found.is_none());
}

#[test]
fn closest_headwords_when_nothing_matches() {
    let root = crate::testdir("dict-near");
    let mut d = open(&root);
    let l = d.lookup("filozofka", Some("pl"));
    assert!(l.found.is_none());
    assert_eq!(l.suggestions.first().map(String::as_str), Some("filozof"));
}

#[test]
fn indexes_are_built_on_demand() {
    aristotle_i18n::with_language("pl", indexes_on_demand);
}

fn indexes_on_demand() {
    let root = crate::testdir("dict-pending");
    let (dicts, cache) = fixtures(&root);
    let mut d = Dictionaries::discover(&[dicts], &cache, None);
    let l = d.lookup("ψυχή", Some("pl"));
    assert!(l.indexing && l.found.is_none());
    let missing = d.missing_indexes();
    assert_eq!(missing.len(), 4);
    build_indexes(&missing);
    let l = d.lookup("ψυχή", Some("pl"));
    assert!(!l.indexing);
    assert_eq!(headwords(&l), ["ψυχή"]);
}

/// Lookups on real dictionaries, with timings:
/// `ARISTOTLE_DICTS=path/to/stardict/folders cargo test ... real_dictionaries -- --ignored --nocapture`
#[test]
#[ignore]
fn real_dictionaries() {
    let Some(dir) = std::env::var_os("ARISTOTLE_DICTS").map(PathBuf::from) else {
        return;
    };
    let cache = std::env::var_os("ARISTOTLE_DICT_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::testdir("dict-real"));
    let mut d = Dictionaries::discover(&[dir], &cache, None);
    for (ifo, out) in d.missing_indexes() {
        let t = Instant::now();
        build_indexes(&[(ifo.clone(), out)]);
        println!(
            "built {} in {:.0} ms",
            ifo.display(),
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
    for (w, lang) in [
        ("ψυχή", "pl"),
        ("ψυχὴ", "pl"),
        ("λόγου", "pl"),
        ("εὐδαιμονίαν", "pl"),
        ("ἀρετῆς", "pl"),
        ("duszy", "pl"),
        ("rzeczywistości", "pl"),
        ("animae", "pl"),
        ("virtutem", "la"),
        ("philosophers", "en"),
    ] {
        let l = d.lookup(w, Some(lang));
        println!(
            "{w:>16} {:>6.2} ms  {:<8} {:?}{}",
            l.millis,
            l.shown.map(|i| d.dicts[i].id.as_str()).unwrap_or("-"),
            headwords(&l),
            if l.found.as_ref().is_some_and(|f| f.approximate) {
                " ≈"
            } else {
                ""
            }
        );
        assert!(l.found.is_some(), "{w}");
    }
}
