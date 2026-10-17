// SPDX-License-Identifier: AGPL-3.0-or-later
//! The transition policy (`docs/ui-framework.md`, "Transitions"), checked
//! on the host: for every user action, how many updates `Ui::render` hands
//! to the panel and with which waveform.
//!
//! - A screen switch is exactly one full-screen GC16 (the device sends it
//!   as one full mode-8 flash).
//! - Touch-down is exactly one A2 update inside the touched control.
//! - An in-screen change is exactly one non-flashing update (DU or GL16)
//!   that is not the whole screen.
//! - The night switch is exactly one full-screen GC16.
//!
//! On the device, `tools/scenarios/aristotle-ui.txt` checks the
//! same with the EPDC driver's own log.

use aristotle_ui::geom::{Point, Rect, SCREEN_HEIGHT, SCREEN_WIDTH};
use aristotle_ui::input::{Phase, TouchEvent};
use aristotle_ui::screens::library::{
    sample_books, sample_items, CoverMode, Filter, Item, Library, Place, Shelf, StatusLine, View,
};
use aristotle_ui::screens::reader::{Reader, Settings as ReaderSettings};
use aristotle_ui::screens::settings::{self as set_ui, SystemView, Usb};
use aristotle_ui::screens::sleep;
use aristotle_ui::text::{FamilyId, Fonts, TextEngine};
use aristotle_ui::theme::Styles;
use aristotle_ui::tree::{Hit, Refresh, Tree};
use aristotle_ui::ui::{Damage, Ui, UiEvent};
use aristotle_ui::Waveform;

const STATUS: StatusLine<'static> = StatusLine {
    clock: "21:38",
    battery: Some(0.8),
    charging: false,
};

fn styles() -> Styles {
    Styles::new(FamilyId(0))
}

fn library_tree(
    text: &mut TextEngine,
    items: &[Item],
    view: View,
    filter: Filter,
) -> (Tree, Library) {
    let books = sample_books();
    let shelf = Shelf {
        place: Place::Home { view, filter },
        hero: Some(&books[0]),
        items,
        sort: "Ostatnie",
        status: STATUS,
        covers: CoverMode::None,
    };
    let (mut t, lib) = Library::build_shelf(text, &styles(), &shelf);
    // Layout happens in the Ui; sync after it.
    let _ = &mut t;
    (t, lib)
}

fn settings_tree(text: &mut TextEngine, night: bool, light: u32) -> (Tree, set_ui::Settings) {
    let about = set_ui::sample_about();
    let v = SystemView {
        status: STATUS,
        light: Some((light, 24)),
        night,
        covers: CoverMode::None,
        sleep_minutes: Some(15),
        clippings: true,
        battery: None,
        storage: Some((1_000_000_000, 6_000_000_000)),
        date: "sobota, 17 października 2026",
        time: "21:38",
        usb: Some(Usb::Network),
        about: &about,
    };
    set_ui::Settings::build(text, &styles(), &v)
}

fn screen() -> Rect {
    Rect::new(0, 0, SCREEN_WIDTH, SCREEN_HEIGHT)
}

fn new_ui(tree: Tree) -> Ui {
    let mut ui = Ui::new(
        tree,
        TextEngine::new(Fonts::embedded()),
        SCREEN_WIDTH as u32,
        SCREEN_HEIGHT as u32,
    );
    ui.render();
    ui
}

fn one_flash(d: &[Damage]) {
    assert_eq!(
        d,
        [Damage {
            rect: screen(),
            waveform: Waveform::GC16
        }],
        "expected exactly one full flash"
    );
}

fn one_quiet(d: &[Damage]) -> Damage {
    assert_eq!(d.len(), 1, "expected exactly one update: {d:?}");
    let u = d[0];
    assert!(
        matches!(u.waveform, Waveform::DU | Waveform::GL16),
        "expected a non-flashing update: {u:?}"
    );
    assert!(u.rect != screen() || u.waveform != Waveform::GC16);
    u
}

fn press(ui: &mut Ui, p: Point) -> Vec<Damage> {
    let ev = ui.handle_touch(TouchEvent {
        phase: Phase::Down,
        id: 0,
        x: p.x,
        y: p.y,
        time_us: 0,
    });
    assert!(ev.is_empty());
    ui.render()
}

fn release(ui: &mut Ui, p: Point) -> Vec<UiEvent> {
    ui.handle_touch(TouchEvent {
        phase: Phase::Up,
        id: 0,
        x: p.x,
        y: p.y,
        time_us: 60_000,
    })
}

/// The centre of the node with action `id`.
fn find(ui: &Ui, id: u32) -> Point {
    for i in 0..ui.tree.len() {
        let n = aristotle_ui::tree::NodeId(i as u32);
        if ui.tree.node(n).action == Some(id) && !ui.tree.rect(n).is_empty() {
            return ui.tree.rect(n).center();
        }
    }
    panic!("no visible node with action {id}");
}

#[test]
fn screen_switches_are_one_flash() {
    let mut text = TextEngine::new(Fonts::embedded());
    let items = sample_items(&sample_books()[1..], View::Books);
    let (tree, _) = library_tree(&mut text, &items, View::Books, Filter::All);
    let mut ui = new_ui(tree);
    // Library -> settings -> library -> screensaver: one flash each.
    let (tree, _) = settings_tree(&mut ui.text, false, 2);
    ui.set_tree(tree);
    one_flash(&ui.render());
    let (tree, _) = library_tree(&mut ui.text, &items, View::Books, Filter::All);
    ui.set_tree(tree);
    one_flash(&ui.render());
    ui.set_tree(sleep::build(&styles(), Some(&sample_books()[0]), false));
    one_flash(&ui.render());
    // Nothing follows a switch: no late redraw.
    assert!(ui.render().is_empty());
}

#[test]
fn touch_down_is_one_fast_update_inside_the_control() {
    let mut text = TextEngine::new(Fonts::embedded());
    let items = sample_items(&sample_books()[1..], View::Books);
    let (tree, _) = library_tree(&mut text, &items, View::Books, Filter::All);
    let mut ui = new_ui(tree);
    let p = find(&ui, aristotle_ui::screens::library::action::FILTER + 1);
    let d = press(&mut ui, p);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].waveform, Waveform::A2);
    assert!(d[0].rect.contains(p));
    assert!(
        d[0].rect.area() < 300 * 200,
        "only the chip: {:?}",
        d[0].rect
    );
}

#[test]
fn in_screen_changes_are_one_quiet_update() {
    use aristotle_ui::screens::library::action as a;
    let mut text = TextEngine::new(Fonts::embedded());
    let books = sample_books();
    let items = sample_items(&books[1..], View::Books);
    let (tree, lib) = library_tree(&mut text, &items, View::Books, Filter::All);
    let mut ui = new_ui(tree);
    ui.layout();
    lib.sync(&mut ui.tree);
    ui.render();

    // Shelf chip: press, release, the app rebuilds with only the changed
    // pixels. One A2, then one quiet update.
    let p = find(&ui, a::FILTER + 1);
    press(&mut ui, p);
    assert!(matches!(release(&mut ui, p)[..], [UiEvent::Action { id, .. }] if id == a::FILTER + 1));
    let (tree, lib) = library_tree(&mut ui.text, &items, View::Books, Filter::Reading);
    ui.set_tree_with(tree, Refresh::Auto);
    ui.layout();
    lib.sync(&mut ui.tree);
    one_quiet(&ui.render());

    // View tab: the same.
    let folders = sample_items(&books, View::Folders);
    let (tree, lib) = library_tree(&mut ui.text, &folders, View::Folders, Filter::All);
    ui.set_tree_with(tree, Refresh::Auto);
    ui.layout();
    lib.sync(&mut ui.tree);
    one_quiet(&ui.render());

    // List page: the list and the pager in one update.
    let (tree, mut lib) = library_tree(&mut ui.text, &items, View::Books, Filter::All);
    ui.set_tree_with(tree, Refresh::Auto);
    ui.layout();
    lib.sync(&mut ui.tree);
    ui.render();
    assert!(lib.handle(&mut ui.tree, a::NEXT_PAGE));
    ui.layout();
    lib.sync(&mut ui.tree);
    let u = one_quiet(&ui.render());
    assert!(u.rect.y > 300, "the header stays: {u:?}");

    // The busy mark: one small black-and-white update in the status line.
    ui.tree.set_hidden(lib.busy, false);
    let d = ui.render();
    let u = one_quiet(&d);
    assert_eq!(u.waveform, Waveform::DU);
    assert!(u.rect.bottom() <= 80 && u.rect.area() < 100 * 40, "{u:?}");
}

#[test]
fn settings_controls_are_one_quiet_update() {
    let mut text = TextEngine::new(Fonts::embedded());
    let (tree, ids) = settings_tree(&mut text, false, 2);
    let mut ui = new_ui(tree);
    ui.layout();
    ids.sync(&mut ui.tree);
    ui.render();
    // A light step: slider, value and preset chips change, in one update.
    let (tree, ids) = settings_tree(&mut ui.text, false, 10);
    ui.set_tree_with(tree, Refresh::Auto);
    ui.layout();
    ids.sync(&mut ui.tree);
    let u = one_quiet(&ui.render());
    assert!(u.rect.bottom() < SCREEN_HEIGHT / 2, "{u:?}");
    // The night switch: exactly one flash into the inverted state, and the
    // screen is inverted.
    let (tree, ids) = settings_tree(&mut ui.text, true, 10);
    ui.set_night(true);
    ui.set_tree_with(tree, Refresh::Auto);
    ui.layout();
    ids.sync(&mut ui.tree);
    one_flash(&ui.render());
    assert_eq!(ui.screen().get(SCREEN_WIDTH - 2, SCREEN_HEIGHT - 2), 0);
    assert!(ui.render().is_empty());
}

#[test]
fn menu_opens_and_closes_in_one_update_each() {
    let mut text = TextEngine::new(Fonts::embedded());
    let fam = FamilyId(0);
    let (tree, mut r) = Reader::build(&mut text, &styles(), [fam; 3], ReaderSettings::default());
    let mut ui = new_ui(tree);
    r.set_menu(&mut ui.tree, true);
    // One update, no flash (GL16: the bars carry anti-aliased labels).
    one_quiet(&ui.render());
    // The page between the bars is not redrawn (Coalesce::Single spans it,
    // but a partial update leaves unchanged pixels alone).
    let minimal = ui.minimal_damage().to_vec();
    assert_eq!(minimal.len(), 2, "top bar and sheet: {minimal:?}");
    // The night toggle inside the menu: one flash.
    let toggle = ui.tree.rect(r.night_toggle).center();
    match ui.tree.hit(toggle) {
        Hit::Target(_) => {}
        h => panic!("toggle not a target: {h:?}"),
    }
    ui.set_night(true);
    one_flash(&ui.render());
    r.set_menu(&mut ui.tree, false);
    let d = ui.render();
    assert_eq!(d.len(), 1, "{d:?}");
}
