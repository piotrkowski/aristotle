// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hyphenation hooks.
//!
//! Justified text in a narrow column needs hyphenation or it fills with
//! wide gaps; Polish words are long, which makes this worse than in
//! English. The layout asks a [`Hyphenator`] for the allowed break points
//! of each word and lets the line breaker weigh them with a penalty, so a
//! hyphen is used only when it buys noticeably better spacing.
//!
//! Soft hyphens (U+00AD) in the source text are always honoured, whatever
//! hyphenator is set. Many EPUBs ship pre-hyphenated that way.
//!
//! The bundled [`PolishSyllables`] is a rule-based heuristic, not a
//! dictionary. Polish spelling rules allow a consonant cluster to be split
//! anywhere (sio-stra, sios-tra and siost-ra are all correct), which makes
//! a syllable rule unusually safe for this language. A pattern-based
//! (Liang) hyphenator can implement the same trait later.

/// Supplies hyphenation points for words.
pub trait Hyphenator {
    /// Appends to `out` the byte offsets in `word` where a hyphen may be
    /// inserted (the break falls before the byte at that offset). `word`
    /// may contain punctuation; implementations should only break inside
    /// runs of letters.
    fn hyphenate(&self, word: &str, out: &mut Vec<usize>);
}

/// Hyphenates nothing; only soft hyphens in the text apply.
pub struct NoHyphenation;

impl Hyphenator for NoHyphenation {
    fn hyphenate(&self, _: &str, _: &mut Vec<usize>) {}
}

/// Syllable-based hyphenation for Polish.
///
/// Rules, applied between consecutive vowel nuclei:
///
/// - `i` before a vowel softens the consonant before it and is not a vowel
///   (nie, pia-ni-no); the digraphs ch, cz, dz, dź, dż, rz, sz are one
///   consonant.
/// - Two vowels split (o-ce-an) unless they form a diphthong (au, eu).
/// - A single consonant goes to the next syllable (ko-ło).
/// - Two consonants split between them (tycz-ny, obec-nie), except an
///   onset that reads naturally together: obstruent plus liquid (przy-szło)
///   or s-like plus stop (mia-sto, przyszło-ścią).
/// - Three or more: after a leading sonorant or w (pań-stwo,
///   Kra-kow-skie-go), after an obstruent-liquid pair (jabł-ko), otherwise
///   before the cluster (sio-stra).
///
/// At least `min_left` and `min_right` letters stay on each side, so no
/// single letter is ever left hanging. Prefixes are not recognised
/// (prze-dmie-ście style errors are possible but rare).
pub struct PolishSyllables {
    pub min_left: usize,
    pub min_right: usize,
    /// Shorter words are never hyphenated.
    pub min_word: usize,
}

impl PolishSyllables {
    pub const DEFAULT: PolishSyllables = PolishSyllables {
        min_left: 2,
        min_right: 2,
        min_word: 5,
    };
}

impl Default for PolishSyllables {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The hyphenator for UI text in the language `code` ("pl", "en"): the
/// UI's hyphenation follows the UI language, while a book's follows the
/// book. Only Polish has one; other languages break UI text at spaces.
pub fn ui_hyphenator(code: &str) -> &'static dyn Hyphenator {
    static POLISH: PolishSyllables = PolishSyllables::DEFAULT;
    match code.split(['-', '_']).next() {
        Some("pl") => &POLISH,
        _ => &NoHyphenation,
    }
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'ą' | 'e' | 'ę' | 'i' | 'o' | 'ó' | 'u' | 'y')
}

fn is_sonorant(c: char) -> bool {
    matches!(c, 'l' | 'ł' | 'r' | 'm' | 'n' | 'ń' | 'j')
}

/// "rz" sounds like "ż", an obstruent, although it starts with r.
fn sonorant(u: &Unit) -> bool {
    is_sonorant(u.first) && !u.digraph_rz
}

fn is_liquid(c: char) -> bool {
    matches!(c, 'l' | 'ł' | 'r')
}

fn is_sibilant(u: &Unit) -> bool {
    matches!(u.first, 's' | 'ś' | 'z' | 'ź') && !u.digraph_rz
}

fn is_stop(c: char) -> bool {
    matches!(c, 'p' | 't' | 'k' | 'c' | 'ć' | 'b' | 'd' | 'g')
}

const DIGRAPHS: [&str; 7] = ["ch", "cz", "dz", "dź", "dż", "rz", "sz"];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Vowel,
    Consonant,
}

struct Unit {
    /// Byte offset of the unit in the word.
    start: usize,
    /// Letter index of the unit, for min_left and min_right.
    letter: usize,
    first: char,
    /// Second letter of a digraph, to tell "rz" from "r".
    digraph_rz: bool,
    kind: Kind,
}

impl PolishSyllables {
    fn hyphenate_letters(&self, word: &str, base: usize, out: &mut Vec<usize>) {
        let chars: Vec<(usize, char)> = word
            .char_indices()
            .map(|(i, c)| (i, c.to_lowercase().next().unwrap_or(c)))
            .collect();
        let n = chars.len();
        if n < self.min_word {
            return;
        }
        // Group letters into units.
        let mut units: Vec<Unit> = Vec::new();
        let mut i = 0;
        while i < n {
            let (start, c) = chars[i];
            let next = chars.get(i + 1).map(|&(_, c)| c);
            if let Some(nc) = next {
                let pair: String = [c, nc].iter().collect();
                if DIGRAPHS.contains(&pair.as_str()) {
                    units.push(Unit {
                        start,
                        letter: i,
                        first: c,
                        digraph_rz: c == 'r',
                        kind: Kind::Consonant,
                    });
                    i += 2;
                    continue;
                }
            }
            let prev_consonant = units.last().is_some_and(|u| u.kind == Kind::Consonant);
            if c == 'i' && prev_consonant && next.is_some_and(is_vowel) {
                // Softening i belongs to the consonant before it.
                i += 1;
                continue;
            }
            units.push(Unit {
                start,
                letter: i,
                first: c,
                digraph_rz: false,
                kind: if is_vowel(c) {
                    Kind::Vowel
                } else {
                    Kind::Consonant
                },
            });
            i += 1;
        }
        // Find breaks between consecutive vowel nuclei.
        let vowels: Vec<usize> = (0..units.len())
            .filter(|&k| units[k].kind == Kind::Vowel)
            .collect();
        for w in vowels.windows(2) {
            let (a, b) = (w[0], w[1]);
            let consonants = b - a - 1;
            let (c1, c2) = (&units[a + 1.min(consonants)], units.get(a + 2));
            let at = match consonants {
                0 => {
                    let pair = (units[a].first, units[b].first);
                    if matches!(pair, ('a', 'u') | ('e', 'u')) {
                        continue;
                    }
                    b
                }
                1 => a + 1,
                2 => {
                    let c2 = c2.unwrap();
                    let onset = (!sonorant(c1) && is_liquid(c2.first) && !c2.digraph_rz)
                        || (is_sibilant(c1) && is_stop(c2.first));
                    if onset {
                        a + 1
                    } else {
                        a + 2
                    }
                }
                _ => {
                    let c2 = c2.unwrap();
                    if sonorant(c1) || c1.first == 'w' {
                        a + 2
                    } else if is_liquid(c2.first) && !c2.digraph_rz {
                        a + 3
                    } else {
                        a + 1
                    }
                }
            };
            let letter = units[at].letter;
            if letter >= self.min_left && n - letter >= self.min_right {
                out.push(base + units[at].start);
            }
        }
    }
}

impl Hyphenator for PolishSyllables {
    fn hyphenate(&self, word: &str, out: &mut Vec<usize>) {
        // Hyphenate each run of letters separately, so "(Wokulski," or
        // "san-stefańskim" only break inside their words.
        let mut start = None;
        for (i, c) in word
            .char_indices()
            .chain(std::iter::once((word.len(), ' ')))
        {
            match (c.is_alphabetic(), start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    self.hyphenate_letters(&word[s..i], s, out);
                    start = None;
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hy(word: &str) -> String {
        let mut pts = Vec::new();
        PolishSyllables::default().hyphenate(word, &mut pts);
        let mut s = String::new();
        let mut last = 0;
        for p in pts {
            s.push_str(&word[last..p]);
            s.push('-');
            last = p;
        }
        s.push_str(&word[last..]);
        s
    }

    #[test]
    fn polish_words() {
        assert_eq!(hy("Krakowskiego"), "Kra-kow-skie-go");
        assert_eq!(hy("galanteryjnego"), "ga-lan-te-ryj-ne-go");
        assert_eq!(hy("pianino"), "pia-ni-no");
        assert_eq!(hy("państwo"), "pań-stwo");
        assert_eq!(hy("jabłko"), "jabł-ko");
        assert_eq!(hy("warszawscy"), "war-szaw-scy");
        assert_eq!(hy("przyszłością"), "przy-szło-ścią");
        assert_eq!(hy("Europa"), "Eu-ro-pa");
        assert_eq!(hy("siostra"), "sio-stra");
        assert_eq!(hy("miasto"), "mia-sto");
        assert_eq!(hy("polityczny"), "po-li-tycz-ny");
        assert_eq!(hy("przedmieścia"), "przed-mie-ścia");
    }

    #[test]
    fn short_words_and_single_letters() {
        assert_eq!(hy("oko"), "oko");
        assert_eq!(hy("okno"), "okno");
        // No single vowel left at the start.
        assert_eq!(hy("obecnie"), "obec-nie");
    }

    #[test]
    fn punctuation_is_not_hyphenated() {
        assert_eq!(hy("(Wokulskiego),"), "(Wo-kul-skie-go),");
    }
}
