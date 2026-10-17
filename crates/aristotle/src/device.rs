// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the device knows about itself, read fresh for the status line and
//! the settings screen: clock, battery, storage, USB mode, versions.

use std::path::Path;

use aristotle_hal::power::{Battery, BatteryStatus, ChargeState};
use aristotle_hal::system::{self, UsbMode};
use aristotle_i18n::{fmt_date, fmt_time, tr, Msg};
use aristotle_ui::screens::library::StatusLine;
use aristotle_ui::screens::settings::{free_of, BatteryInfo, Usb};

pub struct Facts {
    pub clock: String,
    pub date: String,
    pub battery: Option<BatteryStatus>,
    /// Free and total bytes of the filesystem holding the books.
    pub storage: Option<(u64, u64)>,
    pub usb: Option<Usb>,
    pub about: Vec<(&'static str, String)>,
}

fn state_label(s: ChargeState) -> &'static str {
    tr(match s {
        ChargeState::Charging => Msg::SettingsBatteryCharging,
        ChargeState::Full => Msg::SettingsBatteryFull,
        ChargeState::Discharging => Msg::SettingsBatteryDischarging,
        ChargeState::NotCharging => Msg::SettingsBatteryNotCharging,
        ChargeState::Unknown => Msg::SettingsBatteryUnknown,
    })
}

/// The time ("21:38") and the date in the UI language, from the local time.
pub fn clock_and_date() -> (String, String) {
    match system::local_time() {
        Some(t) => (
            fmt_time(t.hour, t.minute),
            fmt_date(t.year, t.month, t.day, t.weekday),
        ),
        None => (String::new(), String::new()),
    }
}

impl Facts {
    /// The cheap facts, for the status line.
    pub fn status(battery: &Option<Battery>) -> Facts {
        let (clock, date) = clock_and_date();
        Facts {
            clock,
            date,
            battery: battery.as_ref().and_then(|b| b.status().ok()),
            storage: None,
            usb: None,
            about: Vec::new(),
        }
    }

    /// Everything the settings screen shows.
    pub fn read(battery: &Option<Battery>, books: &Path) -> Facts {
        let mut f = Facts::status(battery);
        f.storage = system::storage(books).ok().map(|s| (s.free, s.total));
        let now = aristotle_library::library::now();
        f.usb = system::usb_mode().map(|m| match m {
            UsbMode::Network => Usb::Network,
            UsbMode::Storage { until } => Usb::Storage {
                minutes_left: ((until - now).max(0) as u32).div_ceil(60),
            },
        });
        f.about.push((
            tr(Msg::SettingsAboutSoftware),
            format!("Aristotle {}", env!("CARGO_PKG_VERSION")),
        ));
        f.about
            .push((tr(Msg::SettingsAboutReader), "Kindle Paperwhite 4".into()));
        if let Some(v) = system::firmware_version() {
            f.about.push((tr(Msg::SettingsAboutFirmware), v));
        }
        if let Ok(m) = system::memory() {
            f.about.push((
                tr(Msg::SettingsAboutMemory),
                free_of(m.available_kb * 1024, m.total_kb * 1024),
            ));
        }
        f
    }

    pub fn status_line(&self) -> StatusLine<'_> {
        StatusLine {
            clock: &self.clock,
            battery: self.battery.map(|b| b.capacity as f32 / 100.0),
            charging: self.battery.is_some_and(|b| b.on_charger()),
        }
    }

    pub fn battery_info(&self) -> Option<BatteryInfo<'static>> {
        self.battery.map(|b| BatteryInfo {
            level: b.capacity as f32 / 100.0,
            charging: b.on_charger(),
            state: state_label(b.state),
            voltage_mv: b.voltage_mv,
            temperature_c: b.temperature_c,
        })
    }
}
