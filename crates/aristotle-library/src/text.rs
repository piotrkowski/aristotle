//! Text helpers: diacritic folding for search and sorting, HTML stripping for
//! descriptions, and author sort names.
//!
//! Folding is table-driven rather than based on Unicode decomposition because
//! decomposition does not help with the letters that matter most here: Polish
//! `ł` has no decomposition, so NFD-based folding would leave "Łódź" and
//! "Lodz" unequal. The table covers Latin-1 Supplement and Latin Extended-A,
//! which is every Latin letter in European book titles; combining marks are
//! dropped so that already-decomposed input folds too.

/// Lowercases, removes diacritics, normalises quotes and collapses whitespace.
///
/// `fold("Zażółć  Gęślą")` is `"zazolc gesla"`.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = true;
    for c in s.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
                space = true;
            }
            continue;
        }
        if ('\u{0300}'..='\u{036F}').contains(&c) {
            continue;
        }
        for lc in c.to_lowercase() {
            push_folded(&mut out, lc);
        }
        space = false;
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

fn push_folded(out: &mut String, c: char) {
    let mapped: &str = match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĳ' => "ij",
        'ĵ' => "j",
        'ķ' | 'ĸ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' | 'ŉ' | 'ŋ' => "n",
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ß' => "ss",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ſ' => "s",
        'ţ' | 'ť' | 'ŧ' => "t",
        'þ' => "th",
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        '‘' | '’' | '‚' | '‛' | '`' | '´' => "'",
        '“' | '”' | '„' | '‟' | '«' | '»' => "\"",
        '–' | '—' | '‐' | '‑' => "-",
        _ => {
            out.push(c);
            return;
        }
    };
    out.push_str(mapped);
}

/// Returns true if every whitespace-separated word of the (already folded)
/// `query` occurs in `haystack` (already folded). Word order does not matter,
/// so "platon uczta" finds "Uczta" by "Platon".
pub fn matches_folded(haystack: &str, folded_query: &str) -> bool {
    folded_query
        .split(' ')
        .filter(|w| !w.is_empty())
        .all(|w| haystack.contains(w))
}

/// Turns an author's display name into a sort name: "Adam Mickiewicz" becomes
/// "Mickiewicz, Adam". Names that already contain a comma are taken as sort names.
///
/// This is only the fallback when the book has no `file-as`; it gets
/// multi-word surnames ("Conan Doyle") wrong, which is why the OPF value wins.
pub fn author_sort_name(name: &str) -> String {
    let name = name.trim();
    if name.contains(',') {
        return name.to_owned();
    }
    let words: Vec<&str> = name.split_whitespace().collect();
    match words.len() {
        0 => String::new(),
        1 => words[0].to_owned(),
        n => {
            // Keep generational suffixes with the surname: "Oliver Wendell Holmes Jr." → "Holmes, Oliver Wendell Jr."
            let suffix = matches!(
                words[n - 1].trim_end_matches('.').to_lowercase().as_str(),
                "jr" | "sr" | "ii" | "iii" | "iv"
            );
            if suffix && n > 2 {
                format!(
                    "{}, {} {}",
                    words[n - 2],
                    words[..n - 2].join(" "),
                    words[n - 1]
                )
            } else {
                format!("{}, {}", words[n - 1], words[..n - 1].join(" "))
            }
        }
    }
}

/// Decodes the five XML entities, numeric references and the HTML named
/// entities that show up in book descriptions. Unknown entities are kept.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let end = rest[1..]
            .find(|c: char| c == ';' || c == '&' || c.is_whitespace())
            .map(|i| i + 1);
        match end {
            Some(e) if rest.as_bytes()[e] == b';' && e <= 12 => {
                let name = &rest[1..e];
                if let Some(c) = entity_char(name) {
                    out.push(c);
                    rest = &rest[e + 1..];
                    continue;
                }
                out.push('&');
                rest = &rest[1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity_char(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = if let Some(hex) = num.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num.parse().ok()?
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{A0}',
        "shy" => '\u{AD}',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "laquo" => '«',
        "raquo" => '»',
        "lsquo" => '‘',
        "rsquo" => '’',
        "sbquo" => '‚',
        "ldquo" => '“',
        "rdquo" => '”',
        "bdquo" => '„',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "deg" => '°',
        "middot" => '·',
        "bull" => '•',
        "eacute" => 'é',
        "oacute" => 'ó',
        "aacute" => 'á',
        "uuml" => 'ü',
        "ouml" => 'ö',
        "auml" => 'ä',
        "szlig" => 'ß',
        _ => return None,
    })
}

/// Converts an HTML fragment (EPUB and MOBI descriptions are often HTML) to
/// plain text: tags removed, block ends turned into blank lines, entities
/// decoded, whitespace collapsed.
pub fn strip_html(s: &str) -> String {
    let mut text = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(lt) = rest.find('<') {
        text.push_str(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>') else {
            // A lone '<' in plain text.
            text.push_str(&rest[lt..]);
            rest = "";
            break;
        };
        let tag = rest[lt + 1..lt + gt]
            .trim_start_matches('/')
            .to_ascii_lowercase();
        let name: String = tag
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if matches!(
            name.as_str(),
            "p" | "div"
                | "br"
                | "li"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "tr"
                | "blockquote"
        ) {
            text.push('\n');
        }
        rest = &rest[lt + gt + 1..];
    }
    text.push_str(rest);
    let text = decode_entities(&text);
    // Collapse runs of spaces inside lines and runs of blank lines.
    let mut out = String::with_capacity(text.len());
    for line in text.split('\n') {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&line);
    }
    out
}

/// Collapses whitespace runs to single spaces and trims.
pub fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_polish() {
        assert_eq!(fold("Zażółć  GĘŚLĄ jaźń"), "zazolc gesla jazn");
        assert_eq!(fold("ŁÓDŹ"), "lodz");
        assert_eq!(fold("Ä\u{0301}"), "a");
        assert_eq!(fold("„Książę”"), "\"ksiaze\"");
    }

    #[test]
    fn search_words_any_order() {
        let h = fold("Uczta Platon");
        assert!(matches_folded(&h, &fold("platon UCZTA")));
        assert!(!matches_folded(&h, &fold("platon fedon")));
    }

    #[test]
    fn author_sort() {
        assert_eq!(author_sort_name("Adam Mickiewicz"), "Mickiewicz, Adam");
        assert_eq!(author_sort_name("Mickiewicz, Adam"), "Mickiewicz, Adam");
        assert_eq!(author_sort_name("Platon"), "Platon");
        assert_eq!(
            author_sort_name("Oliver Wendell Holmes Jr."),
            "Holmes, Oliver Wendell Jr."
        );
    }

    #[test]
    fn html_to_text() {
        let d = "<p>Pierwszy &amp; <b>drugi</b>.</p><p>Trzeci&nbsp;akapit &#x105; &unknown;</p>";
        assert_eq!(
            strip_html(d),
            "Pierwszy & drugi.\n\nTrzeci akapit ą &unknown;"
        );
        assert_eq!(strip_html("a < b"), "a < b");
    }
}
