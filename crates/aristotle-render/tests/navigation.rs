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
