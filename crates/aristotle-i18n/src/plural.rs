//! Plural rules, per language code.
//!
//! The build script includes this file too, so the forms it requires of a
//! locale file are the forms these rules can pick. A language without a
//! rule here uses [`Rule::Fallback`] ("one" for 1, "other" otherwise),
//! which is right for many languages and readable for the rest; add a rule
//! when a language needs more forms.

/// A plural category, as CLDR names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    One,
    Few,
    Many,
    Other,
}

impl Category {
    pub const ALL: [Category; 4] = [
        Category::One,
        Category::Few,
        Category::Many,
        Category::Other,
    ];

    /// The key of this form in a locale file.
    pub const fn key(self) -> &'static str {
        match self {
            Category::One => "one",
            Category::Few => "few",
            Category::Many => "many",
            Category::Other => "other",
        }
    }
}

/// How a language picks a plural form for a whole number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// English: "one" for 1, "other" otherwise.
    OneOther,
    /// Polish: "one" for 1; "few" when n mod 10 is 2 to 4 and n mod 100 is
    /// not 12 to 14 (2, 3, 22 książki); "many" otherwise (0, 5, 12, 25
    /// książek).
    Polish,
    /// Languages without a rule of their own: as English.
    Fallback,
}

impl Rule {
    /// The rule for a language code ("en", "pl", "pt-BR": only the language
    /// part counts).
    pub fn for_language(code: &str) -> Rule {
        let lang = code.split(['-', '_']).next().unwrap_or(code);
        match lang {
            "en" => Rule::OneOther,
            "pl" => Rule::Polish,
            _ => Rule::Fallback,
        }
    }

    /// The forms a locale must give every plural message.
    pub fn categories(self) -> &'static [Category] {
        match self {
            Rule::OneOther | Rule::Fallback => &[Category::One, Category::Other],
            Rule::Polish => &[Category::One, Category::Few, Category::Many],
        }
    }

    pub fn select(self, n: u64) -> Category {
        match self {
            Rule::OneOther | Rule::Fallback => {
                if n == 1 {
                    Category::One
                } else {
                    Category::Other
                }
            }
            Rule::Polish => {
                if n == 1 {
                    Category::One
                } else if (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100)) {
                    Category::Few
                } else {
                    Category::Many
                }
            }
        }
    }
}
