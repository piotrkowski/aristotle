// SPDX-License-Identifier: AGPL-3.0-or-later
//! Battery and frontlight through sysfs.
//!
//! Paths verified on our PW4: the ROHM BD71827 PMIC exposes the battery as
//! `/sys/class/power_supply/bd71827_bat` and the charger as `bd71827_ac`; the
//! TI LM3695 frontlight is `/sys/class/backlight/bl` with a 0–2047 range.
//! Devices are looked up by class so other Rex units with different names
//! fail loudly instead of writing to the wrong node.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const POWER_SUPPLY: &str = "/sys/class/power_supply";
const BACKLIGHT: &str = "/sys/class/backlight";

fn read_trimmed(path: &Path) -> io::Result<String> {
    Ok(fs::read_to_string(path)?.trim().to_string())
}

fn read_num(path: &Path) -> io::Result<i64> {
    read_trimmed(path)?.parse().map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {e}", path.display()),
        )
    })
}

/// First entry of a sysfs class directory whose `type` file (if any) matches.
fn find_class_entry(class: &str, wanted_type: Option<&str>) -> io::Result<PathBuf> {
    for entry in fs::read_dir(class)? {
        let path = entry?.path();
        match wanted_type {
            Some(t) if read_trimmed(&path.join("type")).ok().as_deref() != Some(t) => continue,
            _ => return Ok(path),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("no matching device in {class}"),
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChargeState {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown,
}

#[derive(Clone, Copy, Debug)]
pub struct BatteryStatus {
    /// State of charge, 0–100 %.
    pub capacity: u8,
    pub state: ChargeState,
    pub voltage_mv: u32,
    /// Battery temperature in °C, as reported by the PMIC.
    pub temperature_c: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct BatteryLevel {
    pub capacity: u8,
    pub state: ChargeState,
}

impl BatteryLevel {
    pub fn on_charger(&self) -> bool {
        matches!(self.state, ChargeState::Charging | ChargeState::Full)
    }
}

pub struct Battery {
    dir: PathBuf,
}

impl Battery {
    pub fn open() -> io::Result<Self> {
        Ok(Self {
            dir: find_class_entry(POWER_SUPPLY, Some("Battery"))?,
        })
    }

    /// Read cached charge data without the live voltage measurement.
    pub fn level(&self) -> io::Result<BatteryLevel> {
        let state = match read_trimmed(&self.dir.join("status"))?.as_str() {
            "Charging" => ChargeState::Charging,
            "Discharging" => ChargeState::Discharging,
            "Full" => ChargeState::Full,
            "Not charging" => ChargeState::NotCharging,
            _ => ChargeState::Unknown,
        };
        Ok(BatteryLevel {
            capacity: read_num(&self.dir.join("capacity"))?.clamp(0, 100) as u8,
            state,
        })
    }

    pub fn status(&self) -> io::Result<BatteryStatus> {
        let level = self.level()?;
        Ok(BatteryStatus {
            capacity: level.capacity,
            state: level.state,
            voltage_mv: (read_num(&self.dir.join("voltage_now"))? / 1000) as u32,
            temperature_c: read_num(&self.dir.join("temp")).unwrap_or(0) as i32,
        })
    }
}

impl BatteryStatus {
    /// Whether energy is flowing in (charging, or topped up on a charger).
    pub fn on_charger(&self) -> bool {
        matches!(self.state, ChargeState::Charging | ChargeState::Full)
    }
}

/// The frontlight, in raw LM3695 steps (`0..=max`, 2047 on our unit). How
/// the stock UI's brightness slider maps onto these steps is not known yet;
/// UIs use the perceptual steps of [`light_raw`] and [`light_step`].
pub struct Frontlight {
    dir: PathBuf,
    max: u32,
}

impl Frontlight {
    pub fn open() -> io::Result<Self> {
        let dir = find_class_entry(BACKLIGHT, None)?;
        let max = read_num(&dir.join("max_brightness"))? as u32;
        Ok(Self { dir, max })
    }

    pub fn max(&self) -> u32 {
        self.max
    }

    pub fn brightness(&self) -> io::Result<u32> {
        Ok(read_num(&self.dir.join("brightness"))? as u32)
    }

    /// Set raw brightness, clamped to `0..=max`. 0 turns the light off.
    pub fn set_brightness(&self, value: u32) -> io::Result<()> {
        fs::write(self.dir.join("brightness"), value.min(self.max).to_string())
    }

    /// The current brightness as a perceptual step, `0..=LIGHT_STEPS`.
    pub fn step(&self) -> io::Result<u32> {
        Ok(light_step(self.brightness()?, self.max))
    }

    /// Set a perceptual step, `0..=LIGHT_STEPS` (clamped); 0 is off.
    pub fn set_step(&self, step: u32) -> io::Result<()> {
        self.set_brightness(light_raw(step, self.max))
    }
}

/// Steps of the brightness control above "off", as many as the stock UI's.
pub const LIGHT_STEPS: u32 = 24;

/// Raw LM3695 value for a perceptual brightness step.
///
/// The LM3695 drives the LEDs with a current proportional to its 11-bit
/// register, so light output is about linear in the raw value, while the
/// eye judges brightness roughly by its cube root. A linear slider would
/// crowd every useful night level into its first few percent (a comfortable
/// level in the dark is about raw 20 of 2047). So the steps are equal in CIE 1976 lightness:
/// step `s` targets L* = 100 s / 24, and the raw value is that L*'s relative
/// luminance times `max`. The first steps fall in L*'s linear toe (9, 19,
/// 30, 44, ... of 2047), fine enough for reading in the dark; the last step
/// is the full range. Whether the LM3695 on our unit really maps linearly
/// (it also has an exponential mode) is an inference from one reading
/// setting (raw 20), not a measurement.
pub fn light_raw(step: u32, max: u32) -> u32 {
    let step = step.min(LIGHT_STEPS);
    if step == 0 || max == 0 {
        return 0;
    }
    let l = 100.0 * step as f64 / LIGHT_STEPS as f64;
    let y = if l > 8.0 {
        ((l + 16.0) / 116.0).powi(3)
    } else {
        l / 903.3
    };
    // A step above "off" never rounds down to off.
    ((y * max as f64).round() as u32).clamp(1, max)
}

/// The perceptual step nearest to a raw value (inverse of [`light_raw`]).
/// Any lit value is at least step 1.
pub fn light_step(raw: u32, max: u32) -> u32 {
    if raw == 0 || max == 0 {
        return 0;
    }
    let y = raw.min(max) as f64 / max as f64;
    let l = if y > 0.008856 {
        116.0 * y.cbrt() - 16.0
    } else {
        903.3 * y
    };
    ((l * LIGHT_STEPS as f64 / 100.0).round() as u32).clamp(1, LIGHT_STEPS)
}

/// Whether a charger (USB or mains) is connected: the `online` file of the
/// first `power_supply` entry of that type (`bd71827_ac` on our unit).
pub fn charger_online() -> io::Result<bool> {
    for entry in fs::read_dir(POWER_SUPPLY)? {
        let path = entry?.path();
        let kind = read_trimmed(&path.join("type")).unwrap_or_default();
        if kind == "Mains" || kind == "USB" {
            if let Ok(v) = read_num(&path.join("online")) {
                return Ok(v != 0);
            }
        }
    }
    Ok(false)
}

/// Keeps `powerd` from starting the stock screensaver over our UI, through
/// the `com.lab126.powerd preventScreenSaver` property (what the USB-network
/// extension sets, and KOReader while it runs). A runtime property: a reboot
/// or a `powerd` restart clears it. Runs the stock `lipc-set-prop`.
pub fn prevent_screensaver(on: bool) -> io::Result<()> {
    crate::powerd::set_prop(
        "com.lab126.powerd",
        "preventScreenSaver",
        if on { "1" } else { "0" },
        false,
    )
}

/// Read `preventScreenSaver`; refuse values that cannot be restored exactly.
pub fn screensaver_prevented() -> io::Result<bool> {
    parse_prevented(&crate::powerd::get_prop(
        "com.lab126.powerd",
        "preventScreenSaver",
        true,
    )?)
}

fn parse_prevented(value: &str) -> io::Result<bool> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid preventScreenSaver value",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: u32 = 2047;

    #[test]
    fn screensaver_policy_requires_a_boolean_value() {
        assert!(!parse_prevented("0").unwrap());
        assert!(parse_prevented("1").unwrap());
        for value in ["", "2", "-1", "error", "0 1"] {
            assert_eq!(
                parse_prevented(value).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }

    #[test]
    fn light_curve_ends_and_monotony() {
        assert_eq!(light_raw(0, MAX), 0);
        assert_eq!(light_raw(LIGHT_STEPS, MAX), MAX);
        assert_eq!(light_raw(LIGHT_STEPS + 5, MAX), MAX);
        let raws: Vec<u32> = (0..=LIGHT_STEPS).map(|s| light_raw(s, MAX)).collect();
        assert!(raws.windows(2).all(|w| w[0] < w[1]), "{raws:?}");
    }

    #[test]
    fn light_curve_is_perceptually_even() {
        // A typical night level (raw 20) is the second step.
        assert_eq!(light_step(20, MAX), 2);
        assert!(light_raw(1, MAX) <= 12);
        // Equal steps in L*: neighbouring raw values grow by a shrinking
        // ratio as the light gets brighter, unlike a linear curve.
        let r = |s| light_raw(s, MAX) as f64;
        assert!(r(2) / r(1) > r(13) / r(12));
        assert!(r(13) / r(12) > r(24) / r(23));
        // Halfway on the control is about a fifth of the current.
        let mid = r(12) / MAX as f64;
        assert!((0.15..0.22).contains(&mid), "{mid}");
    }

    #[test]
    fn light_step_inverts_light_raw() {
        for max in [255, 1023, MAX] {
            for s in 0..=LIGHT_STEPS {
                assert_eq!(light_step(light_raw(s, max), max), s, "max {max}");
            }
        }
        assert_eq!(light_step(1, MAX), 1, "a lit LED is never shown as off");
        assert_eq!(light_step(5000, MAX), LIGHT_STEPS);
        assert_eq!(light_step(0, MAX), 0);
    }
}
