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

pub struct Battery {
    dir: PathBuf,
}

impl Battery {
    pub fn open() -> io::Result<Self> {
        Ok(Self {
            dir: find_class_entry(POWER_SUPPLY, Some("Battery"))?,
        })
    }

    pub fn status(&self) -> io::Result<BatteryStatus> {
        let state = match read_trimmed(&self.dir.join("status"))?.as_str() {
            "Charging" => ChargeState::Charging,
            "Discharging" => ChargeState::Discharging,
            "Full" => ChargeState::Full,
            "Not charging" => ChargeState::NotCharging,
            _ => ChargeState::Unknown,
        };
        Ok(BatteryStatus {
            capacity: read_num(&self.dir.join("capacity"))?.clamp(0, 100) as u8,
            state,
            voltage_mv: (read_num(&self.dir.join("voltage_now"))? / 1000) as u32,
            temperature_c: read_num(&self.dir.join("temp")).unwrap_or(0) as i32,
        })
    }
}

/// The frontlight, in raw LM3695 steps (`0..=max`, 2047 on our unit). How
/// the stock UI's brightness slider maps onto these steps is not known yet.
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
}
