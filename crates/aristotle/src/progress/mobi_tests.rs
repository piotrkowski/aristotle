// SPDX-License-Identifier: AGPL-3.0-or-later
//! Imported positions against synthetic single-part and joint containers.

use super::*;
use aristotle_render::style::Style;

fn mobi_file(boundary: Option<u32>) -> Vec<u8> {
    let html = format!(
        "<html><body>{}</body></html>",
        "<p>Original test paragraph.</p>".repeat(80)
    );
    let mut header = vec![0u8; 16 + 0xe8];
    header[0..2].copy_from_slice(&1u16.to_be_bytes());
    header[4..8].copy_from_slice(&(html.len() as u32).to_be_bytes());
    header[8..10].copy_from_slice(&1u16.to_be_bytes());
    header[10..12].copy_from_slice(&4096u16.to_be_bytes());
    header[16..20].copy_from_slice(b"MOBI");
    header[20..24].copy_from_slice(&0xe8u32.to_be_bytes());
    header[28..32].copy_from_slice(&65001u32.to_be_bytes());
    header[36..40].copy_from_slice(&6u32.to_be_bytes());
    if let Some(boundary) = boundary {
        header[128..132].copy_from_slice(&0x40u32.to_be_bytes());
        header.extend_from_slice(b"EXTH");
        for value in [24u32, 1, 121, 12, boundary] {
            header.extend_from_slice(&value.to_be_bytes());
        }
    }
    let mut records = vec![header, html.into_bytes()];
    if boundary == Some(3) {
        // A KF8 record-0 header, not a complete KF8 publication.
        let mut kf8 = vec![0u8; 40];
        kf8[4..8].copy_from_slice(&65_536u32.to_be_bytes());
        kf8[16..20].copy_from_slice(b"MOBI");
        kf8[36..40].copy_from_slice(&8u32.to_be_bytes());
        records.extend([b"BOUNDARY".to_vec(), kf8]);
    }
    let mut out = vec![0u8; 78 + 8 * records.len() + 2];
    out[60..68].copy_from_slice(b"BOOKMOBI");
    out[76..78].copy_from_slice(&(records.len() as u16).to_be_bytes());
    for (index, record) in records.into_iter().enumerate() {
        let entry = 78 + index * 8;
        let offset = out.len() as u32;
        out[entry..entry + 4].copy_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&record);
    }
    out
}

#[test]
fn imported_azw3_positions_require_a_single_mobi7_address_space() {
    let cache = std::env::var_os("ARISTOTLE_CACHE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/aristotle")))
        .unwrap();
    let data = std::env::var_os("ARISTOTLE_RENDER_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| cache.join("render/host/data"));
    if !data.join("fonts").is_dir() {
        assert!(
            std::env::var_os("CI").is_none(),
            "no render data directory in CI"
        );
        eprintln!("skipped: no render data directory (run tools/build-render.sh host)");
        return;
    }
    aristotle_render::use_data_dir(&data).unwrap();
    let dir = crate::testdir("mobi-address-spaces");
    for boundary in [None, Some(u32::MAX), Some(0), Some(3)] {
        let path = dir.join(format!("book-{boundary:?}.azw3"));
        std::fs::write(&path, mobi_file(boundary)).unwrap();
        let book = Book::open(&path, &Style::pw4(), Some(&dir.join("cache"))).unwrap();
        let conversion = book
            .mobi()
            .expect("MOBI 7 content must convert, including .azw3");
        let expected = boundary.filter(|&i| i != u32::MAX);
        assert_eq!(conversion.info.kf8_boundary, expected);
        assert_eq!(stock_positions_compatible(&book), expected.is_none());
        let mut saved = Progress {
            fraction: 0.6,
            locator: Locator::Kindle { position: 0 },
            updated: 1,
            source: None,
            page: None,
        };
        for source in [Some("kindle"), None, Some("future-import")] {
            saved.source = source.map(String::from);
            let candidates = positions(Some(&saved), stock_positions_compatible(&book));
            if expected.is_some() {
                assert_eq!(candidates, vec![Position::Fraction(0.6)]);
                assert_eq!(
                    candidates.iter().find_map(|p| book.location_of(p)),
                    book.location_of(&Position::Fraction(0.6))
                );
            } else {
                assert_eq!(candidates[0], Position::Kindle(0));
            }
        }
        saved.source = Some("aristotle".into());
        assert_eq!(
            positions(Some(&saved), stock_positions_compatible(&book))[0],
            Position::Kindle(0)
        );
        drop(book);
        let warm = Book::open(&path, &Style::pw4(), Some(&dir.join("cache"))).unwrap();
        assert!(!warm.mobi().unwrap().converted_now);
        assert_eq!(stock_positions_compatible(&warm), expected.is_none());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
