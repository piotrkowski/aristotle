// SPDX-License-Identifier: AGPL-3.0-or-later
//! Renders the prototype screens headless and times each step. Runs on the
//! host and, built for armv7-unknown-linux-musleabihf, on the device, where
//! it doubles as the binary-size probe for the toolkit.
//!
//! Usage: `render-bench [RAW_OUT]` writes the last frame as raw 8-bit gray
//! (1072 × 1448) if a path is given.

use aristotle_ui::geom::{SCREEN_HEIGHT, SCREEN_WIDTH};
use aristotle_ui::screens::library::{self, Library};
use aristotle_ui::screens::reader::{Reader, Settings};
use aristotle_ui::text::{FamilyId, Fonts, TextEngine};
use aristotle_ui::theme::Styles;
use aristotle_ui::tree::Refresh;
use aristotle_ui::ui::Ui;
use std::time::Instant;

fn main() {
    let (w, h) = (SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
    let t = Instant::now();
    let mut text = TextEngine::new(Fonts::embedded());
    let styles = Styles::new(FamilyId(0));
    let fam = FamilyId(0);
    let books = library::sample_books();
    let (tree, lib) = Library::build(&mut text, &styles, &books, library::Filter::All);
    let mut ui = Ui::new(tree, text, w, h);
    ui.layout();
    lib.sync(&mut ui.tree);
    ui.render();
    println!(
        "library, cold (fonts, shaping, glyph cache): {:?}",
        t.elapsed()
    );

    let t = Instant::now();
    let (tree, mut reader) = Reader::build(&mut ui.text, &styles, [fam; 3], Settings::default());
    ui.set_tree(tree);
    ui.render();
    println!("reader build and first frame: {:?}", t.elapsed());

    let t = Instant::now();
    reader.turn(&mut ui.tree, true);
    let d = ui.render();
    println!("page turn: {:?} -> {:?}", t.elapsed(), d);

    let t = Instant::now();
    reader.set_menu(&mut ui.tree, true);
    let d = ui.render();
    println!("open menu: {:?} -> {:?}", t.elapsed(), d);

    let t = Instant::now();
    let (tree, _) = Reader::build(
        &mut ui.text,
        &styles,
        [fam; 3],
        Settings {
            size: 4,
            ..Default::default()
        },
    );
    ui.set_tree_with(tree, Refresh::Auto);
    let d = ui.render();
    println!(
        "relayout at a larger size: {:?} -> {} rect(s)",
        t.elapsed(),
        d.len()
    );

    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(&path, ui.screen().packed()).expect("write raw frame");
        println!("wrote {path}");
    }
}
