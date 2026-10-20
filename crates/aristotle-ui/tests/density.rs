// SPDX-License-Identifier: AGPL-3.0-or-later

use aristotle_ui::geom::{min_target, Density, Point, Screen};
use aristotle_ui::input::{Gesture, GestureConfig, GestureRecognizer, Phase, TouchEvent};
use aristotle_ui::text::{FamilyId, Fonts, TextEngine};
use aristotle_ui::theme::Styles;
use aristotle_ui::tree::{Hit, Node, Tree};
use aristotle_ui::ui::Ui;
use aristotle_ui::widgets::Toggle;

#[test]
fn native_text_measured_boxes_and_touch_targets_share_one_density() {
    for (w, h, dpi) in [
        (600, 800, 167),
        (758, 1024, 212),
        (1072, 1448, 300),
        (1440, 1920, 400),
    ] {
        let screen = Screen::new(w, h, dpi).unwrap();
        let styles = Styles::new(FamilyId(0)).with_density(screen.density);
        let mut text = TextEngine::new(Fonts::embedded());
        let measured = text.measure("Density", &styles.ui()).ceil() as i32;
        let mut tree = Tree::new();
        let label = tree.label("Density", styles.ui());
        let native = tree.fixed_pixels(Some(measured), None, Some(label));
        let button = tree.button(Node::plain(), native, 1);
        let toggle = tree.custom(Toggle { on: false });
        let root = tree.add(Node::column(16), [button, toggle]);
        tree.set_root(root);
        let mut ui = Ui::with_screen(tree, text, screen);
        ui.render();
        assert_eq!(ui.tree.rect(native).w, measured);
        assert_eq!(ui.tree.rect(button).h, min_target(screen.density));
        assert_eq!(ui.tree.rect(toggle).h, screen.density.px(58));
        assert_eq!(
            ui.tree.hit(ui.tree.rect(button).center()),
            Hit::Target(button)
        );
        assert!((styles.ui().size * 72.0 / dpi as f32 - 8.5).abs() < 0.001);
        let native_style = styles.ui();
        assert_eq!(
            native_style.clone().at_density(screen.density),
            native_style
        );

        let mut tree = Tree::new();
        let label = tree.label("Again", Styles::new(FamilyId(0)).ui());
        let button = tree.button(Node::plain(), label, 2);
        let root = tree.add(Node::column(0), [button]);
        tree.set_root(root);
        ui.set_tree(tree);
        ui.render();
        assert_eq!(ui.tree.rect(button).h, min_target(screen.density));
    }
}

#[test]
fn gesture_thresholds_follow_millimetres_not_pixel_counts() {
    for dpi in [167, 212, 300] {
        let density = Density::new(dpi).unwrap();
        for (travel_mm, swipe) in [(3.0, false), (9.0, true)] {
            let mut g = GestureRecognizer::new(GestureConfig::for_density(density));
            let start = Point {
                x: density.mm(20.0),
                y: density.mm(20.0),
            };
            let event = |phase, x, time_us| TouchEvent {
                phase,
                id: 0,
                x,
                y: start.y,
                time_us,
            };
            g.feed(event(Phase::Down, start.x, 0));
            g.feed(event(Phase::Move, start.x + density.mm(travel_mm), 30_000));
            let events = g.feed(event(Phase::Up, start.x + density.mm(travel_mm), 60_000));
            assert_eq!(
                events.iter().any(|e| matches!(e, Gesture::Swipe { .. })),
                swipe
            );
            assert_eq!(events.iter().any(|e| matches!(e, Gesture::Tap(_))), !swipe);
        }
    }
}
