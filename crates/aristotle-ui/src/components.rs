// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared controls built from the retained tree's standard node kinds.

use crate::geom::Insets;
use crate::theme::{Styles, INK_2, RULE};
use crate::tree::{ActionId, Feedback, Kind, Node, NodeId, Tree};

/// Horizontal padding inside a chip, in reference pixels.
pub const CHIP_PADDING_X: i32 = 26;

/// Builds an interactive chip with persistent selection and press inversion.
/// Paint size follows the label; `Tree::hit` expands small targets to 9 mm.
/// Leave room for the expanded target around the chip, not inside its capsule.
pub fn chip(t: &mut Tree, s: &Styles, label: &str, selected: bool, id: ActionId) -> NodeId {
    let style = if selected {
        s.ui_bold()
    } else {
        s.ui().gray(INK_2)
    };
    let l = t.label(label, style);
    t.add(
        Node::new(Kind::Padding(Insets::vh(10, CHIP_PADDING_X)))
            .border(if selected { 0 } else { 2 }, RULE, 40)
            .inverted(selected)
            .feedback(Feedback::Invert)
            .action(id),
        [l],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{Point, MIN_TARGET};
    use crate::input::{Phase, TouchEvent};
    use crate::text::{FamilyId, Fonts, TextEngine};
    use crate::theme::Space;
    use crate::tree::Hit;
    use crate::ui::{Ui, UiEvent};
    use crate::waveform::Waveform;

    fn shown(label: &str, selected: bool, width: u32) -> (Ui, NodeId, NodeId) {
        let mut t = Tree::new();
        let s = Styles::new(FamilyId(0));
        let control = chip(&mut t, &s, label, selected, 42);
        let row = t.add(Node::row(0), [control]);
        let passive = t.label("Status", s.ui());
        let column = t.add(Node::column(Space::L), [row, passive]);
        let root = t.pad(Insets::all(Space::L), column);
        t.set_root(root);
        let mut ui = Ui::new(t, TextEngine::new(Fonts::embedded()), width, 600);
        ui.layout();
        ui.render();
        (ui, control, passive)
    }

    fn touch(ui: &mut Ui, phase: Phase, p: Point, ms: u64) -> Vec<UiEvent> {
        ui.handle_touch(TouchEvent {
            phase,
            id: 0,
            x: p.x,
            y: p.y,
            time_us: ms * 1000,
        })
    }

    #[test]
    fn selected_and_unselected_chips_restore_after_tap_and_cancel_in_both_themes() {
        for selected in [false, true] {
            for night in [false, true] {
                let (mut ui, id, _) = shown("Reading", selected, 500);
                ui.set_night(night);
                ui.render();
                let baseline = ui.screen().pixels.clone();
                let p = ui.tree.rect(id).center();
                assert!(touch(&mut ui, Phase::Down, p, 0).is_empty());
                let damage = ui.render();
                assert_eq!(damage.len(), 1);
                assert_eq!(damage[0].waveform, Waveform::A2);
                assert_ne!(ui.screen().pixels, baseline);
                let events = touch(&mut ui, Phase::Up, p, 80);
                assert!(matches!(events[..], [UiEvent::Action { id: 42, .. }]));
                ui.render();
                assert_eq!(ui.screen().pixels, baseline);

                touch(&mut ui, Phase::Down, p, 1000);
                ui.render();
                let moved = Point {
                    x: p.x + 100,
                    y: p.y + 100,
                };
                let mut events = touch(&mut ui, Phase::Move, moved, 1050);
                events.extend(touch(&mut ui, Phase::Up, moved, 1100));
                assert!(!events.iter().any(|e| matches!(e, UiEvent::Action { .. })));
                ui.render();
                assert_eq!(ui.screen().pixels, baseline);
            }
        }
    }

    #[test]
    fn expanded_targets_work_without_making_passive_labels_interactive() {
        let (mut ui, id, passive) = shown("All", false, 500);
        let painted = ui.tree.rect(id);
        let expanded = painted.expand_to(MIN_TARGET);
        let p = Point {
            x: painted.center().x,
            y: expanded.y + 1,
        };
        assert!(!painted.contains(p));
        assert!(matches!(ui.tree.hit(p), Hit::Target(n) if n == id));
        touch(&mut ui, Phase::Down, p, 0);
        let events = touch(&mut ui, Phase::Up, p, 80);
        assert!(matches!(events[..], [UiEvent::Action { id: 42, .. }]));
        let p = ui.tree.rect(passive).center();
        assert!(matches!(ui.tree.hit(p), Hit::Nothing));
        touch(&mut ui, Phase::Down, p, 1000);
        let events = touch(&mut ui, Phase::Up, p, 1080);
        assert!(!events.iter().any(|e| matches!(e, UiEvent::Action { .. })));
    }

    #[test]
    fn translated_labels_wrap_within_their_available_width() {
        for code in ["en", "pl"] {
            aristotle_i18n::with_language(code, || {
                for selected in [false, true] {
                    let (short, short_id, _) = shown("All", selected, 240);
                    let label = aristotle_i18n::tr(aristotle_i18n::Msg::SettingsReturnKindle);
                    let (ui, id, passive) = shown(label, selected, 240);
                    let r = ui.tree.rect(id);
                    assert!(r.h > short.tree.rect(short_id).h);
                    assert!(r.x >= Space::L && r.right() <= 240 - Space::L);
                    assert!(r.bottom() < ui.tree.rect(passive).y);
                    assert!(matches!(ui.tree.hit(r.center()), Hit::Target(n) if n == id));
                }
            });
        }
    }
}
