//! System facts for the settings screen: storage, memory, firmware, local
//! time, and the USB mode of the aristotle agent.
//!
//! Everything here is read-only except [`set_usb_mode`], which asks the
//! aristotle agent (`devloop/kindle/aristotle-usbnet/agent/`) to switch through
//! its own `aristotlectl`, the same command a developer runs over SSH.

use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// Size and free space of a filesystem, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Storage {
    pub total: u64,
    /// Free for unprivileged users (`f_bavail`), what a copy can use.
    pub free: u64,
}

/// `statvfs` of the filesystem holding `path` (the userstore is `/mnt/us`).
pub fn storage(path: &Path) -> io::Result<Storage> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // SAFETY: statvfs only writes into the struct passed; all-zero is valid.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` lives for the call.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let block = st.f_frsize as u64;
    Ok(Storage {
        total: st.f_blocks as u64 * block,
        free: st.f_bavail as u64 * block,
    })
}

/// Memory from `/proc/meminfo`, in kilobytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Memory {
    pub total_kb: u64,
    pub available_kb: u64,
}

pub fn memory() -> io::Result<Memory> {
    Ok(parse_meminfo(&fs::read_to_string("/proc/meminfo")?))
}

fn parse_meminfo(s: &str) -> Memory {
    let mut m = Memory::default();
    for line in s.lines() {
        let mut it = line.split_whitespace();
        let (Some(key), Some(v)) = (it.next(), it.next()) else {
            continue;
        };
        let v = v.parse().unwrap_or(0);
        match key {
            "MemTotal:" => m.total_kb = v,
            "MemAvailable:" => m.available_kb = v,
            _ => {}
        }
    }
    m
}

/// The stock firmware version, e.g. "5.13.6", from `/etc/prettyversion.txt`
/// ("Kindle 5.13.6 (4110410039) ..." on firmware 5.x, per KOReader's Kindle
/// support [inference for our unit]); `None` off the device.
pub fn firmware_version() -> Option<String> {
    let s = fs::read_to_string("/etc/prettyversion.txt").ok()?;
    parse_pretty_version(&s)
}

fn parse_pretty_version(s: &str) -> Option<String> {
    s.split_whitespace()
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()) && w.contains('.'))
        .map(String::from)
}

/// Broken-down local time. musl reads the zone from `TZ` or `/etc/localtime`
/// (a link to `/var/local/system/tz` on the Kindle).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LocalTime {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
    pub day: u32,
    /// 0 is Sunday, as in C.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
}

pub fn local_time() -> Option<LocalTime> {
    // SAFETY: time and localtime_r only write into the values passed.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return None;
        }
        Some(LocalTime {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u32,
            day: tm.tm_mday as u32,
            weekday: tm.tm_wday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
        })
    }
}

/// How the USB port is used, as the aristotle agent keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbMode {
    /// USB network with SSH (the agent's default).
    Network,
    /// USB drive until the given UNIX time, then back to the network.
    Storage { until: i64 },
}

/// The agent's state directory (`/var/local/aristotle`).
pub const ARISTOTLE_DIR: &str = "/var/local/aristotle";

/// The mode the agent applies, from its `usb-mode` file. `None` when the
/// agent is not installed.
pub fn usb_mode() -> Option<UsbMode> {
    let dir = Path::new(ARISTOTLE_DIR);
    if !dir.join("aristotlectl").exists() {
        return None;
    }
    let s = fs::read_to_string(dir.join("usb-mode")).unwrap_or_default();
    Some(parse_usb_mode(&s, now()))
}

fn parse_usb_mode(s: &str, now: i64) -> UsbMode {
    let mut it = s.split_whitespace();
    match (it.next(), it.next().and_then(|v| v.parse::<i64>().ok())) {
        (Some("storage"), Some(until)) if until > now => UsbMode::Storage { until },
        _ => UsbMode::Network,
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Ask the agent for a USB mode: `aristotlectl mode net`, or `mode storage N`
/// for drive mode lasting `minutes`. The agent applies it within 15 s. In
/// drive mode SSH drops and the device cannot write `/mnt/us` while the host
/// has it mounted.
pub fn set_usb_mode(storage_minutes: Option<u32>) -> io::Result<()> {
    let ctl = Path::new(ARISTOTLE_DIR).join("aristotlectl");
    let mut cmd = Command::new("/bin/sh");
    cmd.arg(&ctl).arg("mode");
    match storage_minutes {
        Some(m) => cmd.arg("storage").arg(m.to_string()),
        None => cmd.arg("net"),
    };
    let status = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("aristotlectl: {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meminfo() {
        let m = parse_meminfo("MemTotal:  505704 kB\nMemFree: 1 kB\nMemAvailable:   401000 kB\n");
        assert_eq!(
            m,
            Memory {
                total_kb: 505704,
                available_kb: 401000
            }
        );
    }

    #[test]
    fn pretty_version() {
        assert_eq!(
            parse_pretty_version("Kindle 5.13.6 (4110410039) Thu Apr 8 2021\n").as_deref(),
            Some("5.13.6")
        );
        assert_eq!(parse_pretty_version("nothing here"), None);
    }

    #[test]
    fn usb_mode_file() {
        assert_eq!(parse_usb_mode("", 100), UsbMode::Network);
        assert_eq!(
            parse_usb_mode("storage 200\n", 100),
            UsbMode::Storage { until: 200 }
        );
        // Expired: the agent is already back on the network.
        assert_eq!(parse_usb_mode("storage 50", 100), UsbMode::Network);
    }

    #[test]
    fn storage_of_the_build_dir() {
        let s = storage(Path::new(".")).unwrap();
        assert!(s.total > 0 && s.free <= s.total);
    }
}
