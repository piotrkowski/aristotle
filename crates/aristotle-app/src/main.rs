//! Aristotle on the device: the `aristotle-ui` screens driven by the real
//! panel, touchscreen and frontlight through `aristotle-hal`.
//!
//! Run it with the stock GUI stopped (`devloop/kindle/takeover.sh start`).
//! Hold the library's title bar for about a second to quit.
//!
//! Presenting a frame follows the measurements in `docs/eink-rendering.md`
//! and `docs/native-vs-aristotle.md`:
//!
//! - Ordinary damage goes to the HAL display thread, which never blocks the
//!   input loop and merges whatever piles up while the panel is busy.
//! - A page turn (a large GL16 rect) becomes one full-screen, full-mode
//!   update with mode 9, exactly like the stock reader's. The faster
//!   A2-then-GL16 turn left heavy ghosting on the real panel.
//!   `ARISTOTLE_TURN=a2` keeps an A2 preview before the mode-9 pass for
//!   comparison, and `ARISTOTLE_TURN=gl16` uses a full GL16 update.

use std::io;
use std::time::Duration;

use aristotle_hal::display::Display;
use aristotle_hal::framebuffer::{Framebuffer, Rect as FbRect, Waveform as FbWaveform};
use aristotle_hal::input::{monotonic_us, Phase as HalPhase, TouchScreen, Transform};
use aristotle_hal::power::Frontlight;
use aristotle_ui::geom::{SCREEN_HEIGHT, SCREEN_WIDTH};
use aristotle_ui::input::{Phase, TouchEvent};
use aristotle_ui::screens::library::{self, Library};
use aristotle_ui::screens::reader::{Command, Reader, Settings};
use aristotle_ui::text::{FamilyId, Fonts, TextEngine};
use aristotle_ui::theme::Styles;
use aristotle_ui::tree::Refresh;
use aristotle_ui::ui::{Damage, Ui, UiEvent};
use aristotle_ui::{Rect, Waveform};

/// A GL16 rect at least this share of the screen is treated as a page turn.
const PAGE_TURN_SHARE: f32 = 0.4;
/// Pixels darker than this become black in the fast first pass.
const THRESHOLD: u8 = 160;
/// The library title bar: holding it quits the app.
const QUIT_ZONE_HEIGHT: i32 = 180;

enum Screen {
    Library(Library),
    Reader(Reader),
}

fn fb_rect(r: Rect) -> FbRect {
    FbRect { top: r.y.max(0) as u32, left: r.x.max(0) as u32, width: r.w.max(0) as u32, height: r.h.max(0) as u32 }
}

fn fb_waveform(w: Waveform) -> FbWaveform {
    FbWaveform(w.mode())
}

/// Copy `rect` of the UI canvas into the framebuffer, optionally reduced to
/// pure black and white.
fn blit(fb: &mut Framebuffer, ui: &Ui, rect: Rect, bw: bool) {
    let c = ui.screen();
    let Some(r) = rect.intersect(&Rect::new(0, 0, c.width as i32, c.height as i32)) else { return };
    let stride = c.stride as usize;
    let start = r.y as usize * stride + r.x as usize;
    if bw {
        let mut tmp = Vec::with_capacity(r.w as usize * r.h as usize);
        for y in 0..r.h as usize {
            let row = &c.pixels[start + y * stride..start + y * stride + r.w as usize];
            tmp.extend(row.iter().map(|&p| if p < THRESHOLD { 0x00 } else { 0xff }));
        }
        fb.blit(&tmp, r.w as usize, fb_rect(r));
    } else {
        fb.blit(&c.pixels[start..], stride, fb_rect(r));
    }
}

/// Mode 9: what the stock reader turns every page with, as a full update of
/// the whole screen (seen in the driver log). Despite FBInk's "GLKW16
/// (night)" label, the PW4 stock UI uses it in day mode; it is the panel's
/// clean, non-flashing text waveform.
const STOCK_PAGE: FbWaveform = FbWaveform(9);
/// Mode 8: the stock UI's full flashing refresh (its GC16 equivalent).
const STOCK_FLASH: FbWaveform = FbWaveform(8);

/// How a page turn reaches the panel (`ARISTOTLE_TURN`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum TurnStrategy {
    /// One full mode-9 update, exactly the stock reader's turn: clean pages.
    Stock,
    /// A2 black and white preview first (readable at about 150 ms), then the
    /// full mode-9 pass. Faster, but A2 leaves residue the owner saw as
    /// heavy ghosting when the second pass was GL16.
    A2ThenStock,
    /// One full GL16 update.
    FullGl16,
}

impl TurnStrategy {
    fn from_env() -> Self {
        match std::env::var("ARISTOTLE_TURN").as_deref() {
            Ok("a2") => TurnStrategy::A2ThenStock,
            Ok("gl16") => TurnStrategy::FullGl16,
            _ => TurnStrategy::Stock,
        }
    }
}

/// Send one frame's damage to the panel.
fn present(fb: &mut Framebuffer, display: &Display, ui: &Ui, damage: &[Damage], turn: TurnStrategy) -> io::Result<()> {
    let screen_area = (SCREEN_WIDTH * SCREEN_HEIGHT) as f32;
    for d in damage {
        let page_turn = d.waveform == Waveform::GL16 && (d.rect.w * d.rect.h) as f32 >= screen_area * PAGE_TURN_SHARE;
        if page_turn {
            // Page turns drive every pixel of the whole screen, as stock does:
            // a smaller or partial update leaves the previous page's ghost.
            let all = fb.full_screen();
            if turn == TurnStrategy::A2ThenStock {
                // A2 on the calling thread, so the anti-aliased pixels cannot
                // reach the panel before the A2 update has read the framebuffer.
                display.sync();
                blit(fb, ui, d.rect, true);
                let m = fb.update(fb_rect(d.rect), FbWaveform::A2, false)?;
                fb.wait(m)?;
            }
            blit(fb, ui, d.rect, false);
            let wf = if turn == TurnStrategy::FullGl16 { FbWaveform::GL16 } else { STOCK_PAGE };
            display.damage_full(all, wf, &[]);
        } else if d.waveform == Waveform::GC16 {
            blit(fb, ui, d.rect, false);
            display.damage_full(fb_rect(d.rect), STOCK_FLASH, &[]);
        } else {
            blit(fb, ui, d.rect, false);
            display.damage(fb_rect(d.rect), fb_waveform(d.waveform), &[]);
        }
    }
    Ok(())
}

fn ui_event(ev: &aristotle_hal::input::TouchEvent) -> TouchEvent {
    TouchEvent {
        phase: match ev.phase {
            HalPhase::Down => Phase::Down,
            HalPhase::Move => Phase::Move,
            HalPhase::Up => Phase::Up,
        },
        id: ev.id,
        x: ev.x as i32,
        y: ev.y as i32,
        time_us: ev.time_us,
    }
}

fn main() -> io::Result<()> {
    let mut fb = Framebuffer::open("/dev/fb0")?;
    let g = fb.geometry();
    let display = Display::new(&fb)?;
    let mut touch = TouchScreen::open(g.width, g.height, Transform::default())?;
    touch.grab(true)?;
    let light = Frontlight::open().ok();
    let turn = TurnStrategy::from_env();

    let started = monotonic_us();
    let styles = Styles::new(FamilyId(0));
    let books = library::sample_books();
    let mut text = TextEngine::new(Fonts::embedded());
    let (tree, lib) = Library::build(&mut text, &styles, &books, library::Filter::All);
    let mut ui = Ui::new(tree, text, SCREEN_WIDTH as u32, SCREEN_HEIGHT as u32);
    ui.layout();
    lib.sync(&mut ui.tree);
    let mut screen = Screen::Library(lib);
    ui.request_full_refresh();
    let damage = ui.render();
    present(&mut fb, &display, &ui, &damage, turn)?;
    eprintln!("first frame after {} ms", (monotonic_us() - started) / 1000);

    loop {
        let now = monotonic_us();
        let timeout = ui
            .next_deadline(now)
            .map_or(Duration::from_millis(200), |t| Duration::from_micros(t.saturating_sub(now).max(1000)));
        let mut events = Vec::new();
        for ev in touch.poll(timeout)? {
            events.extend(ui.handle_touch(ui_event(&ev)));
        }
        events.extend(ui.tick(monotonic_us()));
        if events.is_empty() {
            continue;
        }

        for ev in events {
            let mut next = None;
            match &mut screen {
                Screen::Library(lib) => match ev {
                    UiEvent::LongPress(p) if p.y < QUIT_ZONE_HEIGHT => return quit(&mut fb, &display),
                    UiEvent::Action { id, .. } if id == library::action::RESUME || id >= library::action::BOOK => {
                        let (tree, reader) = Reader::build(&mut ui.text, &styles, [FamilyId(0); 3], Settings::default());
                        ui.set_tree_with(tree, Refresh::Full);
                        next = Some(Screen::Reader(reader));
                    }
                    UiEvent::Action { id, .. } => {
                        lib.handle(&mut ui.tree, id);
                        ui.layout();
                        lib.sync(&mut ui.tree);
                    }
                    _ => {}
                },
                Screen::Reader(reader) => {
                    let before = reader.settings.light;
                    if reader.handle(&mut ui, ev) == Command::Library {
                        let (tree, lib) = Library::build(&mut ui.text, &styles, &books, library::Filter::All);
                        ui.set_tree_with(tree, Refresh::Full);
                        ui.layout();
                        lib.sync(&mut ui.tree);
                        next = Some(Screen::Library(lib));
                    } else if reader.settings.light != before {
                        if let Some(l) = &light {
                            // Perceived brightness is roughly quadratic in drive current.
                            let v = reader.settings.light.clamp(0.0, 1.0);
                            let _ = l.set_brightness((v * v * l.max() as f32) as u32);
                        }
                    }
                }
            }
            if let Some(s) = next {
                screen = s;
            }
        }
        let damage = ui.render();
        present(&mut fb, &display, &ui, &damage, turn)?;
    }
}

/// Leave a clean white screen behind for the stock UI.
fn quit(fb: &mut Framebuffer, display: &Display) -> io::Result<()> {
    display.sync();
    let all = fb.full_screen();
    fb.fill(all, 0xff);
    let m = fb.update(all, FbWaveform::GC16, true)?;
    fb.wait(m)?;
    Ok(())
}
