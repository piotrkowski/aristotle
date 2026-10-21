// SPDX-License-Identifier: AGPL-3.0-or-later
//! Touch input from the evdev touchscreen (Goodix gt9xx or Parade cyttsp5,
//! depending on the unit). The device is found by name, never by `eventN`.

use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::time::Duration;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const BTN_TOUCH: u16 = 0x14a;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;

/// `_IOW('E', 0x90, int)`
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;
/// `_IOW('E', 0xa0, int)`: select the clock used for event timestamps.
const EVIOCSCLOCKID: libc::c_ulong = 0x4004_45a0;

/// `EVIOCGMTSLOTS(len)`, `_IOC(_IOC_READ, 'E', 0x0a, len)`: the values of
/// one multi-touch axis in every slot.
const fn eviocgmtslots(len: usize) -> libc::c_ulong {
    0x8000_450a | ((len as libc::c_ulong) << 16)
}

/// `_IOR('E', 0x40 + abs, struct input_absinfo)`
const fn eviocgabs(abs: u16) -> libc::c_ulong {
    0x8018_4540 + abs as libc::c_ulong
}

/// Touch controllers we know how to drive, matched against evdev names.
const TOUCH_NAMES: &[&str] = &["goodix-ts", "cyttsp5_mt", "cyttsp5"];
const MAX_SLOTS: usize = 10;

/// Kernel 4.1 `struct input_event` on 32-bit ARM: a 32-bit `timeval`, so it is
/// 16 bytes. Don't use `libc::input_event`: musl's time64 layout is 24 bytes.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RawEvent {
    sec: u32,
    usec: u32,
    kind: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct AbsInfo {
    pub value: i32,
    pub minimum: i32,
    pub maximum: i32,
    pub fuzz: i32,
    pub flat: i32,
    pub resolution: i32,
}

/// Maps raw panel coordinates to framebuffer coordinates.
#[derive(Clone, Copy, Debug, Default)]
pub struct Transform {
    pub swap_xy: bool,
    pub mirror_x: bool,
    pub mirror_y: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Down,
    Move,
    Up,
}

#[derive(Clone, Copy, Debug)]
pub struct TouchEvent {
    pub phase: Phase,
    /// Slot index of the contact.
    pub id: usize,
    pub x: u32,
    pub y: u32,
    pub raw_x: i32,
    pub raw_y: i32,
    /// Kernel timestamp of the report, CLOCK_MONOTONIC microseconds
    /// (comparable with [`monotonic_us`]).
    pub time_us: u64,
}

/// CLOCK_MONOTONIC in microseconds, the clock touch timestamps use.
pub fn monotonic_us() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid out-pointer for clock_gettime.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000
}

#[derive(Clone, Copy, Default)]
struct Slot {
    active: bool,
    was_active: bool,
    dirty: bool,
    x: i32,
    y: i32,
}

pub struct TouchScreen {
    file: File,
    path: String,
    name: String,
    abs_x: AbsInfo,
    abs_y: AbsInfo,
    screen: (u32, u32),
    transform: Transform,
    slots: [Slot; MAX_SLOTS],
    slot: usize,
    grabbed: bool,
    raw_events: u64,
}

/// Find the event node of the first known touch controller.
pub fn find_touchscreen() -> io::Result<(String, String)> {
    let devices = fs::read_to_string("/proc/bus/input/devices")?;
    for block in devices.split("\n\n") {
        let name = block
            .lines()
            .find_map(|l| l.strip_prefix("N: Name=\""))
            .map(|n| n.trim_end_matches('"').to_string());
        let handler = block
            .lines()
            .find_map(|l| l.strip_prefix("H: Handlers="))
            .and_then(|h| {
                h.split_whitespace()
                    .find(|t| t.starts_with("event"))
                    .map(str::to_string)
            });
        if let (Some(name), Some(handler)) = (name, handler) {
            if TOUCH_NAMES.contains(&name.as_str()) {
                return Ok((format!("/dev/input/{handler}"), name));
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no known touchscreen in /proc/bus/input/devices",
    ))
}

/// The kernel's value of the multi-touch axis `code` in each slot.
fn mt_slots(file: &File, code: u16) -> io::Result<[i32; MAX_SLOTS]> {
    let mut buf = [0i32; MAX_SLOTS + 1];
    buf[0] = code as i32;
    // SAFETY: EVIOCGMTSLOTS reads the code from the first 4 bytes and fills
    // at most len - 4 bytes of values after it; len is the buffer's size.
    let rc = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            eviocgmtslots(std::mem::size_of_val(&buf)) as _,
            buf.as_mut_ptr(),
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut values = [0; MAX_SLOTS];
    values.copy_from_slice(&buf[1..]);
    Ok(values)
}

fn abs_info(file: &File, axis: u16) -> io::Result<AbsInfo> {
    let mut info = AbsInfo::default();
    // SAFETY: EVIOCGABS fills a 24-byte input_absinfo, which AbsInfo mirrors.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), eviocgabs(axis) as _, &mut info) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(info)
    }
}

impl TouchScreen {
    /// Open the touchscreen and map it onto a `width` x `height` framebuffer.
    pub fn open(width: u32, height: u32, transform: Transform) -> io::Result<Self> {
        let (path, name) = find_touchscreen()?;
        let file = File::open(&path)?;
        // Timestamps default to CLOCK_REALTIME; monotonic makes them comparable
        // with `monotonic_us()` for latency measurements.
        // SAFETY: EVIOCSCLOCKID takes a pointer to an int clock id.
        let clock: libc::c_int = libc::CLOCK_MONOTONIC;
        if unsafe { libc::ioctl(file.as_raw_fd(), EVIOCSCLOCKID as _, &clock) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let (abs_x, abs_y) = match (
            abs_info(&file, ABS_MT_POSITION_X),
            abs_info(&file, ABS_MT_POSITION_Y),
        ) {
            (Ok(x), Ok(y)) if x.maximum > 0 && y.maximum > 0 => (x, y),
            _ => (abs_info(&file, ABS_X)?, abs_info(&file, ABS_Y)?),
        };
        // The input core drops a value equal to the last one it passed on
        // for that slot, before we opened the device too: a first touch at
        // the same x (or y) as the last touch before the app started arrives
        // without it. Start every slot where the kernel says it is. For
        // multi-touch axes that takes EVIOCGMTSLOTS: the kernel keeps their
        // values per slot, and EVIOCGABS reports one it never updates (seen
        // on the device: a first tap at 950,800, where the app before had
        // its last tap, opened the reader's menu).
        let slot = abs_info(&file, ABS_MT_SLOT)
            .map_or(0, |s| s.value.clamp(0, MAX_SLOTS as i32 - 1) as usize);
        let mut slots = [Slot::default(); MAX_SLOTS];
        match (
            mt_slots(&file, ABS_MT_POSITION_X),
            mt_slots(&file, ABS_MT_POSITION_Y),
        ) {
            (Ok(xs), Ok(ys)) => {
                for (s, (x, y)) in slots.iter_mut().zip(xs.into_iter().zip(ys)) {
                    s.x = x;
                    s.y = y;
                }
            }
            _ => {
                slots[slot].x = abs_x.value;
                slots[slot].y = abs_y.value;
            }
        }
        Ok(Self {
            file,
            path,
            name,
            abs_x,
            abs_y,
            screen: (width, height),
            transform,
            slots,
            slot,
            grabbed: false,
            raw_events: 0,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Raw kernel events read so far (diagnostics).
    pub fn raw_events(&self) -> u64 {
        self.raw_events
    }

    pub fn axes(&self) -> (AbsInfo, AbsInfo) {
        (self.abs_x, self.abs_y)
    }

    /// Take exclusive access so the stock UI stops receiving touches.
    pub fn grab(&mut self, exclusive: bool) -> io::Result<()> {
        // SAFETY: EVIOCGRAB takes an int by value.
        let rc = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                EVIOCGRAB as _,
                exclusive as libc::c_int,
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        self.grabbed = exclusive;
        Ok(())
    }

    fn map(&self, raw_x: i32, raw_y: i32) -> (u32, u32) {
        let (w, h) = self.screen;
        let t = self.transform;
        let (ax, ay, rx, ry) = if t.swap_xy {
            (self.abs_y, self.abs_x, raw_y, raw_x)
        } else {
            (self.abs_x, self.abs_y, raw_x, raw_y)
        };
        let scale = |v: i32, info: AbsInfo, size: u32, mirror: bool| -> u32 {
            let span = (info.maximum - info.minimum).max(1) as i64;
            let pos =
                ((v - info.minimum).clamp(0, span as i32) as i64 * (size as i64 - 1) / span) as u32;
            if mirror {
                size - 1 - pos
            } else {
                pos
            }
        };
        (scale(rx, ax, w, t.mirror_x), scale(ry, ay, h, t.mirror_y))
    }

    /// Wait up to `timeout` for input and return the touch events it produced.
    pub fn poll(&mut self, timeout: Duration) -> io::Result<Vec<TouchEvent>> {
        self.poll_with(timeout, None).map(|(events, _)| events)
    }

    /// [`poll`](Self::poll), also waking when `other` becomes readable (the
    /// powerd events' descriptor, say). Returns the touch events and whether
    /// `other` is readable.
    pub fn poll_with(
        &mut self,
        timeout: Duration,
        other: Option<RawFd>,
    ) -> io::Result<(Vec<TouchEvent>, bool)> {
        self.poll_with_worker(timeout, other, None)
    }

    /// Poll touch, power events and worker completions in one wait.
    pub fn poll_with_worker(
        &mut self,
        timeout: Duration,
        other: Option<RawFd>,
        worker: Option<RawFd>,
    ) -> io::Result<(Vec<TouchEvent>, bool)> {
        let mut fds = [
            libc::pollfd {
                fd: self.file.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: other.unwrap_or(-1),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: worker.unwrap_or(-1),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let count = fds.len() as libc::nfds_t;
        // SAFETY: `count` valid pollfds for the duration of the call.
        let n = unsafe {
            libc::poll(
                fds.as_mut_ptr(),
                count,
                if timeout == Duration::MAX {
                    -1
                } else {
                    timeout.as_millis().min(i32::MAX as u128) as i32
                },
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        let other_ready = other.is_some() && fds[1].revents & libc::POLLIN != 0;
        if fds[0].revents & libc::POLLIN == 0 {
            return Ok((Vec::new(), other_ready));
        }
        let mut buf = [0u8; std::mem::size_of::<RawEvent>() * 64];
        let len = self.file.read(&mut buf)?;
        let mut out = Vec::new();
        let (events, _) = buf[..len].as_chunks::<{ std::mem::size_of::<RawEvent>() }>();
        for chunk in events {
            // SAFETY: chunk is exactly one RawEvent; read_unaligned handles alignment.
            let ev: RawEvent = unsafe { std::ptr::read_unaligned(chunk.as_ptr().cast()) };
            self.raw_events += 1;
            self.feed(ev, &mut out);
        }
        Ok((out, other_ready))
    }

    fn feed(&mut self, ev: RawEvent, out: &mut Vec<TouchEvent>) {
        match (ev.kind, ev.code) {
            (EV_ABS, ABS_MT_SLOT) => self.slot = (ev.value.max(0) as usize).min(MAX_SLOTS - 1),
            (EV_ABS, ABS_MT_TRACKING_ID) => {
                let s = &mut self.slots[self.slot];
                s.active = ev.value >= 0;
                s.dirty = true;
            }
            (EV_ABS, ABS_MT_POSITION_X) | (EV_ABS, ABS_X) => {
                let s = &mut self.slots[self.slot];
                s.x = ev.value;
                s.dirty = true;
            }
            (EV_ABS, ABS_MT_POSITION_Y) | (EV_ABS, ABS_Y) => {
                let s = &mut self.slots[self.slot];
                s.y = ev.value;
                s.dirty = true;
            }
            // Some controllers never send TRACKING_ID -1; BTN_TOUCH 0 lifts everything.
            (EV_KEY, BTN_TOUCH) => {
                if ev.value == 0 {
                    for s in self.slots.iter_mut().filter(|s| s.active) {
                        s.active = false;
                        s.dirty = true;
                    }
                } else if !self.slots[self.slot].active {
                    self.slots[self.slot].active = true;
                    self.slots[self.slot].dirty = true;
                }
            }
            (EV_SYN, SYN_REPORT) => {
                for id in 0..MAX_SLOTS {
                    let s = self.slots[id];
                    if !s.dirty {
                        continue;
                    }
                    let phase = match (s.was_active, s.active) {
                        (false, true) => Phase::Down,
                        (true, true) => Phase::Move,
                        (true, false) => Phase::Up,
                        (false, false) => {
                            self.slots[id].dirty = false;
                            continue;
                        }
                    };
                    let (x, y) = self.map(s.x, s.y);
                    let time_us = ev.sec as u64 * 1_000_000 + ev.usec as u64;
                    out.push(TouchEvent {
                        phase,
                        id,
                        x,
                        y,
                        raw_x: s.x,
                        raw_y: s.y,
                        time_us,
                    });
                    self.slots[id].was_active = s.active;
                    self.slots[id].dirty = false;
                }
            }
            _ => {}
        }
    }
}

impl Drop for TouchScreen {
    fn drop(&mut self) {
        if self.grabbed {
            let _ = self.grab(false);
        }
    }
}
