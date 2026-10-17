// SPDX-License-Identifier: AGPL-3.0-or-later
//! Settings: light, night mode, covers, sleep, the UI language, whether
//! highlights go to `My Clippings.txt`, and what the device knows about
//! itself. Offline only: there is no Wi-Fi, account or
//! store section, because Aristotle has none of those.
//!
//! Sections are whole blocks in a paged list, so the screen turns pages like
//! every other list. Every control is a 9 mm target and answers on
//! touch-down (A2 inversion); its new state is drawn in black and white so
//! the change itself goes out with DU.

use super::icon_button;
use super::library::{chip, status_line, CoverMode, StatusLine};
use crate::geom::{mm, Insets, MIN_TARGET};
use crate::icons::Icon;
use crate::text::{Align, Paragraph, TextEngine};
use crate::theme::{margin, IconSize, Space, Styles, INK, RULE};
use crate::tree::{pages, progress, ActionId, Cross, Feedback, Kind, Node, NodeId, Tree};
use crate::widgets::{Battery, Slider, Toggle};
use aristotle_i18n::{fmt_bytes, fmt_decimal, tf, tr, Msg};

pub mod action {
    use crate::tree::ActionId;
    pub const BACK: ActionId = 1;
    pub const PREV_PAGE: ActionId = 5;
    pub const NEXT_PAGE: ActionId = 6;
    pub const LIGHT_DOWN: ActionId = 10;
    pub const LIGHT_UP: ActionId = 11;
    pub const LIGHT_SET: ActionId = 12;
    /// `LIGHT_PRESET + i` picks `LIGHT_PRESETS[i]`.
    pub const LIGHT_PRESET: ActionId = 20;
    pub const NIGHT: ActionId = 30;
    /// `COVERS + i` picks `CoverMode::ALL[i]`.
    pub const COVERS: ActionId = 40;
    /// `SLEEP + i` picks `SLEEP_MINUTES[i]`.
    pub const SLEEP: ActionId = 50;
    /// `USB + 0` is the network, `USB + 1` drive mode.
    pub const USB: ActionId = 60;
    /// `LANGUAGE + i` picks `aristotle_i18n::locales()[i]`.
    pub const LANGUAGE: ActionId = 70;
    /// Toggles "Save highlights to My Clippings.txt".
    pub const CLIPPINGS: ActionId = 80;
}

/// Light presets: a name and a step of `aristotle_hal::power::light_raw`'s
/// 24-step scale. "Night" is a dim evening level (step 2 is raw 19 of
/// 2047, next to the raw 20 the stock UI had been set to on the test unit).
pub const LIGHT_PRESETS: [(Msg, u32); 4] = [
    (Msg::SettingsPresetOff, 0),
    (Msg::SettingsPresetNight, 2),
    (Msg::SettingsPresetMedium, 10),
    (Msg::SettingsPresetBright, 18),
];

/// Screensaver delays offered, in minutes; `None` is never.
pub const SLEEP_MINUTES: [Option<u32>; 4] = [Some(5), Some(15), Some(30), None];

/// The USB port, as the device agent reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Usb {
    Network,
    /// Drive mode with this many minutes left.
    Storage {
        minutes_left: u32,
    },
}

/// Battery facts for the power section.
#[derive(Clone, Copy, Debug)]
pub struct BatteryInfo<'a> {
    pub level: f32,
    pub charging: bool,
    /// "Charging", "Charged", "Discharging".
    pub state: &'a str,
    pub voltage_mv: u32,
    pub temperature_c: i32,
}

/// Everything the settings screen shows.
pub struct SystemView<'a> {
    pub status: StatusLine<'a>,
    /// Current light step and the number of steps; `None` without a light.
    pub light: Option<(u32, u32)>,
    pub night: bool,
    pub covers: CoverMode,
    pub sleep_minutes: Option<u32>,
    /// Whether new highlights are appended to `My Clippings.txt`.
    pub clippings: bool,
    pub battery: Option<BatteryInfo<'a>>,
    /// Free and total bytes of the userstore.
    pub storage: Option<(u64, u64)>,
    /// "Saturday, 17 October 2026" (`aristotle_i18n::fmt_date`).
    pub date: &'a str,
    pub time: &'a str,
    /// `None` when the device agent is not installed.
    pub usb: Option<Usb>,
    /// "About" lines: a label and a value.
    pub about: &'a [(&'a str, String)],
}

/// Handles to the nodes the app updates in place.
pub struct Settings {
    pub list: NodeId,
    pub pager_label: NodeId,
    pub prev: NodeId,
    pub next: NodeId,
    pub title: NodeId,
    /// The busy mark in the status line, hidden.
    pub busy: NodeId,
    /// The light [`Slider`], if there is a light.
    pub light: Option<NodeId>,
}

/// "3.98 V" ("3,98 V").
pub fn volts(mv: u32) -> String {
    tf!(Msg::FormatVolts, value = fmt_decimal(mv as f64 / 1000.0, 2))
}

/// "4.1 GB free of 6.2 GB".
pub fn free_of(free: u64, total: u64) -> String {
    tf!(
        Msg::SettingsFreeOf,
        free = fmt_bytes(free),
        total = fmt_bytes(total)
    )
}

/// A section as items of the paged list: the small-caps heading travels
/// with the first row, so a heading never ends a page alone; every row is
/// followed by a hairline.
fn section(t: &mut Tree, s: &Styles, heading: &str, rows: Vec<NodeId>) -> Vec<NodeId> {
    let h = t.label(heading, s.caps());
    let h = t.pad(Insets::new(Space::L, 0, 0, 0), h);
    let mut items = Vec::new();
    for (i, r) in rows.into_iter().enumerate() {
        let line = t.rule(1, RULE);
        let children = if i == 0 {
            vec![h, r, line]
        } else {
            vec![r, line]
        };
        items.push(t.add(Node::column(0), children));
    }
    items
}

/// A labelled row: an icon, a label with an optional hint below it, and a
/// control at the right. Rows with a control are at least 9 mm tall.
fn row(
    t: &mut Tree,
    s: &Styles,
    icon: Icon,
    label: &str,
    hint: Option<&str>,
    control: Option<NodeId>,
) -> NodeId {
    let min = if control.is_some() { MIN_TARGET } else { 0 };
    row_with(t, s, icon, label, hint, control, min)
}

/// A row that only shows a value at the right, as tall as its words.
fn value_row(t: &mut Tree, s: &Styles, icon: Icon, label: &str, value: &str) -> NodeId {
    let v = t.label(value, s.ui_bold());
    row_with(t, s, icon, label, None, Some(v), 0)
}

fn row_with(
    t: &mut Tree,
    s: &Styles,
    icon: Icon,
    label: &str,
    hint: Option<&str>,
    control: Option<NodeId>,
    min: i32,
) -> NodeId {
    let i = t.icon(icon, IconSize::STANDARD, INK);
    let l = t.label(label, s.ui());
    let mut words = vec![l];
    if let Some(h) = hint {
        let mut p = Paragraph::plain(h, s.small(), Align::Left);
        p.style.max_lines = Some(2);
        words.push(t.text(p));
    }
    let words = t.add(Node::column(2).flex(1), words);
    let mut children = vec![i, words];
    children.extend(control);
    let r = t.add(Node::row(Space::M), children);
    let r = t.add(
        Node::new(Kind::MinSize(crate::geom::Size::new(0, min))),
        [r],
    );
    t.pad(Insets::vh(Space::S, 0), r)
}

/// A segmented choice; `selected` is drawn solid.
fn choice(
    t: &mut Tree,
    s: &Styles,
    labels: &[&str],
    selected: Option<usize>,
    first: ActionId,
) -> NodeId {
    let chips: Vec<NodeId> = labels
        .iter()
        .enumerate()
        .map(|(i, l)| chip(t, s, l, selected == Some(i), first + i as ActionId))
        .collect();
    t.add(Node::row(Space::S), chips)
}

impl Settings {
    pub fn build(_text: &mut TextEngine, s: &Styles, v: &SystemView) -> (Tree, Self) {
        let mut t = Tree::new();
        let (status, busy) = status_line(&mut t, s, &v.status);
        let back = icon_button(&mut t, Icon::ArrowLeft, IconSize::STANDARD, action::BACK);
        let title = t.label(tr(Msg::SettingsTitle), s.display());
        let g = t.gap(Space::XS, 1);
        let header = t.add(Node::row(0), [back, g, title]);
        let header = t.pad(Insets::new(0, 0, 0, -26), header);

        let mut sections = Vec::new();
        let mut light_node = None;

        // Light and screen.
        let mut rows = Vec::new();
        if let Some((step, steps)) = v.light {
            let value = if step == 0 {
                tr(Msg::SettingsLightOff).to_string()
            } else {
                tf!(Msg::SettingsLightStep, step = step, steps = steps)
            };
            let head = value_row(&mut t, s, Icon::Sun, tr(Msg::SettingsLight), &value);
            let dim = icon_button(
                &mut t,
                Icon::SunSmall,
                IconSize::STANDARD,
                action::LIGHT_DOWN,
            );
            let slider = t.add(
                Node::new(Kind::Custom(Box::new(Slider {
                    value: step as f32 / steps.max(1) as f32,
                })))
                .action(action::LIGHT_SET)
                .feedback(Feedback::None),
                [],
            );
            light_node = Some(slider);
            let slider = t.add(Node::column(0).flex(1).cross(Cross::Stretch), [slider]);
            let bright = icon_button(&mut t, Icon::Sun, IconSize::STANDARD, action::LIGHT_UP);
            let line = t.add(Node::row(0), [dim, slider, bright]);
            let preset = LIGHT_PRESETS.iter().position(|(_, p)| *p == step);
            let names: Vec<&str> = LIGHT_PRESETS.iter().map(|(n, _)| tr(*n)).collect();
            let presets = choice(&mut t, s, &names, preset, action::LIGHT_PRESET);
            let presets = t.pad(Insets::new(0, 0, Space::M, Space::XL + Space::M), presets);
            let block = t.add(Node::column(0), [head, line, presets]);
            rows.push(block);
        }
        let toggle = t.custom(Toggle { on: v.night });
        let night = row(
            &mut t,
            s,
            Icon::Moon,
            tr(Msg::SettingsNight),
            Some(tr(Msg::SettingsNightHint)),
            Some(toggle),
        );
        let night = t.add(
            Node::new(Kind::Padding(Insets::all(0)))
                .action(action::NIGHT)
                .feedback(Feedback::Outline),
            [night],
        );
        rows.push(night);
        let labels: Vec<&str> = CoverMode::ALL.iter().map(|c| c.label()).collect();
        let selected = CoverMode::ALL.iter().position(|c| *c == v.covers);
        let covers_head = row(
            &mut t,
            s,
            Icon::Library,
            tr(Msg::SettingsCovers),
            Some(tr(Msg::SettingsCoversHint)),
            None,
        );
        let covers = choice(&mut t, s, &labels, selected, action::COVERS);
        let covers = t.pad(Insets::new(0, 0, Space::M, Space::XL + Space::M), covers);
        rows.push(t.add(Node::column(0), [covers_head, covers]));
        sections.extend(section(&mut t, s, tr(Msg::SettingsSectionScreen), rows));

        // Power.
        let mut rows = Vec::new();
        if let Some(b) = &v.battery {
            let pct = t.label(&format!("{:.0}%", b.level * 100.0), s.ui_bold());
            let bat = t.custom(Battery {
                level: b.level,
                charging: b.charging,
            });
            let ctl = t.add(Node::row(Space::S), [pct, bat]);
            let hint = format!(
                "{} · {} · {} °C",
                b.state,
                volts(b.voltage_mv),
                b.temperature_c
            );
            let icon = if b.charging {
                Icon::BatteryCharging
            } else {
                Icon::Battery
            };
            let name = tr(Msg::SettingsBattery);
            rows.push(row(&mut t, s, icon, name, Some(&hint), Some(ctl)));
        }
        let sleep_head = row(
            &mut t,
            s,
            Icon::Power,
            tr(Msg::SettingsSleep),
            Some(tr(Msg::SettingsSleepHint)),
            None,
        );
        let labels: Vec<String> = SLEEP_MINUTES
            .iter()
            .map(|m| match m {
                Some(m) => tf!(Msg::FormatMinutes, m = m),
                None => tr(Msg::SettingsSleepNever).to_string(),
            })
            .collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let selected = SLEEP_MINUTES.iter().position(|m| *m == v.sleep_minutes);
        let sleep = choice(&mut t, s, &labels, selected, action::SLEEP);
        let sleep = t.pad(Insets::new(0, 0, Space::M, Space::XL + Space::M), sleep);
        rows.push(t.add(Node::column(0), [sleep_head, sleep]));
        sections.extend(section(&mut t, s, tr(Msg::SettingsSectionPower), rows));

        // Device.
        let mut rows = Vec::new();
        let time = t.label(v.time, s.ui_bold());
        rows.push(row(
            &mut t,
            s,
            Icon::Clock,
            tr(Msg::SettingsDateTime),
            Some(v.date),
            Some(time),
        ));
        if let Some((free, total)) = v.storage {
            let hint = free_of(free, total);
            let used = 1.0 - free as f32 / total.max(1) as f32;
            let bar = t.add(progress(used, 12), []);
            let bar = t.fixed(Some(180), None, Some(bar));
            rows.push(row(
                &mut t,
                s,
                Icon::Storage,
                tr(Msg::SettingsStorage),
                Some(&hint),
                Some(bar),
            ));
        }
        if let Some(usb) = v.usb {
            let hint = match usb {
                Usb::Network => tr(Msg::SettingsUsbNetworkHint).to_string(),
                Usb::Storage { minutes_left } => {
                    tf!(Msg::SettingsUsbStorageHint, minutes = minutes_left)
                }
            };
            let name = tr(Msg::SettingsUsb);
            let head = row(&mut t, s, Icon::Usb, name, Some(&hint), None);
            let selected = match usb {
                Usb::Network => 0,
                Usb::Storage { .. } => 1,
            };
            let c = choice(
                &mut t,
                s,
                &[tr(Msg::SettingsUsbNetwork), tr(Msg::SettingsUsbStorage)],
                Some(selected),
                action::USB,
            );
            let c = t.pad(Insets::new(0, 0, Space::M, Space::XL + Space::M), c);
            rows.push(t.add(Node::column(0), [head, c]));
        }
        // The UI language, each named in itself.
        let name = tr(Msg::SettingsLanguage);
        let head = row(&mut t, s, Icon::Language, name, None, None);
        let locales = aristotle_i18n::locales();
        let names: Vec<&str> = locales.iter().map(|l| l.name).collect();
        let current = locales
            .iter()
            .position(|l| l.code == aristotle_i18n::language());
        let c = choice(&mut t, s, &names, current, action::LANGUAGE);
        let c = t.pad(Insets::new(0, 0, Space::M, Space::XL + Space::M), c);
        rows.push(t.add(Node::column(0), [head, c]));
        sections.extend(section(&mut t, s, tr(Msg::SettingsSectionDevice), rows));

        // Reading. After the device section, so the rows before it keep
        // their places on the first two pages (the device scenarios tap
        // them by coordinates).
        let toggle = t.custom(Toggle { on: v.clippings });
        let clippings = row(
            &mut t,
            s,
            Icon::Bookmark,
            tr(Msg::SettingsClippings),
            Some(tr(Msg::SettingsClippingsHint)),
            Some(toggle),
        );
        let clippings = t.add(
            Node::new(Kind::Padding(Insets::all(0)))
                .action(action::CLIPPINGS)
                .feedback(Feedback::Outline),
            [clippings],
        );
        sections.extend(section(
            &mut t,
            s,
            tr(Msg::SettingsSectionReading),
            vec![clippings],
        ));

        // About.
        let mut lines = Vec::new();
        for (label, value) in v.about {
            let l = t.label(label, s.small());
            let l = t.fixed(Some(mm(25.0)), None, Some(l));
            let mut p = Paragraph::plain(value.as_str(), s.ui(), Align::Left);
            p.style.max_lines = Some(2);
            let val = t.text(p);
            let val = t.add(Node::column(0).flex(1), [val]);
            lines.push(t.add(Node::row(Space::S).cross(Cross::Start), [l, val]));
        }
        let lines = t.add(Node::column(Space::S), lines);
        let info = t.icon(Icon::Info, IconSize::STANDARD, INK);
        let about = t.add(Node::row(Space::M).cross(Cross::Start), [info, lines]);
        let about = t.pad(Insets::vh(Space::M, 0), about);
        sections.extend(section(
            &mut t,
            s,
            tr(Msg::SettingsSectionAbout),
            vec![about],
        ));

        let list = t.add(pages(0).flex(1), sections);
        let prev = icon_button(
            &mut t,
            Icon::ChevronLeft,
            IconSize::STANDARD,
            action::PREV_PAGE,
        );
        let next = icon_button(
            &mut t,
            Icon::ChevronRight,
            IconSize::STANDARD,
            action::NEXT_PAGE,
        );
        let pager_label = t.text(Paragraph::plain("", s.small(), Align::Center));
        let pager_label = t.add(Node::column(0).flex(1), [pager_label]);
        let pager = t.add(Node::row(0), [prev, pager_label, next]);
        let col = t.add(Node::column(0), [status, header, list, pager]);
        let root = t.pad(Insets::new(0, margin(), mm(1.5), margin()), col);
        t.set_root(root);
        let ids = Settings {
            list,
            pager_label: t.node(pager_label).children[0],
            prev: t.node(prev).children[0],
            next: t.node(next).children[0],
            title,
            busy,
            light: light_node,
        };
        (t, ids)
    }

    /// Updates the pager after layout.
    pub fn sync(&self, t: &mut Tree) {
        let (page, count) = t.page(self.list);
        t.set_text(
            self.pager_label,
            &super::library::page_of(page + 1, count.max(1)),
        );
        t.set_icon_gray(self.prev, if page == 0 { RULE } else { INK });
        t.set_icon_gray(self.next, if page + 1 >= count { RULE } else { INK });
    }

    /// Turns the section pages; true if `id` was a pager action.
    pub fn handle(&self, t: &mut Tree, id: ActionId) -> bool {
        let (page, _) = t.page(self.list);
        match id {
            action::PREV_PAGE => t.set_page(self.list, page.saturating_sub(1)),
            action::NEXT_PAGE => t.set_page(self.list, page + 1),
            _ => return false,
        };
        true
    }
}

/// A plausible device, for the preview and tests.
pub fn sample_about() -> Vec<(&'static str, String)> {
    vec![
        (
            tr(Msg::SettingsAboutSoftware),
            format!("Aristotle {}", env!("CARGO_PKG_VERSION")),
        ),
        (
            tr(Msg::SettingsAboutReader),
            "Kindle Paperwhite 4".to_string(),
        ),
        (tr(Msg::SettingsAboutFirmware), "5.13.6".to_string()),
        (
            tr(Msg::SettingsAboutMemory),
            free_of(392_000_000, 494_000_000),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{SCREEN_HEIGHT, SCREEN_WIDTH};
    use crate::text::{FamilyId, Fonts};
    use crate::ui::Ui;

    fn view<'a>(about: &'a [(&'a str, String)]) -> SystemView<'a> {
        SystemView {
            status: StatusLine {
                clock: "21:38",
                battery: Some(0.82),
                charging: false,
            },
            light: Some((2, 24)),
            night: false,
            covers: CoverMode::None,
            sleep_minutes: Some(15),
            clippings: true,
            battery: Some(BatteryInfo {
                level: 0.82,
                charging: false,
                state: "Discharging",
                voltage_mv: 3980,
                temperature_c: 26,
            }),
            storage: Some((4_100_000_000, 6_200_000_000)),
            date: "Saturday, 17 October 2026",
            time: "21:38",
            usb: Some(Usb::Network),
            about,
        }
    }

    #[test]
    fn formats() {
        aristotle_i18n::with_language("pl", || {
            assert_eq!(volts(3980), "3,98 V");
            assert_eq!(
                free_of(4_100_000_000, 6_200_000_000),
                "4,1 GB wolne z 6,2 GB"
            );
        });
        aristotle_i18n::with_language("en", || {
            assert_eq!(volts(3980), "3.98 V");
            assert_eq!(free_of(392_000_000, 494_000_000), "392 MB free of 494 MB");
        });
    }

    #[test]
    fn every_language_is_offered_and_the_current_one_is_selected() {
        for code in ["en", "pl"] {
            aristotle_i18n::with_language(code, || {
                let about = sample_about();
                let mut text = TextEngine::new(Fonts::embedded());
                let (tree, _) =
                    Settings::build(&mut text, &Styles::new(FamilyId(0)), &view(&about));
                let chips: Vec<_> = (0..tree.len())
                    .map(|i| crate::tree::NodeId(i as u32))
                    .filter(|&n| {
                        tree.node(n)
                            .action
                            .is_some_and(|a| (action::LANGUAGE..action::LANGUAGE + 10).contains(&a))
                    })
                    .collect();
                assert_eq!(chips.len(), aristotle_i18n::locales().len());
                let selected: Vec<bool> = chips.iter().map(|&n| tree.node(n).inverted).collect();
                let at = aristotle_i18n::locales()
                    .iter()
                    .position(|l| l.code == code)
                    .unwrap();
                assert!(selected[at] && selected.iter().filter(|s| **s).count() == 1);
            });
        }
    }

    #[test]
    fn sections_page_and_controls_are_targets() {
        let about = sample_about();
        let mut text = TextEngine::new(Fonts::embedded());
        let (tree, ids) = Settings::build(&mut text, &Styles::new(FamilyId(0)), &view(&about));
        let mut ui = Ui::new(tree, text, SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
        ui.layout();
        ids.sync(&mut ui.tree);
        ui.render();
        let (_, count) = ui.tree.page(ids.list);
        assert!(count >= 2, "the settings take more than one page");
        // The night row reacts anywhere on it, not only on the switch.
        let mut night = None;
        for y in (0..SCREEN_HEIGHT).step_by(20) {
            if let crate::tree::Hit::Target(n) = ui.tree.hit(crate::geom::Point { x: 300, y }) {
                if ui.tree.node(n).action == Some(action::NIGHT) {
                    night = Some(n);
                }
            }
        }
        let night = night.expect("night row on the first page");
        assert!(ui.tree.rect(night).h >= MIN_TARGET);
    }

    /// The page on which `action` is a target, and its rectangle.
    fn find(ui: &mut Ui, ids: &Settings, action: ActionId) -> Option<(usize, crate::geom::Rect)> {
        let (_, count) = ui.tree.page(ids.list);
        for page in 0..count {
            ui.tree.set_page(ids.list, page);
            ui.layout();
            for y in (0..SCREEN_HEIGHT).step_by(10) {
                if let crate::tree::Hit::Target(n) = ui.tree.hit(crate::geom::Point { x: 300, y }) {
                    if ui.tree.node(n).action == Some(action) {
                        return Some((page, ui.tree.rect(n)));
                    }
                }
            }
        }
        None
    }

    #[test]
    fn clippings_row_follows_the_device_section() {
        let about = sample_about();
        let mut text = TextEngine::new(Fonts::embedded());
        let (tree, ids) = Settings::build(&mut text, &Styles::new(FamilyId(0)), &view(&about));
        let mut ui = Ui::new(tree, text, SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
        ui.layout();
        let (clip_page, clip) = find(&mut ui, &ids, action::CLIPPINGS).expect("clippings row");
        assert!(clip.h >= MIN_TARGET);
        // The rows before it keep their pages: night on the first, the
        // language on the second (device scenarios tap them).
        assert_eq!(find(&mut ui, &ids, action::NIGHT).map(|f| f.0), Some(0));
        let (lang_page, lang) = find(&mut ui, &ids, action::LANGUAGE).expect("language chips");
        assert_eq!(lang_page, 1);
        assert!(clip_page > lang_page || clip.y > lang.y);
    }
}
