// SPDX-License-Identifier: AGPL-3.0-or-later
//! `aristotle-observe`: measure any UI on the device, stock or ours, the same way.
//!
//! It turns on the EPDC driver's debug log, reads `/dev/kmsg` and the
//! touchscreen side by side (without grabbing it, so the UI under test still
//! gets every touch), and pairs each touch interaction with the panel updates
//! that follow it. It can also inject touches, so a scenario runs identically
//! on the stock UI and on ours.
//!
//! ```text
//! aristotle-observe watch SECONDS          observe real use
//! aristotle-observe run SCRIPT [--trace PATH] inject a script and save timings
//! aristotle-observe replay TRACE [--json]  analyze a saved trace on any host
//! aristotle-observe tap X Y                inject one tap
//! aristotle-observe caps                   the axes the touchscreen declares
//! aristotle-observe raw SECONDS            the touchscreen's raw evdev stream
//! aristotle-observe bench ...              EPDC benchmarks (`bench` module)
//! ```
//!
//! Script lines: `tap X Y`, `hold X Y MS`, `swipe X1 Y1 X2 Y2 MS`,
//! `drag X1 Y1 X2 Y2 HOLD MS` (rest HOLD ms, then move over MS ms without
//! lifting: a long press that extends a text selection), `wait MS`, `label TEXT`, `go-home` (stock Home via the app manager, so a
//! scenario starts from a known state), `snap NAME` (save the screen and
//! the active app, to verify each step), `# comment`.
//!
//! For every interaction it reports: time from release to the first update
//! starting and finishing, time until the screen settles (no update for
//! 1.5 s), number of updates, flashes (full GC16) and updated area. The
//! driver logs with its own clock, so `kmsg` timestamps are calibrated against
//! CLOCK_MONOTONIC by writing a marker line first.

mod bench;
mod kmsg;
mod report;
mod trace;

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::os::fd::AsRawFd;
use std::thread;
use std::time::Duration;

use aristotle_hal::input::{find_touchscreen, monotonic_us, Phase, TouchScreen, Transform};
use kmsg::{parse_record, UpdateLog, EPDC_DEBUG};

/// Maximum quiet gap between release or the last update and the next update.
const SETTLE_US: u64 = 1_500_000;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Interaction {
    label: String,
    down_us: u64,
    up_us: u64,
    x: u32,
    y: u32,
}

// ---------------------------------------------------------------- injection

/// Kernel 4.1 `struct input_event` on 32-bit ARM (16 bytes).
#[repr(C)]
struct RawEvent {
    sec: u32,
    usec: u32,
    kind: u16,
    code: u16,
    value: i32,
}

const EV_SYN: u16 = 0;
const EV_ABS: u16 = 3;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TOUCH_MAJOR: u16 = 0x30;
const ABS_MT_WIDTH_MAJOR: u16 = 0x32;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_PRESSURE: u16 = 0x3a;

/// Writes synthetic touches into the real touchscreen's evdev node. The
/// kernel injects them through the input core, so every reader (X, the
/// framework, us) sees them exactly like physical touches; no uinput needed.
///
/// The PW4's Goodix declares only multi-touch axes: slot, touch and width
/// major, position, tool type, tracking id and pressure. It has no BTN_TOUCH
/// and no ABS_X/ABS_Y, and the input core silently drops undeclared events.
/// A contact must therefore carry pressure and size, or the stock UI ignores
/// it. Without them, taps only worked when slot 0 still held pressure from
/// the last real finger.
struct Injector {
    dev: File,
    tracking_id: i32,
}

impl Injector {
    fn open() -> io::Result<Self> {
        let (path, _) = find_touchscreen()?;
        // Tracking ids must stay inside the declared 0..=255 range (the stock
        // UI ignores contacts outside it) and differ between runs, because the
        // input core drops ABS events whose value did not change.
        let tracking_id = (monotonic_us() / 1000 % 250) as i32 + 1;
        Ok(Self {
            dev: OpenOptions::new().write(true).open(path)?,
            tracking_id,
        })
    }

    fn emit(&mut self, kind: u16, code: u16, value: i32) -> io::Result<()> {
        let ev = RawEvent {
            sec: 0,
            usec: 0,
            kind,
            code,
            value,
        };
        // SAFETY: RawEvent is a 16-byte #[repr(C)] POD struct.
        let bytes =
            unsafe { std::slice::from_raw_parts((&ev as *const RawEvent).cast::<u8>(), 16) };
        self.dev.write_all(bytes)
    }

    fn down(&mut self, x: u32, y: u32) -> io::Result<()> {
        self.tracking_id = self.tracking_id % 255 + 1;
        self.emit(EV_ABS, ABS_MT_SLOT, 0)?;
        self.emit(EV_ABS, ABS_MT_TRACKING_ID, self.tracking_id)?;
        self.emit(EV_ABS, ABS_MT_TOUCH_MAJOR, 30)?;
        self.emit(EV_ABS, ABS_MT_WIDTH_MAJOR, 30)?;
        self.emit(EV_ABS, ABS_MT_PRESSURE, 40)?;
        self.moved(x, y)?;
        self.emit(EV_SYN, 0, 0)
    }

    fn moved(&mut self, x: u32, y: u32) -> io::Result<()> {
        self.emit(EV_ABS, ABS_MT_POSITION_X, x as i32)?;
        self.emit(EV_ABS, ABS_MT_POSITION_Y, y as i32)
    }

    fn up(&mut self) -> io::Result<()> {
        self.emit(EV_ABS, ABS_MT_SLOT, 0)?;
        self.emit(EV_ABS, ABS_MT_PRESSURE, 0)?;
        self.emit(EV_ABS, ABS_MT_TRACKING_ID, -1)?;
        self.emit(EV_SYN, 0, 0)
    }

    fn tap(&mut self, x: u32, y: u32, hold_ms: u64) -> io::Result<()> {
        self.down(x, y)?;
        thread::sleep(Duration::from_millis(hold_ms));
        self.up()
    }

    fn swipe(&mut self, from: (u32, u32), to: (u32, u32), ms: u64) -> io::Result<()> {
        self.drag(from, to, 0, ms)
    }

    /// Down at `from`, rest `hold` ms, move to `to` over `ms`, lift.
    fn drag(&mut self, from: (u32, u32), to: (u32, u32), hold: u64, ms: u64) -> io::Result<()> {
        let steps = (ms / 16).max(2);
        self.down(from.0, from.1)?;
        thread::sleep(Duration::from_millis(hold));
        for i in 1..=steps {
            let t = i as i64;
            let s = steps as i64;
            let x = from.0 as i64 + (to.0 as i64 - from.0 as i64) * t / s;
            let y = from.1 as i64 + (to.1 as i64 - from.1 as i64) * t / s;
            thread::sleep(Duration::from_millis(ms / steps));
            self.moved(x as u32, y as u32)?;
            self.emit(EV_SYN, 0, 0)?;
        }
        self.up()
    }
}

// ---------------------------------------------------------------- observing

struct Observer {
    kmsg: File,
    /// kmsg timestamp minus CLOCK_MONOTONIC, in microseconds.
    offset: i64,
    touch: TouchScreen,
    log: UpdateLog,
    interactions: Vec<Interaction>,
    open: Option<Interaction>,
    label: String,
    /// Count only the interactions we inject. Set by scripts: an app that
    /// grabs the touchscreen (Aristotle does) hides touches from us.
    injected_only: bool,
    lost_records: u64,
}

fn set_epdc_debug(on: bool) -> io::Result<()> {
    fs::write(EPDC_DEBUG, if on { "1" } else { "0" })
}

impl Observer {
    fn start() -> io::Result<Self> {
        set_epdc_debug(true)?;
        let mut kmsg = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kmsg")?;
        // Skip history: only records written from now on.
        // SAFETY: lseek on a valid fd.
        unsafe { libc::lseek(kmsg.as_raw_fd(), 0, libc::SEEK_END) };
        let mark = monotonic_us();
        writeln!(kmsg, "aristotle-observe-sync {mark}")?;
        let mut offset = 0;
        let mut buf = vec![0u8; 8192];
        for _ in 0..100 {
            let n = kmsg.read(&mut buf)?;
            let record = String::from_utf8_lossy(&buf[..n]);
            if let Some((ts, msg)) = parse_record(&record) {
                if msg.ends_with(&format!("aristotle-observe-sync {mark}")) {
                    offset = ts as i64 - mark as i64;
                    break;
                }
            }
        }
        // SAFETY: switch kmsg to non-blocking for the poll loop.
        unsafe {
            let flags = libc::fcntl(kmsg.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(kmsg.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        // Don't grab: the UI under test must keep receiving touches.
        let touch = TouchScreen::open(1072, 1448, Transform::default())?;
        Ok(Self {
            kmsg,
            offset,
            touch,
            log: UpdateLog::default(),
            interactions: Vec::new(),
            open: None,
            label: String::new(),
            injected_only: false,
            lost_records: 0,
        })
    }

    /// Read whatever is pending on both sources, for up to `timeout`.
    fn pump(&mut self, timeout: Duration) -> io::Result<()> {
        for ev in self.touch.poll(timeout)? {
            if self.injected_only {
                continue;
            }
            match ev.phase {
                Phase::Down if ev.id == 0 => {
                    self.open = Some(Interaction {
                        label: self.label.clone(),
                        down_us: ev.time_us,
                        up_us: 0,
                        x: ev.x,
                        y: ev.y,
                    });
                }
                Phase::Up if ev.id == 0 => {
                    if let Some(mut i) = self.open.take() {
                        i.up_us = ev.time_us;
                        self.interactions.push(i);
                    }
                }
                _ => {}
            }
        }
        let mut buf = vec![0u8; 8192];
        loop {
            match self.kmsg.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let record = String::from_utf8_lossy(&buf[..n]).into_owned();
                    if let Some((ts, msg)) = parse_record(&record) {
                        let mono = (ts as i64 - self.offset).max(0) as u64;
                        self.log.feed(mono, msg);
                    }
                }
                Ok(_) => break,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                // EPIPE: records were overwritten before we read them.
                Err(e) if e.raw_os_error() == Some(libc::EPIPE) => {
                    self.lost_records += 1;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Record an interaction we injected between `down_us` and `up_us`.
    fn injected(&mut self, (x, y): (u32, u32), down_us: u64, up_us: u64) {
        self.interactions.push(Interaction {
            label: self.label.clone(),
            down_us,
            up_us,
            x,
            y,
        });
    }

    fn pump_for(&mut self, ms: u64) -> io::Result<()> {
        let until = monotonic_us() + ms * 1000;
        while monotonic_us() < until {
            self.pump(Duration::from_millis(5))?;
        }
        Ok(())
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        let _ = set_epdc_debug(false);
    }
}

// ---------------------------------------------------------------- commands

fn finish(obs: &Observer, trace_path: Option<&str>) -> io::Result<()> {
    let updates = obs.log.updates();
    if let Some(path) = trace_path {
        trace::Capture::new(obs.interactions.clone(), updates.clone(), obs.lost_records)
            .write(path)?;
    }
    report::print(&obs.interactions, &updates, obs.lost_records);
    Ok(())
}

fn run_script(path: &str, trace_path: Option<&str>) -> io::Result<()> {
    let script = io::BufReader::new(File::open(path)?);
    let mut obs = Observer::start()?;
    let mut inj = Injector::open()?;
    obs.injected_only = true;
    obs.pump_for(300)?;
    for line in script.lines() {
        let line = line?;
        let words: Vec<&str> = line.split_whitespace().collect();
        let n = |i: usize| {
            words
                .get(i)
                .and_then(|w| w.parse::<u32>().ok())
                .unwrap_or(0)
        };
        match words.first().copied() {
            Some("label") => obs.label = words[1..].join(" "),
            Some("tap") => {
                let down = monotonic_us();
                inj.tap(n(1), n(2), 60)?;
                obs.injected((n(1), n(2)), down, monotonic_us());
            }
            Some("hold") => {
                let down = monotonic_us();
                inj.tap(n(1), n(2), n(3) as u64)?;
                obs.injected((n(1), n(2)), down, monotonic_us());
            }
            Some("swipe") => {
                let down = monotonic_us();
                inj.swipe((n(1), n(2)), (n(3), n(4)), n(5) as u64)?;
                obs.injected((n(1), n(2)), down, monotonic_us());
            }
            Some("drag") => {
                let down = monotonic_us();
                inj.drag((n(1), n(2)), (n(3), n(4)), n(5) as u64, n(6) as u64)?;
                obs.injected((n(1), n(2)), down, monotonic_us());
            }
            Some("wait") => obs.pump_for(n(1) as u64)?,
            Some("snap") => snap(words.get(1).copied().unwrap_or("snap"))?,
            Some("go-home") => {
                go_home()?;
                obs.pump_for(1500)?;
            }
            _ => continue,
        }
        obs.pump(Duration::from_millis(1))?;
    }
    obs.pump_for(SETTLE_US / 1000 + 500)?;
    finish(&obs, trace_path)
}

/// Where snaps go: the userstore, not `/var/tmp`. `/var` is a 64 MB tmpfs
/// shared with the stock services, and 1.5 MB snaps filled it once.
const SNAP_DIR: &str = "/mnt/us/aristotle-snaps";

/// Save the visible framebuffer (1088-byte stride × 1448 rows) to
/// `SNAP_DIR/NAME.raw`, so each scenario step can be checked by eye with
/// `tools/host/fb2png.py`.
fn snap(name: &str) -> io::Result<()> {
    let mut fb = File::open("/dev/fb0")?;
    let mut buf = vec![0u8; 1088 * 1448];
    fb.read_exact(&mut buf)?;
    fs::create_dir_all(SNAP_DIR)?;
    fs::write(format!("{SNAP_DIR}/{name}.raw"), &buf)?;
    println!("snap {name}: app {}", active_app());
    Ok(())
}

fn active_app() -> String {
    std::process::Command::new("lipc-get-prop")
        .args(["com.lab126.appmgrd", "activeApp"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Put the stock UI on its Home screen through the app manager, without
/// injecting touches, so every scenario starts from a known state.
fn go_home() -> io::Result<()> {
    if active_app().ends_with("booklet.home") {
        return Ok(());
    }
    std::process::Command::new("lipc-set-prop")
        .args([
            "com.lab126.appmgrd",
            "start",
            "app://com.lab126.booklet.home",
        ])
        .status()?;
    for _ in 0..60 {
        if active_app().ends_with("booklet.home") {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "stock Home did not become active",
    ))
}

/// Print which event types, keys and axes the touchscreen declares. The input
/// core silently drops injected events the device does not declare.
fn caps() -> io::Result<()> {
    let (path, name) = find_touchscreen()?;
    let dev = File::open(&path)?;
    let bits = |ev: u32, len: usize| -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        // EVIOCGBIT(ev, len) = _IOC(_IOC_READ, 'E', 0x20 + ev, len)
        let req = 0x8000_0000u32 | ((len as u32) << 16) | (0x45 << 8) | (0x20 + ev);
        // SAFETY: the buffer is `len` bytes, as the request says.
        let rc = unsafe { libc::ioctl(dev.as_raw_fd(), req as _, buf.as_mut_ptr()) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(buf)
    };
    let set = |b: &[u8]| -> Vec<usize> {
        (0..b.len() * 8)
            .filter(|i| b[i / 8] & (1 << (i % 8)) != 0)
            .collect()
    };
    println!("{name} at {path}");
    println!("event types: {:?}", set(&bits(0, 4)?));
    println!("keys: {:x?}", set(&bits(1, 96)?));
    let axes = set(&bits(3, 8)?);
    println!("abs axes: {axes:x?}");
    for axis in axes {
        let mut info = [0i32; 6];
        // EVIOCGABS(axis) = _IOR('E', 0x40 + axis, struct input_absinfo)
        let req = 0x8018_4540u32 + axis as u32;
        // SAFETY: input_absinfo is six i32 values.
        unsafe { libc::ioctl(dev.as_raw_fd(), req as _, info.as_mut_ptr()) };
        println!(
            "  axis {axis:#04x}: value {} min {} max {} fuzz {} flat {} res {}",
            info[0], info[1], info[2], info[3], info[4], info[5]
        );
    }
    Ok(())
}

/// Print the touchscreen's raw evdev stream (type, code, value) so injected
/// sequences can copy exactly what the real controller sends.
fn raw(seconds: u64) -> io::Result<()> {
    let (path, name) = find_touchscreen()?;
    let mut dev = File::open(&path)?;
    println!("raw events from {name} ({path}) for {seconds} s; touch the screen");
    let until = monotonic_us() + seconds * 1_000_000;
    let mut buf = [0u8; 16 * 64];
    while monotonic_us() < until {
        let mut fds = libc::pollfd {
            fd: dev.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        if unsafe { libc::poll(&mut fds, 1, 200) } <= 0 {
            continue;
        }
        let len = dev.read(&mut buf)?;
        for ev in buf[..len].as_chunks::<16>().0 {
            let kind = u16::from_ne_bytes([ev[8], ev[9]]);
            let code = u16::from_ne_bytes([ev[10], ev[11]]);
            let value = i32::from_ne_bytes([ev[12], ev[13], ev[14], ev[15]]);
            let usec = u32::from_ne_bytes([ev[4], ev[5], ev[6], ev[7]]);
            println!("{usec:>7} type {kind:#04x} code {code:#05x} value {value}");
        }
    }
    Ok(())
}

fn watch(seconds: u64, trace_path: Option<&str>) -> io::Result<()> {
    let mut obs = Observer::start()?;
    println!("observing for {seconds} s; touch the screen");
    obs.pump_for(seconds * 1000)?;
    finish(&obs, trace_path)
}

fn trace_path(args: &[String]) -> io::Result<Option<&str>> {
    match args {
        [] => Ok(None),
        [flag, path] if flag == "--trace" => Ok(Some(path)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected --trace PATH",
        )),
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n = |i: usize| args.get(i).and_then(|a| a.parse::<u32>().ok()).unwrap_or(0);
    match args.first().map(String::as_str) {
        Some("watch") => watch(
            n(1).max(1) as u64,
            trace_path(args.get(2..).unwrap_or_default())?,
        ),
        Some("run") => run_script(
            args.get(1)
                .map(String::as_str)
                .unwrap_or("/var/tmp/scenario.txt"),
            trace_path(args.get(2..).unwrap_or_default())?,
        ),
        Some("replay") => trace::replay(&args[1..]),
        Some("tap") => Injector::open()?.tap(n(1), n(2), 60),
        Some("raw") => raw(n(1).max(1) as u64),
        Some("caps") => caps(),
        Some("bench") => bench::run(&args[1..]),
        _ => {
            eprintln!(
                "usage: aristotle-observe watch SECONDS [--trace PATH] | run SCRIPT [--trace PATH] | replay TRACE [--json] | tap X Y | caps | raw SECONDS | {}",
                bench::USAGE
            );
            std::process::exit(2);
        }
    }
}
