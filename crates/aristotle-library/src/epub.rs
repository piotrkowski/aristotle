// SPDX-License-Identifier: AGPL-3.0-or-later
//! EPUB 2 and 3 metadata, plus CBZ which shares the ZIP plumbing.
//!
//! The path is `META-INF/container.xml` → the first rootfile with the OPF
//! media type → the OPF `<metadata>` and `<manifest>`. Only those two members
//! and the cover are read; the content documents are never opened, except
//! one cover page when the cover is only reachable through the guide.
//!
//! EPUB 3 moved several fields into `<meta refines="#id" property="...">`
//! elements; calibre still writes EPUB 2 style `opf:` attributes and
//! `calibre:*` metas, often both. We read both and prefer EPUB 3 where they
//! disagree, because calibre keeps the two in sync and other tools only
//! write the EPUB 3 form.

use std::collections::HashMap;
use std::path::Path;

use roxmltree::{Document, Node, ParsingOptions};

use crate::encoding::decode_xml;
use crate::error::{Error, Result};
use crate::metadata::{Author, Extracted, Identifier, Metadata, Series};
use crate::text;
use crate::zip::ZipArchive;

pub fn extract(path: &Path, want_cover: bool) -> Result<Extracted> {
    let mut zip = ZipArchive::open(path)?;
    let opf_path = match zip.read("META-INF/container.xml") {
        Ok(bytes) => rootfile(&decode_xml(&bytes))?,
        Err(_) => None,
    };
    // Some broken files lack container.xml; any OPF will do then.
    let opf_path = match opf_path {
        Some(p) => p,
        None => zip
            .entries()
            .iter()
            .find(|e| e.name.to_ascii_lowercase().ends_with(".opf"))
            .map(|e| e.name.clone())
            .ok_or_else(|| Error::format("epub: no package document"))?,
    };
    let opf = decode_xml(&zip.read(&opf_path)?);
    let base = opf_path
        .rfind('/')
        .map(|i| &opf_path[..i + 1])
        .unwrap_or("");
    let pkg = parse_opf(&opf, base)?;
    let mut cover = None;
    if want_cover {
        cover = find_cover(&mut zip, &pkg);
    }
    Ok(Extracted {
        meta: pkg.meta,
        cover,
        dictionary: false,
    })
}

fn rootfile(container: &str) -> Result<Option<String>> {
    let doc = parse_xml(container)?;
    let mut first = None;
    for n in doc
        .descendants()
        .filter(|n| n.has_tag_name_local("rootfile"))
    {
        let Some(p) = n.attribute("full-path") else {
            continue;
        };
        if n.attribute("media-type")
            .is_none_or(|m| m == "application/oebps-package+xml")
        {
            return Ok(Some(p.to_owned()));
        }
        first.get_or_insert_with(|| p.to_owned());
    }
    Ok(first)
}

/// Parses XML leniently: DTDs allowed, and HTML named entities (`&nbsp;` in a
/// description) rewritten to numeric references on a second attempt, since
/// XML parsers rightly reject them and real OPFs contain them anyway.
pub(crate) fn parse_xml(s: &str) -> Result<Document<'_>> {
    let opts = ParsingOptions {
        allow_dtd: true,
        ..ParsingOptions::default()
    };
    Document::parse_with_options(s, opts).map_err(|e| Error::format(format!("xml: {e}")))
}

fn parse_xml_lenient(s: &str) -> Result<(Option<String>, ())> {
    match parse_xml(s) {
        Ok(_) => Ok((None, ())),
        Err(e) => {
            let fixed = html_entities_to_numeric(s);
            if fixed == s {
                return Err(e);
            }
            parse_xml(&fixed)?;
            Ok((Some(fixed), ()))
        }
    }
}

fn html_entities_to_numeric(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let semi = rest
            .find(';')
            .filter(|&i| i <= 10 && rest[1..i].chars().all(|c| c.is_ascii_alphanumeric()));
        if let Some(i) = semi {
            let name = &rest[1..i];
            if !matches!(name, "amp" | "lt" | "gt" | "quot" | "apos") && !name.starts_with('#') {
                let decoded = text::decode_entities(&rest[..=i]);
                if let Some(c) = decoded
                    .chars()
                    .next()
                    .filter(|_| decoded.chars().count() == 1)
                {
                    out.push_str(&format!("&#{};", c as u32));
                    rest = &rest[i + 1..];
                    continue;
                }
            }
        }
        out.push('&');
        rest = &rest[1..];
    }
    out.push_str(rest);
    out
}

trait NodeExt {
    fn has_tag_name_local(&self, name: &str) -> bool;
    fn attr_local(&self, name: &str) -> Option<&str>;
    fn text_all(&self) -> String;
}

impl NodeExt for Node<'_, '_> {
    /// Matches on the local name only: OPFs in the wild put Dublin Core
    /// elements in the wrong namespace or none at all.
    fn has_tag_name_local(&self, name: &str) -> bool {
        self.is_element() && self.tag_name().name().eq_ignore_ascii_case(name)
    }

    /// `opf:role`, `role` and `ns0:role` all count as "role".
    fn attr_local(&self, name: &str) -> Option<&str> {
        self.attributes()
            .find(|a| a.name() == name)
            .map(|a| a.value())
    }

    fn text_all(&self) -> String {
        text::clean(
            &self
                .descendants()
                .filter(|n| n.is_text())
                .filter_map(|n| n.text())
                .collect::<String>(),
        )
    }
}

#[derive(Debug, Default)]
struct Package {
    meta: Metadata,
    /// Manifest: id → (href resolved against the ZIP root, media type, properties).
    items: HashMap<String, (String, String, String)>,
    spine_first: Option<String>,
    meta_cover: Option<String>,
    guide_cover: Option<String>,
}

fn parse_opf(opf: &str, base: &str) -> Result<Package> {
    let (fixed, ()) = parse_xml_lenient(opf)?;
    let src = fixed.as_deref().unwrap_or(opf);
    let doc = parse_xml(src)?;
    let root = doc.root_element();
    let mut pkg = Package::default();

    let Some(md) = root
        .descendants()
        .find(|n| n.has_tag_name_local("metadata"))
    else {
        return Err(Error::format("epub: OPF without metadata"));
    };

    // EPUB 3 refinements: id → [(property, value, scheme)].
    let mut refines: HashMap<&str, Vec<(String, String)>> = HashMap::new();
    let mut named: Vec<(String, String)> = Vec::new();
    for m in md.descendants().filter(|n| n.has_tag_name_local("meta")) {
        if let (Some(r), Some(p)) = (m.attribute("refines"), m.attribute("property")) {
            refines
                .entry(r.trim_start_matches('#'))
                .or_default()
                .push((p.to_owned(), m.text_all()));
        } else if let Some(name) = m.attribute("name") {
            named.push((
                name.to_owned(),
                m.attribute("content").unwrap_or("").to_owned(),
            ));
        } else if let Some(p) = m.attribute("property") {
            // EPUB 3 calibre writes some calibre:* fields as properties.
            named.push((
                p.to_owned(),
                m.attribute("content")
                    .map(str::to_owned)
                    .unwrap_or_else(|| m.text_all()),
            ));
        }
    }
    let refined = |n: Node, prop: &str| -> Option<String> {
        let id = n.attribute("id")?;
        refines
            .get(id)?
            .iter()
            .find(|(p, _)| p == prop)
            .map(|(_, v)| v.clone())
    };
    let named_get = |key: &str| named.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());

    let m = &mut pkg.meta;
    // Title: the one refined as "main", else the first.
    let titles: Vec<Node> = md
        .descendants()
        .filter(|n| n.has_tag_name_local("title"))
        .collect();
    let main = titles
        .iter()
        .find(|t| refined(**t, "title-type").as_deref() == Some("main"))
        .or(titles.first());
    if let Some(t) = main {
        m.title = t.text_all();
    }

    // Creators with role "aut" or no role. Translators and illustrators are not authors.
    let mut creators: Vec<(u32, Author, Option<String>)> = Vec::new();
    for (i, c) in md
        .descendants()
        .filter(|n| n.has_tag_name_local("creator"))
        .enumerate()
    {
        let role = refined(c, "role").or_else(|| c.attr_local("role").map(str::to_owned));
        let file_as = refined(c, "file-as").or_else(|| c.attr_local("file-as").map(str::to_owned));
        let seq = refined(c, "display-seq")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1000 + i as u32);
        creators.push((
            seq,
            Author::new(&c.text_all(), file_as.as_deref()),
            role.map(|r| r.to_ascii_lowercase()),
        ));
    }
    creators.sort_by_key(|c| c.0);
    m.authors = creators
        .iter()
        .filter(|c| c.2.as_deref().is_none_or(|r| r == "aut"))
        .map(|c| c.1.clone())
        .collect();
    if m.authors.is_empty() {
        m.authors = creators
            .iter()
            .filter(|c| matches!(c.2.as_deref(), Some("edt" | "com")))
            .map(|c| c.1.clone())
            .collect();
    }

    m.language = md
        .descendants()
        .find(|n| n.has_tag_name_local("language"))
        .map(|n| n.text_all());
    m.description = md
        .descendants()
        .find(|n| n.has_tag_name_local("description"))
        .map(|n| {
            text::strip_html(
                &n.descendants()
                    .filter(|n| n.is_text())
                    .filter_map(|n| n.text())
                    .collect::<String>(),
            )
        });
    m.publisher = md
        .descendants()
        .find(|n| n.has_tag_name_local("publisher"))
        .map(|n| n.text_all());
    let dates: Vec<Node> = md
        .descendants()
        .filter(|n| n.has_tag_name_local("date"))
        .collect();
    m.published = dates
        .iter()
        .find(|d| d.attr_local("event") == Some("publication"))
        .or_else(|| {
            dates
                .iter()
                .find(|d| d.attr_local("event").is_none_or(|e| e != "modification"))
        })
        .map(|d| d.text_all());
    m.subjects = md
        .descendants()
        .filter(|n| n.has_tag_name_local("subject"))
        .map(|n| n.text_all())
        .filter(|s| !s.is_empty())
        .collect();
    for idn in md
        .descendants()
        .filter(|n| n.has_tag_name_local("identifier"))
    {
        let scheme =
            refined(idn, "identifier-type").or_else(|| idn.attr_local("scheme").map(str::to_owned));
        if let Some(id) = identifier(scheme.as_deref(), &idn.text_all()) {
            if !m.identifiers.contains(&id) {
                m.identifiers.push(id);
            }
        }
    }

    // Series: EPUB 3 collections first, then calibre's metas.
    let collections = md.descendants().filter(|n| {
        n.has_tag_name_local("meta") && n.attribute("property") == Some("belongs-to-collection")
    });
    for c in collections {
        let kind = refined(c, "collection-type");
        if kind.as_deref().is_none_or(|k| k == "series") {
            let index = refined(c, "group-position").and_then(|s| parse_index(&s));
            m.series = Some(Series {
                name: c.text_all(),
                index,
            });
            break;
        }
    }
    if m.series.is_none() {
        if let Some(name) = named_get("calibre:series").filter(|s| !s.trim().is_empty()) {
            let index = named_get("calibre:series_index").and_then(|s| parse_index(&s));
            m.series = Some(Series {
                name: text::clean(&name),
                index,
            });
        }
    }
    // calibre's title_sort and author_sort are not used: our sort keys come
    // from per-author file-as, which calibre also writes.

    if let Some(manifest) = root
        .descendants()
        .find(|n| n.has_tag_name_local("manifest"))
    {
        for it in manifest.children().filter(|n| n.has_tag_name_local("item")) {
            let (Some(id), Some(href)) = (it.attribute("id"), it.attribute("href")) else {
                continue;
            };
            pkg.items.insert(
                id.to_owned(),
                (
                    resolve(base, href),
                    it.attribute("media-type").unwrap_or("").to_owned(),
                    it.attribute("properties").unwrap_or("").to_owned(),
                ),
            );
        }
    }
    pkg.meta_cover = named_get("cover").filter(|s| !s.is_empty());
    if let Some(guide) = root.descendants().find(|n| n.has_tag_name_local("guide")) {
        pkg.guide_cover = guide
            .children()
            .filter(|n| n.has_tag_name_local("reference"))
            .find(|r| {
                r.attribute("type")
                    .is_some_and(|t| t.eq_ignore_ascii_case("cover"))
            })
            .and_then(|r| r.attribute("href"))
            .map(|h| resolve(base, h));
    }
    pkg.spine_first = root
        .descendants()
        .find(|n| n.has_tag_name_local("spine"))
        .and_then(|s| s.children().find(|n| n.has_tag_name_local("itemref")))
        .and_then(|r| r.attribute("idref"))
        .map(str::to_owned);
    Ok(pkg)
}

fn parse_index(s: &str) -> Option<f64> {
    s.trim()
        .replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

fn identifier(scheme: Option<&str>, value: &str) -> Option<Identifier> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let lower = value.to_ascii_lowercase();
    for (prefix, s) in [
        ("urn:isbn:", "isbn"),
        ("isbn:", "isbn"),
        ("urn:uuid:", "uuid"),
        ("uuid:", "uuid"),
        ("doi:", "doi"),
        ("urn:doi:", "doi"),
    ] {
        if lower.starts_with(prefix) {
            return Some(Identifier {
                scheme: s.into(),
                value: value[prefix.len()..].to_owned(),
            });
        }
    }
    let scheme = scheme.unwrap_or("").trim().to_ascii_lowercase();
    let scheme = match scheme.as_str() {
        "mobi-asin" | "amazon" | "asin" => "asin".to_owned(),
        "15" => "isbn".to_owned(), // ONIX code list 5, used by EPUB 3 identifier-type
        _ => scheme,
    };
    Some(Identifier {
        scheme,
        value: value.to_owned(),
    })
}

/// Resolves an OPF-relative href to a ZIP member name: strips the fragment,
/// percent-decodes, and folds `.` and `..` segments.
pub(crate) fn resolve(base: &str, href: &str) -> String {
    let href = href.split('#').next().unwrap_or("");
    let href = percent_decode(href);
    let joined = match href.strip_prefix('/') {
        Some(abs) => abs.to_owned(),
        None => format!("{base}{href}"),
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_owned();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (
                (b[i + 1] as char).to_digit(16),
                (b[i + 2] as char).to_digit(16),
            ) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Cover lookup, in order of reliability:
/// 1. EPUB 3 manifest item with `properties="cover-image"`;
/// 2. EPUB 2 `<meta name="cover" content="item-id">` (some tools put the href there);
/// 3. the guide's `type="cover"` page, then the first image it shows;
/// 4. a manifest image whose id or href says "cover" (or Polish "okladka");
/// 5. the first spine page if it is named like a cover page.
fn find_cover(zip: &mut ZipArchive, pkg: &Package) -> Option<Vec<u8>> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some((href, _, _)) = pkg
        .items
        .values()
        .find(|(_, _, p)| p.split_whitespace().any(|p| p == "cover-image"))
    {
        candidates.push(href.clone());
    }
    if let Some(c) = &pkg.meta_cover {
        if let Some((href, _, _)) = pkg.items.get(c) {
            candidates.push(href.clone());
        } else if let Some((href, _, _)) =
            pkg.items.values().find(|(h, _, _)| h.ends_with(c.as_str()))
        {
            candidates.push(href.clone());
        }
    }
    if let Some(g) = &pkg.guide_cover {
        candidates.push(g.clone());
    }
    let mut named: Vec<(&String, &(String, String, String))> = pkg
        .items
        .iter()
        .filter(|(id, (href, mt, _))| {
            let k = format!("{id} {href}").to_lowercase();
            mt.starts_with("image/")
                && (k.contains("cover") || k.contains("okladka") || k.contains("okładka"))
        })
        .collect();
    named.sort();
    candidates.extend(named.into_iter().map(|(_, (h, _, _))| h.clone()));
    if let Some((href, _, _)) = pkg.spine_first.as_ref().and_then(|id| pkg.items.get(id)) {
        let k = href.to_lowercase();
        if k.contains("cover") || k.contains("okladka") || k.contains("title") {
            candidates.push(href.clone());
        }
    }
    for href in candidates {
        let Ok(data) = zip.read(&href) else { continue };
        if crate::cover::sniff_image(&data).is_some() {
            return Some(data);
        }
        // A cover page: follow its first image.
        let page = String::from_utf8_lossy(&data);
        let dir = href.rfind('/').map(|i| &href[..i + 1]).unwrap_or("");
        if let Some(src) = first_image_src(&page) {
            if let Ok(img) = zip.read(&resolve(dir, &src)) {
                if crate::cover::sniff_image(&img).is_some() {
                    return Some(img);
                }
            }
        }
    }
    None
}

/// Finds the first `<img src>` or SVG `<image xlink:href>` in an XHTML page
/// with a plain text scan, because cover pages often contain HTML entities
/// or broken markup that an XML parser would reject.
fn first_image_src(page: &str) -> Option<String> {
    let lower = page.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<im") {
        let start = at + i;
        let end = lower[start..].find('>').map(|e| start + e)?;
        let tag = &page[start..end];
        let ltag = &lower[start..end];
        if ltag.starts_with("<img") || ltag.starts_with("<image") {
            for attr in ["xlink:href", "src", "href"] {
                if let Some(v) = attr_value(tag, ltag, attr) {
                    return Some(text::decode_entities(&v));
                }
            }
        }
        at = end;
    }
    None
}

fn attr_value(tag: &str, ltag: &str, attr: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = ltag[from..].find(attr) {
        let p = from + i;
        let before_ok = p > 0 && ltag.as_bytes()[p - 1].is_ascii_whitespace();
        let rest = ltag[p + attr.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let vstart = ltag.len() - rest.len() + 1;
            let v = tag[vstart..].trim_start();
            let q = v.chars().next()?;
            if q == '"' || q == '\'' {
                let v = &v[1..];
                return Some(v[..v.find(q)?].to_owned());
            }
        }
        from = p + attr.len();
    }
    None
}

/// CBZ: title from the file name (left empty here), page count, the first
/// image as the cover, and `ComicInfo.xml` when present.
pub fn extract_cbz(path: &Path, want_cover: bool) -> Result<Extracted> {
    let mut zip = ZipArchive::open(path)?;
    let mut images: Vec<_> = zip
        .entries()
        .iter()
        .filter(|e| crate::cbz::is_page_name(&e.name))
        .cloned()
        .collect();
    images.sort_by(|a, b| crate::cbz::page_order(&a.name, &b.name));
    let mut ex = Extracted::default();
    ex.meta.pages = Some(images.len() as u32);
    if let Ok(info) = zip.read("ComicInfo.xml") {
        let s = decode_xml(&info);
        if let Ok(doc) = parse_xml(&s) {
            let get = |tag: &str| {
                doc.descendants()
                    .find(|n| n.has_tag_name_local(tag))
                    .map(|n| n.text_all())
                    .filter(|s| !s.is_empty())
            };
            ex.meta.title = get("Title").unwrap_or_default();
            if let Some(w) = get("Writer") {
                ex.meta.authors = w.split(',').map(|a| Author::new(a, None)).collect();
            }
            if let Some(s) = get("Series") {
                ex.meta.series = Some(Series {
                    name: s,
                    index: get("Number").and_then(|n| parse_index(&n)),
                });
            }
            ex.meta.description = get("Summary");
            ex.meta.language = get("LanguageISO");
            ex.meta.publisher = get("Publisher");
        }
    }
    if want_cover {
        if let Some(first) = images.first() {
            if let Ok(bytes) = zip.read_entry(first) {
                if matches!(
                    crate::cover::sniff_image(&bytes),
                    Some(crate::cover::ImageKind::Jpeg | crate::cover::ImageKind::Png)
                ) {
                    ex.cover = Some(bytes);
                }
            }
        }
    }
    Ok(ex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_hrefs() {
        assert_eq!(
            resolve("OEBPS/", "Images/cover%20big.jpg#x"),
            "OEBPS/Images/cover big.jpg"
        );
        assert_eq!(
            resolve("OEBPS/Text/", "../Images/c.jpg"),
            "OEBPS/Images/c.jpg"
        );
        assert_eq!(resolve("", "c.jpg"), "c.jpg");
    }

    #[test]
    fn finds_image_in_cover_page() {
        let svg = r#"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><image width="1" xlink:href="../Images/cover.jpeg"/></svg>"#;
        assert_eq!(
            first_image_src(svg).as_deref(),
            Some("../Images/cover.jpeg")
        );
        let html = "<p>&nbsp;</p><IMG alt='x' SRC='okladka.jpg'>";
        assert_eq!(first_image_src(html).as_deref(), Some("okladka.jpg"));
    }

    #[test]
    fn epub2_calibre_opf() {
        let opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="uid">
 <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
  <dc:title>Państwo</dc:title>
  <dc:creator opf:role="aut" opf:file-as="Platon">Platon</dc:creator>
  <dc:creator opf:role="trl" opf:file-as="Witwicki, Władysław">Władysław Witwicki</dc:creator>
  <dc:language>pl</dc:language>
  <dc:identifier opf:scheme="ISBN">978-83-0000-000-0</dc:identifier>
  <dc:identifier id="uid">urn:uuid:1234</dc:identifier>
  <dc:description>&lt;p&gt;Dialog o sprawiedliwości&amp;nbsp;i państwie.&lt;/p&gt;</dc:description>
  <meta name="calibre:series" content="Dialogi"/>
  <meta name="calibre:series_index" content="3.0"/>
  <meta name="cover" content="cover-img"/>
 </metadata>
 <manifest><item id="cover-img" href="Images/okładka.jpg" media-type="image/jpeg"/></manifest>
</package>"#;
        let pkg = parse_opf(opf, "OEBPS/").unwrap();
        let m = &pkg.meta;
        assert_eq!(m.title, "Państwo");
        assert_eq!(
            m.authors,
            vec![Author {
                name: "Platon".into(),
                sort: "Platon".into()
            }]
        );
        assert_eq!(m.language.as_deref(), Some("pl"));
        assert_eq!(
            m.series,
            Some(Series {
                name: "Dialogi".into(),
                index: Some(3.0)
            })
        );
        assert_eq!(
            m.description.as_deref(),
            Some("Dialog o sprawiedliwości i państwie.")
        );
        assert_eq!(
            m.identifiers[0],
            Identifier {
                scheme: "isbn".into(),
                value: "978-83-0000-000-0".into()
            }
        );
        assert_eq!(
            m.identifiers[1],
            Identifier {
                scheme: "uuid".into(),
                value: "1234".into()
            }
        );
        assert_eq!(pkg.meta_cover.as_deref(), Some("cover-img"));
        assert_eq!(pkg.items["cover-img"].0, "OEBPS/Images/okładka.jpg");
    }

    #[test]
    fn epub3_refines_and_collections() {
        let opf = r##"<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
 <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
  <dc:title id="t2">Podtytuł</dc:title>
  <dc:title id="t1">Placówka</dc:title>
  <meta refines="#t1" property="title-type">main</meta>
  <meta refines="#t2" property="title-type">subtitle</meta>
  <dc:creator id="c2">Anna Tłumaczka</dc:creator>
  <meta refines="#c2" property="role" scheme="marc:relators">trl</meta>
  <dc:creator id="c1">Bolesław Prus</dc:creator>
  <meta refines="#c1" property="file-as">Prus, Bolesław</meta>
  <meta refines="#c1" property="role" scheme="marc:relators">aut</meta>
  <meta property="belongs-to-collection" id="s">Nowele &amp; powieści</meta>
  <meta refines="#s" property="collection-type">series</meta>
  <meta refines="#s" property="group-position">2</meta>
  <dc:description>Bez &nbsp; encji</dc:description>
 </metadata>
 <manifest><item id="ci" href="cover.png" media-type="image/png" properties="cover-image"/></manifest>
</package>"##;
        let pkg = parse_opf(opf, "").unwrap();
        let m = &pkg.meta;
        assert_eq!(m.title, "Placówka");
        assert_eq!(
            m.authors,
            vec![Author {
                name: "Bolesław Prus".into(),
                sort: "Prus, Bolesław".into()
            }]
        );
        assert_eq!(
            m.series,
            Some(Series {
                name: "Nowele & powieści".into(),
                index: Some(2.0)
            })
        );
        assert_eq!(m.description.as_deref(), Some("Bez encji"));
        assert_eq!(pkg.items["ci"].2, "cover-image");
    }
}
