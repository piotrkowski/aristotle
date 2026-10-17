//! Location-based navigation against Plato's engine, on a generated book.
//!
//! Needs the runtime data directory (fonts, stylesheets, hyphenation
//! patterns) that `tools/build-render.sh host` assembles. The test looks for
//! it in `$ARISTOTLE_RENDER_DATA`, then in `~/.cache/aristotle/render/host/data`,
//! and is skipped (with a message) when neither exists.

use std::path::PathBuf;

use aristotle_render::book::{Book, Position};
use aristotle_render::mobi::{write_epub, Mobi, MobiInfo};
use aristotle_render::style::Style;

fn data_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("ARISTOTLE_RENDER_DATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".cache/aristotle/render/host/data"))
        })?;
    dir.join("fonts").is_dir().then_some(dir)
}

fn sample_epub() -> PathBuf {
    let mut html = String::from("<html><body>");
    for ch in 0..4 {
        html.push_str(&format!("<h1>Rozdział {ch}</h1>"));
        for p in 0..60 {
            html.push_str(&format!(
                "<p>Akapit {ch}-{p}. Zażółć gęślą jaźń, a potem jeszcze raz to samo, \
                 żeby tekst wypełnił kilka stron i podzielił się na wiersze.</p>"
            ));
        }
        html.push_str("<mbp:pagebreak/>");
    }
    html.push_str("</body></html>");
    let mobi = Mobi {
        info: MobiInfo {
            encoding: 65001,
            language: Some("pl".into()),
            title: Some("Próba".into()),
            ..Default::default()
        },
        html,
        images: Vec::new(),
    };
    let dir = std::env::temp_dir().join(format!(
        "aristotle-render-nav-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("proba.epub");
    write_epub(&mobi, &path).unwrap();
    path
}

#[test]
fn locations_agree_with_pagination() {
    let Some(data) = data_dir() else {
        eprintln!("skipped: no render data directory (run tools/build-render.sh host)");
        return;
    };
    let path = sample_epub();
    aristotle_render::use_data_dir(&data).unwrap();
    let style = Style::pw4();

    // Walking page by page from the start finds exactly the pages that a
    // full pagination finds.
    let mut full = Book::open(&path, &style, None).unwrap();
    let total = full.paginate();
    let pages = full.pagination().unwrap().to_vec();
    assert_eq!(pages.len(), total);
    assert!(total > 8, "{total} pages");

    let mut book = Book::open(&path, &style, None).unwrap();
    let mut walked = vec![book.page_start(0).unwrap()];
    while let Some(next) = book.next_page(*walked.last().unwrap()) {
        walked.push(next);
    }
    assert_eq!(walked, pages);

    // Backwards from the end, across chapter boundaries.
    let mut fresh = Book::open(&path, &style, None).unwrap();
    let mut back = vec![fresh.page_start(*pages.last().unwrap()).unwrap()];
    while let Some(prev) = fresh.previous_page(*back.last().unwrap()) {
        back.push(prev);
    }
    back.reverse();
    assert_eq!(back, pages);

    // Any location inside a page resolves to that page's start.
    let mid = pages[5] + (pages[6] - pages[5]) / 2;
    assert_eq!(book.page_start(mid), Some(pages[5]));

    // Index and fraction once a pagination is adopted.
    let mut opened = Book::open(&path, &style, None).unwrap();
    assert_eq!(opened.index_of(pages[3]), None);
    opened.set_pagination(pages.clone());
    assert_eq!(opened.index_of(pages[3]), Some(3));
    assert_eq!(opened.index_of(mid), Some(5));
    assert!((opened.fraction(*pages.last().unwrap()) - 1.0).abs() < 1e-6);

    // Saved positions come back to the same page.
    for &loc in &[pages[0], pages[4], mid, pages[total - 1]] {
        let pos = opened.position_at(loc);
        assert!(matches!(pos, Position::Spine { .. }), "{pos:?}");
        let again = opened.location_of(&pos).unwrap();
        assert_eq!(opened.page_start(again), opened.page_start(loc));
    }
    let half = opened.location_of(&Position::Fraction(0.5)).unwrap();
    assert!(half > pages[total / 4] && half < pages[3 * total / 4]);

    // Rendering at a location draws the header and footer without needing
    // an index.
    let mut chrome = aristotle_render::page::Chrome::load(&style).unwrap();
    let page = book.render_location(pages[2], &mut chrome).unwrap();
    assert_eq!(page.data.len(), style.stride * style.height as usize);
    assert!(page.data.iter().any(|&p| p < 0x80));
}

/// A MOBI 7 file (uncompressed UTF-8 text in 4096-byte records) as the
/// stock Kindle would have it.
fn mobi_file(title: &str, text: &str) -> Vec<u8> {
    let text = text.as_bytes();
    let chunks: Vec<&[u8]> = text.chunks(4096).collect();
    let mut r0 = vec![0u8; 16 + 0xe8];
    r0[0..2].copy_from_slice(&1u16.to_be_bytes()); // no compression
    r0[4..8].copy_from_slice(&(text.len() as u32).to_be_bytes());
    r0[8..10].copy_from_slice(&(chunks.len() as u16).to_be_bytes());
    r0[16..20].copy_from_slice(b"MOBI");
    r0[20..24].copy_from_slice(&0xe8u32.to_be_bytes());
    r0[28..32].copy_from_slice(&65001u32.to_be_bytes());
    r0[36..40].copy_from_slice(&6u32.to_be_bytes());
    let title_at = r0.len() as u32;
    r0[84..88].copy_from_slice(&title_at.to_be_bytes());
    r0[88..92].copy_from_slice(&(title.len() as u32).to_be_bytes());
    r0.extend_from_slice(title.as_bytes());
    let mut records = vec![r0];
    records.extend(chunks.iter().map(|c| c.to_vec()));
    let mut out = vec![0u8; 78 + 8 * records.len() + 2];
    out[60..68].copy_from_slice(b"BOOKMOBI");
    out[76..78].copy_from_slice(&(records.len() as u16).to_be_bytes());
    for (i, r) in records.iter().enumerate() {
        let at = 78 + 8 * i;
        let offset = out.len() as u32;
        out[at..at + 4].copy_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(r);
    }
    out
}

/// A converted MOBI saves stock Kindle positions through its position map.
/// Every page must come back to itself, or each reopen would land a page
/// early (seen on the device with a long MOBI novel).
#[test]
fn kindle_positions_reopen_on_the_same_page() {
    let Some(data) = data_dir() else {
        eprintln!("skipped: no render data directory (run tools/build-render.sh host)");
        return;
    };
    // One long chapter with markup the converter rewrites, like a real book.
    let mut html = String::from("<html><body>");
    for p in 0..400 {
        html.push_str(&format!(
            "<p width=\"0\" align=\"justify\"><font size=\"3\">Akapit {p}. </font>Zażółć \
             <i>gęślą</i> jaźń, a potem <b>jeszcze</b> raz to samo, żeby tekst \
             wypełnił wiele stron.</p>"
        ));
        if p % 7 == 0 {
            html.push_str("<p height=\"1em\" width=\"0\" align=\"center\">* * *</p>");
        }
    }
    html.push_str("</body></html>");
    let dir = std::env::temp_dir().join(format!(
        "aristotle-render-kindle-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lalka.mobi");
    std::fs::write(&path, mobi_file("Lalka", &html)).unwrap();
    aristotle_render::use_data_dir(&data).unwrap();
    let style = Style::pw4();
    let cache = dir.join("cache");

    let mut book = Book::open(&path, &style, Some(&cache)).unwrap();
    assert!(book.mobi().is_some_and(|m| m.posmap.is_some()));
    book.paginate();
    let pages = book.pagination().unwrap().to_vec();
    assert!(pages.len() > 20, "{} pages", pages.len());
    let mut fresh = Book::open(&path, &style, Some(&cache)).unwrap();
    let mut wrong = Vec::new();
    for &start in &pages {
        let pos = book.position_at(start);
        assert!(matches!(pos, Position::Kindle(_)), "{pos:?}");
        let loc = fresh.location_of(&pos).unwrap();
        if fresh.page_start(loc) != Some(start) {
            wrong.push((start, pos, loc));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} pages move: {wrong:?}",
        wrong.len(),
        pages.len()
    );
}

/// Plato lays a spine item out only as far as needed and resumes later
/// (the Plato fork's `core: lazy epub layout`). Placing a location in a fresh copy,
/// turning from there both ways, and paginating everything must agree, also
/// with nested blocks, lists, quotes and a table in one long chapter.
/// (Against a build without it: `aristotle-render pages BOOK 20`.)
#[test]
fn lazy_layout_agrees_with_a_full_walk() {
    let Some(data) = data_dir() else {
        eprintln!("skipped: no render data directory (run tools/build-render.sh host)");
        return;
    };
    let mut html = String::from("<html><body><h1>Jeden rozdział</h1>");
    for s in 0..30 {
        html.push_str(&format!("<h2>Część {s}</h2><div class=\"sekcja\">"));
        for p in 0..8 {
            html.push_str(&format!(
                "<p>Akapit {s}-{p}. Zażółć gęślą jaźń, a potem jeszcze raz to samo, \
                 żeby tekst wypełnił kilka stron i podzielił się na wiersze.</p>"
            ));
        }
        html.push_str("<div><blockquote><p>Cytat w środku, dwa poziomy niżej.</p></blockquote>");
        html.push_str("<ul><li>pierwszy punkt listy</li><li>drugi punkt listy</li></ul></div>");
        if s % 10 == 5 {
            html.push_str(
                "<table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>",
            );
        }
        html.push_str("</div>");
    }
    html.push_str("</body></html>");
    let mobi = Mobi {
        info: MobiInfo {
            encoding: 65001,
            language: Some("pl".into()),
            title: Some("Jeden".into()),
            ..Default::default()
        },
        html,
        images: Vec::new(),
    };
    let dir = std::env::temp_dir().join(format!(
        "aristotle-render-lazy-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("jeden.epub");
    write_epub(&mobi, &path).unwrap();
    aristotle_render::use_data_dir(&data).unwrap();
    let style = Style::pw4();

    let mut full = Book::open(&path, &style, None).unwrap();
    full.paginate();
    let pages = full.pagination().unwrap().to_vec();
    assert!(pages.len() > 30, "{} pages", pages.len());
    for (i, &start) in pages.iter().enumerate().step_by(7) {
        // Mid-page, in a fresh copy, as a reader opening the book.
        let loc = match pages.get(i + 1) {
            Some(&next) => start + (next - start) / 2,
            None => start,
        };
        let mut book = Book::open(&path, &style, None).unwrap();
        assert_eq!(book.page_start(loc), Some(start), "placing {loc}");
        assert_eq!(
            book.next_page(start),
            pages.get(i + 1).copied(),
            "after {start}"
        );
        if i > 0 {
            assert_eq!(
                book.previous_page(start),
                Some(pages[i - 1]),
                "before {start}"
            );
        }
        // Then on to the end, page by page, as a reader turning.
        let mut at = start;
        let mut walked = vec![at];
        while let Some(n) = book.next_page(at) {
            walked.push(n);
            at = n;
        }
        assert_eq!(walked, pages[i..], "walking on from {start}");
    }
}
