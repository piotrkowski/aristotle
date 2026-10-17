//! Getting pixels onto the PW4 panel, by the rules measured on the device
//! (`docs/eink-rendering.md`, `docs/native-vs-aristotle.md`):
//!
//! - **Book pages** go out as ONE full-mode update of the WHOLE screen with
//!   the stock reader's waveform, mode 9 (or mode 7, DU4, if chosen). That
//!   is exactly what the stock reader does. Two-step A2 previews and
//!   partial GL16 updates left ghosting the owner rejected. A page is
//!   submitted directly (about 1 ms when the panel is idle) so the caller
//!   can prerender the next page while the panel works.
//! - **UI chrome** (library, menu) goes through the HAL display thread,
//!   which never blocks the input loop: DU for black and white changes,
//!   GL16 for grays (the UI's own hints), and mode 8 full for screen
//!   switches and cleanups (the stock UI's flashing refresh). When the book
//!   page itself changed behind the open menu (new font size, chapter jump),
//!   the frame goes out as a full mode-9 update of the whole screen like a
//!   page turn ([`Content::Page`]).
//!
//! The driver runs one update at a time and reads the framebuffer when it
//! processes an update, so a new page is only blitted once the previous
//! page update has finished and the display thread is idle. That is the
//! only place the input loop waits for the panel.

use std::collections::HashMap;
use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use aristotle_hal::display::Display;
use aristotle_hal::framebuffer::{Framebuffer, Rect as FbRect, Waveform as FbWaveform};
use aristotle_render::page::Page;
use aristotle_ui::ui::{Damage, Ui};
use aristotle_ui::{Rect, Waveform};

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

pub struct Panel {
    fb: Framebuffer,
    display: Display,
    completions: Completions,
    page: Option<InFlight>,
    /// Waveform for page turns.
    pub turn: FbWaveform,
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

impl Panel {
    pub fn open(turn_mode: u32) -> io::Result<Panel> {
        let fb = Framebuffer::open("/dev/fb0")?;
        let display = Display::new(&fb)?;
        let completions = Completions::new(&fb)?;
        Ok(Panel {
            fb,
            display,
            completions,
            page: None,
            turn: FbWaveform(turn_mode),
            full_updates: 0,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        let g = self.fb.geometry();
        (g.width, g.height)
    }

    /// Send a UI frame's damage (see the module documentation).
    pub fn present(&mut self, ui: &Ui, damage: &[Damage], content: Content) -> io::Result<()> {
        for d in damage {
            let page = content == Content::Page && d.waveform == Waveform::GL16;
            if page || d.waveform == Waveform::GC16 {
                // Full-screen, full-mode: clears ghosting.
                let wf = if page { self.turn } else { FLASH };
                self.blit_ui_wait(ui, Rect::screen())?;
                let all = self.fb.full_screen();
                let m = self.fb.update(all, wf, true)?;
                self.full_updates += 1;
                self.track(m, if page { "page (menu open)" } else { "flash" });
            } else {
                // No wait: the driver latched the last page when it started
                // it, and the display thread queues behind it by itself.
                self.blit_ui(ui, d.rect);
                self.display
                    .damage(fb_rect(d.rect), FbWaveform(d.waveform.mode()), &[]);
            }
        }
        Ok(())
    }

    /// Show a book page: one full update of the whole screen with the turn
    /// waveform (`flash` uses mode 8 instead, for a screen switch). Returns
    /// once submitted; the panel works on while the caller prerenders.
    pub fn show_page(&mut self, page: &Page, flash: bool, label: &str) -> io::Result<Duration> {
        let waited = self.wait_idle();
        let rect = self.fb.full_screen();
        self.fb.blit(&page.data, page.stride, rect);
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
        while self.display.busy() && Instant::now() < deadline {
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

    /// Leave a clean white screen behind for the stock UI.
    pub fn clear(&mut self) -> io::Result<()> {
        self.wait_idle();
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
