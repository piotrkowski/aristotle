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
//! aristotle-observe run SCRIPT             inject a script and observe it
//! aristotle-observe tap X Y                inject one tap
//! ```
//!
//! Script lines: `tap X Y`, `hold X Y MS`, `swipe X1 Y1 X2 Y2 MS`,
//! `wait MS`, `label TEXT`, `go-home` (stock Home via the app manager, so a
//! scenario starts from a known state), `snap NAME` (save the screen and
//! the active app, to verify each step), `# comment`.
//!
//! For every interaction it reports: time from release to the first update
//! starting and finishing, time until the screen settles (no update for
//! 1.5 s), number of updates, flashes (full GC16) and updated area. The
//! driver logs with its own clock, so `kmsg` timestamps are calibrated against
//! CLOCK_MONOTONIC by writing a marker line first.

mod kmsg;

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::os::fd::AsRawFd;
use std::thread;
use std::time::Duration;

use aristotle_hal::input::{find_touchscreen, monotonic_us, Phase, TouchScreen, Transform};
use kmsg::{parse_record, Update, UpdateLog, EPDC_DEBUG};

/// An update belongs to the interaction it follows if it starts within this
/// long after the release (or before the next interaction).
const SETTLE_US: u64 = 1_500_000;

#[derive(Clone, Debug)]
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
        let steps = (ms / 16).max(2);
        self.down(from.0, from.1)?;
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
        })
    }

    /// Read whatever is pending on both sources, for up to `timeout`.
    fn pump(&mut self, timeout: Duration) -> io::Result<()> {
        for ev in self.touch.poll(timeout)? {
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
                Err(e) if e.raw_os_error() == Some(libc::EPIPE) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
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

// ---------------------------------------------------------------- reporting

fn ms(us: u64) -> f64 {
    us as f64 / 1000.0
}

fn report(interactions: &[Interaction], updates: &[Update]) {
    if std::env::var_os("OBSERVE_DEBUG").is_some() {
        eprintln!("parsed {} updates", updates.len());
        for u in updates.iter().take(5) {
            eprintln!(
                "  update start {} end {:?} {} {:?}",
                u.start_us, u.end_us, u.waveform, u.rect
            );
        }
        for i in interactions.iter().take(3) {
            eprintln!("  touch down {} up {}", i.down_us, i.up_us);
        }
    }
    println!(
        "{:<14} {:>9} {:>9} {:>9} {:>4} {:>5} {:>7}  waveforms",
        "interaction", "first>", "first✓", "settled", "upd", "flash", "area%"
    );
    let screen = 1072.0 * 1448.0;
    let (mut firsts, mut settles) = (Vec::new(), Vec::new());
    let mut flashes_total = 0;
    for (k, i) in interactions.iter().enumerate() {
        let next = interactions.get(k + 1).map_or(u64::MAX, |n| n.down_us);
        let mine: Vec<&Update> = updates
            .iter()
            .filter(|u| {
                u.start_us >= i.down_us && u.start_us < next && u.start_us < i.up_us + SETTLE_US
            })
            .collect();
        let label = if i.label.is_empty() {
            format!("({},{})", i.x, i.y)
        } else {
            i.label.clone()
        };
        let Some(first) = mine.first() else {
            println!("{label:<14} no update");
            continue;
        };
        let first_start = first.start_us.saturating_sub(i.up_us);
        let first_end = first
            .end_us
            .unwrap_or(first.start_us)
            .saturating_sub(i.up_us);
        let settled = mine
            .iter()
            .filter_map(|u| u.end_us)
            .max()
            .unwrap_or(first.start_us)
            .saturating_sub(i.up_us);
        let flashes = mine.iter().filter(|u| u.flashes()).count();
        flashes_total += flashes;
        let area: u64 = mine.iter().map(|u| u.area()).sum();
        let mut wf: Vec<&str> = mine.iter().map(|u| u.waveform.as_str()).collect();
        wf.dedup();
        println!(
            "{label:<14} {:>8.0}ms {:>8.0}ms {:>8.0}ms {:>4} {:>5} {:>6.0}%  {}",
            ms(first_start),
            ms(first_end),
            ms(settled),
            mine.len(),
            flashes,
            area as f64 / screen * 100.0,
            wf.join(",")
        );
        firsts.push(first_end);
        settles.push(settled);
    }
    firsts.sort_unstable();
    settles.sort_unstable();
    if !firsts.is_empty() {
        let med = |v: &[u64]| ms(v[v.len() / 2]);
        println!(
            "summary: {} interactions, median release->first update done {:.0} ms, median settled {:.0} ms, flashes {}",
            firsts.len(), med(&firsts), med(&settles), flashes_total
        );
    }
}

// ---------------------------------------------------------------- commands

fn run_script(path: &str) -> io::Result<()> {
    let script = io::BufReader::new(File::open(path)?);
    let mut obs = Observer::start()?;
    let mut inj = Injector::open()?;
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
            Some("tap") => inj.tap(n(1), n(2), 60)?,
            Some("hold") => inj.tap(n(1), n(2), n(3) as u64)?,
            Some("swipe") => inj.swipe((n(1), n(2)), (n(3), n(4)), n(5) as u64)?,
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
    report(&obs.interactions, &obs.log.done);
    Ok(())
}

/// Save the visible framebuffer (1088-byte stride × 1448 rows) to
/// `/var/tmp/snap-NAME.raw`, so each scenario step can be checked by eye
/// with `devloop/host/fb2png.py`.
fn snap(name: &str) -> io::Result<()> {
    let mut fb = File::open("/dev/fb0")?;
    let mut buf = vec![0u8; 1088 * 1448];
    fb.read_exact(&mut buf)?;
    fs::write(format!("/var/tmp/snap-{name}.raw"), &buf)?;
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

fn watch(seconds: u64) -> io::Result<()> {
    let mut obs = Observer::start()?;
    println!("observing for {seconds} s; touch the screen");
    obs.pump_for(seconds * 1000)?;
    report(&obs.interactions, &obs.log.done);
    Ok(())
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n = |i: usize| args.get(i).and_then(|a| a.parse::<u32>().ok()).unwrap_or(0);
    match args.first().map(String::as_str) {
        Some("watch") => watch(n(1).max(1) as u64),
        Some("run") => run_script(
            args.get(1)
                .map(String::as_str)
                .unwrap_or("/var/tmp/scenario.txt"),
        ),
        Some("tap") => Injector::open()?.tap(n(1), n(2), 60),
        Some("raw") => raw(n(1).max(1) as u64),
        Some("caps") => caps(),
        _ => {
            eprintln!("usage: aristotle-observe watch SECONDS | run SCRIPT | tap X Y");
            std::process::exit(2);
        }
    }
}
