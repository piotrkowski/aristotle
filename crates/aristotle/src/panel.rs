// SPDX-License-Identifier: AGPL-3.0-or-later
//! Getting pixels onto the PW4 panel, by the rules measured on the device
//! (`docs/ui-framework.md`, "E-ink rendering"; `docs/benchmarks.md`):
//!
//! - **Book pages** go out as ONE full-mode update of the WHOLE screen with
//!   the stock reader's waveform, mode 9 (or mode 7, DU4, if chosen). That
//!   is exactly what the stock reader does. Two-step A2 previews and
//!   partial GL16 updates left visible ghosting in real reading. A page is
//!   submitted directly (about 1 ms when the panel is idle) so the caller
//!   can prerender the next page while the panel works.
//! - **UI chrome** (library, menu) goes through the HAL display thread,
//!   which never blocks the input loop: DU for black and white changes,
//!   GL16 for grays (the UI's own hints), and mode 8 full for screen
//!   switches and cleanups (the stock UI's flashing refresh). When the book
//!   page itself changed behind the open menu (new font size, chapter jump),
//!   the frame goes out as a full mode-9 update of the whole screen like a
//!   page turn ([`Content::Page`]).
//! - **Night mode** keeps all of the above, so page turns stay one full
//!   mode-9 update and flashes full mode 8 (the waveforms FBInk names
//!   night waveforms). Gray chrome updates switch from GL16 to GLKW16
//!   (mode 9, partial). Pages are inverted in the copy to the framebuffer
//!   (software night) or by the EPDC (hardware night, [`Panel::set_night`]).
//!   The book's images are inverted beforehand ([`night_page`]), so they
//!   show as they are by day rather than as negatives.
//!
//! The driver runs one update at a time and reads the framebuffer when it
//! processes an update, so a new page is only blitted once the previous
//! page update has finished and the display thread is idle. That is the
//! only place the input loop waits for the panel.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use aristotle_hal::display::Display;
use aristotle_hal::framebuffer::{Framebuffer, Rect as FbRect, Waveform as FbWaveform};
use aristotle_render::page::Page;
use aristotle_ui::ui::{Damage, Ui};
use aristotle_ui::{Rect, Waveform};

use crate::prefs::NightMethod;

/// `page` as it goes to the panel: in night mode, either method, with its
/// images inverted, so the inversion of the whole page brings them back to
/// their day look. Pages without images are not copied.
pub fn night_page(page: &Page, night: bool) -> Cow<'_, Page> {
    if night && !page.images.is_empty() {
        let mut p = page.clone();
        p.invert_images();
        Cow::Owned(p)
    } else {
        Cow::Borrowed(page)
    }
}

/// The stock UI's full flashing refresh (GC16 equivalent on the PW4).
pub const FLASH: FbWaveform = FbWaveform::GCK16;
/// What a UI frame shows, which decides how gray changes reach the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    /// Chrome only: the UI's own DU and GL16 hints.
    Chrome,
    /// The book page behind the menu changed: any gray change goes out as
    /// a full mode-9 update of the whole screen, like a page turn, so the
    /// old page leaves no ghost.
    Page,
}

/// What one UI damage rectangle becomes on the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// A full-mode update of the whole screen.
    FullScreen(FbWaveform),
    /// A partial update of the damaged rectangle, through the display
    /// thread.
    Partial(FbWaveform),
}

/// The transition policy at the panel (`docs/ui-framework.md`,
/// "Transitions"):
///
/// - GC16 (a screen switch, the night switch, a cleanup) is one full
///   update of the whole screen with mode 8, the stock flash;
/// - a gray change of the book page behind the menu is one full update of
///   the whole screen with the turn waveform (mode 9), like a page turn;
/// - A2 (touch-down feedback) and DU (black-and-white changes) are partial;
/// - GL16 (gray chrome) is partial, and becomes GLKW16 (mode 9) at night:
///   the gray waveform Lab126 made for dark pages (FBInk; KOReader's
///   `waveform_night` on Rex and Zelda).
pub fn plan(hint: Waveform, content: Content, night: bool, turn: FbWaveform) -> Request {
    match hint {
        Waveform::GC16 => Request::FullScreen(FLASH),
        Waveform::GL16 if content == Content::Page => Request::FullScreen(turn),
        Waveform::GL16 if night => Request::Partial(FbWaveform::GLKW16),
        other => Request::Partial(FbWaveform(other.mode())),
    }
}

/// Waits for page-update markers on a thread of its own, so the input loop
/// can learn when a page reached the panel (for the log) and block only
/// when the next page needs it.
struct Completions {
    track: Sender<u32>,
    done: Receiver<(u32, Instant)>,
    seen: HashMap<u32, Instant>,
}

impl Completions {
    fn new(fb: &Framebuffer) -> io::Result<Completions> {
        let waiter = fb.waiter()?;
        let (track, pending) = mpsc::channel::<u32>();
        let (finished, done) = mpsc::channel();
        thread::Builder::new()
            .name("page-completions".into())
            .spawn(move || {
                for marker in pending {
                    // On error the driver forgot the marker: report it anyway.
                    let _ = waiter.wait(marker);
                    if finished.send((marker, Instant::now())).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Completions {
            track,
            done,
            seen: HashMap::new(),
        })
    }

    fn collect(&mut self) {
        for (m, t) in self.done.try_iter() {
            self.seen.insert(m, t);
        }
    }

    /// When `marker` finished, blocking until it has.
    fn wait(&mut self, marker: u32) -> Instant {
        loop {
            if let Some(t) = self.seen.remove(&marker) {
                return t;
            }
            match self.done.recv_timeout(Duration::from_secs(3)) {
                Ok((m, t)) => {
                    self.seen.insert(m, t);
                }
                // A lost marker must not hang the reader.
                Err(_) => return Instant::now(),
            }
        }
    }
}

/// A page submitted to the panel and not yet reported as shown.
struct InFlight {
    marker: u32,
    submitted: Instant,
    label: String,
}

/// Submits short fixed sequences of updates, in order, on a thread of its
/// own: the driver blocks a submit while the previous update is on the
/// panel, and the input loop must never wait for that.
struct Sequencer {
    jobs: Sender<Vec<(FbRect, FbWaveform, bool)>>,
    pending: Arc<AtomicUsize>,
}

impl Sequencer {
    fn new(fb: &Framebuffer) -> io::Result<Sequencer> {
        // Markers apart from the display thread's (which count from
        // 0x8000_0000) and the page path's (from 1).
        let mut updater = fb.updater()?.with_marker_base(0xc000_0000);
        let waiter = fb.waiter()?;
        let (jobs, rx) = mpsc::channel::<Vec<(FbRect, FbWaveform, bool)>>();
        let pending = Arc::new(AtomicUsize::new(0));
        let left = pending.clone();
        thread::Builder::new()
            .name("panel-sequence".into())
            .spawn(move || {
                for job in rx {
                    let mut last = None;
                    for (rect, wf, full) in job {
                        match updater.update(rect, wf, full) {
                            Ok(Some(m)) => last = Some(m),
                            Ok(None) => {}
                            Err(e) => eprintln!("aristotle: panel sequence: {e}"),
                        }
                    }
                    if let Some(m) = last {
                        let _ = waiter.wait(m);
                    }
                    left.fetch_sub(1, Ordering::SeqCst);
                }
            })?;
        Ok(Sequencer { jobs, pending })
    }

    fn submit(&self, job: Vec<(FbRect, FbWaveform, bool)>) {
        self.pending.fetch_add(1, Ordering::SeqCst);
        if self.jobs.send(job).is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn busy(&self) -> bool {
        self.pending.load(Ordering::SeqCst) > 0
    }
}

/// The real panel.
pub struct Device {
    fb: Framebuffer,
    display: Display,
    completions: Completions,
    sequencer: Sequencer,
    page: Option<InFlight>,
    /// Waveform for page turns.
    pub turn: FbWaveform,
    /// Night mode and how it inverts, `None` by day.
    night: Option<NightMethod>,
    /// Full-screen full-mode updates submitted so far (pages and flashes).
    full_updates: u64,
}

fn fb_rect(r: Rect) -> FbRect {
    FbRect {
        top: r.y.max(0) as u32,
        left: r.x.max(0) as u32,
        width: r.w.max(0) as u32,
        height: r.h.max(0) as u32,
    }
}

impl Device {
    pub fn open(turn_mode: u32) -> io::Result<Device> {
        let fb = Framebuffer::open("/dev/fb0")?;
        let display = Display::new(&fb)?;
        let completions = Completions::new(&fb)?;
        let sequencer = Sequencer::new(&fb)?;
        Ok(Device {
            fb,
            display,
            completions,
            sequencer,
            page: None,
            turn: FbWaveform(turn_mode),
            night: None,
            full_updates: 0,
        })
    }

    /// Switch night mode. With [`NightMethod::Software`] the caller inverts
    /// the UI (`Ui::set_night`) and book pages are inverted here on their way
    /// to the framebuffer; with [`NightMethod::Hardware`] every update from
    /// now on carries the EPDC inversion flag and nothing is inverted in
    /// memory. Either way the caller follows with a full refresh.
    pub fn set_night(&mut self, night: Option<NightMethod>) {
        self.wait_idle();
        self.fb.set_hw_invert(night == Some(NightMethod::Hardware));
        self.night = night;
    }

    /// Whether our pixels must be inverted in memory (software night).
    pub fn inverts_pixels(&self) -> bool {
        self.night == Some(NightMethod::Software)
    }

    pub fn size(&self) -> (u32, u32) {
        let g = self.fb.geometry();
        (g.width, g.height)
    }

    /// Send a UI frame's damage (see the module documentation).
    pub fn present(&mut self, ui: &Ui, damage: &[Damage], content: Content) -> io::Result<()> {
        for d in damage {
            match plan(d.waveform, content, self.night.is_some(), self.turn) {
                Request::FullScreen(wf) => {
                    // Full-screen, full-mode: clears ghosting.
                    self.blit_ui_wait(ui, Rect::screen())?;
                    let all = self.fb.full_screen();
                    let m = self.fb.update(all, wf, true)?;
                    self.full_updates += 1;
                    self.track(
                        m,
                        if wf == FLASH {
                            "flash"
                        } else {
                            "page (menu open)"
                        },
                    );
                }
                Request::Partial(wf) => {
                    // No wait: the driver latched the last page when it
                    // started it, and the display thread queues behind it.
                    self.blit_ui(ui, d.rect);
                    self.display.damage(fb_rect(d.rect), wf, &[]);
                }
            }
        }
        Ok(())
    }

    /// Show an overlay that just appeared over the page, such as the reader
    /// menu's top bar and bottom sheet (`parts`), in two steps that never
    /// touch the page between the parts:
    ///
    /// 1. one A2 update over the parts, partial, so only pixels that
    ///    changed move: the menu is there in about 150 ms, in black and
    ///    white;
    /// 2. one full-mode gray update of each part alone (GL16, GLKW16 at
    ///    night), which brings the grays and clears what A2 left; full
    ///    mode because a partial update would skip the pixels A2 already
    ///    moved.
    ///
    /// A single gray update over the bounding box of both parts would cover
    /// almost the whole page and show nothing for a full waveform.
    pub fn present_overlay(&mut self, ui: &Ui, parts: &[Rect]) {
        let parts: Vec<Rect> = parts.iter().copied().filter(|r| !r.is_empty()).collect();
        let Some(union) = parts.iter().copied().reduce(|a, b| a.union(&b)) else {
            return;
        };
        self.wait_idle();
        for p in &parts {
            self.blit_ui(ui, *p);
        }
        let gray = if self.night.is_some() {
            FbWaveform::GLKW16
        } else {
            FbWaveform::GL16
        };
        let mut job = vec![(fb_rect(union), FbWaveform::A2, false)];
        job.extend(parts.iter().map(|p| (fb_rect(*p), gray, true)));
        self.sequencer.submit(job);
    }

    /// Show a book page: one full update of the whole screen with the turn
    /// waveform (`flash` uses mode 8 instead, for a screen switch). Returns
    /// once submitted; the panel works on while the caller prerenders.
    pub fn show_page(&mut self, page: &Page, flash: bool, label: &str) -> io::Result<Duration> {
        let page = night_page(page, self.night.is_some());
        let waited = self.wait_idle();
        let rect = self.fb.full_screen();
        if self.inverts_pixels() {
            self.fb.blit_inverted(&page.data, page.stride, rect);
        } else {
            self.fb.blit(&page.data, page.stride, rect);
        }
        let wf = if flash { FLASH } else { self.turn };
        let m = self.fb.update(rect, wf, true)?;
        self.full_updates += 1;
        self.track(m, label);
        Ok(waited)
    }

    /// How many full-screen full-mode updates went out so far, so a caller
    /// can tell whether an action redrew the whole screen.
    pub fn full_updates(&self) -> u64 {
        self.full_updates
    }

    fn track(&mut self, marker: Option<u32>, label: &str) {
        if let Some(marker) = marker {
            let _ = self.completions.track.send(marker);
            self.page = Some(InFlight {
                marker,
                submitted: Instant::now(),
                label: label.to_string(),
            });
        }
    }

    /// Block until the panel is idle: the last page update finished and the
    /// display thread has nothing queued or in flight. Returns the wait.
    fn wait_idle(&mut self) -> Duration {
        let t = Instant::now();
        self.finish_page();
        self.display.sync();
        let deadline = t + Duration::from_secs(3);
        while (self.display.busy() || self.sequencer.busy()) && Instant::now() < deadline {
            self.display.drain_shown();
            thread::sleep(Duration::from_millis(2));
        }
        t.elapsed()
    }

    fn finish_page(&mut self) {
        if let Some(p) = self.page.take() {
            let done = self.completions.wait(p.marker);
            log_shown(&p, done);
        }
    }

    /// Log pages that reached the panel meanwhile; call from the loop.
    pub fn poll(&mut self) {
        self.completions.collect();
        self.display.drain_shown();
        if let Some(p) = &self.page {
            if let Some(done) = self.completions.seen.remove(&p.marker) {
                let p = self.page.take().expect("checked");
                log_shown(&p, done);
            }
        }
    }

    fn blit_ui(&mut self, ui: &Ui, rect: Rect) {
        let c = ui.screen();
        let Some(r) = rect.intersect(&Rect::new(0, 0, c.width as i32, c.height as i32)) else {
            return;
        };
        let stride = c.stride as usize;
        let start = r.y as usize * stride + r.x as usize;
        self.fb.blit(&c.pixels[start..], stride, fb_rect(r));
    }

    fn blit_ui_wait(&mut self, ui: &Ui, rect: Rect) -> io::Result<()> {
        self.wait_idle();
        self.blit_ui(ui, rect);
        Ok(())
    }

    /// Leave a clean white screen behind for the stock UI, in day mode
    /// whatever ours was.
    pub fn clear(&mut self) -> io::Result<()> {
        self.wait_idle();
        self.fb.set_hw_invert(false);
        self.night = None;
        let all = self.fb.full_screen();
        self.fb.fill(all, 0xff);
        let m = self.fb.update(all, FLASH, true)?;
        self.fb.wait(m)
    }
}

fn log_shown(p: &InFlight, done: Instant) {
    crate::perf!(
        "panel: {} on the panel {:.0} ms after submit",
        p.label,
        crate::ms(done.saturating_duration_since(p.submitted))
    );
}

/// One update as the memory panel recorded it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub rect: Rect,
    /// EPDC waveform mode.
    pub mode: u32,
    pub full_screen: bool,
    pub label: String,
}

/// A panel in memory for the host: the same calls as the device, applied
/// to a pixel buffer, with every update recorded under the same
/// [`plan`]. The preview drives the real app through it
/// (`aristotle --preview`), so what it shows is what the device code does.
pub struct Memory {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    /// What the framebuffer holds (inverted already in software night).
    pub pixels: Vec<u8>,
    pub updates: Vec<Update>,
    turn: FbWaveform,
    night: Option<NightMethod>,
}

impl Memory {
    pub fn new(width: u32, height: u32, turn_mode: u32) -> Memory {
        let stride = 1088.max(width as usize);
        Memory {
            width,
            height,
            stride,
            pixels: vec![0xff; stride * height as usize],
            updates: Vec::new(),
            turn: FbWaveform(turn_mode),
            night: None,
        }
    }

    /// Whether the EPDC would invert what it shows (hardware night): the
    /// pixels here are then the day pixels.
    pub fn hw_inverted(&self) -> bool {
        self.night == Some(NightMethod::Hardware)
    }

    fn all(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }

    fn copy_ui(&mut self, ui: &Ui, rect: Rect) {
        let c = ui.screen();
        let Some(r) = rect.intersect(&Rect::new(0, 0, c.width as i32, c.height as i32)) else {
            return;
        };
        for y in r.y..r.y + r.h {
            let src = y as usize * c.stride as usize + r.x as usize;
            let dst = y as usize * self.stride + r.x as usize;
            self.pixels[dst..dst + r.w as usize]
                .copy_from_slice(&c.pixels[src..src + r.w as usize]);
        }
    }

    fn record(&mut self, rect: Rect, mode: u32, label: &str) {
        let full_screen = rect == self.all();
        self.updates.push(Update {
            rect,
            mode,
            full_screen,
            label: label.to_string(),
        });
    }

    fn present(&mut self, ui: &Ui, damage: &[Damage], content: Content) {
        for d in damage {
            match plan(d.waveform, content, self.night.is_some(), self.turn) {
                Request::FullScreen(wf) => {
                    let all = self.all();
                    self.copy_ui(ui, all);
                    let label = if wf == FLASH { "flash" } else { "page (ui)" };
                    self.record(all, wf.0, label);
                }
                Request::Partial(wf) => {
                    self.copy_ui(ui, d.rect);
                    self.record(d.rect, wf.0, "ui");
                }
            }
        }
    }

    fn show_page(&mut self, page: &Page, flash: bool, label: &str) {
        let page = night_page(page, self.night.is_some());
        let invert = self.night == Some(NightMethod::Software);
        for y in 0..self.height as usize {
            let src = y * page.stride;
            let dst = y * self.stride;
            let w = (page.width as usize).min(self.width as usize);
            let row = &page.data[src..src + w];
            for (d, s) in self.pixels[dst..dst + w].iter_mut().zip(row) {
                *d = if invert { 255 - *s } else { *s };
            }
        }
        let mode = if flash { FLASH.0 } else { self.turn.0 };
        self.record(self.all(), mode, label);
    }
}

/// The panel the app draws to: the device, or memory on the host.
pub enum Panel {
    Device(Box<Device>),
    Memory(Memory),
}

impl Panel {
    pub fn open(turn_mode: u32) -> io::Result<Panel> {
        Device::open(turn_mode).map(|d| Panel::Device(Box::new(d)))
    }

    pub fn memory(width: u32, height: u32, turn_mode: u32) -> Panel {
        Panel::Memory(Memory::new(width, height, turn_mode))
    }

    /// See [`Device::set_night`].
    pub fn set_night(&mut self, night: Option<NightMethod>) {
        match self {
            Panel::Device(d) => d.set_night(night),
            Panel::Memory(m) => m.night = night,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        match self {
            Panel::Device(d) => d.size(),
            Panel::Memory(m) => (m.width, m.height),
        }
    }

    /// See [`Device::present`].
    pub fn present(&mut self, ui: &Ui, damage: &[Damage], content: Content) -> io::Result<()> {
        match self {
            Panel::Device(d) => d.present(ui, damage, content),
            Panel::Memory(m) => {
                m.present(ui, damage, content);
                Ok(())
            }
        }
    }

    /// See [`Device::present_overlay`].
    pub fn present_overlay(&mut self, ui: &Ui, parts: &[Rect]) {
        match self {
            Panel::Device(d) => d.present_overlay(ui, parts),
            Panel::Memory(m) => {
                let gray = if m.night.is_some() {
                    FbWaveform::GLKW16
                } else {
                    FbWaveform::GL16
                };
                if let Some(u) = parts.iter().copied().reduce(|a, b| a.union(&b)) {
                    m.copy_ui(ui, u);
                    m.record(u, FbWaveform::A2.0, "overlay");
                }
                for p in parts {
                    m.record(*p, gray.0, "overlay");
                }
            }
        }
    }

    /// See [`Device::show_page`].
    pub fn show_page(&mut self, page: &Page, flash: bool, label: &str) -> io::Result<Duration> {
        match self {
            Panel::Device(d) => d.show_page(page, flash, label),
            Panel::Memory(m) => {
                m.show_page(page, flash, label);
                Ok(Duration::ZERO)
            }
        }
    }

    pub fn full_updates(&self) -> u64 {
        match self {
            Panel::Device(d) => d.full_updates(),
            Panel::Memory(m) => m.updates.iter().filter(|u| u.full_screen).count() as u64,
        }
    }

    pub fn poll(&mut self) {
        if let Panel::Device(d) = self {
            d.poll();
        }
    }

    pub fn clear(&mut self) -> io::Result<()> {
        match self {
            Panel::Device(d) => d.clear(),
            Panel::Memory(m) => {
                m.night = None;
                m.pixels.fill(0xff);
                let all = m.all();
                m.record(all, FLASH.0, "clear");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURN: FbWaveform = FbWaveform::GLKW16;

    #[test]
    fn screen_switches_and_night_switch_are_one_mode_8_flash() {
        for night in [false, true] {
            for content in [Content::Chrome, Content::Page] {
                assert_eq!(
                    plan(Waveform::GC16, content, night, TURN),
                    Request::FullScreen(FbWaveform::GCK16)
                );
            }
        }
    }

    #[test]
    fn page_behind_the_menu_goes_out_like_a_turn() {
        assert_eq!(
            plan(Waveform::GL16, Content::Page, false, TURN),
            Request::FullScreen(FbWaveform(9))
        );
        assert_eq!(
            plan(Waveform::GL16, Content::Page, true, FbWaveform(7)),
            Request::FullScreen(FbWaveform(7)),
            "the chosen turn waveform, day or night"
        );
    }

    #[test]
    fn chrome_is_partial_and_night_grays_use_glkw16() {
        let p = |w, night| plan(w, Content::Chrome, night, TURN);
        assert_eq!(p(Waveform::A2, false), Request::Partial(FbWaveform::A2));
        assert_eq!(p(Waveform::A2, true), Request::Partial(FbWaveform::A2));
        assert_eq!(p(Waveform::DU, true), Request::Partial(FbWaveform::DU));
        assert_eq!(p(Waveform::GL16, false), Request::Partial(FbWaveform::GL16));
        assert_eq!(
            p(Waveform::GL16, true),
            Request::Partial(FbWaveform::GLKW16)
        );
    }

    #[test]
    fn night_shows_images_as_by_day() {
        use aristotle_render::book::Bounds;
        use aristotle_render::style::Style;
        let mut page = Page::white(&Style::pw4());
        // Black text at the top, a gray image below it.
        page.data[100 * page.stride + 100] = 0x00;
        let image = Bounds {
            x0: 200,
            y0: 400,
            x1: 600,
            y1: 700,
        };
        for y in image.y0..image.y1 {
            let row = y as usize * page.stride;
            page.data[row + image.x0 as usize..row + image.x1 as usize].fill(0x40);
        }
        page.images.push(image);
        let at = |m: &Memory, x: usize, y: usize| m.pixels[y * m.stride + x];
        for (night, hw_inverted) in [
            (NightMethod::Software, false),
            (NightMethod::Hardware, true),
        ] {
            let mut m = Memory::new(1072, 1448, 9);
            m.night = Some(night);
            m.show_page(&page, false, "turn");
            // What the panel shows: the framebuffer, inverted by the EPDC
            // in hardware night.
            let shown = |x, y| {
                let p = at(&m, x, y);
                if hw_inverted {
                    255 - p
                } else {
                    p
                }
            };
            assert_eq!(shown(100, 100), 0xff, "{night:?}: text is white");
            assert_eq!(shown(10, 10), 0x00, "{night:?}: paper is black");
            assert_eq!(shown(400, 500), 0x40, "{night:?}: the image as by day");
        }
        // By day, and without images, the page goes out as it is.
        assert!(matches!(night_page(&page, false), Cow::Borrowed(_)));
        page.images.clear();
        assert!(matches!(night_page(&page, true), Cow::Borrowed(_)));
    }
}
