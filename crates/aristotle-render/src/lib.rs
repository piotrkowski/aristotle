//! Aristotle's e-book page renderer for the Kindle Paperwhite 4.
//!
//! It drives Plato's document engine (`plato-core`, AGPL-3.0, by Bastien
//! Dejean; <https://github.com/baskerville/plato>) without any of Plato's
//! Kobo UI: Plato's HTML engine lays out EPUB with our font, justification
//! and Liang hyphenation, and MuPDF (through Plato) handles PDF, MOBI and the
//! other formats. This crate adds what a reader around it needs:
//!
//! - [`style::Style`]: page geometry and Polish-friendly typesetting defaults;
//! - [`book::Book`]: opening, pagination, chapter titles, page rendering;
//! - [`page::Page`]: full-screen 8-bit buffers with the framebuffer's stride,
//!   the header and footer ([`page::Chrome`]) and the black and white
//!   version used for fast page turns;
//! - [`screen::Screen`]: the measured page-turn strategy (A2, then GL16).
//! - [`notes`], [`richtext`]: footnotes behind links, and small HTML
//!   fragments as styled paragraphs for popups.
//!
//! # Runtime data
//!
//! Plato opens its fonts (`fonts/`), stylesheets (`css/`) and hyphenation
//! patterns (`hyphenation-patterns/`) by paths relative to the current
//! directory. [`use_data_dir`] changes into a directory laid out like that;
//! `tools/build-render.sh` assembles it next to the binary as `data/`.

pub mod book;
pub mod lang;
pub mod mobi;
pub mod notes;
pub mod page;
pub mod position;
pub mod richtext;
pub mod screen;
pub mod style;

/// Plato's core, for callers that need its types (errors, locations)
/// without depending on the patched source tree themselves.
pub use plato_core;

use std::io;
use std::path::Path;

/// Make `dir` the current directory, so Plato finds its runtime data. Turn
/// any relative book path into an absolute one before calling this.
pub fn use_data_dir(dir: &Path) -> io::Result<()> {
    for needed in ["fonts", "css", "hyphenation-patterns"] {
        if !dir.join(needed).is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} has no {needed}/ (see docs/reader.md)", dir.display()),
            ));
        }
    }
    std::env::set_current_dir(dir)
}
