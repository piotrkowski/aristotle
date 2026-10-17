//! `aristotle`: the reader app for the Kindle Paperwhite 4.
//!
//! The library from `aristotle-library` (scanned incrementally on start,
//! covers, progress, the book to continue), pages from `aristotle-render`
//! (Plato's engine, Polish hyphenation), and the menu from `aristotle-ui`,
//! on the panel and touchscreen through `aristotle-hal`.
//!
//! ```text
//! aristotle [--data DIR] [--library DIR] [--root DIR] [--turn-mode N] [--no-scan]
//!
//!   --data DIR      renderer data (fonts/, css/, hyphenation-patterns/,
//!                   serif/); default: data/ next to the binary
//!   --library DIR   library index, covers, page cache and preferences;
//!                   default: $ARISTOTLE_DATA or /mnt/us/.aristotle
//!   --root DIR      where the books are; default: /mnt/us/documents
//!   --turn-mode N   page-turn waveform: 9 (stock, default) or 7 (DU4);
//!                   saved in reader.json
//!   --no-scan       don't look for new or changed books on start
//! ```
//!
//! Run it with the stock GUI stopped: `devloop/kindle/run-aristotle.sh`.
//! Hold the "Biblioteka" title for a second to quit. Timings go to stderr
//! as `[perf ...]` lines; see docs/reader.md.

mod pagecache;
mod panel;
mod prefs;
mod progress;
mod session;
mod shelf;
mod turns;

use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use aristotle_hal::input::{monotonic_us, Phase as HalPhase, TouchScreen, Transform};
use aristotle_hal::power::{Battery, Frontlight};
use aristotle_library::{Library, ScanOptions, Status};
use aristotle_render::page::Page;
use aristotle_render::plato_core::anyhow::{bail, format_err, Context, Error};
use aristotle_ui::geom::{Rect, Size, MIN_TARGET};
use aristotle_ui::input::{Phase, TouchEvent};
use aristotle_ui::screens::library::{self as ui_lib, Filter};
use aristotle_ui::screens::reader::{action as menu_action, build_menu, Menu, MenuSpec, Settings};
use aristotle_ui::text::{FamilyId, Fonts, TextEngine};
use aristotle_ui::theme::Styles;
use aristotle_ui::tree::{Constraints, Kind, Node, NodeId, PaintCx, Refresh, Tree, Widget};
use aristotle_ui::ui::{Ui, UiEvent};
use aristotle_ui::widgets::{Slider, Steps};

use crate::panel::{Content, Panel};
use crate::prefs::Prefs;
use crate::progress::{Debounce, Saver};
use crate::session::Session;
use crate::shelf::{Covers, ShelfModel, SORTS};
use crate::turns::{Intent, Turn, TurnGestures};

static START: OnceLock<Instant> = OnceLock::new();

/// Milliseconds since the process started.
pub fn uptime_ms() -> f64 {
    ms(START.get_or_init(Instant::now).elapsed())
}

pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// A timing line on stderr: `[perf   1234.5] what happened`.
#[macro_export]
macro_rules! perf {
    ($($arg:tt)*) => {
        eprintln!("[perf {:>9.1}] {}", $crate::uptime_ms(), format!($($arg)*))
    };
}

/// Set by SIGTERM and SIGINT so the loop saves progress and leaves cleanly.
static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

struct Args {
    data: PathBuf,
    library: PathBuf,
    root: PathBuf,
    turn_mode: Option<u32>,
    scan: bool,
}

fn usage() -> &'static str {
    "usage: aristotle [--data DIR] [--library DIR] [--root DIR] [--turn-mode 9|7] [--no-scan]"
}

fn parse_args() -> Result<Args, Error> {
    let exe_dir = std::env::current_exe()?
        .parent()
        .ok_or_else(|| format_err!("binary has no parent directory"))?
        .to_path_buf();
    let cwd = std::env::current_dir()?;
    let mut a = Args {
        data: exe_dir.join("data"),
        library: std::env::var_os("ARISTOTLE_DATA")
            .map_or_else(|| PathBuf::from("/mnt/us/.aristotle"), PathBuf::from),
        root: PathBuf::from("/mnt/us/documents"),
        turn_mode: None,
        scan: true,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format_err!("{name} needs a value"));
        match arg.as_str() {
            "--data" => a.data = cwd.join(value("--data")?),
            "--library" => a.library = cwd.join(value("--library")?),
            "--root" => a.root = cwd.join(value("--root")?),
            "--turn-mode" => a.turn_mode = Some(value("--turn-mode")?.parse()?),
            "--no-scan" => a.scan = false,
            "-h" | "--help" => bail!("{}", usage()),
            other => bail!("unknown argument {other}\n{}", usage()),
        }
    }
    Ok(a)
}

fn main() -> ExitCode {
    START.get_or_init(Instant::now);
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aristotle: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Error> {
    let args = parse_args()?;
    // SAFETY: the handler only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
    // Plato opens its fonts and patterns relative to the current directory.
    aristotle_render::use_data_dir(&args.data)?;
    std::fs::create_dir_all(&args.library)
        .with_context(|| format!("can't create {}", args.library.display()))?;

    let mut prefs = Prefs::load(&args.library);
    if let Some(mode) = args.turn_mode {
        if mode != prefs::STOCK_TURN && mode != prefs::DU4_TURN {
            eprintln!(
                "aristotle: turn mode {mode} is untested; 9 (stock) and 7 (DU4) were measured"
            );
        }
        prefs.turn_mode = mode;
        let _ = prefs.save(&args.library);
    }
    let panel = Panel::open(prefs.turn_mode).context("can't open the framebuffer")?;
    let (w, h) = panel.size();
    let mut touch =
        TouchScreen::open(w, h, Transform::default()).context("can't open the touchscreen")?;
    touch.grab(true)?;
    let lib = Library::open(&args.library)?;
    let saver = Saver::new(&args.library);
    perf!(
        "startup: devices and library index ready ({} books, turn mode {})",
        lib.books().len(),
        prefs.turn_mode
    );

    let mut text = TextEngine::new(Fonts::embedded());
    let styles = Styles::new(FamilyId(0));
    let mut covers = Covers::default();
    let battery = Battery::open().ok();
    let model = ShelfModel::build(&lib, Filter::All, 0, &mut covers);
    let clock = clock();
    let level = battery_level(&battery);
    let (tree, view) =
        ui_lib::Library::build_shelf(&mut text, &styles, &model.shelf(&clock, level));
    let ui = Ui::new(tree, text, w, h);
    let mut app = App {
        panel,
        light: Frontlight::open().ok(),
        battery,
        mobi_cache: args.library.join("epub-cache"),
        lib,
        saver,
        debounce: Debounce::default(),
        prefs,
        ui,
        styles,
        covers,
        screen: Screen::Library { model, view },
        filter: Filter::All,
        sort: 0,
        shelf_turns: 0,
    };
    app.enter_library_ui();
    app.ui.request_full_refresh();
    app.render_ui()?;
    perf!("startup: first frame submitted");

    if args.scan {
        app.scan(&args.root)?;
    }
    let result = app.run(&mut touch);
    app.leave()?;
    result
}

/// A Plato page shown as the reader screen's background.
struct PageView {
    page: Rc<Page>,
}

impl Widget for PageView {
    fn measure(&mut self, c: Constraints, _: &mut TextEngine) -> Size {
        c.constrain(c.max)
    }

    fn paint(&self, r: Rect, cx: &mut PaintCx) {
        let p = &self.page;
        cx.painter
            .blit_gray(r.x, r.y, p.width, p.height, p.stride, &p.data);
    }
}

struct Reading {
    session: Session,
    menu: Menu,
    page_node: NodeId,
    menu_open: bool,
    gestures: TurnGestures,
    settings: Settings,
}

// One screen exists at a time and it is never moved in a hot path, so the
// size difference between the variants costs nothing.
#[allow(clippy::large_enum_variant)]
enum Screen {
    Library {
        model: ShelfModel,
        view: ui_lib::Library,
    },
    Reader(Box<Reading>),
}

struct App {
    panel: Panel,
    light: Option<Frontlight>,
    battery: Option<Battery>,
    mobi_cache: PathBuf,
    lib: Library,
    saver: Saver,
    debounce: Debounce,
    prefs: Prefs,
    ui: Ui,
    styles: Styles,
    covers: Covers,
    screen: Screen,
    filter: Filter,
    sort: usize,
    /// Shelf page turns since the last flash.
    shelf_turns: u32,
}

/// What a handler wants the loop to do next.
#[derive(PartialEq, Eq)]
enum Flow {
    Continue,
    Quit,
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

/// Local time as "HH:MM" (musl reads the zone from `TZ`).
fn clock() -> String {
    // SAFETY: time and localtime_r only write into the values passed.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return String::new();
        }
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    }
}

fn battery_level(b: &Option<Battery>) -> Option<f32> {
    b.as_ref()?.status().ok().map(|s| s.capacity as f32 / 100.0)
}

/// The quit target: the library title, hit-tested at least 9 mm high.
fn quit_zone(ui: &Ui, title: NodeId) -> Rect {
    ui.tree.rect(title).expand_to(MIN_TARGET)
}

impl App {
    /// Render and send UI chrome changes.
    fn render_ui(&mut self) -> Result<(), Error> {
        self.render_ui_with(Content::Chrome)
    }

    fn render_ui_with(&mut self, content: Content) -> Result<(), Error> {
        let damage = self.ui.render();
        self.panel.present(&self.ui, &damage, content)?;
        Ok(())
    }

    /// Settings for a freshly installed library tree.
    fn enter_library_ui(&mut self) {
        self.ui.layout();
        if let Screen::Library { view, .. } = &self.screen {
            view.sync(&mut self.ui.tree);
        }
        // Quitting is drastic: ask for a full second's hold.
        self.ui.gesture_config_mut().long_press_us = 1_000_000;
    }

    /// Rebuild the library screen from the index.
    fn show_library(&mut self, refresh: Refresh) -> Result<(), Error> {
        let model = ShelfModel::build(&self.lib, self.filter, self.sort, &mut self.covers);
        let clock = clock();
        let level = battery_level(&self.battery);
        let (tree, view) = ui_lib::Library::build_shelf(
            &mut self.ui.text,
            &self.styles,
            &model.shelf(&clock, level),
        );
        self.ui.set_tree_with(tree, refresh);
        self.screen = Screen::Library { model, view };
        self.enter_library_ui();
        self.render_ui()
    }

    /// Bring the index in line with the books on the device.
    fn scan(&mut self, root: &std::path::Path) -> Result<(), Error> {
        if !root.is_dir() {
            eprintln!("aristotle: no library folder {}", root.display());
            return Ok(());
        }
        let r = self.lib.scan(root, &ScanOptions::default())?;
        perf!(
            "scan: {:.0} ms ({} unchanged, {} added, {} updated, {} moved, {} removed, {} thumbnails, {} sidecars imported, {} errors)",
            ms(r.total_time),
            r.unchanged,
            r.added,
            r.updated,
            r.moved,
            r.removed,
            r.thumbnails,
            r.sidecars_imported,
            r.errors.len()
        );
        for (path, err) in &r.errors {
            eprintln!("aristotle: scan: {path}: {err}");
        }
        if r.added + r.updated + r.moved + r.removed + r.restored + r.sidecars_imported > 0 {
            self.saver.save(&self.lib);
            if matches!(self.screen, Screen::Library { .. }) {
                self.show_library(Refresh::Auto)?;
            }
        }
        Ok(())
    }

    fn run(&mut self, touch: &mut TouchScreen) -> Result<(), Error> {
        loop {
            if STOP.load(Ordering::Relaxed) {
                return Ok(());
            }
            let now = monotonic_us();
            let mut timeout = Duration::from_secs(1);
            if let Some(t) = self.ui.next_deadline(now) {
                timeout = timeout.min(Duration::from_micros(t.saturating_sub(now).max(1000)));
            }
            if let Some(d) = self.debounce.deadline() {
                timeout = timeout.min(d.saturating_duration_since(Instant::now()));
            }
            if let Screen::Reader(r) = &self.screen {
                if r.session.paginating() {
                    timeout = timeout.min(Duration::from_millis(250));
                }
            }
            let events = match touch.poll(timeout.max(Duration::from_millis(1))) {
                Ok(e) => e,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            };
            for ev in &events {
                if self.on_touch(ev)? == Flow::Quit {
                    return Ok(());
                }
            }
            let ticked = self.ui.tick(monotonic_us());
            for ev in ticked {
                if self.on_ui_event(ev)? == Flow::Quit {
                    return Ok(());
                }
            }
            if !events.is_empty() && self.ui_driven() {
                self.render_ui()?;
            }
            self.panel.poll();
            self.background()?;
        }
    }

    /// Whether the UI tree (not the page-turn path) owns input right now.
    fn ui_driven(&self) -> bool {
        match &self.screen {
            Screen::Library { .. } => true,
            Screen::Reader(r) => r.menu_open,
        }
    }

    /// Work done between events: adopting page numbers, saving progress.
    fn background(&mut self) -> Result<(), Error> {
        if let Screen::Reader(r) = &mut self.screen {
            if r.session.poll_pagination() {
                let (pn, pp) = r.session.prerender();
                perf!(
                    "paginate: page numbers adopted; prerender again {:.0} + {:.0} ms",
                    ms(pn),
                    ms(pp)
                );
            }
        }
        if self.debounce.due(Instant::now()) {
            self.saver.save(&self.lib);
            self.debounce.saved();
        }
        Ok(())
    }

    fn on_touch(&mut self, ev: &aristotle_hal::input::TouchEvent) -> Result<Flow, Error> {
        let ev = ui_event(ev);
        if let Screen::Reader(r) = &mut self.screen {
            if !r.menu_open {
                let intents = r.gestures.feed(ev);
                for i in intents {
                    self.on_intent(i)?;
                }
                return Ok(Flow::Continue);
            }
        }
        for e in self.ui.handle_touch(ev) {
            if self.on_ui_event(e)? == Flow::Quit {
                return Ok(Flow::Quit);
            }
        }
        Ok(Flow::Continue)
    }

    fn on_ui_event(&mut self, ev: UiEvent) -> Result<Flow, Error> {
        match &self.screen {
            Screen::Library { .. } => self.on_library_event(ev),
            Screen::Reader(_) => self.on_menu_event(ev).map(|_| Flow::Continue),
        }
    }

    fn on_library_event(&mut self, ev: UiEvent) -> Result<Flow, Error> {
        let Screen::Library { model, view } = &mut self.screen else {
            return Ok(Flow::Continue);
        };
        match ev {
            UiEvent::LongPress(p) if quit_zone(&self.ui, view.title).contains(p) => {
                perf!("quit: title held");
                return Ok(Flow::Quit);
            }
            UiEvent::Action { id, .. } => match id {
                ui_lib::action::RESUME => {
                    if let Some(book) = model.hero_id().map(String::from) {
                        self.open_book(&book)?;
                    }
                }
                ui_lib::action::SORT => {
                    self.sort = (self.sort + 1) % SORTS.len();
                    self.show_library(Refresh::Auto)?;
                }
                id if id >= ui_lib::action::BOOK => {
                    if let Some(book) = model.book_id((id - ui_lib::action::BOOK) as usize) {
                        let book = book.to_string();
                        self.open_book(&book)?;
                    }
                }
                id if (ui_lib::action::FILTER..ui_lib::action::FILTER + 4).contains(&id) => {
                    self.filter = Filter::ALL[(id - ui_lib::action::FILTER) as usize];
                    self.show_library(Refresh::Auto)?;
                }
                _ => {
                    if view.handle(&mut self.ui.tree, id) {
                        self.ui.layout();
                        view.sync(&mut self.ui.tree);
                        // Every few shelf pages, a flash clears the ghosts
                        // of covers, as the stock UI does.
                        self.shelf_turns += 1;
                        if self.shelf_turns.is_multiple_of(5) {
                            self.ui.request_full_refresh();
                        }
                    }
                }
            },
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn open_book(&mut self, id: &str) -> Result<(), Error> {
        let Some(rec) = self.lib.book(id).cloned() else {
            return Ok(());
        };
        let t0 = Instant::now();
        let (mut session, times) =
            match Session::open(&self.lib, &rec, self.prefs.style(), &self.mobi_cache) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("aristotle: {}: {e:#}", rec.meta.title);
                    return Ok(());
                }
            };
        let page = session.page(session.current)?;
        let light = self.light_level();
        let settings = self.prefs.menu_settings(light);
        let (tree, menu, page_node) = reader_tree(&self.styles, &session.title, &settings, page);
        // A screen switch: the stock UI's flashing full refresh (mode 8).
        self.ui.set_tree_with(tree, Refresh::Full);
        self.ui.gesture_config_mut().long_press_us = 500_000;
        self.render_ui()?;
        perf!(
            "open: \"{}\": book {:.0} ms, position {:.0} ms, first page {:.0} ms, submitted after {:.0} ms; page numbers {}",
            session.title,
            ms(times.open),
            ms(times.place),
            ms(times.render),
            ms(t0.elapsed()),
            if times.pages_cached { "from cache" } else { "being computed in the background" }
        );
        self.lib.mark_opened(id, aristotle_library::library::now());
        self.debounce.changed(Instant::now());
        let (pn, pp) = session.prerender();
        perf!(
            "open: prerender next {:.0} ms, previous {:.0} ms",
            ms(pn),
            ms(pp)
        );
        self.screen = Screen::Reader(Box::new(Reading {
            session,
            menu,
            page_node,
            menu_open: false,
            gestures: TurnGestures::new(self.ui.bounds()),
            settings,
        }));
        Ok(())
    }

    fn light_level(&self) -> f32 {
        self.light
            .as_ref()
            .and_then(|l| l.brightness().ok().map(|b| prefs::raw_to_light(b, l.max())))
            .unwrap_or(0.0)
    }

    fn on_intent(&mut self, intent: Intent) -> Result<(), Error> {
        match intent {
            Intent::Turn { dir, at_us } => self.turn(dir, at_us),
            Intent::OpenMenu => self.open_menu(),
        }
    }

    /// A page turn: show the (normally prerendered) page with one full
    /// update, then prepare the next ones while the panel works.
    fn turn(&mut self, dir: Turn, at_us: u64) -> Result<(), Error> {
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        let s = &mut r.session;
        let Some(target) = s.neighbor(s.current, dir) else {
            if dir == Turn::Forward {
                self.lib.set_status(&s.id, Status::Finished);
                self.debounce.changed(Instant::now());
            }
            return Ok(());
        };
        let t = Instant::now();
        let ready = s.is_rendered(target);
        let page = s.page(target)?;
        let t_render = t.elapsed();
        let waited = self.panel.show_page(&page, false, "turn")?;
        let tap_to_submit = monotonic_us().saturating_sub(at_us) as f64 / 1000.0;
        s.current = target;
        let (pn, pp) = s.prerender();
        let t = Instant::now();
        if let Some(w) = self.ui.tree.widget_mut::<PageView>(r.page_node) {
            w.page = page;
        }
        self.ui.tree.invalidate(r.page_node, Refresh::Auto);
        // The panel already shows the page; this only updates the UI's copy.
        self.ui.render();
        let t_sync = t.elapsed();
        perf!(
            "turn {:?}: tap->submit {:.0} ms (page {}, {:.0} ms; waited for the panel {:.0} ms); prerender next {:.0} ms, previous {:.0} ms; ui copy {:.0} ms",
            dir,
            tap_to_submit,
            if ready { "prerendered" } else { "rendered now" },
            ms(t_render),
            ms(waited),
            ms(pn),
            ms(pp),
            ms(t_sync)
        );
        let now = aristotle_library::library::now();
        self.lib.set_progress(&s.id, s.progress(now));
        if s.at_end() {
            self.lib.set_status(&s.id, Status::Finished);
        }
        self.debounce.changed(Instant::now());
        Ok(())
    }

    fn open_menu(&mut self) -> Result<(), Error> {
        let light = self.light_level();
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        r.menu_open = true;
        update_menu_labels(&mut self.ui, r);
        r.settings.light = light;
        if let Some(w) = self.ui.tree.widget_mut::<Slider>(r.menu.light) {
            w.value = light;
        }
        self.ui.tree.invalidate(r.menu.light, Refresh::Auto);
        self.ui.tree.set_hidden(r.menu.root, false);
        self.ui.reset_gestures();
        self.render_ui()
    }

    /// Close the menu: the page comes back with a full mode-9 update of the
    /// whole screen, which also clears what the menu's updates left behind.
    fn close_menu(&mut self) -> Result<(), Error> {
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        r.menu_open = false;
        r.gestures.reset();
        self.ui.tree.set_hidden(r.menu.root, true);
        self.ui.render();
        let page = r.session.page(r.session.current)?;
        self.panel.show_page(&page, false, "menu closed")?;
        Ok(())
    }

    fn on_menu_event(&mut self, ev: UiEvent) -> Result<(), Error> {
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        if !r.menu_open {
            return Ok(());
        }
        match ev {
            // A tap on the page between the bars closes the menu.
            UiEvent::Tap(_) => self.close_menu(),
            UiEvent::Action { id, node, at } => {
                let mut s = r.settings;
                match id {
                    menu_action::BACK => self.back_to_library(),
                    menu_action::LIGHT_SET | menu_action::LIGHT_DOWN | menu_action::LIGHT_UP => {
                        s.light = match id {
                            menu_action::LIGHT_SET => {
                                Slider::value_at(self.ui.tree.rect(node), at.x)
                            }
                            menu_action::LIGHT_DOWN => (s.light - 1.0 / 24.0).max(0.0),
                            _ => (s.light + 1.0 / 24.0).min(1.0),
                        };
                        if let Some(l) = &self.light {
                            let _ = l.set_brightness(prefs::light_to_raw(s.light, l.max()));
                        }
                        if let Some(w) = self.ui.tree.widget_mut::<Slider>(r.menu.light) {
                            w.value = s.light;
                        }
                        self.ui.tree.invalidate(r.menu.light, Refresh::Auto);
                        r.settings = s;
                        Ok(())
                    }
                    menu_action::PREV_CHAPTER | menu_action::NEXT_CHAPTER => {
                        let forward = id == menu_action::NEXT_CHAPTER;
                        if let Some(loc) = r.session.chapter(forward) {
                            self.jump(loc)?;
                        }
                        Ok(())
                    }
                    menu_action::SIZE_DOWN => {
                        s.size = s.size.saturating_sub(1);
                        self.restyle(s)
                    }
                    menu_action::SIZE_UP => {
                        s.size = (s.size + 1).min(prefs::FONT_PT.len() - 1);
                        self.restyle(s)
                    }
                    i if (menu_action::SPACING..menu_action::SPACING + 3).contains(&i) => {
                        s.spacing = (i - menu_action::SPACING) as usize;
                        self.restyle(s)
                    }
                    i if (menu_action::MARGINS..menu_action::MARGINS + 3).contains(&i) => {
                        s.margins = (i - menu_action::MARGINS) as usize;
                        self.restyle(s)
                    }
                    _ => Ok(()),
                }
            }
            _ => Ok(()),
        }
    }

    /// Show the page at `location` behind the open menu.
    fn jump(&mut self, location: usize) -> Result<(), Error> {
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        r.session.current = location;
        let page = r.session.page(location)?;
        if let Some(w) = self.ui.tree.widget_mut::<PageView>(r.page_node) {
            w.page = page;
        }
        self.ui.tree.invalidate(r.page_node, Refresh::Auto);
        update_menu_labels(&mut self.ui, r);
        let now = aristotle_library::library::now();
        self.lib
            .set_progress(&r.session.id, r.session.progress(now));
        self.debounce.changed(Instant::now());
        self.render_ui_with(Content::Page)?;
        if let Screen::Reader(r) = &mut self.screen {
            r.session.prerender();
        }
        Ok(())
    }

    /// New typography from the menu: lay the book out again at the same
    /// place and rebuild the screen with the menu still open.
    fn restyle(&mut self, settings: Settings) -> Result<(), Error> {
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        if settings == r.settings {
            return Ok(());
        }
        self.prefs.apply(&settings);
        if let Err(e) = self.prefs.save(self.lib.data_dir()) {
            eprintln!("aristotle: can't save preferences: {e}");
        }
        let t = Instant::now();
        r.session.restyle(self.prefs.style())?;
        let page = r.session.page(r.session.current)?;
        let t_layout = t.elapsed();
        r.settings = settings;
        let (tree, menu, page_node) = reader_tree(&self.styles, &r.session.title, &settings, page);
        r.menu = menu;
        r.page_node = page_node;
        self.ui.set_tree_with(tree, Refresh::Auto);
        self.ui.tree.set_hidden(r.menu.root, false);
        update_menu_labels(&mut self.ui, r);
        perf!(
            "restyle: {} pt, line height {}, margin {} mm: layout and page {:.0} ms",
            r.session.style.font_size,
            r.session.style.line_height,
            r.session.style.margin_mm,
            ms(t_layout)
        );
        self.render_ui_with(Content::Page)?;
        if let Screen::Reader(r) = &mut self.screen {
            r.session.prerender();
        }
        Ok(())
    }

    fn back_to_library(&mut self) -> Result<(), Error> {
        if let Screen::Reader(r) = &self.screen {
            let now = aristotle_library::library::now();
            self.lib
                .set_progress(&r.session.id, r.session.progress(now));
        }
        self.saver.save(&self.lib);
        self.debounce.saved();
        self.show_library(Refresh::Full)
    }

    /// Save progress and leave a white screen for the stock UI.
    fn leave(self) -> Result<(), Error> {
        let mut app = self;
        if let Screen::Reader(r) = &app.screen {
            let now = aristotle_library::library::now();
            app.lib.set_progress(&r.session.id, r.session.progress(now));
        }
        app.saver.save(&app.lib);
        app.saver.flush();
        app.panel.clear()?;
        perf!("quit: progress saved, screen cleared");
        Ok(())
    }
}

/// The reader screen: the page with the (hidden) menu over it.
fn reader_tree(
    styles: &Styles,
    title: &str,
    settings: &Settings,
    page: Rc<Page>,
) -> (Tree, Menu, NodeId) {
    let mut t = Tree::new();
    let page_node = t.custom(PageView { page });
    let spec = MenuSpec {
        title,
        // Only Literata ships with the renderer, and night mode would need
        // inverted pages; both stay out of the menu until they work.
        families: None,
        night: false,
    };
    let menu = build_menu(&mut t, styles, &spec, settings);
    let root = t.add(Node::new(Kind::Stack), [page_node, menu.root]);
    t.set_root(root);
    (t, menu, page_node)
}

fn update_menu_labels(ui: &mut Ui, r: &mut Reading) {
    let (chapter, page, fraction) = r.session.labels();
    ui.tree.set_text(r.menu.chapter_label, &chapter);
    ui.tree.set_text(r.menu.page_label, &page);
    ui.tree.set_progress(r.menu.progress, fraction);
    if let Some(w) = ui.tree.widget_mut::<Steps>(r.menu.size_steps) {
        w.value = r.settings.size;
    }
}

/// A fresh, empty directory for one test, under `$TMPDIR` (the build
/// script points it at `~/.cache/aristotle/tmp`, not the RAM-backed /tmp).
#[cfg(test)]
pub(crate) fn testdir(name: &str) -> PathBuf {
    use std::sync::atomic::AtomicUsize;
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let dir = std::env::temp_dir()
        .join("aristotle-reader-tests")
        .join(format!("{name}-{}-{n}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
