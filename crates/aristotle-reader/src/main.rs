//! `aristotle`: the reader app for the Kindle Paperwhite 4.
//!
//! The library from `aristotle-library` (scanned incrementally on start,
//! covers, progress, the book to continue), pages from `aristotle-render`
//! (Plato's engine, Polish hyphenation), and the menu from `aristotle-ui`,
//! on the panel and touchscreen through `aristotle-hal`.
//!
//! ```text
//! aristotle [--data DIR] [--library DIR] [--root DIR] [--turn-mode N] [--no-scan]
//!           [--open TITLE] [--dict DIR]... [--bench-open TITLE] [--preview OUT BOOK]
//!
//!   --data DIR      renderer data (fonts/, css/, hyphenation-patterns/,
//!                   serif/); default: data/ next to the binary
//!   --library DIR   library index, covers, page cache and preferences;
//!                   default: $ARISTOTLE_DATA or /mnt/us/.aristotle
//!   --root DIR      where the books are; default: /mnt/us/documents
//!   --turn-mode N   page-turn waveform: 9 (stock, default) or 7 (DU4);
//!                   saved in reader.json
//!   --no-scan       don't look for new or changed books on start
//!   --open TITLE    open the first book whose title contains TITLE after
//!                   start (device scenarios)
//!   --dict DIR      look for StarDict dictionaries here (repeatable);
//!                   default: /mnt/us/dictionaries, /mnt/us/koreader/data/dict
//!   --index-dicts   build the dictionaries' lookup indexes into
//!                   LIBRARY/dict-index and exit (on the host, to copy to
//!                   the device instead of indexing there)
//!   --bench-open T  time opening the first book whose title contains T,
//!                   without panel or touchscreen, and exit (see open.rs)
//!   --preview OUT BOOK  drive the reading interactions on BOOK with
//!                   synthetic touches on an in-memory panel and write PNGs
//!                   to OUT (host; see preview.rs)
//! ```
//!
//! Run it with the stock GUI stopped: `devloop/kindle/run-aristotle.sh`.
//! Hold the "Biblioteka" title for a second to quit. Timings go to stderr
//! as `[perf ...]` lines; see docs/reader.md.

mod dict;
mod marks;
mod open;
mod overlay;
mod pagecache;
mod panel;
mod prefs;
mod preview;
mod progress;
mod reading;
mod selection;
mod session;
mod shelf;
mod snapshot;
mod speed;
mod turns;

use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use aristotle_hal::input::{monotonic_us, Phase as HalPhase, TouchScreen, Transform};
use aristotle_hal::power::{Battery, Frontlight};
use aristotle_library::{Library, Progress, ScanOptions, Status};
use aristotle_render::page::Page;
use aristotle_render::plato_core::anyhow::{bail, format_err, Context, Error};
use aristotle_render::style::Style;
use aristotle_ui::geom::{Rect, Size, MIN_TARGET};
use aristotle_ui::input::{Phase, TouchEvent};
use aristotle_ui::screens::library::{self as ui_lib, Filter};
use aristotle_ui::screens::reader::{action as menu_action, build_menu, Menu, MenuSpec, Settings};
use aristotle_ui::text::{FamilyId, Fonts, TextEngine};
use aristotle_ui::theme::Styles;
use aristotle_ui::tree::{Constraints, Kind, Node, NodeId, PaintCx, Refresh, Tree, Widget};
use aristotle_ui::ui::{Ui, UiEvent};
use aristotle_ui::widgets::{Slider, Steps};

use crate::dict::service::Service;
use crate::marks::ExportPaths;
use crate::open::Opening;
use crate::panel::{Content, Panel};
use crate::prefs::Prefs;
use crate::progress::{Debounce, Saver};
use crate::reading::{Overlay, OverlayNodes};
use crate::session::{Prepared, Session, Source};
use crate::shelf::{Covers, ShelfModel, SORTS};
use crate::speed::Speed;
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

#[derive(Clone)]
struct Args {
    data: PathBuf,
    library: PathBuf,
    root: PathBuf,
    turn_mode: Option<u32>,
    scan: bool,
    bench_open: Option<String>,
    dicts: Vec<PathBuf>,
    preview: Option<(PathBuf, PathBuf)>,
    open: Option<String>,
    index_dicts: bool,
}

fn usage() -> &'static str {
    "usage: aristotle [--data DIR] [--library DIR] [--root DIR] [--turn-mode 9|7] [--no-scan] [--dict DIR]... [--preview OUT BOOK]"
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
        bench_open: None,
        dicts: Vec::new(),
        preview: None,
        open: None,
        index_dicts: false,
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
            "--bench-open" => a.bench_open = Some(value("--bench-open")?),
            "--dict" => a.dicts.push(cwd.join(value("--dict")?)),
            "--open" => a.open = Some(value("--open")?),
            "--index-dicts" => a.index_dicts = true,
            "--preview" => {
                let out = cwd.join(value("--preview")?);
                let book = cwd.join(value("--preview")?);
                a.preview = Some((out, book));
            }
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

    if let Some(title) = &args.bench_open {
        let root = args.scan.then_some(args.root.as_path());
        return open::bench(&args.library, root, title);
    }
    if let Some((out, book)) = &args.preview {
        return preview::run(&args, out, book);
    }
    if args.index_dicts {
        let mut d =
            dict::Dictionaries::discover(&dict_dirs(&args), &args.library.join("dict-index"), None);
        let missing = d.missing_indexes();
        dict::build_indexes(&missing);
        perf!(
            "dict: {} indexes built in {}",
            missing.len(),
            args.library.join("dict-index").display()
        );
        return Ok(());
    }

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
    let mut app = build_app(&args, panel, prefs, start_dictionaries(&args))?;
    perf!("startup: first frame submitted");
    open::warm_up();

    if args.scan {
        app.scan(&args.root)?;
    }
    if let Some(title) = &args.open {
        app.open_by_title(title)?;
    }
    let result = app.run(&mut touch);
    app.leave()?;
    result
}

/// The app on `panel`, showing the library.
fn build_app(args: &Args, panel: Panel, prefs: Prefs, dict: Service) -> Result<App, Error> {
    let (w, h) = panel.size();
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
        snapshots: snapshot::Writer::new(&args.library),
        opening: None,
        debounce: Debounce::default(),
        prefs,
        ui,
        styles,
        covers,
        screen: Screen::Library { model, view },
        filter: Filter::All,
        sort: 0,
        shelf_turns: 0,
        dict: Some(dict),
        speed: Speed::load(&args.library),
        exports: ExportPaths::for_root(&args.root),
    };
    app.enter_library_ui();
    app.ui.request_full_refresh();
    app.render_ui()?;
    Ok(app)
}

/// Where to look for dictionaries: `--dict`, else the device defaults.
fn dict_dirs(args: &Args) -> Vec<PathBuf> {
    if args.dicts.is_empty() {
        dict::SEARCH_DIRS.iter().map(PathBuf::from).collect()
    } else {
        args.dicts.clone()
    }
}

/// Find the dictionaries and start the lookup thread (and the index
/// builder if some dictionary has no index yet).
fn start_dictionaries(args: &Args) -> Service {
    let dirs = dict_dirs(args);
    let t = Instant::now();
    let s = Service::start(
        dirs,
        args.library.join("dict-index"),
        args.library.join("dictionaries.json"),
    );
    perf!(
        "startup: {} dictionaries ({}) in {:.0} ms",
        s.dicts.len(),
        s.dicts
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        ms(t.elapsed())
    );
    s
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
    /// The row of reading actions shown with the menu.
    nav: NodeId,
    menu_open: bool,
    gestures: TurnGestures,
    settings: Settings,
    /// What covers the page (selection, popups, lists); see reading.rs.
    overlay: Overlay,
    nodes: OverlayNodes,
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
    /// A book shown from its snapshot while it opens; see `App::opening`.
    Opening,
}

struct App {
    panel: Panel,
    light: Option<Frontlight>,
    battery: Option<Battery>,
    mobi_cache: PathBuf,
    lib: Library,
    saver: Saver,
    snapshots: snapshot::Writer,
    /// The book being opened in the background ([`Screen::Opening`]).
    opening: Option<Box<Opening>>,
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
    /// Dictionary lookups (a thread of their own).
    dict: Option<Service>,
    speed: Speed,
    /// Where highlights are exported.
    exports: ExportPaths,
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
                // A long press fires while the finger rests.
                if let Some(t) = r.gestures.next_deadline(now) {
                    timeout = timeout.min(Duration::from_micros(t.max(1000)));
                }
                if matches!(r.overlay, Overlay::Waiting { .. }) {
                    timeout = timeout.min(Duration::from_millis(20));
                }
            }
            if self.opening.is_some() {
                timeout = timeout.min(Duration::from_millis(10));
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
            self.tick_reading(monotonic_us())?;
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
            Screen::Reader(r) => r.menu_open || !r.overlay.is_none(),
            Screen::Opening => self.opening.as_ref().is_some_and(|o| o.menu_open),
        }
    }

    /// Work done between events: adopting page numbers, saving progress.
    fn background(&mut self) -> Result<(), Error> {
        if let Some(result) = self.opening.as_ref().and_then(|o| o.worker.poll()) {
            self.adopt(result)?;
        }
        self.poll_lookup()?;
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
            self.save_snapshot();
            self.saver.save(&self.lib);
            self.speed.save();
            self.debounce.saved();
        }
        Ok(())
    }

    fn on_touch(&mut self, ev: &aristotle_hal::input::TouchEvent) -> Result<Flow, Error> {
        let ev = ui_event(ev);
        if self.opening.is_some() {
            return self.on_opening_touch(ev);
        }
        if let Screen::Reader(r) = &mut self.screen {
            // The page's gestures own the touch unless the menu or an
            // overlay is up; a hold keeps it until the finger lifts.
            if r.gestures.holding() || (!r.menu_open && r.overlay.is_none()) {
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
            Screen::Reader(r) => {
                let reading_action = matches!(ev, UiEvent::Action { id, .. } if id >= 1000);
                if !r.overlay.is_none() || reading_action {
                    self.on_overlay_event(ev).map(|_| Flow::Continue)
                } else {
                    self.on_menu_event(ev).map(|_| Flow::Continue)
                }
            }
            Screen::Opening => self.on_opening_ui_event(ev),
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

    /// Open the first book whose title contains `title` (case and
    /// diacritics ignored).
    fn open_by_title(&mut self, title: &str) -> Result<(), Error> {
        let want = aristotle_library::text::fold(title);
        let id = self
            .lib
            .books()
            .iter()
            .find(|b| aristotle_library::text::fold(&b.meta.title).contains(&want))
            .map(|b| b.id.clone());
        match id {
            Some(id) => self.open_book(&id),
            None => {
                eprintln!("aristotle: no book titled {title:?}");
                Ok(())
            }
        }
    }

    fn open_book(&mut self, id: &str) -> Result<(), Error> {
        let Some(rec) = self.lib.book(id).cloned() else {
            return Ok(());
        };
        let t0 = Instant::now();
        let style = self.prefs.style();
        let src = match Source::new(&self.lib, &rec, &self.mobi_cache) {
            Ok(mut s) => {
                s.secs_per_page = self.speed.secs_per_page();
                s
            }
            Err(e) => {
                eprintln!("aristotle: {}: {e:#}", rec.meta.title);
                return Ok(());
            }
        };
        if let Some(snap) = open::find_snapshot(&src, &style) {
            return self.open_from_snapshot(src, style, snap, t0);
        }
        // Nothing to show before the layout: open it here.
        let prepared = match Session::prepare(src, style, false) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("aristotle: {}: {e:#}", rec.meta.title);
                return Ok(());
            }
        };
        let times = prepared.times;
        let mut session = Session::from_prepared(prepared)?;
        session.set_speed(self.speed.secs_per_page());
        let page = session.page(session.current)?;
        let light = self.light_level();
        let settings = self.prefs.menu_settings(light);
        let (tree, menu, page_node, nav) =
            reader_tree(&self.styles, &session.title, &settings, page, &mut |_| {
                Vec::new()
            });
        // A screen switch: the stock UI's flashing full refresh (mode 8).
        self.ui.set_tree_with(tree, Refresh::Full);
        self.ui.gesture_config_mut().long_press_us = 500_000;
        self.render_ui()?;
        perf!(
            "open: \"{}\": book {:.0} ms, position {:.0} ms, first page {:.0} ms, submitted after {:.0} ms; page numbers {}; no snapshot",
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
        let gestures = TurnGestures::new(self.ui.bounds());
        self.enter_reader(session, menu, page_node, nav, settings, gestures);
        Ok(())
    }

    /// Show the page the book was left at from its snapshot, and open the
    /// book itself on a worker thread (see open.rs).
    fn open_from_snapshot(
        &mut self,
        src: Source,
        style: Style,
        mut snap: snapshot::Loaded,
        t0: Instant,
    ) -> Result<(), Error> {
        let light = self.light_level();
        let settings = self.prefs.menu_settings(light);
        let page = snap.page.clone();
        let (tree, menu, page_node, nav) = reader_tree(
            &self.styles,
            &src.title,
            &settings,
            page.clone(),
            &mut |_| Vec::new(),
        );
        // A screen switch: the stock UI's flashing full refresh (mode 8).
        self.ui.set_tree_with(tree, Refresh::Full);
        self.ui.gesture_config_mut().long_press_us = 500_000;
        self.render_ui()?;
        perf!(
            "open: \"{}\": snapshot submitted after {:.0} ms (read {:.0} ms, {} KB; decode {:.0} ms); opening the book in the background",
            src.title,
            ms(t0.elapsed()),
            ms(snap.read),
            snap.bytes / 1024,
            ms(snap.decode)
        );
        let (title, id, stamp) = (src.title.clone(), src.id.clone(), src.stamp);
        let worker = open::spawn(src, style.clone());
        let t = Instant::now();
        let next = snap.take_next().map(|(i, p)| (i, Rc::new(p)));
        if next.is_some() {
            perf!(
                "open: snapshot's next page decoded in {:.0} ms",
                ms(t.elapsed())
            );
        }
        self.lib.mark_opened(&id, aristotle_library::library::now());
        self.debounce.changed(Instant::now());
        self.screen = Screen::Opening;
        self.opening = Some(Box::new(Opening {
            title,
            id,
            stamp,
            style,
            worker,
            tapped: t0,
            current: (snap.info, page),
            next,
            turned: false,
            menu_open: false,
            menu,
            page_node,
            nav,
            settings,
            gestures: TurnGestures::new(self.ui.bounds()),
        }));
        Ok(())
    }

    /// Input while the book opens. Turns between the snapshot's two pages
    /// and the menu work at once; anything else waits for the book, then
    /// goes the normal way.
    fn on_opening_touch(&mut self, ev: TouchEvent) -> Result<Flow, Error> {
        let Some(o) = self.opening.as_mut() else {
            return Ok(Flow::Continue);
        };
        if o.menu_open {
            for e in self.ui.handle_touch(ev) {
                if self.on_ui_event(e)? == Flow::Quit {
                    return Ok(Flow::Quit);
                }
            }
            return Ok(Flow::Continue);
        }
        for intent in o.gestures.feed(ev) {
            if self.opening_intent(intent)? {
                continue;
            }
            self.finish_opening()?;
            self.on_intent(intent)?;
        }
        Ok(Flow::Continue)
    }

    /// Handle `intent` from the snapshot alone; false if it needs the book.
    fn opening_intent(&mut self, intent: Intent) -> Result<bool, Error> {
        let Some(o) = self.opening.as_mut() else {
            return Ok(false);
        };
        match intent {
            Intent::Turn { dir, at_us } => {
                let target = match (dir, o.turned) {
                    (Turn::Forward, false) => o.next.as_ref(),
                    (Turn::Back, true) => Some(&o.current),
                    _ => None,
                };
                let Some((_, page)) = target.cloned() else {
                    return Ok(false);
                };
                let waited = self.panel.show_page(&page, false, "turn")?;
                let tap_to_submit = monotonic_us().saturating_sub(at_us) as f64 / 1000.0;
                o.turned = dir == Turn::Forward;
                if let Some(w) = self.ui.tree.widget_mut::<PageView>(o.page_node) {
                    w.page = page;
                }
                self.ui.tree.invalidate(o.page_node, Refresh::Auto);
                // The panel already shows the page; this only updates the UI's copy.
                self.ui.render();
                perf!(
                    "turn {:?}: tap->submit {:.0} ms (a snapshot page, the book still opening; waited for the panel {:.0} ms)",
                    dir,
                    tap_to_submit,
                    ms(waited)
                );
                Ok(true)
            }
            Intent::OpenMenu => {
                let light = self.light_level();
                let Some(o) = self.opening.as_mut() else {
                    return Ok(false);
                };
                o.menu_open = true;
                let info = o.shown().0.clone();
                self.ui.tree.set_text(o.menu.chapter_label, &info.chapter);
                self.ui.tree.set_text(o.menu.page_label, &info.label);
                self.ui.tree.set_progress(o.menu.progress, info.fraction);
                o.settings.light = light;
                if let Some(w) = self.ui.tree.widget_mut::<Slider>(o.menu.light) {
                    w.value = light;
                }
                self.ui.tree.invalidate(o.menu.light, Refresh::Auto);
                self.ui.tree.set_hidden(o.menu.root, false);
                self.ui.reset_gestures();
                self.render_ui()?;
                Ok(true)
            }
            // Holding, dragging and link taps need the book: they wait.
            _ => Ok(false),
        }
    }

    /// The menu while the book opens: closing it and going back to the
    /// library need only the snapshot; everything else waits for the book.
    fn on_opening_ui_event(&mut self, ev: UiEvent) -> Result<Flow, Error> {
        let Some(o) = self.opening.as_mut() else {
            return Ok(Flow::Continue);
        };
        if !o.menu_open {
            return Ok(Flow::Continue);
        }
        match ev {
            UiEvent::Tap(_) => {
                o.menu_open = false;
                o.gestures.reset();
                self.ui.tree.set_hidden(o.menu.root, true);
                self.ui.render();
                let page = o.shown().1.clone();
                self.panel.show_page(&page, false, "menu closed")?;
            }
            UiEvent::Action {
                id: menu_action::BACK,
                ..
            } => self.abandon_opening()?,
            UiEvent::Action { .. } => {
                self.finish_opening()?;
                self.on_menu_event(ev)?;
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    /// Back to the library before the book finished opening: drop the
    /// worker and keep the page shown as the progress (and its snapshot).
    fn abandon_opening(&mut self) -> Result<(), Error> {
        let Some(o) = self.opening.take() else {
            return Ok(());
        };
        let (info, page) = o.shown().clone();
        if o.turned {
            let progress = Progress {
                fraction: info.fraction,
                locator: info.locator.clone(),
                updated: aristotle_library::library::now(),
                source: Some("aristotle".into()),
                page: None,
            };
            self.lib.set_progress(&o.id, progress);
            let key = snapshot::Key::new(&o.id, o.stamp, &o.style, info.locator.clone());
            self.snapshots.save(key, (info, page), None);
        }
        perf!(
            "open: \"{}\": back to the library {:.0} ms after the tap, before the book was ready",
            o.title,
            ms(o.tapped.elapsed())
        );
        // Dropping the worker lowers its thread to idle priority.
        drop(o);
        self.saver.save(&self.lib);
        self.debounce.saved();
        self.show_library(Refresh::Full)
    }

    /// Wait for the book being opened and adopt it.
    fn finish_opening(&mut self) -> Result<(), Error> {
        let Some(o) = self.opening.as_ref() else {
            return Ok(());
        };
        let t = Instant::now();
        let result = o.worker.wait();
        perf!("open: waited {:.0} ms for the book", ms(t.elapsed()));
        self.adopt(result)
    }

    /// Take over the session the worker made, and make sure the panel shows
    /// its page: no update if the snapshot showed exactly that page, else
    /// one full mode-9 update.
    fn adopt(&mut self, result: Result<Prepared, Error>) -> Result<(), Error> {
        let Some(o) = self.opening.take() else {
            return Ok(());
        };
        let prepared = match result {
            Ok(p) => p,
            Err(e) => {
                eprintln!("aristotle: {}: {e:#}", o.title);
                return self.show_library(Refresh::Full);
            }
        };
        let t = Instant::now();
        let times = prepared.times;
        let (at, right) = open::reconcile(
            o.current.0.location,
            o.next.as_ref().map(|(i, _)| i.location),
            o.turned,
            prepared.current,
            prepared.next(),
        );
        let mut session = Session::from_prepared(prepared)?;
        session.set_speed(self.speed.secs_per_page());
        session.current = at;
        let page = session.page(at)?;
        let verdict = if right {
            open::compare(&o.shown().1, &page, &session.style)
        } else {
            open::Verdict::Page
        };
        if let Some(w) = self.ui.tree.widget_mut::<PageView>(o.page_node) {
            w.page = page.clone();
        }
        match verdict {
            // The UI's copy of the screen already has these pixels: no
            // update at all.
            open::Verdict::Same => {}
            // Never a partial fix-up: one full mode-9 update, like a page
            // turn (behind an open menu, like a chapter jump).
            _ if o.menu_open => {
                self.ui.tree.invalidate(o.page_node, Refresh::Auto);
                self.render_ui_with(Content::Page)?;
            }
            _ => {
                self.panel.show_page(&page, false, "corrected page")?;
                self.ui.tree.invalidate(o.page_node, Refresh::Auto);
                // The panel already shows the page; this only updates the UI's copy.
                self.ui.render();
            }
        }
        perf!(
            "open: \"{}\": book ready {:.0} ms after the tap (worker: book {:.0} ms, position {:.0} ms, first page {:.0} ms, neighbours {:.0} ms; adopted in {:.0} ms); snapshot {:?}; page numbers {}",
            o.title,
            ms(o.tapped.elapsed()),
            ms(times.open),
            ms(times.place),
            ms(times.render),
            ms(times.neighbours),
            ms(t.elapsed()),
            verdict,
            if times.pages_cached { "from cache" } else { "being computed in the background" }
        );
        if o.turned || !right {
            let now = aristotle_library::library::now();
            self.lib.set_progress(&session.id, session.progress(now));
            self.debounce.changed(Instant::now());
        }
        let o = *o;
        let menu_open = o.menu_open;
        self.enter_reader(session, o.menu, o.page_node, o.nav, o.settings, o.gestures);
        if let Screen::Reader(r) = &mut self.screen {
            if menu_open {
                r.menu_open = true;
                update_menu_labels(&mut self.ui, r);
                self.render_ui()?;
            }
        }
        if let Screen::Reader(r) = &mut self.screen {
            r.session.prerender();
        }
        Ok(())
    }

    fn enter_reader(
        &mut self,
        session: Session,
        menu: Menu,
        page_node: NodeId,
        nav: NodeId,
        settings: Settings,
        gestures: TurnGestures,
    ) {
        self.screen = Screen::Reader(Box::new(Reading {
            session,
            menu,
            page_node,
            nav,
            menu_open: false,
            gestures,
            settings,
            overlay: Overlay::None,
            nodes: OverlayNodes::default(),
        }));
        self.speed.page_shown(Instant::now());
        self.page_changed();
    }

    /// Store the current page (and the next, if rendered) for the next
    /// open, with the progress it belongs to; see snapshot.rs. Call before
    /// saving the index.
    fn save_snapshot(&mut self) {
        let Screen::Reader(r) = &mut self.screen else {
            return;
        };
        let s = &mut r.session;
        let progress = s.progress(aristotle_library::library::now());
        // The snapshot is only shown if the index stores this locator.
        let stored = self.lib.book(&s.id).and_then(|b| b.progress.as_ref());
        if stored.map(|p| &p.locator) != Some(&progress.locator) {
            self.lib.set_progress(&s.id, progress.clone());
        }
        let key = snapshot::Key::new(&s.id, s.stamp(), &s.style, progress.locator);
        let here = s.current;
        let Ok(current) = s.page(here) else {
            return;
        };
        let next = s
            .known_next(here)
            .and_then(|l| s.rendered(l).map(|p| (open::page_info(s, l), p)));
        let info = open::page_info(s, here);
        self.snapshots.save(key, (info, current), next);
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
            Intent::Hold { at, undo } => self.hold(at, undo),
            Intent::Drag(p) => self.drag(p),
            Intent::Release(p) => self.release(p),
            Intent::Tap(p) => self.quiet_tap(p),
        }
    }

    /// Fire a long press on the page while the finger rests.
    fn tick_reading(&mut self, now_us: u64) -> Result<(), Error> {
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        for i in r.gestures.tick(now_us) {
            self.on_intent(i)?;
        }
        Ok(())
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
        if dir == Turn::Forward {
            self.speed.turned_forward(Instant::now());
        } else {
            self.speed.page_shown(Instant::now());
        }
        s.set_speed(self.speed.secs_per_page());
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
        self.page_changed();
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
        self.ui.tree.set_hidden(r.nav, false);
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
        self.ui.tree.set_hidden(r.nav, true);
        self.ui.render();
        let page = r.session.page(r.session.current)?;
        self.panel.show_page(&page, false, "menu closed")?;
        self.page_changed();
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
        r.session.page(r.session.current)?;
        let t_layout = t.elapsed();
        r.settings = settings;
        self.rebuild_reader(Refresh::Auto)?;
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
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
        self.save_snapshot();
        self.saver.save(&self.lib);
        self.debounce.saved();
        self.show_library(Refresh::Full)
    }

    /// Save progress and leave a white screen for the stock UI.
    fn leave(self) -> Result<(), Error> {
        let mut app = self;
        // A forward turn made while the book was opening counts.
        app.finish_opening()?;
        if let Screen::Reader(r) = &app.screen {
            let now = aristotle_library::library::now();
            app.lib.set_progress(&r.session.id, r.session.progress(now));
        }
        app.save_snapshot();
        app.saver.save(&app.lib);
        app.speed.save();
        app.saver.flush();
        app.snapshots.flush();
        app.panel.clear()?;
        perf!("quit: progress saved, screen cleared");
        Ok(())
    }
}

/// The reader screen: the page, the layers `extra` adds over it (the
/// selection, a popup), and the (hidden) menu with its row of reading
/// actions. Returns the tree, the menu, the page node and the row.
fn reader_tree(
    styles: &Styles,
    title: &str,
    settings: &Settings,
    page: Rc<Page>,
    extra: &mut dyn FnMut(&mut Tree) -> Vec<NodeId>,
) -> (Tree, Menu, NodeId, NodeId) {
    let mut t = Tree::new();
    let page_node = t.custom(PageView { page });
    let mut layers = vec![page_node];
    layers.extend(extra(&mut t));
    let spec = MenuSpec {
        title,
        // Only Literata ships with the renderer, and night mode would need
        // inverted pages; both stay out of the menu until they work.
        families: None,
        night: false,
    };
    let menu = build_menu(&mut t, styles, &spec, settings);
    let nav = overlay::nav_bar(&mut t, styles);
    layers.push(menu.root);
    layers.push(nav);
    let root = t.add(Node::new(Kind::Stack), layers);
    t.set_root(root);
    (t, menu, page_node, nav)
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
