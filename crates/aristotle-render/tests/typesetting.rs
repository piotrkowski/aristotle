// SPDX-License-Identifier: AGPL-3.0-or-later
//! Typesetting regressions on small generated EPUBs: forced page breaks,
//! hyphenation by element language, the Greek koronis and images.
//!
//! Needs the runtime data directory that `tools/build-render.sh host`
//! assembles (fonts as shipped, hyphenation patterns), found like in
//! `navigation.rs`: `$ARISTOTLE_RENDER_DATA`, then `render/host/data` in the
//! cache directory. Skipped, with a message, when neither exists.

use std::io::Write;
use std::path::PathBuf;

use aristotle_render::book::{Book, Word};
use aristotle_render::page::Chrome;
use aristotle_render::style::Style;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn data_dir() -> Option<PathBuf> {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    let dir = var("ARISTOTLE_RENDER_DATA")
        .map(PathBuf::from)
        .or_else(|| {
            var("ARISTOTLE_CACHE")
                .map(PathBuf::from)
                .or_else(|| {
                    var("XDG_CACHE_HOME")
                        .map(PathBuf::from)
                        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".cache")))
                        .map(|c| c.join("aristotle"))
                })
                .map(|c| c.join("render/host/data"))
        })?;
    dir.join("fonts").is_dir().then_some(dir)
}

/// Change into the data directory, or say why the test is skipped.
fn use_data() -> bool {
    match data_dir() {
        Some(d) => {
            aristotle_render::use_data_dir(&d).unwrap();
            true
        }
        // CI builds the data first, so a missing directory there is a
        // broken job, not a reason to skip.
        None if std::env::var_os("CI").is_some() => {
            panic!("no render data directory in CI (run tools/build-render.sh host first)")
        }
        None => {
            eprintln!("skipped: no render data directory (run tools/build-render.sh host)");
            false
        }
    }
}

/// An EPUB 3 with one spine item per chapter body, `css` as its stylesheet,
/// `dc:language` `lang` and the given images (name, bytes).
fn epub(
    name: &str,
    lang: &str,
    css: &str,
    chapters: &[&str],
    images: &[(&str, Vec<u8>)],
) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "aristotle-render-typesetting-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.epub"));
    let mut zip = ZipWriter::new(std::fs::File::create(&path).unwrap());
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut put = |file: &str, data: &[u8]| {
        zip.start_file(file, opts).unwrap();
        zip.write_all(data).unwrap();
    };
    put("mimetype", b"application/epub+zip");
    put(
        "META-INF/container.xml",
        b"<?xml version=\"1.0\"?>\n<container version=\"1.0\" \
          xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles>\
          <rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/>\
          </rootfiles></container>\n",
    );
    put("style.css", css.as_bytes());
    let mut manifest = String::from(
        "<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>\n\
         <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n",
    );
    let mut spine = String::new();
    let mut nav = String::new();
    for (n, body) in chapters.iter().enumerate() {
        let file = format!("c{n}.xhtml");
        put(
            &file,
            format!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" \
                 lang=\"{lang}\" xml:lang=\"{lang}\"><head><title>{n}</title>\
                 <link rel=\"stylesheet\" type=\"text/css\" href=\"style.css\"/></head>\
                 <body>{body}</body></html>\n"
            )
            .as_bytes(),
        );
        manifest.push_str(&format!(
            "<item id=\"c{n}\" href=\"{file}\" media-type=\"application/xhtml+xml\"/>\n"
        ));
        spine.push_str(&format!("<itemref idref=\"c{n}\"/>\n"));
        nav.push_str(&format!("<li><a href=\"{file}\">Chapter {n}</a></li>"));
    }
    for (i, (file, data)) in images.iter().enumerate() {
        put(file, data);
        manifest.push_str(&format!(
            "<item id=\"i{i}\" href=\"{file}\" media-type=\"image/png\"/>\n"
        ));
    }
    put(
        "nav.xhtml",
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" \
             xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>Contents</title></head><body>\
             <nav epub:type=\"toc\"><ol>{nav}</ol></nav></body></html>\n"
        )
        .as_bytes(),
    );
    put(
        "content.opf",
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" \
             version=\"3.0\" unique-identifier=\"id\"><metadata \
             xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:identifier id=\"id\">{name}</dc:identifier>\
             <dc:title>{name}</dc:title><dc:language>{lang}</dc:language></metadata>\
             <manifest>{manifest}</manifest><spine>{spine}</spine></package>\n"
        )
        .as_bytes(),
    );
    zip.finish().unwrap();
    path
}

/// Every page start of `book`, by walking from the first page.
fn page_starts(book: &mut Book) -> Vec<usize> {
    let mut pages = vec![book.page_start(0).unwrap()];
    while let Some(next) = book.next_page(*pages.last().unwrap()) {
        pages.push(next);
    }
    pages
}

fn words(book: &mut Book) -> Vec<Word> {
    let starts = page_starts(book);
    starts.into_iter().flat_map(|p| book.words(p)).collect()
}

// Aristotle, Nicomachean Ethics I.1, in D. P. Chase's translation (1847).
const ENGLISH: &str = "Every art, and every science reduced to a teachable form, and in like \
    manner every action and moral choice, aims, it is thought, at some good: for which reason \
    a common and by no means a bad description of the Chief Good is, that which all things \
    aim at.";

// Aristotle, Nicomachean Ethics 1094a1-6 (Bywater's text).
const GREEK: &str = "Πᾶσα τέχνη καὶ πᾶσα μέθοδος, ὁμοίως δὲ πρᾶξίς τε καὶ προαίρεσις, \
    ἀγαθοῦ τινὸς ἐφίεσθαι δοκεῖ· διὸ καλῶς ἀπεφήναντο τἀγαθόν, οὗ πάντ᾽ ἐφίεται. διαφορὰ \
    δέ τις φαίνεται τῶν τελῶν· τὰ μὲν γάρ εἰσιν ἐνέργειαι, τὰ δὲ παρ᾽ αὐτὰς ἔργα τινά. \
    ὧν δ᾽ εἰσὶ τέλη τινὰ παρὰ τὰς πράξεις, ἐν τούτοις βελτίω πέφυκε τῶν ἐνεργειῶν τὰ ἔργα.";

#[test]
fn forced_break_at_the_top_of_a_page_leaves_no_blank_page() {
    if !use_data() {
        return;
    }
    // As in Project Gutenberg's EPUBs: the chapter opens with an element
    // that has an id, and its first heading has a forced break before it;
    // an empty block with a break after it follows the first chapter's text.
    let css = "h2 { page-break-before: always } div.end { page-break-after: always }";
    let first = format!(
        "<header id=\"pg-header\"><h2 id=\"h\">The Project Gutenberg eBook</h2>\
         <p>{ENGLISH}</p></header><div class=\"end\"></div><div class=\"end\"></div>\
         <h2>Book I</h2><p>{ENGLISH}</p>"
    );
    let path = epub(
        "breaks",
        "en",
        css,
        &[&first, &format!("<h2>Book II</h2><p>{ENGLISH}</p>")],
        &[],
    );
    let mut book = Book::open(&path, &Style::pw4(), None).unwrap();
    let starts = page_starts(&mut book);
    for &p in &starts {
        assert!(!book.words(p).is_empty(), "blank page at {p} of {starts:?}");
    }
    // Title page, Book I, Book II.
    assert_eq!(starts.len(), 3, "{starts:?}");
}

/// The pieces of Greek words that end at a hyphenation break.
fn greek_breaks(words: &[Word]) -> usize {
    words
        .iter()
        .filter(|w| w.text.ends_with('\u{AD}'))
        .filter(|w| {
            w.text.chars().any(|c| {
                ('\u{370}'..='\u{3FF}').contains(&c) || ('\u{1F00}'..='\u{1FFF}').contains(&c)
            })
        })
        .count()
}

#[test]
fn element_lang_selects_hyphenation_patterns() {
    if !use_data() {
        return;
    }
    // A narrow column, so the Greek can only be justified with hyphens.
    let style = Style {
        font_size: 15.0,
        ..Style::pw4()
    };
    let greek = |attrs: &str| {
        let body = format!(
            "<p>{ENGLISH}</p><p{attrs}>{GREEK}</p><p>{ENGLISH} <span{attrs}>{GREEK}</span></p>"
        );
        let path = epub("greek", "en", "", &[&body], &[]);
        let mut book = Book::open(&path, &style, None).unwrap();
        greek_breaks(&words(&mut book))
    };
    // English patterns have nothing for Greek letters.
    assert_eq!(greek(""), 0);
    assert!(greek(" lang=\"grc\"") > 2, "lang");
    assert!(greek(" xml:lang=\"grc\"") > 2, "xml:lang");
}

#[test]
fn koronis_is_as_wide_as_an_apostrophe() {
    if !use_data() {
        return;
    }
    // U+1FBD stands for the elision mark in many Greek texts (πάντ᾽), and
    // should look like U+2019, the elision mark Unicode recommends.
    let body = "<p style=\"text-align: left\">πάντ\u{1FBD} ἐφίεται</p>\
                <p style=\"text-align: left\">πάντ\u{2019} ἐφίεται</p>";
    let path = epub("koronis", "grc", "p { text-align: left }", &[body], &[]);
    let mut book = Book::open(&path, &Style::pw4(), None).unwrap();
    let start = book.page_start(0).unwrap();
    let w = book.words(start);
    let find = |t: &str| {
        w.iter()
            .find(|x| x.text == t)
            .unwrap_or_else(|| panic!("no {t:?} in {w:?}"))
            .rect
    };
    let (koronis, apostrophe) = (find("πάντ\u{1FBD}"), find("πάντ\u{2019}"));
    let (next_k, next_a) = (
        w.iter()
            .filter(|x| x.text == "ἐφίεται")
            .map(|x| x.rect)
            .next()
            .unwrap(),
        w.iter()
            .filter(|x| x.text == "ἐφίεται")
            .map(|x| x.rect)
            .nth(1)
            .unwrap(),
    );
    assert_eq!(koronis.x1 - koronis.x0, apostrophe.x1 - apostrophe.x0);
    assert_eq!(next_k.x0, next_a.x0);
}

/// A gray-level PNG, `w` × `h`, all `gray`, deflate-stored.
fn png(w: u32, h: u32, gray: u8) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }
    fn adler32(data: &[u8]) -> u32 {
        let (mut a, mut b) = (1u32, 0u32);
        for &x in data {
            a = (a + x as u32) % 65521;
            b = (b + a) % 65521;
        }
        (b << 16) | a
    }
    let mut raw = Vec::new();
    for _ in 0..h {
        raw.push(0); // filter: none
        raw.extend(std::iter::repeat_n(gray, w as usize));
    }
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, block) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        let n = block.len() as u16;
        z.extend(n.to_le_bytes());
        z.extend((!n).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend(adler32(&raw).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend((data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend(crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 0, 0, 0, 0]); // 8-bit gray
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

#[test]
fn pages_know_where_their_images_are() {
    if !use_data() {
        return;
    }
    let body = format!("<p>{ENGLISH}</p><p><img src=\"gray.png\" alt=\"\"/></p><p>{ENGLISH}</p>");
    let path = epub(
        "image",
        "en",
        "",
        &[&body],
        &[("gray.png", png(300, 200, 0x40))],
    );
    let style = Style::pw4();
    let mut book = Book::open(&path, &style, None).unwrap();
    let mut chrome = Chrome::load(&style).unwrap();
    let start = book.page_start(0).unwrap();
    let page = book.render_location(start, &mut chrome).unwrap();
    assert_eq!(page.images.len(), 1, "{:?}", page.images);
    let r = page.images[0];
    assert!(r.x1 - r.x0 >= 250 && r.y1 - r.y0 >= 150, "{r:?}");
    // The image's pixels, not the text's.
    let (cx, cy) = ((r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2);
    assert_eq!(page.data[cy as usize * page.stride + cx as usize], 0x40);
    // Night mode inverts them first, and twice gives the page back.
    let mut night = page.clone();
    night.invert_images();
    assert_eq!(night.data[cy as usize * page.stride + cx as usize], 0xbf);
    assert_eq!(
        night.data[..r.y0 as usize * page.stride],
        page.data[..r.y0 as usize * page.stride]
    );
    night.invert_images();
    assert!(night.data == page.data);
    // Text-only pages have none.
    let text = epub("text", "en", "", &[&format!("<p>{ENGLISH}</p>")], &[]);
    let mut book = Book::open(&text, &style, None).unwrap();
    let start = book.page_start(0).unwrap();
    assert!(book
        .render_location(start, &mut chrome)
        .unwrap()
        .images
        .is_empty());
}
