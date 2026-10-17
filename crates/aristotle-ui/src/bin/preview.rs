// SPDX-License-Identifier: AGPL-3.0-or-later
//! Desktop preview: renders every screen to PNG, in day and night mode.
//!
//! Every screen goes through the same `Ui` the device runs, including
//! simulated touches, so the damage overlays show the real update
//! rectangles and waveforms. Images are quantized to the 16 gray levels the
//! panel can show. Night images are `Ui::set_night`, the same inversion the
//! device applies.
//!
//! Usage: `aristotle-ui-preview [--lang CODE] [OUT_DIR]`, default
//! `ui-preview/v2` in the cache directory, `$ARISTOTLE_CACHE` (as
//! `tools/env.sh` sets it). `--lang` picks the UI language (`en`, the
//! default, or any embedded locale such as `pl`); `--lang all` renders every
//! language, each into `OUT_DIR/<code>`.
//! `ARISTOTLE_STOCK_HOME=PNG` adds `compare-library.png`, the stock home
//! screen next to ours.

use aristotle_i18n::{fmt_date, tr, Msg};
use aristotle_ui::canvas::{Canvas, Painter};
use aristotle_ui::geom::{Point, Rect, SCREEN_HEIGHT, SCREEN_WIDTH};
use aristotle_ui::icons::{self, Icon};
use aristotle_ui::input::{Phase, TouchEvent};
use aristotle_ui::keyboard::{self, Key, Layer};
use aristotle_ui::screens::library::{
    self, sample_items, CoverMode, Filter, Item, Library, Place, Shelf, StatusLine, View,
};
use aristotle_ui::screens::reader::{self, Reader, Settings, Zone};
use aristotle_ui::screens::settings::{self as settings_screen, BatteryInfo, SystemView, Usb};
use aristotle_ui::screens::sleep;
use aristotle_ui::screens::text_input::{self, TextInput};
use aristotle_ui::screens::wifi::{self, ListState, WifiScreen, WifiView};
use aristotle_ui::text::{
    layout_paragraph, Align, FamilyId, Fonts, NoHyphenation, Paragraph, ParagraphLayout,
    TextEngine, TextStyle,
};
use aristotle_ui::theme::Styles;
use aristotle_ui::tree::{Hit, Node, Tree};
use aristotle_ui::ui::{Damage, Ui, UiEvent};
use aristotle_ui::widgets::{dashed_rect, Cover, CoverMark};
use std::path::{Path, PathBuf};

const W: u32 = SCREEN_WIDTH as u32;
const H: u32 = SCREEN_HEIGHT as u32;

fn save(c: &Canvas, path: &Path) {
    let mut q = c.clone();
    q.quantize16();
    let f = std::fs::File::create(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), q.width, q.height);
    e.set_color(png::ColorType::Grayscale);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()
        .and_then(|mut w| w.write_image_data(&q.packed()))
        .expect("png encoding");
    println!("wrote {}", path.display());
}

/// The screen as it is, then the same screen in night mode.
fn day_night(ui: &mut Ui, out: &Path, name: &str) {
    ui.render();
    save(ui.screen(), &out.join(format!("{name}.png")));
    ui.set_night(true);
    ui.render();
    save(ui.screen(), &out.join(format!("{name}-night.png")));
    ui.set_night(false);
    ui.render();
}

fn invert(c: &Canvas) -> Canvas {
    let mut n = c.clone();
    for p in n.pixels.iter_mut() {
        *p = 255 - *p;
    }
    n
}

fn load_gray(path: &Path) -> Option<Canvas> {
    let d = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let ch = info.color_type.samples();
    let mut c = Canvas::new(info.width, info.height);
    for (i, px) in c.pixels.iter_mut().enumerate() {
        *px = buf[i * ch];
    }
    Some(c)
}

fn touch(ui: &mut Ui, phase: Phase, x: i32, y: i32, ms: u64) -> Vec<UiEvent> {
    ui.handle_touch(TouchEvent {
        phase,
        id: 0,
        x,
        y,
        time_us: ms * 1000,
    })
}

fn tap(ui: &mut Ui, p: Point, ms: u64) -> Vec<UiEvent> {
    let mut ev = touch(ui, Phase::Down, p.x, p.y, ms);
    ev.extend(touch(ui, Phase::Up, p.x, p.y, ms + 60));
    ev
}

fn set(text: &mut TextEngine, s: &str, style: TextStyle, width: f32) -> ParagraphLayout {
    let para = Paragraph::plain(s, style, Align::Left);
    layout_paragraph(text, &para, width, &NoHyphenation)
}

/// The screen with each minimal damage rectangle of the last render
/// outlined (large ones labelled), the coalesced update described in the
/// caption.
fn damage_overlay(ui: &mut Ui, sent: &[Damage], caption: &str) -> Canvas {
    let damage = ui.minimal_damage().to_vec();
    let caption = format!(
        "{caption}. Sent as: {}.",
        sent.iter()
            .map(|d| format!("{:?} {}×{}", d.waveform, d.rect.w, d.rect.h))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut c = ui.screen().clone();
    let all = Rect::new(0, 0, W as i32, H as i32);
    let label = TextStyle::new(FamilyId(0), 30.0).bold().gray(255);
    for d in &damage {
        let mut p = Painter::new(&mut c, all);
        dashed_rect(&mut p, d.rect.outset(3), 4, 14, 0);
        if d.rect.area() < 20_000 {
            continue;
        }
        let text = format!("{:?} {}×{}", d.waveform, d.rect.w, d.rect.h);
        let l = set(&mut ui.text, &text, label.clone(), 600.0);
        let tag = Rect::new(d.rect.x, d.rect.y, l.width as i32 + 24, l.height as i32 + 8);
        p.fill_rect(tag, 0);
        l.draw(
            &mut ui.text,
            &mut p,
            tag.x as f32 + 12.0,
            tag.y as f32 + 4.0,
        );
    }
    let style = TextStyle::new(FamilyId(0), 34.0).italic();
    let l = set(&mut ui.text, &caption, style, W as f32 - 80.0);
    let mut p = Painter::new(&mut c, all);
    let band = Rect::new(
        0,
        H as i32 - l.height as i32 - 24,
        W as i32,
        l.height as i32 + 24,
    );
    p.fill_rect(band, 235);
    l.draw(&mut ui.text, &mut p, 40.0, band.y as f32 + 12.0);
    c
}

fn log(step: &str, ui: &Ui, damage: &[Damage]) {
    println!(
        "{step}: {} minimal rect(s), sent:",
        ui.minimal_damage().len()
    );
    if damage.is_empty() {
        println!("  (no damage)");
    }
    for d in damage {
        println!(
            "  {:<5} {:>4}×{:<4} at ({}, {})",
            format!("{:?}", d.waveform),
            d.rect.w,
            d.rect.h,
            d.rect.x,
            d.rect.y
        );
    }
}

fn engine(fonts_dir: &Path) -> (TextEngine, [FamilyId; 3]) {
    let mut fonts = Fonts::embedded();
    let lit = FamilyId(0);
    let charis = fonts
        .load_family("Charis", fonts_dir, "Charis")
        .unwrap_or(lit);
    let gentium = fonts
        .load_family("Gentium", fonts_dir, "GentiumBook")
        .unwrap_or(lit);
    (TextEngine::new(fonts), [lit, charis, gentium])
}

const STATUS: StatusLine<'static> = StatusLine {
    clock: "21:38",
    battery: Some(0.82),
    charging: false,
};

/// A library screen in a fresh `Ui`, laid out and synced.
fn shelf_ui(shelf: &Shelf) -> (Ui, Library) {
    let mut text = TextEngine::new(Fonts::embedded());
    let styles = Styles::new(FamilyId(0));
    let (tree, lib) = Library::build_shelf(&mut text, &styles, shelf);
    let mut ui = Ui::new(tree, text, W, H);
    ui.layout();
    lib.sync(&mut ui.tree);
    (ui, lib)
}

fn library_screens(out: &Path) {
    let books = library::sample_books();
    let hero = &books[0];
    let rest = &books[1..];
    let home = |view, filter, covers, items: &[Item]| -> (Ui, Library) {
        shelf_ui(&Shelf {
            place: Place::Home { view, filter },
            hero: Some(hero),
            items,
            sort: tr(Msg::LibrarySortRecent),
            status: STATUS,
            covers,
        })
    };

    let all = sample_items(rest, View::Books);
    let (mut ui, mut lib) = home(View::Books, Filter::All, CoverMode::None, &all);
    {
        let d = ui.render();
        log("library: first frame", &ui, &d)
    };
    day_night(&mut ui, out, "library");

    // Finger down on the second row: an instant A2 outline.
    let c = ui.tree.rect(lib.rows[1]).center();
    touch(&mut ui, Phase::Down, c.x, c.y, 0);
    let d = ui.render();
    log("library: press on a book row", &ui, &d);
    save(ui.screen(), &out.join("library-pressed.png"));
    let o = damage_overlay(
        &mut ui,
        &d,
        "Touch-down on a row: only the outline goes out, with A2",
    );
    save(&o, &out.join("library-pressed-damage.png"));
    touch(&mut ui, Phase::Move, c.x + 200, c.y, 40);
    touch(&mut ui, Phase::Up, c.x + 200, c.y, 80);
    ui.render();

    // Next page of the list.
    let next = ui.tree.rect(lib.next).center();
    for ev in tap(&mut ui, next, 1000) {
        if let UiEvent::Action { id, .. } = ev {
            lib.handle(&mut ui.tree, id);
        }
    }
    ui.layout();
    lib.sync(&mut ui.tree);
    let d = ui.render();
    log("library: page 2", &ui, &d);
    day_night(&mut ui, out, "library-page2");
    let o = damage_overlay(&mut ui, &d, "List page turn: the list and the pager only");
    save(&o, &out.join("library-page2-damage.png"));

    let (mut ui, _) = home(View::Books, Filter::All, CoverMode::Cards, &all);
    day_night(&mut ui, out, "library-cards");
    let (mut ui, _) = home(View::Books, Filter::Reading, CoverMode::None, &all);
    day_night(&mut ui, out, "library-reading");
    let (mut ui, _) = home(View::Books, Filter::New, CoverMode::Cards, &all);
    day_night(&mut ui, out, "library-new-cards");
    let folders = sample_items(&books, View::Folders);
    let (mut ui, _) = home(View::Folders, Filter::All, CoverMode::None, &folders);
    day_night(&mut ui, out, "library-folders");
    let series = sample_items(&books, View::Series);
    let (mut ui, _) = home(View::Series, Filter::All, CoverMode::None, &series);
    day_night(&mut ui, out, "library-series");

    // One folder and one series as their own shelves.
    let platon: Vec<Item> = books
        .iter()
        .filter(|b| b.folder.as_deref() == Some("Platon"))
        .cloned()
        .map(Item::Book)
        .collect();
    let (mut ui, _) = shelf_ui(&Shelf {
        place: Place::Folder { name: "Platon" },
        hero: None,
        items: &platon,
        sort: tr(Msg::LibrarySortTitle),
        status: STATUS,
        covers: CoverMode::Cards,
    });
    day_night(&mut ui, out, "folder");
    let witcher: Vec<Item> = books
        .iter()
        .filter(|b| b.series.is_some())
        .cloned()
        .map(Item::Book)
        .collect();
    let (mut ui, _) = shelf_ui(&Shelf {
        place: Place::Series { name: "Trylogia" },
        hero: None,
        items: &witcher,
        sort: "",
        status: STATUS,
        covers: CoverMode::None,
    });
    day_night(&mut ui, out, "series");

    // A library with no books yet.
    let (mut ui, _) = shelf_ui(&Shelf {
        place: Place::Home {
            view: View::Books,
            filter: Filter::All,
        },
        hero: None,
        items: &[],
        sort: tr(Msg::LibrarySortRecent),
        status: STATUS,
        covers: CoverMode::None,
    });
    day_night(&mut ui, out, "library-empty");

    // Side by side with the stock home screen, when a capture of it is
    // given: ARISTOTLE_STOCK_HOME=path/to/stock-home.png (1072 x 1448 gray).
    let stock = std::env::var_os("ARISTOTLE_STOCK_HOME")
        .map(PathBuf::from)
        .and_then(|p| load_gray(&p));
    if let Some(s) = stock {
        let (mut ui, _) = home(View::Books, Filter::All, CoverMode::None, &all);
        ui.render();
        let gap = 40;
        let mut both = Canvas::new(W * 2 + gap, H);
        both.fill(Rect::new(W as i32, 0, gap as i32, H as i32), 128);
        for y in 0..H as i32 {
            for x in 0..W as i32 {
                let i = (y as u32 * both.stride + x as u32) as usize;
                if x < s.width as i32 && y < s.height as i32 {
                    both.pixels[i] = s.get(x, y);
                }
                both.pixels[i + (W + gap) as usize] = ui.screen().get(x, y);
            }
        }
        save(&both, &out.join("compare-library.png"));
    }
}

fn settings_screens(out: &Path) {
    let about = settings_screen::sample_about();
    let date = fmt_date(2026, 10, 17, 6);
    let view = SystemView {
        status: STATUS,
        light: Some((2, 24)),
        night: false,
        covers: CoverMode::None,
        sleep_minutes: Some(15),
        clippings: true,
        battery: Some(BatteryInfo {
            level: 0.82,
            charging: false,
            state: tr(Msg::SettingsBatteryDischarging),
            voltage_mv: 3980,
            temperature_c: 26,
        }),
        storage: Some((4_100_000_000, 6_200_000_000)),
        date: &date,
        time: "21:38",
        usb: Some(Usb::Network),
        about: &about,
        wifi: Some(tr(Msg::WifiOff)),
    };
    let mut text = TextEngine::new(Fonts::embedded());
    let styles = Styles::new(FamilyId(0));
    let (tree, ids) = settings_screen::Settings::build(&mut text, &styles, &view);
    let mut ui = Ui::new(tree, text, W, H);
    ui.layout();
    ids.sync(&mut ui.tree);
    day_night(&mut ui, out, "settings");
    let mut page = 1;
    loop {
        let (p, count) = ui.tree.page(ids.list);
        if p + 1 >= count {
            break;
        }
        ids.handle(&mut ui.tree, settings_screen::action::NEXT_PAGE);
        ui.layout();
        ids.sync(&mut ui.tree);
        page += 1;
        let d = ui.render();
        log(&format!("settings: page {page}"), &ui, &d);
        day_night(&mut ui, out, &format!("settings-page{page}"));
    }
    // Night mode on, as the toggle shows it, and charging in drive mode.
    let view = SystemView {
        night: true,
        light: Some((10, 24)),
        battery: Some(BatteryInfo {
            level: 0.47,
            charging: true,
            state: tr(Msg::SettingsBatteryCharging),
            voltage_mv: 4105,
            temperature_c: 28,
        }),
        status: StatusLine {
            charging: true,
            battery: Some(0.47),
            ..STATUS
        },
        usb: Some(Usb::Storage { minutes_left: 9 }),
        ..view
    };
    let mut text = TextEngine::new(Fonts::embedded());
    let (tree, ids) = settings_screen::Settings::build(&mut text, &styles, &view);
    let mut ui = Ui::new(tree, text, W, H);
    ui.layout();
    ids.sync(&mut ui.tree);
    ui.set_night(true);
    ui.render();
    save(ui.screen(), &out.join("settings-on-night.png"));
    ids.handle(&mut ui.tree, settings_screen::action::NEXT_PAGE);
    ui.layout();
    ids.sync(&mut ui.tree);
    ui.render();
    save(ui.screen(), &out.join("settings-page2-charging-night.png"));
}

/// A text input screen in a fresh `Ui`, rendered once.
fn text_input_ui(spec: &text_input::Spec) -> (Ui, TextInput) {
    let mut text = TextEngine::new(Fonts::embedded());
    let (tree, input) = TextInput::build(&mut text, &Styles::new(FamilyId(0)), spec);
    let mut ui = Ui::new(tree, text, W, H);
    ui.layout();
    ui.render();
    (ui, input)
}

/// Taps the key `slot`: down (rendered, the A2 step), up (handled).
/// Returns the damage of both steps.
fn press_key(
    ui: &mut Ui,
    input: &mut TextInput,
    slot: usize,
    ms: u64,
) -> (Vec<Damage>, Vec<Damage>) {
    let c = input.keyboard.centre(&ui.tree, slot);
    touch(ui, Phase::Down, c.x, c.y, ms);
    let down = ui.render();
    for ev in touch(ui, Phase::Up, c.x, c.y, ms + 60) {
        input.handle(&mut ui.tree, &ev);
    }
    let up = ui.render();
    (down, up)
}

fn slot_of(layer: Layer, key: Key) -> usize {
    keyboard::keys(layer)
        .iter()
        .position(|k| *k == key)
        .expect("key on layer")
}

fn text_input_screens(out: &Path) {
    let name = "Dom Wokulskiego";
    let title = aristotle_i18n::tf!(Msg::WifiPasswordTitle, name = name);
    let spec = text_input::Spec {
        hint: Some(tr(Msg::WifiPasswordHint)),
        password: true,
        submit: tr(Msg::WifiConnect),
        max_chars: 63,
        ..text_input::Spec::new(STATUS, &title)
    };
    let (mut ui, mut input) = text_input_ui(&spec);
    day_night(&mut ui, out, "keyboard");
    let mut ms = 0;
    for c in "izabela".chars() {
        press_key(
            &mut ui,
            &mut input,
            slot_of(Layer::Letters, Key::Char(c)),
            ms,
        );
        ms += 1000;
    }
    // The next key: finger down, then up, with the damage of each step.
    let s = slot_of(Layer::Letters, Key::Char('l'));
    let c = input.keyboard.centre(&ui.tree, s);
    touch(&mut ui, Phase::Down, c.x, c.y, ms);
    let d = ui.render();
    log("keyboard: key down", &ui, &d);
    save(ui.screen(), &out.join("keyboard-pressed.png"));
    let o = damage_overlay(&mut ui, &d, "Touch-down on a key: the key alone, with A2");
    save(&o, &out.join("keyboard-pressed-damage.png"));
    for ev in touch(&mut ui, Phase::Up, c.x, c.y, ms + 60) {
        input.handle(&mut ui.tree, &ev);
    }
    let d = ui.render();
    log("keyboard: key up", &ui, &d);
    let o = damage_overlay(
        &mut ui,
        &d,
        "Release: the key comes back and a dot appears, one DU update",
    );
    save(&o, &out.join("keyboard-typed-damage.png"));
    day_night(&mut ui, out, "keyboard-password");
    // Shown, with a message under the field.
    let ev = UiEvent::Action {
        id: text_input::action::REVEAL,
        node: input.field,
        at: Point::default(),
    };
    input.handle(&mut ui.tree, &ev);
    input.set_message(&mut ui.tree, Some(tr(Msg::WifiPasswordShort)));
    ui.render();
    save(ui.screen(), &out.join("keyboard-password-shown.png"));
    // Shift, then the Polish layer.
    press_key(
        &mut ui,
        &mut input,
        slot_of(Layer::Letters, Key::Shift),
        ms + 2000,
    );
    save(ui.screen(), &out.join("keyboard-shift.png"));
    press_key(
        &mut ui,
        &mut input,
        slot_of(Layer::Letters, Key::Polish),
        ms + 3000,
    );
    let d = ui.render();
    log("keyboard: Polish layer", &ui, &d);
    save(ui.screen(), &out.join("keyboard-polish.png"));
    // The symbol layers.
    let (mut ui, mut input) = text_input_ui(&text_input::Spec {
        initial: "https://dash.example.org:8443/",
        ..text_input::Spec::new(STATUS, "Server address")
    });
    press_key(
        &mut ui,
        &mut input,
        slot_of(Layer::Letters, Key::ToSymbols),
        0,
    );
    day_night(&mut ui, out, "keyboard-symbols");
    press_key(
        &mut ui,
        &mut input,
        slot_of(Layer::Symbols, Key::ToMore),
        1000,
    );
    save(ui.screen(), &out.join("keyboard-more.png"));
}

fn wifi_ui(v: &WifiView) -> (Ui, WifiScreen) {
    let mut text = TextEngine::new(Fonts::embedded());
    let (tree, ids) = WifiScreen::build(&mut text, &Styles::new(FamilyId(0)), v);
    let mut ui = Ui::new(tree, text, W, H);
    ui.layout();
    ids.sync(&mut ui.tree);
    (ui, ids)
}

fn wifi_screens(out: &Path) {
    let nets = wifi::sample_networks();
    let connected = aristotle_i18n::tf!(
        Msg::WifiConnectedIp,
        name = "Soplicowo",
        ip = "192.168.1.23"
    );
    let v = WifiView {
        status: STATUS,
        on: true,
        state: &connected,
        busy: false,
        list: ListState::Networks,
        networks: &nets,
        policy: wifi::Policy::Stock,
    };
    let (mut ui, _) = wifi_ui(&v);
    day_night(&mut ui, out, "wifi");
    let off = WifiView {
        on: false,
        state: tr(Msg::WifiOff),
        list: ListState::Off,
        networks: &[],
        ..v
    };
    let (mut ui, _) = wifi_ui(&off);
    day_night(&mut ui, out, "wifi-off");
    let scanning = WifiView {
        state: tr(Msg::WifiOn),
        busy: true,
        list: ListState::Scanning,
        networks: &[],
        policy: wifi::Policy::Scoped {
            hosts: 2,
            no_probe: false,
        },
        ..v
    };
    let (mut ui, _) = wifi_ui(&scanning);
    ui.render();
    save(ui.screen(), &out.join("wifi-scoped-scanning.png"));
    let mut armed = nets.clone();
    armed[4].forget_armed = true;
    let wrong = aristotle_i18n::tf!(Msg::WifiErrorPassword, name = "Dom Wokulskiego");
    let failed = WifiView {
        state: &wrong,
        networks: &armed,
        policy: wifi::Policy::Scoped {
            hosts: 1,
            no_probe: true,
        },
        ..v
    };
    let (mut ui, _) = wifi_ui(&failed);
    ui.render();
    save(ui.screen(), &out.join("wifi-error-forget.png"));
    // The settings row that leads here, and a state change as damage.
    let (mut ui, _) = wifi_ui(&scanning);
    ui.render();
    let (tree, ids) = WifiScreen::build(&mut ui.text, &Styles::new(FamilyId(0)), &v);
    ui.set_tree_with(tree, aristotle_ui::tree::Refresh::Auto);
    ui.layout();
    ids.sync(&mut ui.tree);
    let d = ui.render();
    log("wifi: scan finished", &ui, &d);
    let o = damage_overlay(&mut ui, &d, "Scan finished: the list replaces the message");
    save(&o, &out.join("wifi-scanned-damage.png"));
}

fn sleep_screens(out: &Path) {
    let books = library::sample_books();
    let styles = Styles::new(FamilyId(0));
    let mut ui = Ui::new(
        sleep::build(&styles, Some(&books[0]), false),
        TextEngine::new(Fonts::embedded()),
        W,
        H,
    );
    day_night(&mut ui, out, "screensaver");
}

/// Every icon at 72, 48 and 32 px with its name.
fn icons_sheet(out: &Path) {
    let mut text = TextEngine::new(Fonts::embedded());
    let mut c = Canvas::new(W, H);
    let all = Rect::new(0, 0, W as i32, H as i32);
    let cols = 7;
    let cell_w = W as i32 / cols;
    let cell_h = 196;
    let top = 60;
    let label = TextStyle::new(FamilyId(0), 24.0).gray(0x44);
    let head = TextStyle::new(FamilyId(0), 34.0).bold();
    {
        let l = set(
            &mut text,
            "Aristotle icons · 24-unit grid · 72, 48 and 32 px",
            head,
            W as f32,
        );
        let mut p = Painter::new(&mut c, all);
        l.draw(&mut text, &mut p, 40.0, 12.0);
    }
    for (i, icon) in Icon::ALL.iter().enumerate() {
        let (col, row) = (i as i32 % cols, i as i32 / cols);
        let x = col * cell_w;
        let y = top + row * cell_h;
        let mut p = Painter::new(&mut c, all);
        icons::draw(
            &mut p,
            *icon,
            Rect::new(x + (cell_w - 72) / 2, y + 8, 72, 72),
            0,
            0.0,
        );
        let base = y + 96;
        icons::draw(
            &mut p,
            *icon,
            Rect::new(x + cell_w / 2 - 56, base, 48, 48),
            0,
            0.0,
        );
        icons::draw(
            &mut p,
            *icon,
            Rect::new(x + cell_w / 2 + 16, base + 8, 32, 32),
            0,
            0.0,
        );
        let l = set(&mut text, icon.name(), label.clone(), cell_w as f32);
        let lx = x as f32 + (cell_w as f32 - l.width) / 2.0;
        l.draw(&mut text, &mut p, lx, (base + 54) as f32);
    }
    save(&c, &out.join("icons.png"));
    save(&invert(&c), &out.join("icons-night.png"));
}

/// The typographic cards: four bindings, a series, and list sizes.
fn covers_sheet(out: &Path) {
    let styles = Styles::new(FamilyId(0));
    let fam = styles.family;
    let books = library::sample_books();
    let mut t = Tree::new();
    let mut large = Vec::new();
    // Pick one book per binding so the sheet shows all four.
    let mut seen = Vec::new();
    for b in &books {
        let cover = Cover::new(&b.title, &b.author, fam).with_mark(b.mark());
        let binding = cover.binding();
        if !seen.contains(&binding) && seen.len() < 4 {
            seen.push(binding);
            let n = t.custom(cover);
            large.push(t.fixed(Some(220), Some(330), Some(n)));
        }
    }
    let large = t.add(Node::row(24), large);
    let mut series = Vec::new();
    for b in books.iter().filter(|b| b.series.is_some()) {
        let n = t.custom(Cover::new(&b.title, &b.author, fam).with_mark(b.mark()));
        series.push(t.fixed(Some(220), Some(330), Some(n)));
    }
    let n = t.custom(
        Cover::new(
            "Bardzo długi tytuł, który się nie mieści",
            "Ktoś Nieznany",
            fam,
        )
        .with_mark(CoverMark::default()),
    );
    series.push(t.fixed(Some(220), Some(330), Some(n)));
    let series = t.add(Node::row(24), series);
    let mut small = Vec::new();
    for b in &books {
        let n = t.custom(Cover::new(&b.title, &b.author, fam).with_mark(b.mark()));
        small.push(t.fixed(Some(72), Some(106), Some(n)));
    }
    let small = t.add(Node::row(16), small);
    let caption = t.label(
        "Typographic cards: binding by folder or series, Tom n for series",
        styles.small(),
    );
    let col = t.add(Node::column(40), [caption, large, series, small]);
    let root = t.pad(aristotle_ui::Insets::all(40), col);
    t.set_root(root);
    let mut ui = Ui::new(t, TextEngine::new(Fonts::embedded()), W, H);
    day_night(&mut ui, out, "covers");
}

fn reader_screens(out: &Path, fonts_dir: &Path) {
    let (mut text, families) = engine(fonts_dir);
    let styles = Styles::new(FamilyId(0));
    let (tree, mut r) = Reader::build(&mut text, &styles, families, Settings::default());
    let mut ui = Ui::new(tree, text, W, H);
    {
        let d = ui.render();
        log("reader: first frame", &ui, &d)
    };
    println!("  chapter set in {} pages", r.page_count());
    day_night(&mut ui, out, "reader");

    // Tap zones overlay.
    let mut z = ui.screen().clone();
    for (zone, rect) in reader::zones(ui.bounds()) {
        let name = match zone {
            Zone::Prev => "Previous",
            Zone::Next => "Next page",
            Zone::Menu => "Menu",
        };
        let style = TextStyle::new(FamilyId(0), 44.0).bold().gray(255);
        let l = set(&mut ui.text, name, style, 600.0);
        let mut p = Painter::new(&mut z, Rect::new(0, 0, W as i32, H as i32));
        dashed_rect(&mut p, rect.outset(-6), 6, 24, 0);
        let c = rect.center();
        let tag = Rect::new(
            c.x - l.width as i32 / 2 - 24,
            c.y - l.height as i32 / 2 - 10,
            l.width as i32 + 48,
            l.height as i32 + 20,
        );
        p.fill_round_rect(tag, 16.0, 0);
        l.draw(
            &mut ui.text,
            &mut p,
            tag.x as f32 + 24.0,
            tag.y as f32 + 10.0,
        );
    }
    save(&z, &out.join("reader-zones.png"));

    // Turn the page with a tap in the right zone.
    for ev in tap(&mut ui, Point { x: 850, y: 900 }, 0) {
        r.handle(&mut ui, ev);
    }
    let d = ui.render();
    log("reader: page turn", &ui, &d);
    save(ui.screen(), &out.join("reader-page2.png"));
    let o = damage_overlay(
        &mut ui,
        &d,
        "Page turn: text and footer; on the device one full mode-9 update",
    );
    save(&o, &out.join("reader-turn-damage.png"));

    // Back to page 1 with a swipe to the right.
    for (ph, x, ms) in [
        (Phase::Down, 300, 2000),
        (Phase::Move, 500, 2050),
        (Phase::Up, 750, 2100),
    ] {
        for ev in touch(&mut ui, ph, x, 800, ms) {
            r.handle(&mut ui, ev);
        }
    }
    ui.render();

    // Open the menu from the top band.
    for ev in tap(&mut ui, Point { x: 536, y: 70 }, 3000) {
        r.handle(&mut ui, ev);
    }
    let d = ui.render();
    log("reader: open menu", &ui, &d);
    day_night(&mut ui, out, "reader-menu");
    let o = damage_overlay(
        &mut ui,
        &d,
        "Opening the menu repaints the two bars; the page between them stays",
    );
    save(&o, &out.join("reader-menu-damage.png"));

    // Press feedback inside the menu, on the larger-text button.
    let size_row = ui.tree.rect(r.night_toggle);
    let probe = Point {
        x: W as i32 - 110,
        y: size_row.y - 3 * 120 - 50,
    };
    if let Hit::Target(node) = ui.tree.hit(probe) {
        let c = ui.tree.rect(node).center();
        touch(&mut ui, Phase::Down, c.x, c.y, 4000);
        let d = ui.render();
        log("reader menu: press", &ui, &d);
        save(ui.screen(), &out.join("reader-menu-pressed.png"));
        touch(&mut ui, Phase::Move, c.x, c.y + 300, 4040);
        touch(&mut ui, Phase::Up, c.x, c.y + 300, 4080);
        ui.render();
    }

    // Night mode with the menu toggle, then close the menu.
    let night = ui.tree.rect(r.night_toggle).center();
    for ev in tap(&mut ui, night, 5000) {
        r.handle(&mut ui, ev);
    }
    {
        let d = ui.render();
        log("reader: night mode from the menu", &ui, &d)
    };
    save(ui.screen(), &out.join("reader-menu-night-on.png"));
    for ev in tap(&mut ui, Point { x: 700, y: 400 }, 6000) {
        r.handle(&mut ui, ev);
    }
    ui.render();

    // Alternative typography: Charis, one size up, loose spacing.
    let (mut text, families) = engine(fonts_dir);
    let s = Settings {
        family: 1,
        size: 3,
        spacing: 2,
        ..Default::default()
    };
    let (tree, _) = Reader::build(&mut text, &styles, families, s);
    let mut ui = Ui::new(tree, text, W, H);
    day_night(&mut ui, out, "reader-charis");
}

/// The centre of every visible control of `ui`, as `screen action x y w h`
/// lines, for writing device scenarios with real coordinates.
fn dump_targets(ui: &mut Ui, screen: &str, out: &mut String) {
    ui.layout();
    for i in 0..ui.tree.len() {
        let id = aristotle_ui::tree::NodeId(i as u32);
        let Some(action) = ui.tree.node(id).action else {
            continue;
        };
        let r = ui.tree.rect(id);
        if r.is_empty() {
            continue;
        }
        let c = r.center();
        out.push_str(&format!(
            "{screen} {action} {} {} {} {}\n",
            c.x, c.y, r.w, r.h
        ));
    }
}

/// Control coordinates of the app's screens with the sample library: home
/// (books, folders, series), a folder, settings pages and the app's reader
/// menu (no font row, night row on).
fn targets(out: &Path) {
    use aristotle_ui::screens::reader::{build_menu, MenuSpec};
    let mut s = String::from("# screen action-id centre-x centre-y width height\n");
    let books = library::sample_books();
    for view in View::ALL {
        let items = sample_items(
            if view == View::Books {
                &books[1..]
            } else {
                &books
            },
            view,
        );
        let (mut ui, _) = shelf_ui(&Shelf {
            place: Place::Home {
                view,
                filter: Filter::All,
            },
            hero: Some(&books[0]),
            items: &items,
            sort: tr(Msg::LibrarySortRecent),
            status: STATUS,
            covers: CoverMode::None,
        });
        dump_targets(&mut ui, &format!("library-{view:?}"), &mut s);
    }
    let platon: Vec<Item> = books
        .iter()
        .filter(|b| b.folder.as_deref() == Some("Platon"))
        .cloned()
        .map(Item::Book)
        .collect();
    let (mut ui, _) = shelf_ui(&Shelf {
        place: Place::Folder { name: "Platon" },
        hero: None,
        items: &platon,
        sort: tr(Msg::LibrarySortTitle),
        status: STATUS,
        covers: CoverMode::None,
    });
    dump_targets(&mut ui, "folder", &mut s);
    let about = settings_screen::sample_about();
    let view = SystemView {
        status: STATUS,
        light: Some((2, 24)),
        night: false,
        covers: CoverMode::None,
        sleep_minutes: Some(15),
        clippings: true,
        battery: Some(BatteryInfo {
            level: 0.82,
            charging: false,
            state: tr(Msg::SettingsBatteryDischarging),
            voltage_mv: 3980,
            temperature_c: 26,
        }),
        storage: Some((4_100_000_000, 6_200_000_000)),
        date: "",
        time: "21:38",
        usb: Some(Usb::Network),
        about: &about,
        wifi: Some(tr(Msg::WifiOff)),
    };
    let mut text = TextEngine::new(Fonts::embedded());
    let (tree, ids) = settings_screen::Settings::build(&mut text, &Styles::new(FamilyId(0)), &view);
    let mut ui = Ui::new(tree, text, W, H);
    ui.layout();
    dump_targets(&mut ui, "settings-1", &mut s);
    ids.handle(&mut ui.tree, settings_screen::action::NEXT_PAGE);
    dump_targets(&mut ui, "settings-2", &mut s);
    ids.handle(&mut ui.tree, settings_screen::action::NEXT_PAGE);
    dump_targets(&mut ui, "settings-3", &mut s);
    let (mut ui, _) = text_input_ui(&text_input::Spec {
        password: true,
        ..text_input::Spec::new(STATUS, "Password")
    });
    dump_targets(&mut ui, "keyboard", &mut s);
    let nets = wifi::sample_networks();
    let (mut ui, _) = wifi_ui(&WifiView {
        status: STATUS,
        on: true,
        state: "",
        busy: false,
        list: ListState::Networks,
        networks: &nets,
        policy: wifi::Policy::Stock,
    });
    dump_targets(&mut ui, "wifi", &mut s);
    let mut t = Tree::new();
    let page = t.spacer(1);
    let spec = MenuSpec {
        title: "Fedon",
        families: None,
        night: true,
        top_extra: 0,
        bottom_extra: 0,
    };
    let menu = build_menu(
        &mut t,
        &Styles::new(FamilyId(0)),
        &spec,
        &Settings::default(),
    );
    let root = t.add(
        Node::new(aristotle_ui::tree::Kind::Stack),
        [page, menu.root],
    );
    t.set_root(root);
    t.set_hidden(menu.root, false);
    let mut ui = Ui::new(t, TextEngine::new(Fonts::embedded()), W, H);
    dump_targets(&mut ui, "reader-menu", &mut s);
    let path = out.join("targets.txt");
    std::fs::write(&path, s).expect("targets.txt");
    println!("wrote {}", path.display());
}

/// `$ARISTOTLE_CACHE`, defaulting as `tools/env.sh` does.
fn cache_dir() -> PathBuf {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    var("ARISTOTLE_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            var("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(var("HOME").expect("HOME is set")).join(".cache"))
                .join("aristotle")
        })
}

fn render_all(out: &Path) {
    std::fs::create_dir_all(out).expect("output directory");
    let fonts_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
    icons_sheet(out);
    covers_sheet(out);
    library_screens(out);
    settings_screens(out);
    text_input_screens(out);
    wifi_screens(out);
    sleep_screens(out);
    reader_screens(out, &fonts_dir);
    targets(out);
}

fn main() {
    let mut lang = String::from("en");
    let mut out = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(a) = args.next() {
        if a == "--lang" {
            let v = args.next().expect("--lang needs a language code or `all`");
            lang = v.to_string_lossy().into_owned();
        } else {
            out = Some(PathBuf::from(a));
        }
    }
    let out = out.unwrap_or_else(|| cache_dir().join("ui-preview/v2"));
    if lang == "all" {
        for l in aristotle_i18n::locales() {
            aristotle_i18n::set_language(l.code);
            render_all(&out.join(l.code));
        }
    } else {
        let codes: Vec<&str> = aristotle_i18n::locales().iter().map(|l| l.code).collect();
        assert!(
            aristotle_i18n::set_language(&lang),
            "unknown language {lang:?}; embedded: {codes:?}"
        );
        render_all(&out);
    }
}
