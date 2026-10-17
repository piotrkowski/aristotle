//! Putting rendered pages on the PW4 panel.
//!
//! The page turn copies the stock reader exactly: one full-mode update of the
//! whole screen with waveform mode 9 (seen in the driver log for every stock
//! page turn). The faster A2-then-GL16 turn measured in
//! `docs/native-vs-aristotle.md` looked fine in a quick demo, but on real
//! reading it left heavy ghosting: A2 leaves residue that GL16 does not
//! clear. It stays available as [`TurnStrategy::A2Preview`] for experiments.

use std::io;
use std::time::{Duration, Instant};

use aristotle_hal::framebuffer::{Framebuffer, Rect, Waveform};

use crate::page::Page;

/// How a page turn reaches the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnStrategy {
    /// One full mode-9 update of the whole screen, like the stock reader.
    Stock,
    /// One full update of the whole screen with another waveform mode, to
    /// compare single-step turns (for example DU4, mode 7, about 344 ms).
    Single(u32),
    /// A thresholded A2 preview first (legible after about 150 ms), then the
    /// full mode-9 update. Only for experiments: may ghost.
    A2Preview,
}

/// When a page turn became visible and when the panel settled, measured
/// from the moment [`Screen::turn`] was called.
#[derive(Clone, Copy, Debug)]
pub struct TurnTimes {
    /// The first update showing the new page has finished: it is legible.
    pub visible: Duration,
    /// The last update has finished: the panel is calm.
    pub settled: Duration,
}

/// The framebuffer, owned for the duration of a command. The stock GUI must
/// be stopped (`stop lab126_gui`), or it will draw over us.
pub struct Screen {
    fb: Framebuffer,
}

impl Screen {
    pub fn open() -> io::Result<Screen> {
        Ok(Screen {
            fb: Framebuffer::open("/dev/fb0")?,
        })
    }

    fn blit(&mut self, page: &Page) -> Rect {
        let rect = Rect {
            top: 0,
            left: 0,
            width: page.width,
            height: page.height,
        };
        self.fb.blit(&page.data, page.stride, rect);
        rect
    }

    /// Show `page` with the stock UI's flashing full refresh (mode 8), to
    /// start from a clean panel without ghosts of whatever was there before.
    pub fn show_clean(&mut self, page: &Page) -> io::Result<Duration> {
        let start = Instant::now();
        let rect = self.blit(page);
        let marker = self.fb.update(rect, Waveform::GCK16, true)?;
        self.fb.wait(marker)?;
        Ok(start.elapsed())
    }

    /// Start a single-step turn to `page` and return at once (about 1 ms),
    /// so the caller can render the next page while the panel works. Finish
    /// with [`Screen::finish`]. Not for [`TurnStrategy::A2Preview`].
    pub fn submit(&mut self, page: &Page, strategy: TurnStrategy) -> io::Result<Option<u32>> {
        self.blit(page);
        let waveform = match strategy {
            TurnStrategy::Single(mode) => Waveform(mode),
            _ => Waveform::GLKW16,
        };
        let all = self.fb.full_screen();
        self.fb.update(all, waveform, true)
    }

    /// Wait until the update started by [`Screen::submit`] is on the panel.
    pub fn finish(&mut self, marker: Option<u32>) -> io::Result<()> {
        self.fb.wait(marker)
    }

    /// Turn to `page`, whose black and white version `bw` is only used by
    /// [`TurnStrategy::A2Preview`].
    pub fn turn(
        &mut self,
        page: &Page,
        bw: &Page,
        strategy: TurnStrategy,
    ) -> io::Result<TurnTimes> {
        let start = Instant::now();
        let mut visible = None;
        if strategy == TurnStrategy::A2Preview {
            let rect = self.blit(bw);
            let marker = self.fb.update(rect, Waveform::A2, false)?;
            self.fb.wait(marker)?;
            visible = Some(start.elapsed());
        }
        self.blit(page);
        // Always the whole screen in full mode: a smaller or partial update
        // leaves the previous page's ghost in pixels that did not change.
        let all = self.fb.full_screen();
        let waveform = match strategy {
            TurnStrategy::Single(mode) => Waveform(mode),
            _ => Waveform::GLKW16,
        };
        let marker = self.fb.update(all, waveform, true)?;
        self.fb.wait(marker)?;
        let settled = start.elapsed();
        Ok(TurnTimes {
            visible: visible.unwrap_or(settled),
            settled,
        })
    }
}

/// Whether the stock Java framework (`cvm`) is running. With it up, the
/// stock UI keeps drawing into the same framebuffer and our timings are
/// meaningless.
pub fn stock_gui_running() -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries
        .flatten()
        .any(|e| std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| c.trim() == "cvm"))
}
