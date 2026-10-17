// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cheap guesses at the dictionary form of an inflected word.
//!
//! The Wiktionary StarDict exports already list most inflected forms as
//! synonyms (1.4 million for Polish, 1.8 million for Ancient Greek), so a
//! fold-index hit is the rule. These suffix rules are the fallback for words
//! the tables miss: rare forms, the small Greek-Polish dictionary, which has
//! almost no forms, and Latin. They work on folded words (see
//! [`super::fold::fold`]: lower case, no accents, final sigma as σ), and
//! every guess is checked against the dictionary, so a wrong guess costs a
//! lookup, not a wrong answer; hits found this way are shown as
//! approximate.

/// (ending, replacements) pairs; longer endings first.
type Rules = &'static [(&'static str, &'static [&'static str])];

/// Polish declension and conjugation endings, folded (no diacritics).
const POLISH: Rules = &[
    ("osciami", &["osc"]),
    ("osciach", &["osc"]),
    ("owania", &["owac", "owanie"]),
    ("owaniu", &["owac", "owanie"]),
    ("owalem", &["owac"]),
    ("owalam", &["owac"]),
    ("owali", &["owac"]),
    ("owaly", &["owac"]),
    ("owal", &["owac"]),
    ("ujemy", &["owac"]),
    ("ujecie", &["owac"]),
    ("uja", &["owac"]),
    ("uje", &["owac"]),
    ("ujesz", &["owac"]),
    ("osci", &["osc"]),
    ("oscia", &["osc"]),
    ("iego", &["i"]),
    ("iemu", &["i"]),
    ("ymi", &["y", ""]),
    ("imi", &["i", ""]),
    ("ych", &["y", ""]),
    ("ich", &["i", ""]),
    ("ego", &["y", "i", ""]),
    ("emu", &["y", "i", ""]),
    ("ami", &["a", "", "o"]),
    ("ach", &["a", "", "o"]),
    ("owi", &[""]),
    ("owie", &[""]),
    ("ow", &["", "a"]),
    ("om", &["", "a", "o"]),
    ("em", &["", "o"]),
    ("ej", &["y", "a", "i"]),
    ("ym", &["y", ""]),
    ("im", &["i", ""]),
    ("lismy", &["c"]),
    ("lysmy", &["c"]),
    ("liscie", &["c"]),
    ("lem", &["c"]),
    ("lam", &["c"]),
    ("les", &["c"]),
    ("las", &["c"]),
    ("li", &["c"]),
    ("ly", &["c"]),
    ("la", &["c"]),
    ("lo", &["c"]),
    ("l", &["c"]),
    ("emy", &["ac", "ec", "c"]),
    ("ecie", &["ac", "ec", "c"]),
    ("esz", &["ac", "ec", "c"]),
    ("isz", &["ic", "iec"]),
    ("ysz", &["yc", "iec"]),
    ("imy", &["ic"]),
    ("ymy", &["yc"]),
    ("ie", &["", "a", "o"]),
    ("a", &["", "o", "y"]),
    ("e", &["", "a", "o", "y"]),
    ("i", &["", "a", "o", "y"]),
    ("u", &["", "o"]),
    ("y", &["", "a"]),
    ("o", &["", "a"]),
];

/// Ancient Greek, folded: nominative singular for nouns and adjectives,
/// first person present (or the middle -ομαι) for verbs.
const GREEK: Rules = &[
    ("ουσιν", &["ω"]),
    ("ουσι", &["ω"]),
    ("ομεν", &["ω"]),
    ("ετε", &["ω"]),
    ("εσθαι", &["ομαι"]),
    ("ονται", &["ομαι"]),
    ("εται", &["ομαι"]),
    ("ειν", &["ω"]),
    ("εισ", &["ω"]),
    ("ουσα", &["ων"]),
    ("οντοσ", &["ων"]),
    ("ει", &["ω"]),
    ("οισ", &["οσ", "ον"]),
    ("ουσ", &["οσ"]),
    ("ου", &["οσ", "ον"]),
    ("ον", &["οσ"]),
    ("οι", &["οσ"]),
    ("ων", &["οσ", "η", "α", "ον"]),
    ("ω", &["οσ", "ον"]),
    ("α", &["ον", "η"]),
    ("αισ", &["η", "α"]),
    ("ασ", &["α", "η"]),
    ("αι", &["η", "α"]),
    ("ην", &["η"]),
    ("ησ", &["η"]),
    ("αν", &["α"]),
    ("εωσ", &["ισ"]),
    ("ιν", &["ισ"]),
    ("ι", &["ισ"]),
];

/// Latin: nominative singular, infinitive for verbs.
const LATIN: Rules = &[
    ("arum", &["a"]),
    ("orum", &["us", "um"]),
    ("ibus", &["is", "", "us"]),
    ("abat", &["are", "o"]),
    ("ebat", &["ere"]),
    ("avit", &["are", "o"]),
    ("amus", &["are", "o"]),
    ("atis", &["are", "o"]),
    ("ant", &["are", "o"]),
    ("ent", &["ere"]),
    ("unt", &["ere", "ire"]),
    // Third declension: virtutem, virtutis → virtus; rationem → ratio.
    ("tem", &["s"]),
    ("tis", &["s"]),
    ("ti", &["s"]),
    ("te", &["s"]),
    ("nem", &["o"]),
    ("nis", &["o"]),
    ("ni", &["o"]),
    ("ne", &["o"]),
    ("rem", &["r"]),
    ("ris", &["r"]),
    ("ae", &["a"]),
    ("am", &["a"]),
    ("as", &["a", "are"]),
    ("at", &["are", "o"]),
    ("et", &["ere"]),
    ("it", &["ere", "ire"]),
    ("is", &["us", "a", "is", "um"]),
    ("os", &["us"]),
    ("um", &["us"]),
    ("em", &["", "is"]),
    ("es", &["is", ""]),
    ("i", &["us", "um", "is"]),
    ("o", &["us", "um", "are"]),
    ("e", &["is", "us"]),
];

fn rules(lang: &str) -> Rules {
    match lang {
        "pl" => POLISH,
        "grc" | "el" => GREEK,
        "la" => LATIN,
        _ => &[],
    }
}

/// Candidate folded base forms of the folded `word`, best first, without
/// the word itself. Stems keep at least two letters.
pub fn candidates(word: &str, lang: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (ending, repl) in rules(lang) {
        let Some(stem) = word.strip_suffix(ending) else {
            continue;
        };
        if stem.chars().count() < 2 {
            continue;
        }
        for r in *repl {
            let c = format!("{stem}{r}");
            if c != word && !out.contains(&c) {
                out.push(c);
            }
        }
    }
    // English: plurals and verb forms.
    if lang == "en" {
        for (ending, repl) in [
            ("ies", "y"),
            ("es", ""),
            ("s", ""),
            ("ed", ""),
            ("ed", "e"),
            ("ing", ""),
            ("ing", "e"),
        ] {
            if let Some(stem) = word.strip_suffix(ending) {
                let c = format!("{stem}{repl}");
                if stem.len() >= 2 && !out.contains(&c) {
                    out.push(c);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::fold::fold;

    #[test]
    fn greek_nouns_and_verbs() {
        let c = candidates(&fold("λόγου"), "grc");
        assert_eq!(c[0], "λογοσ");
        assert!(candidates(&fold("ψυχῆς"), "grc").contains(&"ψυχη".to_string()));
        assert!(candidates(&fold("λέγουσιν"), "grc").contains(&"λεγω".to_string()));
        assert!(candidates(&fold("φιλοσοφίαν"), "grc").contains(&"φιλοσοφια".to_string()));
    }

    #[test]
    fn polish_and_latin() {
        assert!(candidates(&fold("rzeczywistości"), "pl").contains(&"rzeczywistosc".to_string()));
        assert!(candidates(&fold("filozofami"), "pl").contains(&"filozof".to_string()));
        assert!(candidates(&fold("animarum"), "la").contains(&"anima".to_string()));
        assert!(candidates(&fold("amabat"), "la").contains(&"amare".to_string()));
        assert!(candidates("dog", "xx").is_empty());
    }
}
