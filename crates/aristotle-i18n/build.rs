// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reads `locales/*.json`, checks every locale against `en.json` (the
//! source of truth) and generates `messages.rs`: the `Msg` and `Plural`
//! enums and one string table per locale. A broken locale fails the build.
//!
//! A locale file is a JSON object of messages, flat or nested one level
//! ("library": {"title": "Library"}), plus the locale data under "@locale".
//! A message is a string with `{name}` placeholders, or a plural object
//! (`{"one": "{n} book", "other": "{n} books"}`).
//!
//! Checks, for every locale but English:
//! - no key that en.json lacks, and the same kind (string or plural);
//! - every key of en.json, unless the locale says `"complete": false`
//!   (then missing messages fall back to English, with a build warning);
//! - the same placeholders as English (a plural form may leave out `{n}`);
//! - every plural form its language's rule picks (`src/plural.rs`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use serde_json::{Map, Value};

#[allow(dead_code)]
#[path = "src/plural.rs"]
mod plural;

use plural::{Category, Rule};

const META: &str = "@locale";

#[derive(Clone)]
enum Message {
    Text(String),
    Plural(BTreeMap<String, String>),
}

struct Meta {
    name: String,
    complete: bool,
    decimal: String,
    months: Vec<String>,
    weekdays: Vec<String>,
    quotes: Vec<String>,
    date: String,
    time: String,
}

struct Locale {
    code: String,
    meta: Meta,
    messages: BTreeMap<String, Message>,
}

fn fail(file: &str, msg: impl std::fmt::Display) -> ! {
    panic!("locales/{file}: {msg}");
}

/// The `{name}` placeholders of `s`; braces must pair up around a name of
/// lower-case letters, digits and underscores.
fn placeholders(file: &str, key: &str, s: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = s;
    while let Some(open) = rest.find(['{', '}']) {
        if rest.as_bytes()[open] == b'}' {
            fail(file, format_args!("{key}: unmatched '}}' in {s:?}"));
        }
        let after = &rest[open + 1..];
        let close = after
            .find('}')
            .unwrap_or_else(|| fail(file, format_args!("{key}: unclosed '{{' in {s:?}")));
        let name = &after[..close];
        let ok = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !ok {
            fail(
                file,
                format_args!("{key}: bad placeholder {{{name}}} in {s:?}"),
            );
        }
        out.insert(name.to_string());
        rest = &after[close + 1..];
    }
    out
}

fn string(file: &str, key: &str, v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        _ => fail(file, format_args!("{key}: expected a string")),
    }
}

fn strings(file: &str, key: &str, v: Option<&Value>, n: usize) -> Vec<String> {
    let list = match v {
        Some(Value::Array(a)) => a.iter().map(|x| string(file, key, x)).collect::<Vec<_>>(),
        _ => fail(
            file,
            format_args!("{META}.{key}: expected a list of {n} strings"),
        ),
    };
    if list.len() != n {
        fail(
            file,
            format_args!("{META}.{key}: expected {n} names, found {}", list.len()),
        );
    }
    list
}

fn is_plural(m: &Map<String, Value>) -> bool {
    !m.is_empty() && m.keys().all(|k| Category::ALL.iter().any(|c| c.key() == k))
}

fn message(file: &str, key: &str, v: &Value) -> Message {
    match v {
        Value::String(s) => {
            placeholders(file, key, s);
            Message::Text(s.clone())
        }
        Value::Object(m) if is_plural(m) => {
            let forms = m
                .iter()
                .map(|(k, v)| {
                    let s = string(file, &format!("{key}.{k}"), v);
                    placeholders(file, key, &s);
                    (k.clone(), s)
                })
                .collect();
            Message::Plural(forms)
        }
        _ => fail(
            file,
            format_args!("{key}: expected a string or plural forms"),
        ),
    }
}

fn meta(file: &str, v: Option<&Value>) -> Meta {
    let Some(Value::Object(m)) = v else {
        fail(file, format_args!("no \"{META}\" object"));
    };
    let s = |k: &str| match m.get(k) {
        Some(v) => string(file, &format!("{META}.{k}"), v),
        None => fail(file, format_args!("{META}.{k} is missing")),
    };
    let complete = match m.get("complete") {
        Some(Value::Bool(b)) => *b,
        None => true,
        _ => fail(
            file,
            format_args!("{META}.complete: expected true or false"),
        ),
    };
    for k in m.keys() {
        let known = [
            "name", "complete", "decimal", "months", "weekdays", "quotes", "date", "time",
        ];
        if !known.contains(&k.as_str()) {
            fail(file, format_args!("{META}.{k}: unknown field"));
        }
    }
    let date = s("date");
    let time = s("time");
    let allowed = |key: &str, p: &str, names: &[&str]| {
        for name in placeholders(file, key, p) {
            if !names.contains(&name.as_str()) {
                fail(
                    file,
                    format_args!("{META}.{key}: {{{name}}} is not one of {names:?}"),
                );
            }
        }
    };
    allowed("date", &date, &["weekday", "day", "month", "year"]);
    allowed("time", &time, &["h", "hh", "mm"]);
    let decimal = s("decimal");
    if decimal.is_empty() {
        fail(file, format_args!("{META}.decimal is empty"));
    }
    Meta {
        name: s("name"),
        complete,
        decimal,
        months: strings(file, "months", m.get("months"), 12),
        weekdays: strings(file, "weekdays", m.get("weekdays"), 7),
        quotes: strings(file, "quotes", m.get("quotes"), 2),
        date,
        time,
    }
}

fn load(path: &Path) -> Locale {
    let file = path.file_name().unwrap().to_string_lossy().into_owned();
    let code = path.file_stem().unwrap().to_string_lossy().into_owned();
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| fail(&file, e));
    let root: Value = serde_json::from_str(&text).unwrap_or_else(|e| fail(&file, e));
    let Value::Object(root) = root else {
        fail(&file, "expected a JSON object");
    };
    let mut messages = BTreeMap::new();
    for (k, v) in &root {
        if k == META {
            continue;
        }
        match v {
            Value::Object(m) if !is_plural(m) => {
                for (k2, v2) in m {
                    if matches!(v2, Value::Object(inner) if !is_plural(inner)) {
                        fail(&file, format_args!("{k}.{k2}: only one level of nesting"));
                    }
                    let key = format!("{k}.{k2}");
                    let msg = message(&file, &key, v2);
                    messages.insert(key, msg);
                }
            }
            _ => {
                messages.insert(k.clone(), message(&file, k, v));
            }
        }
    }
    Locale {
        code,
        meta: meta(&file, root.get(META)),
        messages,
    }
}

/// "library.continue_reading" -> "LibraryContinueReading".
fn variant(key: &str) -> String {
    let mut out = String::new();
    for part in key.split(['.', '_', '-']) {
        let mut c = part.chars();
        if let Some(first) = c.next() {
            out.extend(first.to_uppercase());
            out.push_str(c.as_str());
        }
    }
    if !out.starts_with(|c: char| c.is_ascii_alphabetic())
        || !out.chars().all(|c| c.is_ascii_alphanumeric())
    {
        fail("en.json", format_args!("{key}: keys must be ASCII words"));
    }
    out
}

fn check(en: &Locale, l: &Locale) -> Vec<String> {
    let file = format!("{}.json", l.code);
    let rule = Rule::for_language(&l.code);
    let mut missing = Vec::new();
    for key in l.messages.keys() {
        if !en.messages.contains_key(key) {
            fail(&file, format_args!("{key}: not in en.json"));
        }
    }
    for (key, src) in &en.messages {
        let Some(msg) = l.messages.get(key) else {
            missing.push(key.clone());
            continue;
        };
        let without_n = |s: &str| {
            let mut p = placeholders(&file, key, s);
            p.remove("n");
            p
        };
        match (src, msg) {
            (Message::Text(a), Message::Text(b)) => {
                let (pa, pb) = (placeholders("en.json", key, a), placeholders(&file, key, b));
                if pa != pb {
                    fail(
                        &file,
                        format_args!("{key}: placeholders {pb:?}, English has {pa:?}"),
                    );
                }
            }
            (Message::Plural(a), Message::Plural(b)) => {
                let pa = without_n(&a["other"]);
                for (form, s) in b {
                    if without_n(s) != pa {
                        fail(
                            &file,
                            format_args!("{key}.{form}: placeholders differ from English"),
                        );
                    }
                }
                for c in rule.categories() {
                    if !b.contains_key(c.key()) {
                        fail(
                            &file,
                            format_args!("{key}: the \"{}\" form is missing", c.key()),
                        );
                    }
                }
            }
            _ => fail(
                &file,
                format_args!("{key}: a string in one file, plural forms in the other"),
            ),
        }
    }
    if !missing.is_empty() && l.meta.complete {
        fail(
            &file,
            format_args!(
                "missing {} (or mark the locale \"complete\": false): {}",
                missing.len(),
                missing.join(", ")
            ),
        );
    }
    missing
}

fn main() {
    let dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("locales");
    println!("cargo:rerun-if-changed=locales");
    println!("cargo:rerun-if-changed=src/plural.rs");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("locales/")
        .map(|e| e.expect("locales/").path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    for p in &paths {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    let mut locales: Vec<Locale> = paths.iter().map(|p| load(p)).collect();
    let en_at = locales
        .iter()
        .position(|l| l.code == "en")
        .expect("locales/en.json, the source of truth, is missing");
    let en = locales.remove(en_at);
    if !en.meta.complete {
        fail("en.json", "the source locale must be complete");
    }
    for (key, m) in &en.messages {
        if let Message::Plural(forms) = m {
            for c in Rule::for_language("en").categories() {
                if !forms.contains_key(c.key()) {
                    fail(
                        "en.json",
                        format_args!("{key}: the \"{}\" form is missing", c.key()),
                    );
                }
            }
        }
    }
    let mut missing = Vec::new();
    for l in &locales {
        let m = check(&en, l);
        if !m.is_empty() {
            println!(
                "cargo:warning=locales/{}.json is incomplete: {} of {} messages fall back to English",
                l.code,
                m.len(),
                en.messages.len()
            );
        }
        missing.push(m);
    }
    locales.insert(0, en);

    let texts: Vec<&String> = locales[0]
        .messages
        .iter()
        .filter(|(_, m)| matches!(m, Message::Text(_)))
        .map(|(k, _)| k)
        .collect();
    let plurals: Vec<&String> = locales[0]
        .messages
        .iter()
        .filter(|(_, m)| matches!(m, Message::Plural(_)))
        .map(|(k, _)| k)
        .collect();

    let mut out = String::new();
    let w = &mut out;
    writeln!(
        w,
        "// Generated by build.rs from locales/*.json. Do not edit."
    )
    .unwrap();

    // Msg.
    writeln!(
        w,
        "/// A message without plural forms; the doc line is its English text."
    )
    .unwrap();
    writeln!(
        w,
        "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum Msg {{"
    )
    .unwrap();
    for k in &texts {
        let Message::Text(s) = &locales[0].messages[*k] else {
            unreachable!()
        };
        writeln!(w, "    /// {k}: {s:?}\n    {},", variant(k)).unwrap();
    }
    writeln!(w, "}}").unwrap();
    writeln!(w, "const MSG_KEYS: [&str; {}] = {:?};", texts.len(), texts).unwrap();
    writeln!(w, "const MSG_PLACEHOLDERS: [&[&str]; {}] = [", texts.len()).unwrap();
    for k in &texts {
        let Message::Text(s) = &locales[0].messages[*k] else {
            unreachable!()
        };
        let p: Vec<String> = placeholders("en.json", k, s).into_iter().collect();
        writeln!(w, "    &{p:?},").unwrap();
    }
    writeln!(w, "];").unwrap();

    // Plural.
    writeln!(
        w,
        "/// A message with plural forms; the doc line is its English \"other\" form."
    )
    .unwrap();
    writeln!(
        w,
        "#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum Plural {{"
    )
    .unwrap();
    for k in &plurals {
        let Message::Plural(f) = &locales[0].messages[*k] else {
            unreachable!()
        };
        writeln!(w, "    /// {k}: {:?}\n    {},", f["other"], variant(k)).unwrap();
    }
    writeln!(w, "}}").unwrap();
    writeln!(
        w,
        "const PLURAL_KEYS: [&str; {}] = {:?};",
        plurals.len(),
        plurals
    )
    .unwrap();

    // Locales.
    writeln!(w, "static LOCALES: [Locale; {}] = [", locales.len()).unwrap();
    let en = &locales[0];
    for l in &locales {
        let m = &l.meta;
        let rule = Rule::for_language(&l.code);
        writeln!(w, "    Locale {{").unwrap();
        writeln!(w, "        code: {:?},", l.code).unwrap();
        writeln!(w, "        name: {:?},", m.name).unwrap();
        writeln!(w, "        complete: {},", m.complete).unwrap();
        writeln!(w, "        rule: Rule::{rule:?},").unwrap();
        writeln!(w, "        decimal: {:?},", m.decimal).unwrap();
        writeln!(w, "        months: {:?},", m.months).unwrap();
        writeln!(w, "        weekdays: {:?},", m.weekdays).unwrap();
        writeln!(w, "        quotes: {:?},", m.quotes).unwrap();
        writeln!(w, "        date: {:?},", m.date).unwrap();
        writeln!(w, "        time: {:?},", m.time).unwrap();
        writeln!(w, "        messages: &[").unwrap();
        for k in &texts {
            let Message::Text(s) = l.messages.get(*k).unwrap_or(&en.messages[*k]) else {
                unreachable!()
            };
            writeln!(w, "            {s:?},").unwrap();
        }
        writeln!(w, "        ],").unwrap();
        writeln!(w, "        plurals: &[").unwrap();
        for k in &plurals {
            let Message::Plural(f) = l.messages.get(*k).unwrap_or(&en.messages[*k]) else {
                unreachable!()
            };
            let forms: Vec<String> = Category::ALL
                .iter()
                .map(|c| match f.get(c.key()) {
                    Some(s) => format!("Some({s:?})"),
                    None => "None".into(),
                })
                .collect();
            writeln!(w, "            [{}],", forms.join(", ")).unwrap();
        }
        writeln!(w, "        ],").unwrap();
        writeln!(w, "    }},").unwrap();
    }
    writeln!(w, "];").unwrap();

    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("messages.rs");
    std::fs::write(dest, out).expect("write messages.rs");
}
