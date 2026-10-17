// SPDX-License-Identifier: AGPL-3.0-or-later
//! Footnotes: what a tapped link points at, as text for a popup.
//!
//! Plato's engine keeps only a link's `href`, so the note is found again in
//! the book's source: the target file is parsed with Plato's XML parser,
//! the element with the fragment's id is located, and the note's text is
//! taken from it ([`note_text`]). Whether a link is a footnote at all
//! ([`is_footnote`]) is decided from the markup around both ends:
//!
//! - EPUB 3 and DPUB-ARIA semantics: `epub:type="noteref"` or
//!   `role="doc-noteref"` on the link, `footnote`, `endnote`, `rearnote`
//!   or `note` (`doc-footnote`, `doc-endnote`) on the target or an
//!   ancestor;
//! - classes and ids that publishers and converters use (`footnote`,
//!   `fn`, `note`, `przypis`, calibre's `filepos12345`);
//! - the link text: a short marker such as `1`, `[12]`, `iv`, `*` or `†`,
//!   usually set as a superscript.
//!
//! Anything else (a cross reference, a table of contents entry) is a plain
//! link the reader follows.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use plato_core::document::html::dom::NodeRef;
use plato_core::document::html::xml::XmlParser;
use plato_core::helpers::Normalize;
use zip::ZipArchive;

use crate::richtext::{self, Block, RichText};

/// Notes longer than this are cut (with an ellipsis): a popup is for a
/// note, not for a chapter that a link happened to point at.
const MAX_NOTE: usize = 2400;

/// A link split into the archive path it points at and its fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub path: String,
    pub fragment: Option<String>,
}

/// Resolve `href` (as written in the book) against the spine item `base`.
/// `None` for external links.
pub fn target(base: &str, href: &str) -> Option<Target> {
    let lower = href.to_ascii_lowercase();
    if lower.contains("://") || lower.starts_with("mailto:") || lower.starts_with("tel:") {
        return None;
    }
    let (file, fragment) = match href.split_once('#') {
        Some((f, frag)) => (f, Some(frag.to_string()).filter(|s| !s.is_empty())),
        None => (href, None),
    };
    let path = if file.is_empty() {
        base.to_string()
    } else {
        let parent = Path::new(base).parent().unwrap_or_else(|| Path::new(""));
        parent.join(file).normalize().to_string_lossy().into_owned()
    };
    Some(Target { path, fragment })
}

/// Reads members of an EPUB, keeping the last one parsed (a chapter's
/// notes are usually all in one file).
pub struct Source {
    zip: ZipArchive<File>,
    cached: Option<(String, String)>,
}

impl Source {
    pub fn open(epub: &Path) -> Option<Source> {
        let zip = ZipArchive::new(File::open(epub).ok()?).ok()?;
        Some(Source { zip, cached: None })
    }

    fn text(&mut self, path: &str) -> Option<&str> {
        if self.cached.as_ref().map(|c| c.0.as_str()) != Some(path) {
            let mut s = String::new();
            self.zip.by_name(path).ok()?.read_to_string(&mut s).ok()?;
            self.cached = Some((path.to_string(), s));
        }
        self.cached.as_ref().map(|c| c.1.as_str())
    }

    /// Byte offset of the element with id `id` in the file `path`, the way
    /// Plato counts locations inside a spine item.
    pub fn id_offset(&mut self, path: &str, id: &str) -> Option<usize> {
        let src = self.text(path)?;
        let tree = XmlParser::new(src).parse();
        let node = tree.root().find_by_id(id)?;
        Some(node.offset())
    }

    /// The footnote behind a link in spine item `base`: its text, if the
    /// link is a footnote. `link_text` is what the link shows (the marker).
    pub fn footnote(&mut self, base: &str, href: &str, link_text: &str) -> Option<RichText> {
        let t = target(base, href)?;
        let frag = t.fragment.clone()?;
        // The link's own markup, from the file it is in.
        let link_semantics = self
            .text(base)
            .map(|src| {
                let tree = XmlParser::new(src).parse();
                let found = tree
                    .root()
                    .descendants()
                    .find(|n| n.tag_name() == Some("a") && n.attribute("href") == Some(href))
                    .map(|a| noteref_markup(a));
                found.unwrap_or(false)
            })
            .unwrap_or(false);
        let src = self.text(&t.path)?;
        let tree = XmlParser::new(src).parse();
        let node = tree.root().find_by_id(&frag)?;
        let text = note_text(node);
        if text.is_empty() {
            return None;
        }
        let note_like = link_semantics
            || is_marker(link_text)
            || note_markup(node)
            || note_id(&frag)
            || node.ancestor_elements().any(note_markup);
        note_like.then_some(text)
    }
}

fn has_word(value: Option<&str>, words: &[&str]) -> bool {
    value.is_some_and(|v| {
        v.split(|c: char| c.is_whitespace() || c == ',')
            .any(|w| words.contains(&w.to_ascii_lowercase().as_str()))
    })
}

/// `epub:type="noteref"` or alike on the link itself.
fn noteref_markup(a: NodeRef) -> bool {
    has_word(a.attribute("epub:type"), &["noteref"])
        || has_word(a.attribute("role"), &["doc-noteref"])
        || a.classes().any(|c| {
            let c = c.to_ascii_lowercase();
            c.contains("noteref") || c.contains("footnote") || c.contains("fnref")
        })
}

/// Footnote semantics on a target element.
fn note_markup(n: NodeRef) -> bool {
    has_word(
        n.attribute("epub:type"),
        &[
            "footnote",
            "endnote",
            "rearnote",
            "note",
            "footnotes",
            "endnotes",
            "rearnotes",
        ],
    ) || has_word(
        n.attribute("role"),
        &["doc-footnote", "doc-endnote", "doc-endnotes"],
    ) || n.classes().any(|c| {
        let c = c.to_ascii_lowercase();
        c.contains("footnote")
            || c.contains("endnote")
            || c.contains("przypis")
            || c == "annotation"
    })
}

/// Ids publishers and converters give notes: fn3, ftn3, note-3, footnote3,
/// przypis3, calibre's filepos12345, Word's _ftn3.
pub fn note_id(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    let id = id.trim_start_matches('_');
    [
        "fn",
        "ftn",
        "note",
        "footnote",
        "endnote",
        "przyp",
        "przypis",
        "filepos",
        "en",
        "nt",
        "annotation",
    ]
    .iter()
    .any(|p| {
        id.strip_prefix(p).is_some_and(|rest| {
            let rest = rest.trim_start_matches(['-', '_', ':', '.']);
            !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '-' || c == '_')
        })
    })
}

/// A footnote marker: 1, [12], (3), iv, *, †, a.
pub fn is_marker(text: &str) -> bool {
    let t = text
        .trim()
        .trim_matches(|c: char| "[]()<>{}.".contains(c))
        .trim();
    if t.is_empty() || t.chars().count() > 4 {
        return false;
    }
    t.chars().all(|c| c.is_ascii_digit())
        || t.chars().all(|c| "*†‡§¶#".contains(c))
        || t.chars().all(|c| "ivxlcIVXLC".contains(c))
        || (t.chars().count() == 1 && t.chars().all(|c| c.is_alphabetic()))
        || t.chars().all(|c| "⁰¹²³⁴⁵⁶⁷⁸⁹".contains(c))
}

fn is_block(n: &NodeRef) -> bool {
    matches!(
        n.tag_name(),
        Some(
            "p" | "li"
                | "aside"
                | "div"
                | "dd"
                | "dt"
                | "section"
                | "td"
                | "blockquote"
                | "body"
                | "ol"
                | "ul"
                | "dl"
        )
    )
}

fn visible_len(n: NodeRef) -> usize {
    n.text().split_whitespace().map(str::len).sum()
}

/// The text of the note whose anchor is `node`.
///
/// - A block with text of its own (`<aside id>`, `<p id>`, `<li id>`) is
///   the note.
/// - An empty or tiny anchor (`<a id="fn1"/>`, `<sup id><a>1</a></sup>`,
///   calibre's `<a id="filepos…">`) stands at the start of its note: the
///   note is the enclosing paragraph, or, when the enclosing block holds
///   many notes, the siblings after the anchor up to the next anchor.
pub fn note_text(node: NodeRef) -> RichText {
    let own = visible_len(node);
    let text = if is_block(&node) && own > 6 && node.tag_name() != Some("body") {
        richtext::from_node(node)
    } else {
        // Climb to the block that holds the anchor.
        let mut anchor = node;
        let mut block = node.parent_element();
        while let Some(b) = block {
            if is_block(&b) {
                break;
            }
            anchor = b;
            block = b.parent_element();
        }
        // A block holding several anchors holds several notes.
        let anchors = |b: NodeRef| b.descendants().filter(|n| n.id().is_some()).count();
        match block {
            Some(b)
                if visible_len(b) <= MAX_NOTE && b.tag_name() != Some("body") && anchors(b) < 2 =>
            {
                richtext::from_node(b)
            }
            Some(_) => siblings_after(anchor),
            None => richtext::from_node(node),
        }
    };
    cut(text)
}

/// The anchor and what follows it in its parent, up to the next element
/// that carries an id (the next note's anchor).
fn siblings_after(anchor: NodeRef) -> RichText {
    let mut out = RichText::default();
    let mut first = true;
    let mut n = Some(anchor);
    while let Some(node) = n {
        if !first && node.id().is_some() && node.is_element() {
            break;
        }
        first = false;
        out.blocks.extend(richtext::from_node(node).blocks);
        if out.len() > MAX_NOTE {
            break;
        }
        n = node.next_sibling();
    }
    // Inline siblings come out as separate blocks; join them.
    if out.blocks.len() > 1 {
        let mut joined = Block::default();
        for b in out.blocks {
            if !joined.runs.is_empty() {
                if let Some(r) = joined.runs.last_mut() {
                    r.text.push(' ');
                }
            }
            joined.runs.extend(b.runs);
        }
        out.blocks = vec![joined];
    }
    out
}

fn cut(mut text: RichText) -> RichText {
    let mut total = 0;
    let mut keep = 0;
    for b in &text.blocks {
        total += b.text().len();
        keep += 1;
        if total > MAX_NOTE {
            break;
        }
    }
    if keep < text.blocks.len() || total > MAX_NOTE {
        text.blocks.truncate(keep);
        if let Some(r) = text.blocks.last_mut().and_then(|b| b.runs.last_mut()) {
            r.text.push_str(" …");
        }
    }
    // Back links ("↩", "^") at the end of a note mean nothing in a popup.
    if let Some(b) = text.blocks.last_mut() {
        if let Some(r) = b.runs.last_mut() {
            let t = r
                .text
                .trim_end_matches(['↩', '↑', '^', '\u{FE0E}', ' '])
                .len();
            r.text.truncate(t);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(xhtml: &str, id: &str) -> String {
        let tree = XmlParser::new(xhtml).parse();
        let node = tree.root().find_by_id(id).expect("id");
        note_text(node).text()
    }

    #[test]
    fn targets() {
        assert_eq!(
            target("OEBPS/text/ch1.xhtml", "../notes.xhtml#fn3"),
            Some(Target {
                path: "OEBPS/notes.xhtml".into(),
                fragment: Some("fn3".into())
            })
        );
        assert_eq!(
            target("OEBPS/ch1.xhtml", "#n1").unwrap().path,
            "OEBPS/ch1.xhtml"
        );
        assert_eq!(target("a.xhtml", "https://pl.wikipedia.org/#x"), None);
    }

    #[test]
    fn markers_and_ids() {
        for m in ["1", "[12]", "(3)", "iv", "*", "†", "a", "²"] {
            assert!(is_marker(m), "{m}");
        }
        for m in ["Rozdział", "12345", "zob. dalej"] {
            assert!(!is_marker(m), "{m}");
        }
        for id in [
            "fn3",
            "ftn-3",
            "note_12",
            "footnote1",
            "przypis4",
            "filepos12345",
            "_ftn2",
        ] {
            assert!(note_id(id), "{id}");
        }
        for id in ["chapter3", "fnord", "toc", "note"] {
            assert!(!note_id(id), "{id}");
        }
    }

    #[test]
    fn epub3_aside() {
        let x = r##"<html><body><p>Tekst<a epub:type="noteref" href="#n1">1</a>.</p>
            <aside epub:type="footnote" id="n1"><p>Zob. <i>Fajdros</i> 245c.</p></aside></body></html>"##;
        assert_eq!(note(x, "n1"), "Zob. Fajdros 245c.");
    }

    #[test]
    fn calibre_empty_anchor_in_a_paragraph() {
        let x = r#"<html><body><div class="notes">
            <p class="calibre1"><a id="filepos1234"></a><sup>1</sup> Platon, <i>Fedon</i>, 64a.</p>
            <p class="calibre1"><a id="filepos1300"></a><sup>2</sup> Tamże.</p></div></body></html>"#;
        assert_eq!(note(x, "filepos1234"), "1 Platon, Fedon, 64a.");
        assert_eq!(note(x, "filepos1300"), "2 Tamże.");
    }

    #[test]
    fn endnote_list_items_and_backlinks() {
        let x = r#"<html><body><ol><li id="fn1"><p>Pierwsza uwaga. <a href="ch1.xhtml#r1">↩</a></p></li>
            <li id="fn2"><p>Druga.</p></li></ol></body></html>"#;
        assert_eq!(note(x, "fn1"), "Pierwsza uwaga.");
    }

    #[test]
    fn many_notes_in_one_block() {
        // Anchors and text as siblings directly in one big block.
        let filler = "x ".repeat(1500);
        let x = format!(
            r#"<html><body><div><span id="fn1">1</span> Pierwsza. <span id="fn2">2</span> Druga. {filler}</div></body></html>"#
        );
        assert_eq!(note(&x, "fn1"), "1 Pierwsza.");
    }
}
