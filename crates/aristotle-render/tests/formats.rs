// SPDX-License-Identifier: AGPL-3.0-or-later
//! Default-bundle format checks with generated content.

use std::io::Write;
use std::path::PathBuf;

use aristotle_library::format::{sniff, Format};
use aristotle_render::book::{Book, Engine, Position};
use aristotle_render::style::Style;
use plato_core::png::{BitDepth, ColorType, Encoder};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn png(gray: u8) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = Encoder::new(&mut out, 12, 16);
        encoder.set_color(ColorType::Grayscale);
        encoder.set_depth(BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[gray; 12 * 16])
            .unwrap();
    }
    out
}

#[test]
fn default_bundle_renders_cbz_pages_in_natural_order() {
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
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "aristotle-formats-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    std::fs::create_dir_all(&scratch.0).unwrap();
    let path = scratch.0.join("generated.cbz");
    let mut archive = ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, gray) in [("page20.png", 224), ("page10.png", 128), ("page2.png", 32)] {
        archive.start_file(name, options).unwrap();
        archive.write_all(&png(gray)).unwrap();
    }
    archive.finish().unwrap();

    for filename in ["generated.cbz", "generated.zip"] {
        let renamed = scratch.0.join(filename);
        if renamed != path {
            std::fs::copy(&path, &renamed).unwrap();
        }
        assert_eq!(sniff(&renamed).unwrap(), Some(Format::Cbz));
        let metadata =
            aristotle_library::metadata::extract(&renamed, Format::Cbz, None, true).unwrap();
        assert_eq!(metadata.meta.pages, Some(3));
        assert_eq!(metadata.cover, Some(png(32)));
        let mut book = Book::open(&renamed, &Style::pw4(), None).unwrap();
        assert_eq!(book.engine(), Engine::Fixed);
        assert_eq!(book.known_page_count(), Some(3));
        for (page, expected) in [32u8, 128, 224].into_iter().enumerate() {
            let location = book.locate(page).unwrap();
            assert_eq!(book.position_at(location), Position::Page(page));
            assert_eq!(book.location_of(&Position::Page(page)), Some(location));
            let pixmap = book.render_body(location).unwrap();
            let center = (pixmap.height / 2 * pixmap.width + pixmap.width / 2) as usize;
            assert!(
                pixmap.data[center].abs_diff(expected) <= 1,
                "{filename}: page {page}"
            );
        }
        assert_eq!(book.next_page(2), None);
        assert_eq!(book.previous_page(1), Some(0));
    }
}
