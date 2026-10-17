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
//!   which never blocks the input loop: A2 only to acknowledge a touch,
//!   DU for black and white changes, GL16 for grays (the UI's own hints),
//!   mode 9 in full mode ([`CLEAN`]) for whatever A2 showed and for new
//!   pages of a list, and mode 8 full for screen switches and cleanups
//!   (the stock UI's flashing refresh). When the book page itself changed
//!   behind the open menu (new font size, chapter jump, a popup), the frame
//!   goes out as a full mode-9 update of the whole screen like a page turn
//!   ([`Content::Page`]).
//! - **Ghost budget**: after [`PARTIALS_PER_FLASH`] partial updates, or
//!   partial updates covering [`PARTIAL_SCREENS_PER_FLASH`] screens, since
//!   the last full-screen update, the next settled update is a flash
//!   ([`GhostBudget`]).
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
/// The cleaning update without a flash: mode 9 in full mode, the update
/// the stock reader turns pages with [dev]. Used for the UI's `GLKW16`
/// hint (the final state after A2, list pages) and the reader menu.
pub const CLEAN: FbWaveform = FbWaveform::GLKW16;

/// Partial updates (DU and GL16, not A2 or [`CLEAN`]) allowed between two
/// full-screen updates before the next settled update becomes a flash.
/// Six is KOReader's default full-refresh interval ("Full refresh rate:
/// every 6 pages"; from KOReader's settings, not measured here). A2 does
/// not count: it is always cleaned by a [`CLEAN`] update.
pub const PARTIALS_PER_FLASH: u32 = 6;
/// The same limit by area: partial and [`CLEAN`] region updates adding up
/// to this many screens since the last full-screen update. An inference:
/// region updates leave residue at their edges, and three screens' worth
/// is about one settings page of toggles.
pub const PARTIAL_SCREENS_PER_FLASH: i64 = 3;

/// A [`CLEAN`] region at least this share of the screen goes out as one
/// full-screen update instead: the same panel time, and no edge between
/// cleaned and uncleaned pixels.
const CLEAN_FULL_SCREEN: (i64, i64) = (1, 2);

/// How much non-flashing partial work the panel took since the last
/// full-screen update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GhostBudget {
    partials: u32,
    area: i64,
}

impl GhostBudget {
    /// Whether the next settled update should be a flash.
    pub fn spent(&self, screen: Rect) -> bool {
        self.partials >= PARTIALS_PER_FLASH
            || self.area >= screen.area() * PARTIAL_SCREENS_PER_FLASH
    }

    /// Account for one request that went out.
    pub fn note(&mut self, req: Request, rect: Rect) {
        match req {
            Request::FullScreen(_) => *self = GhostBudget::default(),
            Request::Partial(wf) if wf == FbWaveform::A2 => {}
            Request::Partial(_) => {
                self.partials += 1;
                self.area += rect.area();
            }
            Request::Clean(_) => self.area += rect.area(),
        }
    }

    /// A full-screen update went out some other way (a page, the menu).
    pub fn reset(&mut self) {
        *self = GhostBudget::default();
    }
}
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
    /// A full-mode update of the damaged rectangle alone (every pixel
    /// driven), through the display thread.
    Clean(FbWaveform),
}

/// The transition policy at the panel (`docs/ui-framework.md`,
/// "Transitions") for one damage rectangle `rect` of `screen`:
///
/// - A2 (touch-down feedback) is partial, and never anything else, so the
///   acknowledgement stays at about 150 ms;
/// - GC16 (a screen switch, the night switch, a cleanup) is one full
///   update of the whole screen with mode 8, the stock flash, and so is
///   any settled update once the ghost budget is `spent`;
/// - a gray change of the book page behind the menu is one full update of
///   the whole screen with the turn waveform (mode 9), like a page turn;
/// - GLKW16 (the cleaning hint) is a full-mode mode-9 update of the
///   rectangle, or of the whole screen when it covers half of it;
/// - DU (black-and-white changes) is partial;
/// - GL16 (gray chrome) is partial, and becomes GLKW16 (mode 9) at night:
///   the gray waveform Lab126 made for dark pages (FBInk; KOReader's
///   `waveform_night` on Rex and Zelda).
pub fn plan(
    hint: Waveform,
    rect: Rect,
    screen: Rect,
    content: Content,
    night: bool,
    turn: FbWaveform,
    spent: bool,
) -> Request {
    match hint {
        Waveform::A2 => Request::Partial(FbWaveform::A2),
        Waveform::GC16 => Request::FullScreen(FLASH),
        _ if spent => Request::FullScreen(FLASH),
        Waveform::GL16 | Waveform::GLKW16 if content == Content::Page => Request::FullScreen(turn),
        Waveform::GLKW16 => {
            let (n, d) = CLEAN_FULL_SCREEN;
            if rect.area() * d >= screen.area() * n {
                Request::FullScreen(CLEAN)
            } else {
                Request::Clean(CLEAN)
            }
        }
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
    ghosts: GhostBudget,
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
            ghosts: GhostBudget::default(),
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
            let req = plan(
                d.waveform,
                d.rect,
                Rect::screen(),
                content,
                self.night.is_some(),
                self.turn,
                self.ghosts.spent(Rect::screen()),
            );
            self.ghosts.note(req, d.rect);
            match req {
                Request::FullScreen(wf) => {
                    // Full-screen, full-mode: clears ghosting.
                    self.blit_ui_wait(ui, Rect::screen())?;
                    let all = self.fb.full_screen();
                    let m = self.fb.update(all, wf, true)?;
                    self.full_updates += 1;
                    self.track(m, if wf == FLASH { "flash" } else { "page (ui)" });
                }
                Request::Partial(wf) => {
                    // No wait: the driver latched the last page when it
                    // started it, and the display thread queues behind it.
                    self.blit_ui(ui, d.rect);
                    self.display.damage(fb_rect(d.rect), wf, &[]);
                }
                Request::Clean(wf) => {
                    self.blit_ui(ui, d.rect);
                    self.display.damage_full(fb_rect(d.rect), wf, &[]);
                }
            }
        }
        Ok(())
    }

    /// Show an overlay that just appeared over the page, such as the reader
    /// menu's top bar and bottom sheet (`parts`): one full-mode mode-9
    /// update of the whole screen ([`CLEAN`]), the update the page itself
    /// came with. Only the parts are copied to the framebuffer, so the page
    /// between them stays as it is.
    ///
    /// The menu used to appear as A2 first and a full-mode GL16 pass per
    /// part after it; GL16 did not clear what A2 left over gray text
    /// (`docs/benchmarks.md`, "What real reading showed"), and the menu
    /// ghosted. One update also makes one coherent change.
    pub fn present_overlay(&mut self, ui: &Ui, parts: &[Rect]) {
        let parts: Vec<Rect> = parts.iter().copied().filter(|r| !r.is_empty()).collect();
        if parts.is_empty() {
            return;
        }
        self.wait_idle();
        for p in &parts {
            self.blit_ui(ui, *p);
        }
        self.ghosts.reset();
        self.sequencer
            .submit(vec![(self.fb.full_screen(), CLEAN, true)]);
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
        self.ghosts.reset();
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
    ghosts: GhostBudget,
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
            ghosts: GhostBudget::default(),
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
        let all = self.all();
        for d in damage {
            let req = plan(
                d.waveform,
                d.rect,
                all,
                content,
                self.night.is_some(),
                self.turn,
                self.ghosts.spent(all),
            );
            self.ghosts.note(req, d.rect);
            match req {
                Request::FullScreen(wf) => {
                    self.copy_ui(ui, all);
                    let label = if wf == FLASH { "flash" } else { "page (ui)" };
                    self.record(all, wf.0, label);
                }
                Request::Partial(wf) => {
                    self.copy_ui(ui, d.rect);
                    self.record(d.rect, wf.0, "ui");
                }
                Request::Clean(wf) => {
                    self.copy_ui(ui, d.rect);
                    self.record(d.rect, wf.0, "clean");
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
        self.ghosts.reset();
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
                let parts: Vec<Rect> = parts.iter().copied().filter(|r| !r.is_empty()).collect();
                if parts.is_empty() {
                    return;
                }
                for p in &parts {
                    m.copy_ui(ui, *p);
                }
                m.ghosts.reset();
                m.record(m.all(), CLEAN.0, "overlay");
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
            // The menu's update leaves the page between its parts as it
            // was, so it does not count, as on the device.
            Panel::Memory(m) => m
                .updates
                .iter()
                .filter(|u| u.full_screen && u.label != "overlay")
                .count() as u64,
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
    const SCREEN: Rect = Rect::screen();
    const BUTTON: Rect = Rect::new(100, 100, 300, 120);

    fn p(w: Waveform, content: Content, night: bool) -> Request {
        plan(w, BUTTON, SCREEN, content, night, TURN, false)
    }

    #[test]
    fn screen_switches_and_night_switch_are_one_mode_8_flash() {
        for night in [false, true] {
            for content in [Content::Chrome, Content::Page] {
                assert_eq!(
                    plan(Waveform::GC16, SCREEN, SCREEN, content, night, TURN, false),
                    Request::FullScreen(FbWaveform::GCK16)
                );
            }
        }
    }

    #[test]
    fn page_behind_the_menu_goes_out_like_a_turn() {
        for hint in [Waveform::GL16, Waveform::GLKW16] {
            assert_eq!(
                p(hint, Content::Page, false),
                Request::FullScreen(FbWaveform(9))
            );
            assert_eq!(
                plan(
                    hint,
                    BUTTON,
                    SCREEN,
                    Content::Page,
                    true,
                    FbWaveform(7),
                    false
                ),
                Request::FullScreen(FbWaveform(7)),
                "the chosen turn waveform, day or night"
            );
        }
    }

    #[test]
    fn chrome_is_partial_and_night_grays_use_glkw16() {
        let p = |w, night| p(w, Content::Chrome, night);
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
    fn cleaning_is_full_mode_mode_9_and_whole_screen_when_large() {
        for night in [false, true] {
            assert_eq!(
                p(Waveform::GLKW16, Content::Chrome, night),
                Request::Clean(FbWaveform(9))
            );
            let half = Rect::new(0, 0, SCREEN.w, SCREEN.h / 2);
            assert_eq!(
                plan(
                    Waveform::GLKW16,
                    half,
                    SCREEN,
                    Content::Chrome,
                    night,
                    TURN,
                    false
                ),
                Request::FullScreen(FbWaveform(9)),
                "a list page: no flash"
            );
            assert_eq!(
                plan(
                    Waveform::GLKW16,
                    SCREEN,
                    SCREEN,
                    Content::Chrome,
                    night,
                    FbWaveform(7),
                    false
                ),
                Request::FullScreen(FbWaveform(9)),
                "mode 9 even when turns use DU4"
            );
        }
    }

    #[test]
    fn a_spent_budget_flashes_the_next_settled_update_but_not_a2() {
        let spent = |w| plan(w, BUTTON, SCREEN, Content::Chrome, false, TURN, true);
        assert_eq!(spent(Waveform::A2), Request::Partial(FbWaveform::A2));
        for w in [Waveform::DU, Waveform::GL16, Waveform::GLKW16] {
            assert_eq!(spent(w), Request::FullScreen(FLASH));
        }
    }

    #[test]
    fn ghost_budget_counts_partials_and_area() {
        let mut b = GhostBudget::default();
        let small = Rect::new(0, 0, 100, 100);
        for _ in 0..PARTIALS_PER_FLASH - 1 {
            b.note(Request::Partial(FbWaveform::DU), small);
            b.note(Request::Partial(FbWaveform::A2), small);
            b.note(Request::Clean(CLEAN), small);
        }
        assert!(!b.spent(SCREEN), "A2 and cleaning updates do not count");
        b.note(Request::Partial(FbWaveform::GL16), small);
        assert!(b.spent(SCREEN));
        b.note(Request::FullScreen(FLASH), SCREEN);
        assert!(!b.spent(SCREEN), "a full-screen update starts over");
        for _ in 0..PARTIAL_SCREENS_PER_FLASH {
            b.note(Request::Clean(CLEAN), SCREEN);
        }
        assert!(b.spent(SCREEN), "by area");
        b.reset();
        assert!(!b.spent(SCREEN));
    }

    #[test]
    fn a_tap_is_a2_then_one_cleaning_update_and_the_menu_one_update() {
        use aristotle_ui::input::TouchEvent;
        use aristotle_ui::text::{FamilyId, TextEngine, TextStyle};
        use aristotle_ui::tree::{Kind, Node, Tree};
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../aristotle-render/assets/fonts");
        let mut t = Tree::new();
        let l = t.label("Open", TextStyle::new(FamilyId(0), 40.0));
        let btn = t.button(Node::new(Kind::Spacer).border(3, 0, 12), l, 42);
        let root = t.align(0.5, 0.5, btn);
        t.set_root(root);
        let fonts = crate::ui_fonts(&dir).unwrap();
        let mut ui = Ui::new(t, TextEngine::new(fonts), 1072, 1448);
        let mut m = Memory::new(1072, 1448, 9);
        let d = ui.render();
        m.present(&ui, &d, Content::Chrome);
        let c = ui.tree.rect(btn).center();
        let touch = |phase, ms: u64| TouchEvent {
            phase,
            id: 0,
            x: c.x,
            y: c.y,
            time_us: ms * 1000,
        };
        ui.handle_touch(touch(aristotle_ui::input::Phase::Down, 0));
        let d = ui.render();
        m.present(&ui, &d, Content::Chrome);
        ui.handle_touch(touch(aristotle_ui::input::Phase::Up, 80));
        let d = ui.render();
        m.present(&ui, &d, Content::Chrome);
        let modes: Vec<(u32, &str)> = m.updates[1..]
            .iter()
            .map(|u| (u.mode, u.label.as_str()))
            .collect();
        assert_eq!(modes, vec![(6, "ui"), (9, "clean")]);
        let mut panel = Panel::Memory(m);
        let before = panel.full_updates();
        panel.present_overlay(
            &ui,
            &[Rect::new(0, 0, 1072, 200), Rect::new(0, 1000, 1072, 448)],
        );
        let Panel::Memory(m) = &panel else {
            unreachable!()
        };
        let last = m.updates.last().unwrap();
        assert_eq!(
            (last.mode, last.full_screen),
            (9, true),
            "menu: one mode-9 update"
        );
        assert_eq!(m.updates.len(), 4);
        assert_eq!(panel.full_updates(), before);
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
