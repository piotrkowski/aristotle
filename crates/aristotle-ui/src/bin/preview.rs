//! Desktop preview: renders the prototype screens to PNG.
//!
//! Every screen goes through the same `Ui` the device will run, including
//! simulated touches, so the damage overlays show the real update
//! rectangles and waveforms. Images are quantized to the 16 gray levels the
//! panel can show.
//!
//! Usage: `aristotle-ui-preview [OUT_DIR]`, default `~/.cache/aristotle/ui-preview`.

use aristotle_ui::canvas::{Canvas, Painter};
use aristotle_ui::geom::{Point, Rect, SCREEN_HEIGHT, SCREEN_WIDTH};
use aristotle_ui::input::{Phase, TouchEvent};
use aristotle_ui::screens::library::{self, Library};
use aristotle_ui::screens::reader::{self, Reader, Settings, Zone};
use aristotle_ui::text::{
    layout_paragraph, Align, FamilyId, Fonts, NoHyphenation, Paragraph, ParagraphLayout,
    TextEngine, TextStyle,
};
use aristotle_ui::theme::Styles;
use aristotle_ui::tree::Hit;
use aristotle_ui::ui::{Damage, Ui, UiEvent};
use aristotle_ui::widgets::dashed_rect;
use std::path::{Path, PathBuf};

const W: u32 = SCREEN_WIDTH as u32;
const H: u32 = SCREEN_HEIGHT as u32;

fn save(c: &Canvas, path: &Path) {
    let mut q = c.clone();
    q.quantize16();
    let f = std::fs::File::create(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), q.width, q.height);
    e.set_color(png::ColorType::Grayscale);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()
        .and_then(|mut w| w.write_image_data(&q.packed()))
        .expect("png encoding");
    println!("wrote {}", path.display());
}

fn load_gray(path: &Path) -> Option<Canvas> {
    let d = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let ch = info.color_type.samples();
    let mut c = Canvas::new(info.width, info.height);
    for (i, px) in c.pixels.iter_mut().enumerate() {
        *px = buf[i * ch];
    }
    Some(c)
}

fn touch(ui: &mut Ui, phase: Phase, x: i32, y: i32, ms: u64) -> Vec<UiEvent> {
    ui.handle_touch(TouchEvent {
        phase,
        id: 0,
        x,
        y,
        time_us: ms * 1000,
    })
}

fn tap(ui: &mut Ui, p: Point, ms: u64) -> Vec<UiEvent> {
    let mut ev = touch(ui, Phase::Down, p.x, p.y, ms);
    ev.extend(touch(ui, Phase::Up, p.x, p.y, ms + 60));
    ev
}

fn set(text: &mut TextEngine, s: &str, style: TextStyle, width: f32) -> ParagraphLayout {
    let para = Paragraph::plain(s, style, Align::Left);
    layout_paragraph(text, &para, width, &NoHyphenation)
}

/// The screen with each minimal damage rectangle of the last render
/// outlined (large ones labelled), the coalesced update described in the
/// caption.
fn damage_overlay(ui: &mut Ui, sent: &[Damage], caption: &str) -> Canvas {
    let damage = ui.minimal_damage().to_vec();
    let caption = format!(
        "{caption}. Sent as: {}.",
        sent.iter()
            .map(|d| format!("{:?} {}×{}", d.waveform, d.rect.w, d.rect.h))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut c = ui.screen().clone();
    let all = Rect::new(0, 0, W as i32, H as i32);
    let label = TextStyle::new(FamilyId(0), 30.0).bold().gray(255);
    for d in &damage {
        let mut p = Painter::new(&mut c, all);
        dashed_rect(&mut p, d.rect.outset(3), 4, 14, 0);
        if d.rect.area() < 20_000 {
            continue;
        }
        let text = format!("{:?} {}×{}", d.waveform, d.rect.w, d.rect.h);
        let l = set(&mut ui.text, &text, label.clone(), 600.0);
        let tag = Rect::new(d.rect.x, d.rect.y, l.width as i32 + 24, l.height as i32 + 8);
        p.fill_rect(tag, 0);
        l.draw(
            &mut ui.text,
            &mut p,
            tag.x as f32 + 12.0,
            tag.y as f32 + 4.0,
        );
    }
    let style = TextStyle::new(FamilyId(0), 34.0).italic();
    let l = set(&mut ui.text, &caption, style, W as f32 - 80.0);
    let mut p = Painter::new(&mut c, all);
    let band = Rect::new(
        0,
        H as i32 - l.height as i32 - 24,
        W as i32,
        l.height as i32 + 24,
    );
    p.fill_rect(band, 235);
    l.draw(&mut ui.text, &mut p, 40.0, band.y as f32 + 12.0);
    c
}

fn log(step: &str, ui: &Ui, damage: &[Damage]) {
    println!(
        "{step}: {} minimal rect(s), sent:",
        ui.minimal_damage().len()
    );
    if damage.is_empty() {
        println!("  (no damage)");
    }
    for d in damage {
        println!(
            "  {:<5} {:>4}×{:<4} at ({}, {})",
            format!("{:?}", d.waveform),
            d.rect.w,
            d.rect.h,
            d.rect.x,
            d.rect.y
        );
    }
}

fn engine(fonts_dir: &Path) -> (TextEngine, [FamilyId; 3]) {
    let mut fonts = Fonts::embedded();
    let lit = FamilyId(0);
    let charis = fonts
        .load_family("Charis", fonts_dir, "Charis")
        .unwrap_or(lit);
    let gentium = fonts
        .load_family("Gentium", fonts_dir, "GentiumBook")
        .unwrap_or(lit);
    (TextEngine::new(fonts), [lit, charis, gentium])
}

fn library_screens(out: &Path, fonts_dir: &Path) {
    let (mut text, _) = engine(fonts_dir);
    let styles = Styles::new(FamilyId(0));
    let books = library::sample_books();
    let (tree, mut lib) = Library::build(&mut text, &styles, &books, library::Filter::All);
    let mut ui = Ui::new(tree, text, W, H);
    ui.layout();
    lib.sync(&mut ui.tree);
    {
        let d = ui.render();
        log("library: first frame", &ui, &d)
    };
    save(ui.screen(), &out.join("library.png"));

    // Finger down on the second shelf row: an instant A2 outline.
    let c = ui.tree.rect(lib.rows[1]).center();
    touch(&mut ui, Phase::Down, c.x, c.y, 0);
    let d = ui.render();
    log("library: press on a book row", &ui, &d);
    save(ui.screen(), &out.join("library-pressed.png"));
    let o = damage_overlay(
        &mut ui,
        &d,
        "Touch-down on a row: only the outline goes out, with A2",
    );
    save(&o, &out.join("library-pressed-damage.png"));
    // Slide away: the press is cancelled and the row restored.
    touch(&mut ui, Phase::Move, c.x + 200, c.y, 40);
    touch(&mut ui, Phase::Up, c.x + 200, c.y, 80);
    {
        let d = ui.render();
        log("library: press cancelled", &ui, &d)
    };

    // Next shelf page.
    let next = ui.tree.rect(lib.next).center();
    for ev in tap(&mut ui, next, 1000) {
        if let UiEvent::Action { id, .. } = ev {
            lib.handle(&mut ui.tree, id);
        }
    }
    ui.layout();
    lib.sync(&mut ui.tree);
    let d = ui.render();
    log("library: shelf page 2", &ui, &d);
    save(ui.screen(), &out.join("library-page2.png"));
    let o = damage_overlay(&mut ui, &d, "Shelf page turn: the list and the pager only");
    save(&o, &out.join("library-page2-damage.png"));

    // Side by side with the stock home screen, if its screenshot exists.
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let stock = home.join(".cache/aristotle/screens/stock-home.png");
    if let Some(s) = load_gray(&stock) {
        let (tree, lib) = Library::build(&mut ui.text, &styles, &books, library::Filter::All);
        ui.set_tree(tree);
        ui.layout();
        lib.sync(&mut ui.tree);
        ui.render();
        let gap = 40;
        let mut both = Canvas::new(W * 2 + gap, H);
        both.fill(Rect::new(W as i32, 0, gap as i32, H as i32), 128);
        for y in 0..H as i32 {
            for x in 0..W as i32 {
                let i = (y as u32 * both.stride + x as u32) as usize;
                if x < s.width as i32 && y < s.height as i32 {
                    both.pixels[i] = s.get(x, y);
                }
                both.pixels[i + (W + gap) as usize] = ui.screen().get(x, y);
            }
        }
        save(&both, &out.join("compare-library.png"));
    }
}

fn reader_screens(out: &Path, fonts_dir: &Path) {
    let (mut text, families) = engine(fonts_dir);
    let styles = Styles::new(FamilyId(0));
    let (tree, mut r) = Reader::build(&mut text, &styles, families, Settings::default());
    let mut ui = Ui::new(tree, text, W, H);
    {
        let d = ui.render();
        log("reader: first frame", &ui, &d)
    };
    println!("  chapter set in {} pages", r.page_count());
    save(ui.screen(), &out.join("reader.png"));

    // Tap zones overlay.
    let mut z = ui.screen().clone();
    for (zone, rect) in reader::zones(ui.bounds()) {
        let name = match zone {
            Zone::Prev => "Previous",
            Zone::Next => "Next page",
            Zone::Menu => "Menu",
        };
        let style = TextStyle::new(FamilyId(0), 44.0).bold().gray(255);
        let l = set(&mut ui.text, name, style, 600.0);
        let mut p = Painter::new(&mut z, Rect::new(0, 0, W as i32, H as i32));
        dashed_rect(&mut p, rect.outset(-6), 6, 24, 0);
        let c = rect.center();
        let tag = Rect::new(
            c.x - l.width as i32 / 2 - 24,
            c.y - l.height as i32 / 2 - 10,
            l.width as i32 + 48,
            l.height as i32 + 20,
        );
        p.fill_round_rect(tag, 16.0, 0);
        l.draw(
            &mut ui.text,
            &mut p,
            tag.x as f32 + 24.0,
            tag.y as f32 + 10.0,
        );
    }
    save(&z, &out.join("reader-zones.png"));

    // Turn the page with a tap in the right zone.
    for ev in tap(&mut ui, Point { x: 850, y: 900 }, 0) {
        r.handle(&mut ui, ev);
    }
    let d = ui.render();
    log("reader: page turn", &ui, &d);
    save(ui.screen(), &out.join("reader-page2.png"));
    let o = damage_overlay(
        &mut ui,
        &d,
        "Page turn: text and footer with GL16; the running head is not refreshed",
    );
    save(&o, &out.join("reader-turn-damage.png"));

    // Back to page 1 with a swipe to the right.
    for (ph, x, ms) in [
        (Phase::Down, 300, 2000),
        (Phase::Move, 500, 2050),
        (Phase::Up, 750, 2100),
    ] {
        for ev in touch(&mut ui, ph, x, 800, ms) {
            r.handle(&mut ui, ev);
        }
    }
    {
        let d = ui.render();
        log("reader: swipe back", &ui, &d)
    };

    // Open the menu from the top band.
    for ev in tap(&mut ui, Point { x: 536, y: 70 }, 3000) {
        r.handle(&mut ui, ev);
    }
    let d = ui.render();
    log("reader: open menu", &ui, &d);
    save(ui.screen(), &out.join("reader-menu.png"));
    let o = damage_overlay(
        &mut ui,
        &d,
        "Opening the menu repaints the two bars; the page between them stays",
    );
    save(&o, &out.join("reader-menu-damage.png"));

    // Press feedback inside the menu, on the larger-text button.
    let size_row = ui.tree.rect(r.night_toggle);
    let probe = Point {
        x: W as i32 - 110,
        y: size_row.y - 3 * 120 - 50,
    };
    if let Hit::Target(node) = ui.tree.hit(probe) {
        let c = ui.tree.rect(node).center();
        touch(&mut ui, Phase::Down, c.x, c.y, 4000);
        let d = ui.render();
        log("reader menu: press", &ui, &d);
        save(ui.screen(), &out.join("reader-menu-pressed.png"));
        touch(&mut ui, Phase::Move, c.x, c.y + 300, 4040);
        touch(&mut ui, Phase::Up, c.x, c.y + 300, 4080);
        {
            let d = ui.render();
            log("reader menu: press cancelled", &ui, &d)
        };
    }

    // Night mode with the menu toggle, then close the menu.
    let night = ui.tree.rect(r.night_toggle).center();
    for ev in tap(&mut ui, night, 5000) {
        r.handle(&mut ui, ev);
    }
    {
        let d = ui.render();
        log("reader: night mode", &ui, &d)
    };
    save(ui.screen(), &out.join("reader-menu-night.png"));
    for ev in tap(&mut ui, Point { x: 700, y: 400 }, 6000) {
        r.handle(&mut ui, ev);
    }
    {
        let d = ui.render();
        log("reader: close menu", &ui, &d)
    };
    save(ui.screen(), &out.join("reader-night.png"));

    // Alternative typography: Charis, one size up, loose spacing.
    let (mut text, families) = engine(fonts_dir);
    let s = Settings {
        family: 1,
        size: 3,
        spacing: 2,
        ..Default::default()
    };
    let (tree, _) = Reader::build(&mut text, &styles, families, s);
    let mut ui = Ui::new(tree, text, W, H);
    ui.render();
    save(ui.screen(), &out.join("reader-charis.png"));
}

fn main() {
    let out = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").expect("HOME is set");
            PathBuf::from(home).join(".cache/aristotle/ui-preview")
        });
    std::fs::create_dir_all(&out).expect("output directory");
    let fonts_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
    library_screens(&out, &fonts_dir);
    reader_screens(&out, &fonts_dir);
}
