// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reading positions in the form KOReader's progress sync (KOSync) speaks.
//!
//! The protocol carries a document ID, a `progress` string and a
//! percentage. The ID is KOReader's partial MD5, which is our book ID
//! without the `-2`/`-3` suffix of duplicate copies. What `progress` holds
//! depends on how KOReader opens the format (its `kosync.koplugin`,
//! `main.lua`): an XPointer for books crengine lays out, a 1-based page
//! number for paged documents.
//!
//! | Our locator | `progress` |
//! | --- | --- |
//! | `epub {spine}` | `/body/DocFragment[N]/body`, the start of the chapter |
//! | `page {index}` (PDF, CBZ) | `index + 1` |
//! | `xpointer` (imported from KOReader) | itself |
//! | `kindle`, `text` (MOBI, AZW3, TXT) | nothing: crengine builds its own DOM for these and no XPointer of ours would resolve |
//!
//! A pulled XPointer that KOReader cannot resolve sends its reader to page
//! one, so nothing is pushed rather than a guess.
//!
//! ## DocFragment numbers
//!
//! crengine makes one `DocFragment` per `<itemref>` whose `idref` names a
//! manifest `<item>` with an `id` and an `href`, in spine order, keeping a
//! placeholder fragment for items it cannot read so that later numbers
//! never shift (`crengine/src/epubfmt.cpp`, `ImportEpubDocument`). Plato
//! drops spine items whose file is missing from the archive, so our spine
//! index and the fragment number differ after such an item. [`Fragments`]
//! maps between the two from the OPF and the archive's member list, the
//! way both readers build their lists.

use std::path::Path;

use roxmltree::Node;

use crate::encoding::decode_xml;
use crate::epub::parse_xml;
use crate::error::{Error, Result};
use crate::format::Format;
use crate::index::Locator;
use crate::zip::ZipArchive;

/// The KOSync document ID of a book: its ID without a duplicate suffix.
pub fn document(book_id: &str) -> &str {
    match book_id.rsplit_once('-') {
        Some((id, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => id,
        _ => book_id,
    }
}

/// Plato's spine items against crengine's DocFragments for one EPUB.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fragments {
    /// The 1-based fragment of each of Plato's spine items.
    by_item: Vec<u32>,
    /// Plato's item of each fragment (index = number - 1), `None` for a
    /// fragment Plato skips.
    items: Vec<Option<usize>>,
}

impl Fragments {
    pub fn read(epub: &Path) -> Result<Fragments> {
        let mut zip = ZipArchive::open(epub)?;
        let container = decode_xml(&zip.read("META-INF/container.xml")?);
        let doc = parse_xml(&container)?;
        let opf_path = doc
            .descendants()
            .find(|n| n.tag_name().name() == "rootfile")
            .and_then(|n| n.attribute("full-path"))
            .ok_or_else(|| Error::format("epub: no rootfile"))?
            .to_string();
        let opf = decode_xml(&zip.read(&opf_path)?);
        let names: Vec<String> = zip.entries().iter().map(|e| e.name.clone()).collect();
        Fragments::from_opf(&opf, &opf_path, |name| names.iter().any(|n| n == name))
    }

    /// The map for an OPF at `opf_path`, with `in_zip` saying whether an
    /// archive member exists under exactly that name (Plato's lookup).
    pub fn from_opf(opf: &str, opf_path: &str, in_zip: impl Fn(&str) -> bool) -> Result<Fragments> {
        let doc = parse_xml(opf)?;
        let root = doc.root_element();
        fn child<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
            n.children().find(|c| c.tag_name().name() == name)
        }
        let manifest = child(root, "manifest").ok_or_else(|| Error::format("epub: no manifest"))?;
        let spine = child(root, "spine").ok_or_else(|| Error::format("epub: no spine"))?;
        let parent = Path::new(opf_path).parent().unwrap_or(Path::new(""));
        let mut f = Fragments::default();
        for itemref in spine.children().filter(|c| c.is_element()) {
            let Some(idref) = itemref.attribute("idref") else {
                continue;
            };
            let item = manifest
                .children()
                .find(|i| i.is_element() && i.attribute("id") == Some(idref));
            let Some(href) = item.and_then(|i| i.attribute("href")) else {
                continue;
            };
            // crengine: an <itemref> naming an <item> with an id and an
            // href makes a fragment.
            let fragment = itemref.tag_name().name() == "itemref"
                && item.is_some_and(|i| i.tag_name().name() == "item")
                && !href.is_empty();
            // Plato: the decoded href joined to the OPF's folder, as is,
            // must be an archive member.
            let decoded = percent_decode(&crate::text::decode_entities(href));
            let path = parent.join(&decoded);
            let plato = path.to_str().is_some_and(&in_zip);
            match (fragment, plato) {
                (true, true) => {
                    f.items.push(Some(f.by_item.len()));
                    f.by_item.push(f.items.len() as u32);
                }
                (true, false) => f.items.push(None),
                // Plato reads an item crengine has no fragment for: no
                // number to give it.
                (false, true) => f.by_item.push(0),
                (false, false) => {}
            }
        }
        Ok(f)
    }

    /// The DocFragment number of Plato's spine item `item`.
    pub fn fragment(&self, item: usize) -> Option<u32> {
        self.by_item.get(item).copied().filter(|&n| n > 0)
    }

    /// Plato's spine item for DocFragment `n`, or the next item Plato has
    /// when that fragment is a placeholder (the reader lands at the start
    /// of what follows).
    pub fn item(&self, n: u32) -> Option<usize> {
        let i = (n as usize).checked_sub(1)?;
        self.items.get(i..)?.iter().find_map(|x| *x)
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `progress` to push for a book at `locator`, or `None` when KOReader
/// could not use what we would send. `fragments` is needed for EPUB
/// locators.
pub fn progress(
    format: Format,
    locator: &Locator,
    fragments: Option<&Fragments>,
) -> Option<String> {
    match (format, locator) {
        (Format::Epub, Locator::Epub { spine, .. }) => fragments?
            .fragment(*spine as usize)
            .map(|n| format!("/body/DocFragment[{n}]/body")),
        (Format::Epub, Locator::XPointer { xpointer })
            if xpointer.starts_with("/body/DocFragment[") =>
        {
            Some(xpointer.clone())
        }
        (Format::Pdf | Format::Cbz, Locator::Page { index }) => Some((index + 1).to_string()),
        _ => None,
    }
}

/// A pulled `progress`, as far as we can place it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Remote {
    /// `/body/DocFragment[N]...`: chapter N; the rest of the path is not
    /// resolved (yet).
    Fragment(u32),
    /// A page number, made 0-based.
    Page(u32),
    /// Something else: an XPointer of a format we lay out differently.
    Other,
}

pub fn parse_remote(progress: &str) -> Remote {
    let p = progress.trim();
    if let Some(rest) = p.strip_prefix("/body/DocFragment[") {
        if let Some(n) = rest.split(']').next().and_then(|n| n.parse::<u32>().ok()) {
            if n > 0 {
                return Remote::Fragment(n);
            }
        }
        return Remote::Other;
    }
    match p.parse::<u32>() {
        Ok(n) if n > 0 => Remote::Page(n - 1),
        _ => Remote::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::testutil;

    const CHAPTER: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Litwo! Ojczyzno moja!</p></body></html>"#;

    /// An EPUB written for the test: a spine with a dangling idref, an
    /// item missing from the archive, an SVG page, a `../` href Plato
    /// cannot find and a percent-encoded name.
    fn book() -> Vec<u8> {
        let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Próba</dc:title></metadata>
  <manifest>
    <item id="c1" href="Text/one.xhtml" media-type="application/xhtml+xml"/>
    <item id="gone" href="Text/missing.xhtml" media-type="application/xhtml+xml"/>
    <item id="pic" href="Images/plate.svg" media-type="image/svg+xml"/>
    <item id="up" href="../outside.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="Text/two%20words.xhtml" media-type="application/xhtml+xml"/>
    <item id="nohref" media-type="application/xhtml+xml"/>
    <item id="c3" href="Text/three.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="c1"/>
    <itemref idref="nothing-here"/>
    <itemref idref="gone"/>
    <itemref idref="pic"/>
    <itemref idref="up"/>
    <itemref idref="c2" linear="no"/>
    <itemref idref="nohref"/>
    <itemref idref="c3"/>
  </spine>
</package>"#;
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#;
        testutil::build(&[
            ("mimetype", b"application/epub+zip", false),
            (
                "META-INF/container.xml",
                br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
                true,
            ),
            ("OEBPS/content.opf", opf.as_bytes(), true),
            ("OEBPS/Text/one.xhtml", CHAPTER.as_bytes(), true),
            ("OEBPS/Images/plate.svg", svg.as_bytes(), true),
            ("outside.xhtml", CHAPTER.as_bytes(), true),
            ("OEBPS/Text/two words.xhtml", CHAPTER.as_bytes(), true),
            ("OEBPS/Text/three.xhtml", CHAPTER.as_bytes(), true),
        ])
    }

    #[test]
    fn fragments_follow_crengine_and_items_follow_plato() {
        let dir = crate::testdir("kosync");
        let path = dir.join("book.epub");
        std::fs::write(&path, book()).unwrap();
        let f = Fragments::read(&path).unwrap();
        // crengine: c1 → 1, gone → 2 (placeholder), pic → 3, up → 4,
        // c2 → 5, c3 → 6. Plato: one, plate.svg, two words, three
        // ("missing" and "../outside" are not found under those names).
        assert_eq!(
            (0..4).map(|i| f.fragment(i)).collect::<Vec<_>>(),
            [Some(1), Some(3), Some(5), Some(6)]
        );
        assert_eq!(f.fragment(4), None);
        assert_eq!(
            (1..=7).map(|n| f.item(n)).collect::<Vec<_>>(),
            // A placeholder lands on the next chapter Plato has.
            [Some(0), Some(1), Some(1), Some(2), Some(2), Some(3), None]
        );
        assert_eq!(f.item(0), None);

        let at = |spine| Locator::Epub { spine, offset: 120 };
        assert_eq!(
            progress(Format::Epub, &at(2), Some(&f)).as_deref(),
            Some("/body/DocFragment[5]/body")
        );
        assert_eq!(progress(Format::Epub, &at(9), Some(&f)), None);
        assert_eq!(progress(Format::Epub, &at(0), None), None);
    }

    #[test]
    fn what_each_format_pushes() {
        let page = Locator::Page { index: 41 };
        assert_eq!(progress(Format::Pdf, &page, None).as_deref(), Some("42"));
        assert_eq!(progress(Format::Cbz, &page, None).as_deref(), Some("42"));
        // Never a made-up position for books crengine lays out itself.
        assert_eq!(
            progress(Format::Mobi, &Locator::Kindle { position: 7510 }, None),
            None
        );
        assert_eq!(
            progress(Format::Azw3, &Locator::Kindle { position: 7510 }, None),
            None
        );
        assert_eq!(
            progress(Format::Txt, &Locator::Text { offset: 99 }, None),
            None
        );
        assert_eq!(progress(Format::Mobi, &page, None), None);
        let x = Locator::XPointer {
            xpointer: "/body/DocFragment[12]/body/div/p[3]/text().40".into(),
        };
        assert_eq!(
            progress(Format::Epub, &x, None).as_deref(),
            Some("/body/DocFragment[12]/body/div/p[3]/text().40")
        );
        assert_eq!(
            progress(
                Format::Epub,
                &Locator::XPointer {
                    xpointer: String::new()
                },
                None
            ),
            None
        );
    }

    #[test]
    fn parses_pulled_positions_and_document_ids() {
        assert_eq!(
            parse_remote("/body/DocFragment[7]/body/div[2]/p[5]/text().12"),
            Remote::Fragment(7)
        );
        assert_eq!(parse_remote("/body/DocFragment[3]"), Remote::Fragment(3));
        assert_eq!(parse_remote("/body/DocFragment[0]/body"), Remote::Other);
        assert_eq!(parse_remote("42"), Remote::Page(41));
        assert_eq!(parse_remote("0"), Remote::Other);
        assert_eq!(parse_remote("#_doc_fragment_3"), Remote::Other);
        assert_eq!(
            document("0123456789abcdef0123456789abcdef"),
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(
            document("0123456789abcdef0123456789abcdef-2"),
            "0123456789abcdef0123456789abcdef"
        );
    }
}
