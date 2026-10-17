//! Aristotle's UI languages.
//!
//! Every user-visible string of the UI lives in `locales/<code>.json`.
//! English (`en.json`) is the source of truth; other languages translate
//! it. The build script checks every locale against English (the same
//! keys, the same `{placeholders}`, the plural forms the language needs)
//! and compiles them into the binary: the device needs no files, and a
//! broken translation fails the build instead of the screen.
//!
//! Code names messages through generated enums, so a misspelt key does not
//! compile:
//!
//! ```
//! use aristotle_i18n::{tf, tr, trn, with_language, Msg, Plural};
//! with_language("en", || {
//!     assert_eq!(tr(Msg::LibraryContinueReading), "Continue reading");
//!     assert_eq!(trn(Plural::LibraryBooks, 3), "3 books");
//!     assert_eq!(tf!(Msg::CommonPageOf, page = 2, count = 5), "Page 2 of 5");
//! });
//! with_language("pl", || assert_eq!(trn(Plural::LibraryBooks, 22), "22 książki"));
//! ```
//!
//! The language is process-wide ([`set_language`]); [`with_language`]
//! overrides it on one thread, for tests and previews. See
//! `docs/ui-framework.md`, "Adding a language".

use std::cell::Cell;
use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};

mod plural;

pub use plural::{Category, Rule};

include!(concat!(env!("OUT_DIR"), "/messages.rs"));

/// One embedded language: its messages and how it writes dates and
/// numbers.
#[derive(Debug)]
pub struct Locale {
    /// The file name: "en", "pl".
    pub code: &'static str,
    /// The language's name in itself: "English", "Polski".
    pub name: &'static str,
    /// False if some messages fall back to English.
    pub complete: bool,
    pub rule: Rule,
    /// "." or ",".
    pub decimal: &'static str,
    /// Month names as a date writes them (genitive in Polish), January
    /// first.
    pub months: [&'static str; 12],
    /// Weekday names, Sunday first.
    pub weekdays: [&'static str; 7],
    /// With `{weekday}`, `{day}`, `{month}`, `{year}`.
    pub date: &'static str,
    /// With `{h}` (hour), `{hh}` (two-digit hour), `{mm}` (minutes).
    pub time: &'static str,
    messages: &'static [&'static str],
    plurals: &'static [[Option<&'static str>; 4]],
}

impl Locale {
    pub fn get(&self, m: Msg) -> &'static str {
        self.messages[m as usize]
    }

    pub fn plural(&self, p: Plural, n: u64) -> &'static str {
        let forms = &self.plurals[p as usize];
        let at = |c: Category| forms[c as usize];
        at(self.rule.select(n))
            .or_else(|| at(Category::Other))
            .or_else(|| forms.iter().flatten().next().copied())
            .unwrap_or("")
    }
}

impl Msg {
    /// The key in the locale files, "library.title".
    pub fn key(self) -> &'static str {
        MSG_KEYS[self as usize]
    }

    /// The names of its `{placeholders}`, sorted.
    pub fn placeholders(self) -> &'static [&'static str] {
        MSG_PLACEHOLDERS[self as usize]
    }
}

impl Plural {
    pub fn key(self) -> &'static str {
        PLURAL_KEYS[self as usize]
    }
}

static CURRENT: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
}

/// The embedded languages, English first.
pub fn locales() -> &'static [Locale] {
    &LOCALES
}

fn index_of(code: &str) -> Option<usize> {
    LOCALES.iter().position(|l| l.code == code)
}

/// The language in use (on this thread, if [`with_language`] set one).
pub fn locale() -> &'static Locale {
    let i = OVERRIDE
        .with(Cell::get)
        .unwrap_or_else(|| CURRENT.load(Ordering::Relaxed));
    &LOCALES[i]
}

/// The code of the language in use.
pub fn language() -> &'static str {
    locale().code
}

/// Switches the whole process to the language `code`. Returns false, and
/// keeps the current language, if no such locale is embedded.
pub fn set_language(code: &str) -> bool {
    match index_of(code) {
        Some(i) => {
            CURRENT.store(i, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// Runs `f` with the language `code` on this thread only (English if no
/// such locale), for tests and previews.
pub fn with_language<R>(code: &str, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<usize>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERRIDE.with(|o| o.set(self.0));
        }
    }
    let _restore = Restore(OVERRIDE.with(|o| o.replace(Some(index_of(code).unwrap_or(0)))));
    f()
}

/// A message in the current language.
pub fn tr(m: Msg) -> &'static str {
    locale().get(m)
}

/// A plural message for `n`, with `{n}` filled in.
pub fn trn(p: Plural, n: usize) -> String {
    fill(locale().plural(p, n as u64), |name| {
        (name == "n").then(|| n.to_string())
    })
}

/// A message with its placeholders filled from `args` (name, value). Use
/// [`tf!`] instead of calling this directly.
pub fn format(m: Msg, args: &[(&str, &dyn Display)]) -> String {
    debug_assert!(
        {
            let mut names: Vec<&str> = args.iter().map(|(n, _)| *n).collect();
            names.sort_unstable();
            names == m.placeholders()
        },
        "{}: arguments must be {:?}",
        m.key(),
        m.placeholders()
    );
    fill(tr(m), |name| {
        args.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.to_string())
    })
}

/// `tf!(Msg::CommonPageOf, page = 2, count = 5)`: a message with its
/// placeholders filled. Debug builds check the names against the message.
#[macro_export]
macro_rules! tf {
    ($msg:expr $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::format($msg, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),*])
    };
}

/// Replaces each `{name}` of `template` with `value(name)`, leaving
/// unknown ones as they are.
fn fill(template: &str, value: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                match value(name) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// "Saturday, 17 October 2026", "sobota, 17 października 2026". `weekday`
/// 0 is Sunday; `month` counts from 1.
pub fn fmt_date(year: i32, month: u32, day: u32, weekday: u32) -> String {
    let l = locale();
    fill(l.date, |name| match name {
        "weekday" => Some(l.weekdays[(weekday % 7) as usize].to_string()),
        "day" => Some(day.to_string()),
        "month" => Some(l.months[(month.clamp(1, 12) - 1) as usize].to_string()),
        "year" => Some(year.to_string()),
        _ => None,
    })
}

/// "21:38".
pub fn fmt_time(hour: u32, minute: u32) -> String {
    fill(locale().time, |name| match name {
        "h" => Some(hour.to_string()),
        "hh" => Some(format!("{hour:02}")),
        "mm" => Some(format!("{minute:02}")),
        _ => None,
    })
}

/// `value` with `places` decimals and the language's separator: "3.98",
/// "3,98".
pub fn fmt_decimal(value: f64, places: usize) -> String {
    let s = format!("{value:.places$}");
    match locale().decimal {
        "." => s,
        sep => s.replacen('.', sep, 1),
    }
}

/// Decimal units as storage is sold: "4.1 GB" from a gigabyte up, else
/// "512 MB".
pub fn fmt_bytes(bytes: u64) -> String {
    const GB: f64 = 1e9;
    const MB: f64 = 1e6;
    let b = bytes as f64;
    if b >= GB {
        tf!(Msg::FormatGigabytes, value = fmt_decimal(b / GB, 1))
    } else {
        tf!(Msg::FormatMegabytes, value = fmt_decimal(b / MB, 0))
    }
}

/// "45 min", "12 h 22 min" ("12 godz. 22 min").
pub fn fmt_duration(minutes: u32) -> String {
    if minutes >= 60 {
        tf!(Msg::FormatHoursMinutes, h = minutes / 60, m = minutes % 60)
    } else {
        tf!(Msg::FormatMinutes, m = minutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NS: [u64; 11] = [0, 1, 2, 5, 12, 14, 21, 22, 25, 112, 122];

    #[test]
    fn english_plural_rule() {
        let got: Vec<Category> = NS.iter().map(|&n| Rule::OneOther.select(n)).collect();
        use Category::*;
        assert_eq!(
            got,
            [Other, One, Other, Other, Other, Other, Other, Other, Other, Other, Other]
        );
    }

    #[test]
    fn polish_plural_rule() {
        let got: Vec<Category> = NS.iter().map(|&n| Rule::Polish.select(n)).collect();
        use Category::*;
        // 0 5 12 14 25 112 książek; 2 22 122 książki; 21 książek.
        assert_eq!(
            got,
            [Many, One, Few, Many, Many, Many, Many, Few, Many, Many, Few]
        );
    }

    #[test]
    fn fallback_rule_for_unknown_languages() {
        assert_eq!(Rule::for_language("xx"), Rule::Fallback);
        assert_eq!(Rule::for_language("pl-PL"), Rule::Polish);
        assert_eq!(Rule::for_language("en_GB"), Rule::OneOther);
        let got: Vec<Category> = NS.iter().map(|&n| Rule::Fallback.select(n)).collect();
        assert_eq!(got[1], Category::One);
        assert!(got
            .iter()
            .enumerate()
            .all(|(i, c)| i == 1 || *c == Category::Other));
    }

    #[test]
    fn plural_messages() {
        with_language("en", || {
            let books: Vec<String> = NS
                .iter()
                .map(|&n| trn(Plural::LibraryBooks, n as usize))
                .collect();
            assert_eq!(books[0], "0 books");
            assert_eq!(books[1], "1 book");
            assert_eq!(books[7], "22 books");
        });
        with_language("pl", || {
            let books: Vec<String> = NS
                .iter()
                .map(|&n| trn(Plural::LibraryBooks, n as usize))
                .collect();
            assert_eq!(
                books,
                [
                    "0 książek",
                    "1 książka",
                    "2 książki",
                    "5 książek",
                    "12 książek",
                    "14 książek",
                    "21 książek",
                    "22 książki",
                    "25 książek",
                    "112 książek",
                    "122 książki",
                ]
            );
            assert_eq!(trn(Plural::LibraryVolumes, 2), "2 tomy");
        });
    }

    #[test]
    fn english_is_the_default_and_unknown_codes_are_refused() {
        assert_eq!(locales()[0].code, "en");
        assert_eq!(locales()[0].name, "English");
        assert!(!set_language("xx"));
        with_language("xx", || assert_eq!(language(), "en"));
    }

    #[test]
    fn every_locale_has_a_name_and_complete_ones_differ_from_english() {
        let codes: Vec<&str> = locales().iter().map(|l| l.code).collect();
        assert!(codes.contains(&"pl"));
        for l in locales() {
            assert!(!l.name.is_empty());
        }
        let pl = &locales()[index_of("pl").unwrap()];
        assert_eq!(pl.name, "Polski");
        assert!(pl.complete);
        assert_eq!(pl.get(Msg::LibraryTitle), "Biblioteka");
    }

    #[test]
    fn placeholders_are_filled() {
        with_language("en", || {
            assert_eq!(tf!(Msg::CommonPageOf, page = 2, count = 5), "Page 2 of 5");
        });
        with_language("pl", || {
            assert_eq!(tf!(Msg::CommonPageOf, count = 5, page = 2), "Strona 2 z 5");
        });
        assert_eq!(
            fill("{a} and {b}", |n| (n == "a").then(|| "x".into())),
            "x and {b}"
        );
        assert_eq!(fill("unclosed {a", |_| None), "unclosed {a");
    }

    #[test]
    #[should_panic(expected = "arguments must be")]
    #[cfg(debug_assertions)]
    fn wrong_arguments_are_caught_in_debug_builds() {
        let _ = tf!(Msg::CommonPageOf, page = 2);
    }

    #[test]
    fn dates_numbers_sizes_and_durations() {
        with_language("en", || {
            assert_eq!(fmt_date(2026, 10, 17, 6), "Saturday, 17 October 2026");
            assert_eq!(fmt_time(9, 5), "09:05");
            assert_eq!(fmt_decimal(3.98, 2), "3.98");
            assert_eq!(fmt_bytes(4_100_000_000), "4.1 GB");
            assert_eq!(fmt_bytes(512_000_000), "512 MB");
            assert_eq!(fmt_duration(742), "12 h 22 min");
            assert_eq!(fmt_duration(45), "45 min");
        });
        with_language("pl", || {
            assert_eq!(fmt_date(2026, 10, 17, 6), "sobota, 17 października 2026");
            assert_eq!(fmt_date(2026, 1, 1, 4), "czwartek, 1 stycznia 2026");
            assert_eq!(fmt_time(21, 38), "21:38");
            assert_eq!(fmt_decimal(3.98, 2), "3,98");
            assert_eq!(fmt_bytes(4_100_000_000), "4,1 GB");
            assert_eq!(fmt_bytes(512_000_000), "512 MB");
            assert_eq!(fmt_duration(742), "12 godz. 22 min");
        });
    }

    #[test]
    fn with_language_is_per_thread_and_restores() {
        with_language("pl", || {
            assert_eq!(language(), "pl");
            with_language("en", || assert_eq!(language(), "en"));
            assert_eq!(language(), "pl");
            std::thread::spawn(|| assert_eq!(language(), "en"))
                .join()
                .unwrap();
        });
    }
}
