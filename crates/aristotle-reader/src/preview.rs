//! `aristotle --preview OUT BOOK`: the reading interactions, driven on the
//! host through the real app.
//!
//! The app runs on an in-memory panel ([`crate::panel::Memory`]) and gets
//! synthetic touches through the same entry points as the touchscreen
//! (`App::on_touch`, the long-press tick, `App::background`), so every
//! frame is what the device code would send. After each step the panel's
//! pixels go to `OUT/<step>.png` and its updates (waveform, rectangle) to
//! `OUT/updates.txt`.
//!
//! Targets (words, buttons) are found on the laid-out page and in the UI
//! tree, and the same taps are written as an `aristotle-observe` scenario
//! to `OUT/reading.txt`: on the device, with the same book and default
//! typography, the coordinates are the same.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use aristotle_hal::input::{monotonic_us, Phase as HalPhase, TouchEvent as HalEvent};
use aristotle_library::{Library, ScanOptions};
use aristotle_render::plato_core::anyhow::{format_err, Context, Error};
use aristotle_ui::geom::Point;
use aristotle_ui::tree::{ActionId, NodeId};

use crate::dict::{self, Dictionaries};
use crate::marks::ExportPaths;
use crate::overlay::act;
use crate::panel::{Memory, Panel};
use crate::prefs::Prefs;
use crate::reading::Overlay;
use crate::{App, Args, Flow, Screen};

struct Driver {
    app: App,
    /// Synthetic CLOCK_MONOTONIC, microseconds.
    t: u64,
    out: PathBuf,
    updates: BufWriter<File>,
    logged: usize,
    scenario: Vec<String>,
}

impl Driver {
    fn event(&mut self, phase: HalPhase, p: Point) -> Result<(), Error> {
        let ev = HalEvent {
            phase,
            id: 0,
            x: p.x.max(0) as u32,
            y: p.y.max(0) as u32,
            raw_x: p.x,
            raw_y: p.y,
            time_us: self.t,
        };
        if self.app.on_touch(&ev)? == Flow::Quit {
            return Err(format_err!("the app quit"));
        }
        self.after_events(true)
    }

    /// What the main loop does after a batch of events.
    fn after_events(&mut self, had_events: bool) -> Result<(), Error> {
        for e in self.app.ui.tick(self.t) {
            self.app.on_ui_event(e)?;
        }
        self.app.tick_reading(self.t)?;
        if had_events && self.app.ui_driven() {
            self.app.render_ui()?;
        }
        self.app.panel.poll();
        self.app.background()
    }

    fn advance(&mut self, ms: u64) -> Result<(), Error> {
        let end = self.t + ms * 1000;
        while self.t < end {
            self.t = (self.t + 20_000).min(end);
            self.after_events(false)?;
        }
        Ok(())
    }

    fn tap(&mut self, label: &str, p: Point) -> Result<(), Error> {
        self.scenario.push(format!("label {label}"));
        self.scenario.push(format!("tap {} {}", p.x, p.y));
        self.scenario.push("wait 1500".into());
        self.event(HalPhase::Down, p)?;
        self.advance(60)?;
        self.event(HalPhase::Up, p)?;
        self.advance(300)
    }

    fn hold(&mut self, label: &str, p: Point) -> Result<(), Error> {
        self.scenario.push(format!("label {label}"));
        self.scenario.push(format!("hold {} {} 700", p.x, p.y));
        self.scenario.push("wait 2000".into());
        self.event(HalPhase::Down, p)?;
        self.advance(700)?;
        self.event(HalPhase::Up, p)?;
        self.settle()
    }

    /// Hold at `from`, then drag to `to` and lift; `mid` snaps the
    /// selection while the finger is still down.
    fn drag(
        &mut self,
        label: &str,
        from: Point,
        to: Point,
        mid: Option<&str>,
    ) -> Result<(), Error> {
        self.scenario.push(format!("label {label}"));
        self.scenario.push(format!(
            "drag {} {} {} {} 700 600",
            from.x, from.y, to.x, to.y
        ));
        self.scenario.push("wait 2000".into());
        self.event(HalPhase::Down, from)?;
        self.advance(700)?;
        self.settle()?;
        for i in 1..=10 {
            let p = Point {
                x: from.x + (to.x - from.x) * i / 10,
                y: from.y + (to.y - from.y) * i / 10,
            };
            self.event(HalPhase::Move, p)?;
            self.advance(60)?;
        }
        if let Some(name) = mid {
            self.snap(name)?;
        }
        self.event(HalPhase::Up, to)?;
        self.advance(300)
    }

    /// Let a slow dictionary answer arrive.
    fn settle(&mut self) -> Result<(), Error> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            let waiting = matches!(&self.app.screen, Screen::Reader(r) if matches!(r.overlay, Overlay::Waiting { .. }));
            if !waiting {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            self.after_events(false)?;
        }
        Ok(())
    }

    fn snap(&mut self, name: &str) -> Result<(), Error> {
        let Panel::Memory(m) = &self.app.panel else {
            return Ok(());
        };
        write_png(m, &self.out.join(format!("{name}.png")))?;
        writeln!(self.updates, "== {name}")?;
        for u in &m.updates[self.logged..] {
            writeln!(
                self.updates,
                "  mode {:>2} {:<10} {}x{}+{}+{} {}",
                u.mode,
                if u.full_screen { "full" } else { "partial" },
                u.rect.w,
                u.rect.h,
                u.rect.x,
                u.rect.y,
                u.label
            )?;
        }
        self.logged = m.updates.len();
        self.scenario.push(format!("snap {name}"));
        println!("{name}.png");
        Ok(())
    }

    /// Centre of a word on the current page whose text (punctuation and
    /// soft hyphens aside) is `text`.
    fn word(&mut self, text: &str) -> Result<Point, Error> {
        let Screen::Reader(r) = &mut self.app.screen else {
            return Err(format_err!("no book open"));
        };
        let loc = r.session.current;
        let words = r.session.words(loc);
        let w = words
            .iter()
            .find(|w| dict::clean_word(&w.text) == text)
            .ok_or_else(|| format_err!("no word {text:?} on the page"))?;
        Ok(Point {
            x: (w.rect.x0 + w.rect.x1) / 2,
            y: (w.rect.y0 + w.rect.y1) / 2,
        })
    }

    /// Centre of the (visible) node carrying action `id`.
    fn button(&mut self, id: ActionId) -> Result<Point, Error> {
        self.app.ui.layout();
        let t = &self.app.ui.tree;
        for i in 0..t.len() {
            let n = NodeId(i as u32);
            let r = t.rect(n);
            if t.node(n).action == Some(id)
                && r.w > 0
                && r.h > 0
                && t.hit(r.center()) == aristotle_ui::tree::Hit::Target(n)
            {
                return Ok(r.center());
            }
        }
        Err(format_err!("no button for action {id}"))
    }

    /// A point on the page clear of the popup.
    fn outside_popup(&mut self) -> Point {
        let h = self.app.ui.bounds().h;
        match self.button(act::CANCEL) {
            Ok(p) if p.y > h / 2 => Point { x: 300, y: 260 },
            _ => Point { x: 300, y: h - 260 },
        }
    }

    fn check(&mut self, what: &str, ok: bool) {
        println!("  {} {what}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            self.scenario
                .push(format!("# preview check failed: {what}"));
        }
    }

    fn overlay_is(&self, f: impl Fn(&Overlay) -> bool) -> bool {
        matches!(&self.app.screen, Screen::Reader(r) if f(&r.overlay))
    }
}

fn write_png(m: &Memory, path: &Path) -> Result<(), Error> {
    let f = BufWriter::new(File::create(path)?);
    let mut enc = png::Encoder::new(f, m.width, m.height);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header()?;
    let mut data = Vec::with_capacity((m.width * m.height) as usize);
    for y in 0..m.height as usize {
        let row = y * m.stride;
        // What the panel can show: 16 levels.
        data.extend(
            m.pixels[row..row + m.width as usize]
                .iter()
                .map(|&p| p / 17 * 17),
        );
    }
    w.write_image_data(&data)?;
    Ok(())
}

pub fn run(args: &Args, out: &Path, book: &Path) -> Result<(), Error> {
    fs::create_dir_all(out)?;
    let stamp = aristotle_library::library::now();
    // A library of its own, and exports beside the PNGs: never into the
    // book's folder.
    let libdir = out.join(format!("library-{stamp}"));
    fs::create_dir_all(&libdir)?;
    let root = book
        .parent()
        .ok_or_else(|| format_err!("{} has no folder", book.display()))?;
    let mut lib = Library::open(&libdir)?;
    lib.scan(root, &ScanOptions::default())?;
    lib.save()?;
    let title = lib
        .books()
        .iter()
        .find(|b| root.join(&b.file.path) == book)
        .map(|b| b.meta.title.clone())
        .ok_or_else(|| format_err!("{} is not a book the library knows", book.display()))?;

    let dirs = crate::dict_dirs(args);
    // Build missing indexes first, so the preview shows answers.
    let cache = libdir.join("dict-index");
    let mut d = Dictionaries::discover(&dirs, &cache, None);
    dict::build_indexes(&d.missing_indexes());
    drop(d);

    let mut a = Args {
        library: libdir.clone(),
        root: root.to_path_buf(),
        ..args.clone()
    };
    a.dicts = dirs;
    let panel = Panel::memory(1072, 1448, crate::prefs::STOCK_TURN);
    let service = crate::start_dictionaries(&a);
    let mut app = crate::build_app(&a, panel, Prefs::default(), service, None)?;
    let export_root = out.join(format!("export-{stamp}/documents"));
    fs::create_dir_all(&export_root)?;
    app.exports = ExportPaths::for_root(&export_root);

    let mut d = Driver {
        app,
        t: monotonic_us(),
        out: out.to_path_buf(),
        updates: BufWriter::new(File::create(out.join("updates.txt"))?),
        logged: 0,
        scenario: vec![
            "# Reading interactions on the test book \"Próba czytania\"".into(),
            "# (devloop/host/make-reading-test-epub.py), default typography.".into(),
            "# Generated by `aristotle --preview`; see docs/reader.md.".into(),
        ],
    };
    d.app.open_by_title(&title)?;
    d.advance(200)?;
    d.snap("00-open")?;

    // Contents: the first chapter, so the page is known.
    let menu = Point { x: 536, y: 70 };
    d.tap("menu", menu)?;
    d.snap("01-menu")?;
    let p = d.button(act::NAV_TOC)?;
    d.tap("contents", p)?;
    d.snap("02-contents")?;
    let ok = d.overlay_is(|o| matches!(o, Overlay::Toc { .. }));
    d.check("contents list open", ok);
    let p = d.button(act::ITEM)?;
    d.tap("first chapter", p)?;
    d.snap("03-chapter-1")?;

    // Dictionary: a Polish word.
    let p = d.word("dusza")?;
    d.hold("hold dusza", p)?;
    d.snap("04-dict-polish")?;
    let ok = d.overlay_is(|o| matches!(o, Overlay::Dict { lookup, .. } if lookup.found.is_some()));
    d.check("Polish definition shown", ok);
    let p = d.outside_popup();
    d.tap("close", p)?;
    d.snap("05-closed")?;
    let ok = d.overlay_is(Overlay::is_none);
    d.check("popup closed", ok);

    // Greek: grc-pl first, then switch to grc-en with one tap.
    let p = d.word("ψυχή")?;
    d.hold("hold psyche", p)?;
    d.snap("06-dict-greek")?;
    let ok = d.overlay_is(|o| matches!(o, Overlay::Dict { lookup, .. } if lookup.found.is_some()));
    d.check("Greek definition shown", ok);
    if let Ok(tab) = d.button(act::TAB + 1).or_else(|_| d.button(act::TAB)) {
        d.tap("other dictionary", tab)?;
        d.snap("07-dict-greek-switched")?;
    }
    let p = d.outside_popup();
    d.tap("close", p)?;
    let p = d.word("λόγου")?;
    d.hold("hold logou", p)?;
    d.snap("08-dict-greek-form")?;
    let p = d.outside_popup();
    d.tap("close", p)?;

    // Select by dragging, then highlight.
    let from = d.word("Kto")?;
    let to = d.word("filozofii")?;
    d.drag("select", from, to, Some("09-selection-drag"))?;
    d.snap("10-selection-actions")?;
    let ok = d.overlay_is(|o| matches!(o, Overlay::Actions { .. }));
    d.check("actions after the drag", ok);
    let p = d.button(act::HIGHLIGHT)?;
    d.tap("highlight", p)?;
    d.snap("11-highlighted")?;

    // Word-by-word gloss of a Latin phrase.
    let from = d.word("memento")?;
    let to = d.word("mori")?;
    d.drag("select latin", from, to, None)?;
    let p = d.button(act::GLOSS)?;
    d.tap("gloss", p)?;
    d.snap("12-gloss")?;
    let p = d.button(act::CANCEL)?;
    d.tap("close", p)?;

    // Footnote: tap the marker after "nieśmiertelna".
    let p = {
        let Screen::Reader(r) = &mut d.app.screen else {
            return Err(format_err!("no book open"));
        };
        let loc = r.session.current;
        let links = r.session.links(loc);
        let l = links
            .iter()
            .find(|l| l.uri.contains("fn1"))
            .ok_or_else(|| format_err!("no footnote link on the page"))?;
        Point {
            x: (l.rect.x0 + l.rect.x1) / 2,
            y: (l.rect.y0 + l.rect.y1) / 2,
        }
    };
    d.tap("footnote", p)?;
    d.snap("13-footnote")?;
    let ok = d.overlay_is(|o| matches!(o, Overlay::Note { .. }));
    d.check("footnote popup", ok);
    let p = d.button(act::CANCEL)?;
    d.tap("close", p)?;

    // Bookmark corner.
    d.tap("bookmark", Point { x: 1030, y: 45 })?;
    d.snap("14-bookmark")?;

    // Highlights list.
    d.tap("menu", menu)?;
    let p = d.button(act::NAV_MARKS)?;
    d.tap("highlights", p)?;
    d.snap("15-highlights")?;
    let p = d.button(act::BACK)?;
    d.tap("back", p)?;

    // Go to 50 %.
    d.tap("menu", menu)?;
    let p = d.button(act::NAV_GOTO)?;
    d.tap("go to", p)?;
    for _ in 0..5 {
        let p = d.button(act::GOTO_PLUS10)?;
        d.tap("+10", p)?;
    }
    d.snap("16-goto")?;
    let p = d.button(act::GOTO_GO)?;
    d.tap("go", p)?;
    d.snap("17-after-goto")?;

    // Exports.
    d.updates.flush()?;
    let clip = d.app.exports.clippings.clone();
    let clippings = fs::read_to_string(&clip).with_context(|| format!("{}", clip.display()))?;
    fs::write(out.join("My Clippings.txt"), &clippings)?;
    println!(
        "export: {} ({} bytes), notes in {}",
        clip.display(),
        clippings.len(),
        d.app.exports.notes.display()
    );
    let mut f = File::create(out.join("reading.txt"))?;
    for line in &d.scenario {
        writeln!(f, "{line}")?;
    }
    println!("scenario: {}", out.join("reading.txt").display());
    let Driver { app, .. } = d;
    app.leave()?;
    Ok(())
}
