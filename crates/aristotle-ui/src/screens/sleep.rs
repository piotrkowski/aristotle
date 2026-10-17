// SPDX-License-Identifier: AGPL-3.0-or-later
//! The screensaver: what the screen shows while nobody reads.
//!
//! E-ink keeps its picture without power, so this is the face of the
//! device most of the time. It shows the book being read as a large card,
//! its title and author, and one quiet line on how to come back: no ads,
//! no "special offers", no clock that would go stale.

use super::library::Book;
use crate::geom::{mm, Insets};
use crate::text::{Align, Paragraph};
use crate::theme::{Space, Styles};
use crate::tree::{Cross, Node, Tree};
use crate::widgets::Cover;
use aristotle_i18n::{tr, Msg};

/// The screensaver for `book` (or a plain one without a book). `photo`
/// shows the book's own cover when it has one.
pub fn build(s: &Styles, book: Option<&Book>, photo: bool) -> Tree {
    let mut t = Tree::new();
    let mut column = Vec::new();
    let top = t.spacer(1);
    column.push(top);
    if let Some(b) = book {
        let cover = Cover::new(&b.title, &b.author, s.family)
            .with_mark(b.mark())
            .with_image(b.cover.clone().filter(|_| photo));
        let cover = t.custom(cover);
        let cover = t.fixed(Some(mm(40.0)), Some(mm(60.0)), Some(cover));
        let g = t.gap(1, Space::XL);
        // The card already names the book; below it, how far along it is.
        let pct = (b.progress * 100.0).floor();
        let line = if b.chapter.is_empty() {
            format!("{pct:.0}%")
        } else {
            format!("{pct:.0}% · {}", b.chapter)
        };
        let mut lp = Paragraph::plain(line, s.small(), Align::Center);
        lp.style.max_lines = Some(1);
        let line = t.text(lp);
        let block = t.add(
            Node::column(Space::XS).cross(Cross::Center),
            [cover, g, line],
        );
        column.push(block);
    } else {
        let l = t.text(Paragraph::plain("Aristotle", s.display(), Align::Center));
        column.push(l);
    }
    let bottom = t.spacer(1);
    let hint = t.text(Paragraph::plain(
        tr(Msg::SleepWakeHint),
        s.caps(),
        Align::Center,
    ));
    column.extend([bottom, hint]);
    let col = t.add(Node::column(0).cross(Cross::Center), column);
    let root = t.pad(Insets::new(mm(10.0), mm(12.0), mm(8.0), mm(12.0)), col);
    t.set_root(root);
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{SCREEN_HEIGHT, SCREEN_WIDTH};
    use crate::screens::library::sample_books;
    use crate::text::{FamilyId, Fonts, TextEngine};
    use crate::ui::Ui;

    #[test]
    fn builds_with_and_without_a_book() {
        let s = Styles::new(FamilyId(0));
        let books = sample_books();
        for b in [Some(&books[0]), None] {
            let mut ui = Ui::new(
                build(&s, b, false),
                TextEngine::new(Fonts::embedded()),
                SCREEN_WIDTH as u32,
                SCREEN_HEIGHT as u32,
            );
            let d = ui.render();
            assert_eq!(d.len(), 1);
            // Something is drawn in the middle of the screen.
            let ink = (0..SCREEN_HEIGHT)
                .filter(|&y| ui.screen().get(SCREEN_WIDTH / 2, y) < 128)
                .count();
            assert!(ink > 10);
        }
    }
}
