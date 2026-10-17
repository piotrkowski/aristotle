// SPDX-License-Identifier: AGPL-3.0-or-later
//! The screens: library and shelves, settings, the screensaver, and the
//! reader page and menu.
//!
//! These are real trees built from the toolkit. The reader app feeds them
//! from the library index and the device; the desktop preview and the
//! tests feed them sample data.

pub mod library;
pub mod reader;
pub mod settings;
pub mod sleep;
pub mod text_input;
pub mod wifi;

use crate::geom::Insets;
use crate::icons::Icon;
use crate::text::TextStyle;
use crate::theme::INK;
use crate::tree::{ActionId, Kind, Node, NodeId, Tree};

/// An icon in a 9 mm square button that inverts while pressed.
pub fn icon_button(t: &mut Tree, icon: Icon, size: i32, action: ActionId) -> NodeId {
    let i = t.icon(icon, size, INK);
    t.button(Node::plain(), i, action)
}

/// A text button padded to at least 9 mm in both directions.
pub fn text_button(t: &mut Tree, label: &str, style: TextStyle, action: ActionId) -> NodeId {
    let l = t.label(label, style);
    let p = t.pad(Insets::vh(0, 24), l);
    t.button(Node::plain(), p, action)
}

/// A segmented control: equal cells in a rounded frame, the selected one
/// shown as an inverted pill. Cell `i` sends `first_action + i`.
pub fn segmented(
    t: &mut Tree,
    cells: Vec<NodeId>,
    selected: usize,
    first_action: ActionId,
) -> NodeId {
    let mut row = Vec::new();
    for (i, content) in cells.into_iter().enumerate() {
        let pill = t.add(
            Node::new(Kind::Align { x: 0.5, y: 0.5 })
                .border(0, INK, 12)
                .inverted(i == selected),
            [content],
        );
        let cell = t.add(
            Node::new(Kind::Padding(Insets::all(7)))
                .border(0, INK, 14)
                .action(first_action + i as ActionId),
            [pill],
        );
        row.push(
            t.add(
                Node::new(Kind::Fixed {
                    w: None,
                    h: Some(crate::geom::MIN_TARGET),
                })
                .flex(1),
                [cell],
            ),
        );
    }
    t.add(Node::row(0).border(3, INK, 18), row)
}
