// SPDX-License-Identifier: AGPL-3.0-or-later
//! OPDS 1.2 catalogs (<https://specs.opds.io/opds-1.2>): Atom feeds of
//! navigation and acquisition entries, paging links, and search through an
//! OpenSearch description or a direct template.
//!
//! Elements are matched by local name, since catalogs disagree on
//! namespaces and prefixes. Links resolve against `xml:base` where a feed
//! sets it (Project Gutenberg does), else against the feed's own URL.
//!
//! - A link with an Atom type is navigation (calibre:
//!   `application/atom+xml;type=feed;profile=opds-catalog;kind=navigation`;
//!   Gutenberg: `rel="subsection"`).
//! - A link whose `rel` is `http://opds-spec.org/acquisition` or
//!   `.../acquisition/open-access` with a book type we read is a download.
//!   `/buy`, `/borrow`, `/subscribe`, `/sample` and indirect acquisitions are
//!   skipped: they lead to shops and loans, not files.

use roxmltree::{Document, Node};

use crate::http::{self, Client, Request, Timeouts};
use crate::url::{encode_component, Url, UrlError};

/// Feeds and search descriptions are limited to this size.
pub const FEED_LIMIT: u64 = 2 * 1024 * 1024;
const ACCEPT: &str = "application/atom+xml, application/xml;q=0.9, */*;q=0.5";

/// A book file type we can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BookType {
    Epub,
    Azw3,
    Mobi,
    Pdf,
    Cbz,
    Txt,
}

impl BookType {
    /// The type for a MIME type (parameters ignored).
    pub fn from_mime(mime: &str) -> Option<BookType> {
        let m = mime
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        Some(match m.as_str() {
            "application/epub+zip" => BookType::Epub,
            "application/x-mobi8-ebook" | "application/vnd.amazon.mobi8-ebook" => BookType::Azw3,
            "application/x-mobipocket-ebook" | "application/vnd.amazon.ebook" => BookType::Mobi,
            "application/pdf" => BookType::Pdf,
            "application/x-cbz" | "application/vnd.comicbook+zip" => BookType::Cbz,
            "text/plain" => BookType::Txt,
            _ => return None,
        })
    }

    /// The file extension, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            BookType::Epub => "epub",
            BookType::Azw3 => "azw3",
            BookType::Mobi => "mobi",
            BookType::Pdf => "pdf",
            BookType::Cbz => "cbz",
            BookType::Txt => "txt",
        }
    }

    /// The name shown on buttons.
    pub fn label(self) -> &'static str {
        match self {
            BookType::Epub => "EPUB",
            BookType::Azw3 => "AZW3",
            BookType::Mobi => "MOBI",
            BookType::Pdf => "PDF",
            BookType::Cbz => "CBZ",
            BookType::Txt => "TXT",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acquisition {
    pub url: Url,
    pub kind: BookType,
    /// The link's title, e.g. Gutenberg's "EPUB3 (E-readers incl.
    /// Send-to-Kindle)".
    pub title: Option<String>,
    /// `length`, in bytes, when the catalog says.
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    /// `summary` or `content` as plain text.
    pub summary: String,
    /// A feed this entry leads to.
    pub navigation: Option<Url>,
    /// Downloads, best first (EPUB, AZW3, MOBI, PDF, CBZ, TXT), one per
    /// type: the first link of each type in the feed's order.
    pub acquisitions: Vec<Acquisition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Search {
    /// An OpenSearch description to fetch for the template.
    Description(Url),
    /// A template with `{searchTerms}`, given directly (calibre).
    Template(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Feed {
    pub title: String,
    pub entries: Vec<Entry>,
    pub next: Option<Url>,
    pub previous: Option<Url>,
    pub search: Option<Search>,
}

#[derive(Debug)]
pub enum Error {
    Http(http::Error),
    /// Not XML, or not an Atom feed (an HTML login page, an OPDS 2 JSON
    /// feed).
    NotAFeed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http(e) => e.fmt(f),
            Error::NotAFeed(s) => write!(f, "not an OPDS catalog: {s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<http::Error> for Error {
    fn from(e: http::Error) -> Self {
        Error::Http(e)
    }
}

impl From<UrlError> for Error {
    fn from(e: UrlError) -> Self {
        Error::Http(e.into())
    }
}

trait NodeExt {
    fn is(&self, local: &str) -> bool;
    fn child(&self, local: &str) -> Option<Node<'_, '_>>;
    fn text_all(&self) -> String;
}

impl NodeExt for Node<'_, '_> {
    fn is(&self, local: &str) -> bool {
        self.is_element() && self.tag_name().name() == local
    }

    fn child(&self, local: &str) -> Option<Node<'_, '_>> {
        self.children().find(|c| c.is(local))
    }

    fn text_all(&self) -> String {
        let mut s = String::new();
        for d in self.descendants().filter(|d| d.is_text()) {
            s.push_str(d.text().unwrap_or(""));
            s.push(' ');
        }
        s
    }
}

/// Collapsed whitespace.
fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// The base for links inside `n`: `xml:base` on it or its ancestors,
/// resolved outward to the document's URL.
fn base_of(n: Node, doc_url: &Url) -> Url {
    let bases: Vec<&str> = n
        .ancestors()
        .filter_map(|a| a.attribute((XML_NS, "base")))
        .collect();
    let mut url = doc_url.clone();
    for b in bases.iter().rev() {
        if let Ok(u) = url.join(b) {
            url = u;
        }
    }
    url
}

/// HTML in `summary` or `content` (escaped `type="html"` or inline XHTML)
/// as plain text.
fn plain_text(n: Node) -> String {
    let raw = n.text_all();
    let html = matches!(n.attribute("type"), Some("html"));
    if !html {
        return clean(&raw);
    }
    let mut out = String::new();
    let mut in_tag = false;
    for c in raw.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    clean(
        &out.replace("&nbsp;", " ")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'"),
    )
}

fn is_atom(mime: &str) -> bool {
    mime.split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("application/atom+xml")
}

fn is_acquisition(rel: &str) -> bool {
    rel == "http://opds-spec.org/acquisition"
        || rel == "http://opds-spec.org/acquisition/open-access"
}

/// Parse an Atom/OPDS feed fetched from `url`.
pub fn parse_feed(xml: &str, url: &Url) -> Result<Feed, Error> {
    let doc = Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|e| Error::NotAFeed(e.to_string()))?;
    let root = doc.root_element();
    if !root.is("feed") {
        return Err(Error::NotAFeed(format!("<{}>", root.tag_name().name())));
    }
    let base = base_of(root, url);
    let mut feed = Feed {
        title: root
            .child("title")
            .map(|t| clean(&t.text_all()))
            .unwrap_or_default(),
        ..Feed::default()
    };
    for link in root.children().filter(|c| c.is("link")) {
        let (Some(rel), Some(href)) = (link.attribute("rel"), link.attribute("href")) else {
            continue;
        };
        let Ok(target) = base.join(href) else {
            continue;
        };
        let mime = link.attribute("type").unwrap_or("");
        match rel {
            "next" => feed.next = Some(target),
            "previous" | "prev" => feed.previous = Some(target),
            "search" => {
                let search = if mime
                    .to_ascii_lowercase()
                    .starts_with("application/opensearchdescription+xml")
                {
                    Some(Search::Description(target))
                } else if is_atom(mime) && href.contains("{searchTerms}") {
                    // Keep the braces: joining would percent-encode them.
                    Some(Search::Template(resolve_template(&base, href)))
                } else {
                    None
                };
                if feed.search.is_none() || matches!(search, Some(Search::Template(_))) {
                    feed.search = search.or(feed.search.take());
                }
            }
            _ => {}
        }
    }
    for e in root.children().filter(|c| c.is("entry")) {
        feed.entries.push(parse_entry(e, url));
    }
    Ok(feed)
}

fn parse_entry(e: Node, doc_url: &Url) -> Entry {
    let base = base_of(e, doc_url);
    let mut entry = Entry {
        id: e
            .child("id")
            .map(|n| clean(&n.text_all()))
            .unwrap_or_default(),
        title: e
            .child("title")
            .map(|n| clean(&n.text_all()))
            .unwrap_or_default(),
        ..Entry::default()
    };
    for a in e.children().filter(|c| c.is("author")) {
        let name = a
            .child("name")
            .map(|n| clean(&n.text_all()))
            .unwrap_or_default();
        if !name.is_empty() {
            entry.authors.push(name);
        }
    }
    entry.summary = e
        .child("summary")
        .or_else(|| e.child("content"))
        .map(plain_text)
        .unwrap_or_default();
    let mut acquisitions: Vec<Acquisition> = Vec::new();
    for link in e.children().filter(|c| c.is("link")) {
        let Some(href) = link.attribute("href") else {
            continue;
        };
        let Ok(url) = base.join(href) else { continue };
        let rel = link.attribute("rel").unwrap_or("");
        let mime = link.attribute("type").unwrap_or("");
        if is_acquisition(rel) {
            // An indirect acquisition wraps the file in something else.
            if link.children().any(|c| c.is("indirectAcquisition")) {
                continue;
            }
            let Some(kind) = BookType::from_mime(mime) else {
                continue;
            };
            if acquisitions.iter().any(|a| a.kind == kind) {
                continue;
            }
            acquisitions.push(Acquisition {
                url,
                kind,
                title: link.attribute("title").map(clean),
                size: link.attribute("length").and_then(|l| l.trim().parse().ok()),
            });
        } else if is_atom(mime)
            && entry.navigation.is_none()
            && !rel.starts_with("http://opds-spec.org/")
            && !matches!(rel, "alternate" | "self" | "up" | "start")
        {
            entry.navigation = Some(url);
        }
    }
    acquisitions.sort_by_key(|a| a.kind);
    entry.acquisitions = acquisitions;
    entry
}

/// A template href resolved against `base`, with `{...}` kept.
fn resolve_template(base: &Url, href: &str) -> String {
    const MARK: &str = "ARISTOTLESEARCHTERMS";
    let marked = href.replace("{searchTerms}", MARK);
    match base.join(&marked) {
        Ok(u) => u.to_string().replace(MARK, "{searchTerms}"),
        Err(_) => href.to_string(),
    }
}

/// The Atom search template of an OpenSearch description.
pub fn parse_opensearch(xml: &str, url: &Url) -> Option<String> {
    let doc = Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .ok()?;
    let urls: Vec<Node> = doc.descendants().filter(|n| n.is("Url")).collect();
    let pick = urls
        .iter()
        .find(|n| {
            n.attribute("type")
                .is_some_and(|t| is_atom(t) && t.contains("opds"))
        })
        .or_else(|| {
            urls.iter()
                .find(|n| n.attribute("type").is_some_and(is_atom))
        })?;
    let template = pick.attribute("template")?;
    Some(resolve_template(url, template))
}

/// The search URL for `terms`: `{searchTerms}` filled in, other optional
/// parameters (`{startPage?}`) dropped, required ones left to the server's
/// default where the spec allows (`{count}` → nothing).
pub fn search_url(template: &str, terms: &str) -> Result<Url, UrlError> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let name = &rest[open + 1..open + close];
        if name == "searchTerms" {
            out.push_str(&encode_component(terms.trim()));
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    Url::parse(&out)
}

/// Fetches feeds with the catalog's credentials.
pub struct Catalog {
    http: Client,
    credentials: Option<crate::auth::Credentials>,
}

impl Catalog {
    pub fn new(http: Client, credentials: Option<crate::auth::Credentials>) -> Catalog {
        Catalog { http, credentials }
    }

    /// The default patience for catalog pages: 10 s to connect, 20 s for
    /// the rest.
    pub fn timeouts() -> Timeouts {
        Timeouts {
            connect: std::time::Duration::from_secs(10),
            response: std::time::Duration::from_secs(20),
            body: std::time::Duration::from_secs(30),
        }
    }

    pub fn client(&self) -> &Client {
        &self.http
    }

    pub fn credentials(&self) -> Option<&crate::auth::Credentials> {
        self.credentials.as_ref()
    }

    fn fetch(&self, url: &Url) -> Result<http::Response, Error> {
        let req = Request::get(url.clone())
            .header("accept", ACCEPT)
            .credentials(self.credentials.clone());
        let r = self.http.send(&req, FEED_LIMIT)?;
        if r.status != 200 {
            return Err(http::status_error(r.status, &r.body).into());
        }
        Ok(r)
    }

    pub fn feed(&self, url: &Url) -> Result<Feed, Error> {
        let r = self.fetch(url)?;
        parse_feed(&decode(&r.body), &r.url)
    }

    /// The search URL for `terms`, fetching the OpenSearch description if
    /// the feed points to one.
    pub fn search_url(&self, search: &Search, terms: &str) -> Result<Url, Error> {
        let template = match search {
            Search::Template(t) => t.clone(),
            Search::Description(url) => {
                let r = self.fetch(url)?;
                parse_opensearch(&decode(&r.body), &r.url)
                    .ok_or_else(|| Error::NotAFeed("no Atom search template".into()))?
            }
        };
        Ok(search_url(&template, terms)?)
    }
}

/// XML bytes as text: UTF-8 (with or without BOM), else Latin-1.
fn decode(b: &[u8]) -> String {
    let b = b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(b);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    const CALIBRE_ROOT: &str = include_str!("../tests/fixtures/calibre-root.xml");
    const CALIBRE_BOOKS: &str = include_str!("../tests/fixtures/calibre-books.xml");
    const GUTENBERG_SEARCH: &str = include_str!("../tests/fixtures/gutenberg-search.xml");
    const GUTENBERG_BOOK: &str = include_str!("../tests/fixtures/gutenberg-book.xml");
    const GUTENBERG_OSD: &str = include_str!("../tests/fixtures/gutenberg-osd.xml");

    #[test]
    fn calibre_navigation_feed() {
        let f = parse_feed(CALIBRE_ROOT, &url("http://192.168.15.201:8080/opds")).unwrap();
        assert_eq!(f.title, "calibre Library");
        assert_eq!(f.entries.len(), 3);
        let e = &f.entries[0];
        assert_eq!(e.title, "By Newest");
        assert_eq!(e.summary, "Books sorted by date");
        assert_eq!(
            e.navigation.as_ref().unwrap().to_string(),
            "http://192.168.15.201:8080/opds/navcatalog/4f6e6577657374?library_id=Calibre_Library"
        );
        assert!(e.acquisitions.is_empty());
        assert_eq!(
            f.search,
            Some(Search::Template(
                "http://192.168.15.201:8080/opds/search?query={searchTerms}".into()
            ))
        );
        let s = search_url(
            match &f.search {
                Some(Search::Template(t)) => t,
                _ => unreachable!(),
            },
            "Pan Tadeusz",
        )
        .unwrap();
        assert_eq!(s.target(), "/opds/search?query=Pan%20Tadeusz");
    }

    #[test]
    fn calibre_acquisition_feed_with_paging() {
        let f = parse_feed(
            CALIBRE_BOOKS,
            &url("http://192.168.15.201:8080/opds/navcatalog/4f6e6577657374"),
        )
        .unwrap();
        assert_eq!(f.entries.len(), 2);
        let lalka = &f.entries[0];
        assert_eq!(lalka.title, "Lalka");
        assert_eq!(lalka.authors, ["Bolesław Prus"]);
        assert!(
            lalka.summary.starts_with("SERIES: Powieści [1]"),
            "{}",
            lalka.summary
        );
        assert!(!lalka.summary.contains('<'));
        assert_eq!(lalka.navigation, None);
        let kinds: Vec<BookType> = lalka.acquisitions.iter().map(|a| a.kind).collect();
        // EPUB first, whatever the feed's order; the cover links are not books.
        assert_eq!(kinds, [BookType::Epub, BookType::Azw3, BookType::Pdf]);
        assert_eq!(lalka.acquisitions[0].size, Some(1_234_567));
        assert_eq!(
            lalka.acquisitions[0].url.to_string(),
            "http://192.168.15.201:8080/get/epub/1/Calibre_Library"
        );
        assert_eq!(
            f.next.as_ref().unwrap().target(),
            "/opds/navcatalog/4f6e6577657374?library_id=Calibre_Library&offset=25"
        );
        assert_eq!(f.previous, None);
    }

    #[test]
    fn gutenberg_feeds_with_xml_base_and_opensearch() {
        let doc = url("https://m.gutenberg.org/ebooks/search.opds/?query=austen");
        let f = parse_feed(GUTENBERG_SEARCH, &doc).unwrap();
        assert_eq!(f.title, "Project Gutenberg: Books: austen");
        // xml:base moves the links to www.gutenberg.org.
        assert_eq!(
            f.next.as_ref().unwrap().to_string(),
            "https://www.gutenberg.org/ebooks/search.opds/?query=austen&start_index=26"
        );
        assert_eq!(
            f.search,
            Some(Search::Description(url(
                "https://www.gutenberg.org/catalog/osd-books.xml"
            )))
        );
        let pride = &f.entries[0];
        assert_eq!(pride.title, "Pride and Prejudice");
        assert_eq!(pride.summary, "Jane Austen");
        assert_eq!(
            pride.navigation.as_ref().unwrap().to_string(),
            "https://www.gutenberg.org/ebooks/1342.opds"
        );
        // The "sort by" and "Next Page" navigation entries are entries too.
        assert!(f.entries.iter().all(|e| e.acquisitions.is_empty()));

        let f = parse_feed(
            GUTENBERG_BOOK,
            &url("https://www.gutenberg.org/ebooks/1342.opds"),
        )
        .unwrap();
        let book = f
            .entries
            .iter()
            .find(|e| !e.acquisitions.is_empty())
            .unwrap();
        assert_eq!(book.authors, ["Austen, Jane"]);
        let kinds: Vec<(BookType, &str)> = book
            .acquisitions
            .iter()
            .map(|a| (a.kind, a.url.target()))
            .collect();
        assert_eq!(
            kinds,
            [
                (BookType::Epub, "/ebooks/1342.epub3.images"),
                (BookType::Mobi, "/ebooks/1342.kf8.images"),
                (BookType::Txt, "/ebooks/1342.txt.utf-8"),
            ]
        );
        assert_eq!(
            book.acquisitions[0].title.as_deref(),
            Some("EPUB3 (E-readers incl. Send-to-Kindle)")
        );

        let t = parse_opensearch(
            GUTENBERG_OSD,
            &url("https://www.gutenberg.org/catalog/osd-books.xml"),
        )
        .unwrap();
        assert_eq!(
            t,
            "https://www.gutenberg.org/ebooks/search.opds/?query={searchTerms}"
        );
        assert_eq!(
            search_url(&t, "moby dick").unwrap().to_string(),
            "https://www.gutenberg.org/ebooks/search.opds/?query=moby%20dick"
        );
        assert_eq!(
            search_url("http://h/s?q={searchTerms}&p={startPage?}", "x")
                .unwrap()
                .target(),
            "/s?q=x&p="
        );
    }

    #[test]
    fn rejects_what_is_not_a_feed_and_skips_shops() {
        assert!(matches!(
            parse_feed("<html><body>Log in</body></html>", &url("http://h/")),
            Err(Error::NotAFeed(_))
        ));
        assert!(matches!(
            parse_feed("{\"metadata\":{}}", &url("http://h/")),
            Err(Error::NotAFeed(_))
        ));
        let xml = r#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Shop</title>
          <entry><title>For sale</title>
            <link rel="http://opds-spec.org/acquisition/buy" type="application/epub+zip" href="/buy/1"/>
            <link rel="http://opds-spec.org/acquisition/borrow" type="application/epub+zip" href="/loan/1"/>
            <link rel="http://opds-spec.org/acquisition" type="application/vnd.adobe.adept+xml" href="/drm/1">
              <opds:indirectAcquisition xmlns:opds="http://opds-spec.org/2010/catalog" type="application/epub+zip"/>
            </link>
            <link rel="http://opds-spec.org/acquisition/open-access" type="application/epub+zip" href="/free/1.epub"/>
            <summary type="html">&lt;p&gt;A &amp;amp; B&amp;nbsp;C&lt;/p&gt;</summary>
          </entry></feed>"#;
        let f = parse_feed(xml, &url("http://h/x/")).unwrap();
        let e = &f.entries[0];
        assert_eq!(e.acquisitions.len(), 1);
        assert_eq!(e.acquisitions[0].url.target(), "/free/1.epub");
        assert_eq!(e.summary, "A & B C");
    }
}
