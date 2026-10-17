//! Putting rendered pages on the PW4 panel with the page-turn strategy
//! measured in `docs/native-vs-aristotle.md` ("Page-turn strategy
//! experiment"): a thresholded black and white page with A2 first, which is
//! legible after about 150 ms, then the anti-aliased page with GL16.

use std::io;
use std::time::{Duration, Instant};

use aristotle_hal::framebuffer::{Framebuffer, Rect, Waveform};

use crate::page::Page;

/// When a page turn became visible and when the panel settled, measured
/// from the moment [`Screen::turn`] was called.
#[derive(Clone, Copy, Debug)]
pub struct TurnTimes {
    /// The A2 (black and white) update has finished: the page is legible.
    pub visible: Duration,
    /// The GL16 (anti-aliased) update has finished: the panel is calm.
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

    /// Show `page` with a full GC16 refresh (with the flash), to start from
    /// a clean panel without ghosts of whatever was there before.
    pub fn show_clean(&mut self, page: &Page) -> io::Result<Duration> {
        let start = Instant::now();
        let rect = self.blit(page);
        let marker = self.fb.update(rect, Waveform::GC16, true)?;
        self.fb.wait(marker)?;
        Ok(start.elapsed())
    }

    /// Turn to `page`, whose black and white version is `bw`: blit `bw`,
    /// A2, wait; blit `page`, GL16, wait. Both updates are partial, so only
    /// pixels that change are driven.
    pub fn turn(&mut self, page: &Page, bw: &Page) -> io::Result<TurnTimes> {
        let start = Instant::now();
        let rect = self.blit(bw);
        let marker = self.fb.update(rect, Waveform::A2, false)?;
        self.fb.wait(marker)?;
        let visible = start.elapsed();
        let rect = self.blit(page);
        let marker = self.fb.update(rect, Waveform::GL16, false)?;
        self.fb.wait(marker)?;
        Ok(TurnTimes {
            visible,
            settled: start.elapsed(),
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
