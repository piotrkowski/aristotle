// SPDX-License-Identifier: AGPL-3.0-or-later
//! `aristotle`: the reader app for the Kindle Paperwhite 4.
//!
//! The library from `aristotle-library` (scanned incrementally on start,
//! folders, series, the book to continue), pages from `aristotle-render`
//! (Plato's engine, Polish hyphenation), the menu, settings and screensaver
//! from `aristotle-ui`, on the panel, touchscreen, frontlight and battery
//! through `aristotle-hal`.
//!
//! ```text
//! aristotle [--data DIR] [--library DIR] [--root DIR] [--turn-mode N]
//!           [--night-method software|hardware] [--no-scan] [--bench-open TITLE]
//!           [--open TITLE] [--dict DIR]... [--index-dicts] [--test-clippings PATH]
//!           [--preview OUT BOOK]
//!
//!   --data DIR      renderer data (fonts/, css/, hyphenation-patterns/,
//!                   serif/); default: data/ next to the binary
//!   --library DIR   library index, covers, page cache and preferences;
//!                   default: $ARISTOTLE_DATA or /mnt/us/.aristotle
//!   --root DIR      where the books are; default: /mnt/us/documents
//!   --turn-mode N   page-turn waveform: 9 (stock, default) or 7 (DU4);
//!                   saved in reader.json
//!   --night-method  how night mode inverts: software (default) or
//!                   hardware (EPDC inversion flag); saved in reader.json
//!   --no-scan       don't look for new or changed books on start
//!   --bench-open T  time opening the first book whose title contains T,
//!                   without panel or touchscreen, and exit (see open.rs)
//!   --open TITLE    open the first book whose title contains TITLE after
//!                   start (device scenarios)
//!   --dict DIR      look for StarDict dictionaries here (repeatable);
//!                   default: /mnt/us/dictionaries, /mnt/us/koreader/data/dict
//!   --index-dicts   build the dictionaries' lookup indexes into
//!                   LIBRARY/dict-index and exit (on the host, to copy to
//!                   the device instead of indexing there)
//!   --test-clippings PATH  export highlights to PATH instead of
//!                   ROOT/My Clippings.txt, and Markdown next to it (test
//!                   runs must not reach the user's own clippings); PATH
//!                   is written even when the setting "Save highlights to
//!                   My Clippings.txt" is off
//!   --preview OUT BOOK  drive the reading interactions on BOOK with
//!                   synthetic touches on an in-memory panel and write PNGs
//!                   to OUT (host; see preview.rs)
//! ```
//!
//! Run it with the stock GUI stopped: `device/app/run-aristotle.sh`.
//! Hold the "Biblioteka" title for a second to quit. Timings go to stderr
//! as `[perf ...]` lines; see docs/reader.md.

mod device;
mod dict;
mod marks;
mod open;
mod overlay;
mod pagecache;
mod panel;
mod prefs;
mod preview;
mod prewarm;
mod progress;
mod reading;
mod selection;
mod session;
mod shelf;
mod snapshot;
mod speed;
mod turns;

use std::cell::Cell;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use aristotle_hal::input::{monotonic_us, Phase as HalPhase, TouchScreen, Transform};
use aristotle_hal::power::{self, Battery, Frontlight, LIGHT_STEPS};
use aristotle_hal::system;
use aristotle_library::{Library, ScanOptions, Status};
use aristotle_render::page::Page;
use aristotle_render::plato_core::anyhow::{bail, format_err, Context, Error};
use aristotle_render::style::Style;
use aristotle_ui::geom::{Rect, Size, MIN_TARGET};
use aristotle_ui::input::{Phase, TouchEvent};
use aristotle_ui::screens::library::{self as ui_lib, CoverMode, Filter, View};
use aristotle_ui::screens::reader::{
    action as menu_action, build_menu, mask_extents, Menu, MenuSpec, Settings,
};
use aristotle_ui::screens::settings::{self as set_ui, SystemView, LIGHT_PRESETS, SLEEP_MINUTES};
use aristotle_ui::screens::sleep;
use aristotle_ui::text::{FamilyId, FontData, Fonts, TextEngine};
use aristotle_ui::theme::Styles;
use aristotle_ui::tree::{Constraints, Kind, Node, NodeId, PaintCx, Refresh, Tree, Widget};
use aristotle_ui::ui::{Ui, UiEvent};
use aristotle_ui::widgets::{Slider, Steps, Toggle};

use crate::device::Facts;
use crate::dict::service::Service;
use crate::marks::ExportPaths;
use crate::open::{Opening, Pending};
use crate::panel::{Content, Panel};
use crate::prefs::{Covers as CoverPref, NightMethod, Prefs};
use crate::prewarm::Prewarm;
use crate::progress::{Debounce, Saver};
use crate::reading::{Overlay, OverlayNodes};
use crate::session::{Prepared, Session, Source};
use crate::shelf::{Covers, Nav, ShelfModel, Target, SORTS};
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
    night_method: Option<NightMethod>,
    scan: bool,
    bench_open: Option<String>,
    dicts: Vec<PathBuf>,
    preview: Option<(PathBuf, PathBuf)>,
    open: Option<String>,
    index_dicts: bool,
    test_clippings: Option<PathBuf>,
    /// `--lang CODE`: the UI language, stored in the preferences.
    lang: Option<String>,
}

fn usage() -> &'static str {
    "usage: aristotle [--data DIR] [--library DIR] [--root DIR] [--turn-mode 9|7] \
     [--night-method software|hardware] [--no-scan] [--bench-open TITLE] [--open TITLE] \
     [--dict DIR]... [--index-dicts] [--test-clippings PATH] [--lang en|pl] \
     [--preview OUT BOOK]"
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
        night_method: None,
        scan: true,
        bench_open: None,
        dicts: Vec::new(),
        preview: None,
        open: None,
        index_dicts: false,
        test_clippings: None,
        lang: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format_err!("{name} needs a value"));
        match arg.as_str() {
            "--data" => a.data = cwd.join(value("--data")?),
            "--library" => a.library = cwd.join(value("--library")?),
            "--root" => a.root = cwd.join(value("--root")?),
            "--turn-mode" => a.turn_mode = Some(value("--turn-mode")?.parse()?),
            "--night-method" => {
                a.night_method = Some(match value("--night-method")?.as_str() {
                    "software" => NightMethod::Software,
                    "hardware" => NightMethod::Hardware,
                    other => bail!("unknown night method {other}\n{}", usage()),
                })
            }
            "--no-scan" => a.scan = false,
            "--bench-open" => a.bench_open = Some(value("--bench-open")?),
            "--open" => a.open = Some(value("--open")?),
            "--dict" => a.dicts.push(cwd.join(value("--dict")?)),
            "--index-dicts" => a.index_dicts = true,
            "--test-clippings" => a.test_clippings = Some(cwd.join(value("--test-clippings")?)),
            "--preview" => {
                let out = cwd.join(value("--preview")?);
                let book = cwd.join(value("--preview")?);
                a.preview = Some((out, book));
            }
            "--lang" => {
                let code = value("--lang")?;
                if !aristotle_i18n::locales().iter().any(|l| l.code == code) {
                    bail!("unknown language {code}\n{}", usage());
                }
                a.lang = Some(code);
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
        let cache = args.library.join("dict-index");
        let mut d = dict::Dictionaries::discover(&dict_dirs(&args), &cache, None);
        let missing = d.missing_indexes();
        dict::build_indexes(&missing);
        perf!(
            "dict: {} indexes built in {}",
            missing.len(),
            cache.display()
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
    if let Some(m) = args.night_method {
        prefs.night_method = m;
        let _ = prefs.save(&args.library);
    }
    if let Some(code) = &args.lang {
        prefs.language = code.clone();
        let _ = prefs.save(&args.library);
    }
    let panel = Panel::open(prefs.turn_mode).context("can't open the framebuffer")?;
    let (w, h) = panel.size();
    let mut touch =
        TouchScreen::open(w, h, Transform::default()).context("can't open the touchscreen")?;
    touch.grab(true)?;
    // Our screensaver replaces the stock one, which powerd would otherwise
    // draw over our screen. The previous value is put back on exit.
    let screensaver_was = power::screensaver_prevented().ok();
    if let Err(e) = power::prevent_screensaver(true) {
        eprintln!("aristotle: can't hold off the stock screensaver: {e}");
    }
    let mut app = build_app(
        &args,
        panel,
        prefs,
        start_dictionaries(&args),
        screensaver_was,
    )?;
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
    let t = Instant::now();
    let s = Service::start(
        dict_dirs(args),
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

/// Where highlights go: the library root's `My Clippings.txt` and
/// `aristotle-notes/` beside the root, or `--test-clippings` and
/// `aristotle-notes/` beside that file.
fn export_paths(args: &Args) -> ExportPaths {
    match &args.test_clippings {
        Some(path) => ExportPaths {
            clippings: path.clone(),
            notes: path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("aristotle-notes"),
            test: true,
        },
        None => ExportPaths::for_root(&args.root),
    }
}

/// The app on `panel`, showing the library.
fn build_app(
    args: &Args,
    panel: Panel,
    prefs: Prefs,
    dict: Service,
    screensaver_was: Option<bool>,
) -> Result<App, Error> {
    let (w, h) = panel.size();
    aristotle_i18n::set_language(&prefs.language);
    let lib = Library::open(&args.library)?;
    let saver = Saver::new(&args.library);
    perf!(
        "startup: devices and library index ready ({} books, turn mode {}, night {} ({:?}))",
        lib.books().len(),
        prefs.turn_mode,
        prefs.night,
        prefs.night_method
    );

    let nav = Nav {
        view: prefs.view(),
        sort: prefs.sort,
        ..Nav::default()
    };
    // The data directory is the current directory by now.
    let ui = Ui::new(
        Tree::new(),
        TextEngine::new(ui_fonts(Path::new("serif"))?),
        w,
        h,
    );
    perf!("startup: UI fonts ready");
    let mut app = App {
        panel,
        light: Frontlight::open().ok(),
        battery: Battery::open().ok(),
        mobi_cache: args.library.join("epub-cache"),
        root: args.root.clone(),
        lib,
        saver,
        snapshots: snapshot::Writer::new(&args.library),
        opening: None,
        pending: None,
        prewarm: Prewarm::default(),
        debounce: Debounce::default(),
        prefs,
        ui,
        styles: Styles::new(FamilyId(0)),
        covers: Covers::default(),
        screen: Screen::Settings { ids: None, page: 0 },
        nav,
        asleep: None,
        last_touch: Instant::now(),
        clock: String::new(),
        screensaver_was,
        swallow: false,
        dict: Some(dict),
        speed: Speed::load(&args.library),
        exports: export_paths(args),
    };
    app.apply_night();
    app.show_library(Refresh::Full)?;
    Ok(app)
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
        // As panel::night_page does for pages sent directly.
        if NIGHT.with(Cell::get) {
            for b in &p.images {
                cx.painter
                    .invert_rect(Rect::new(r.x + b.x0, r.y + b.y0, b.x1 - b.x0, b.y1 - b.y0));
            }
        }
    }
}

thread_local! {
    /// Night mode, either method: [`PageView`] then inverts the book's
    /// images, so the inversion of the whole screen shows them as by day.
    static NIGHT: Cell<bool> = const { Cell::new(false) };
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
    /// `ids` is `None` only before the first screen is built.
    Settings {
        ids: Option<set_ui::Settings>,
        page: usize,
    },
}

struct App {
    panel: Panel,
    light: Option<Frontlight>,
    battery: Option<Battery>,
    mobi_cache: PathBuf,
    root: PathBuf,
    lib: Library,
    saver: Saver,
    snapshots: snapshot::Writer,
    /// The book being opened in the background ([`Screen::Opening`]).
    opening: Option<Box<Opening>>,
    /// A book without a snapshot opening on a worker, the library still
    /// on screen.
    pending: Option<Box<Pending>>,
    /// Snapshots made ahead of time while the library is idle.
    prewarm: Prewarm,
    debounce: Debounce,
    prefs: Prefs,
    ui: Ui,
    styles: Styles,
    covers: Covers,
    screen: Screen,
    nav: Nav,
    /// The screensaver is up; holds the light step to restore.
    asleep: Option<u32>,
    last_touch: Instant,
    /// The time the status line shows, to refresh it each minute.
    clock: String,
    /// `preventScreenSaver` before we set it.
    screensaver_was: Option<bool>,
    /// The touch that woke the screensaver: ignored until it lifts.
    swallow: bool,
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

/// The UI font: the renderer's Literata in `dir` (`serif/` of the data
/// directory). Compiling it in would ship the same megabyte twice. The
/// files are mapped rather than read, so their pages stay file-backed like
/// the binary's own.
fn ui_fonts(dir: &Path) -> Result<Fonts, Error> {
    let mut fonts = Fonts::new();
    let mut faces = [None; 4];
    for (face, style) in faces
        .iter_mut()
        .zip(["Regular", "Italic", "Bold", "BoldItalic"])
    {
        let path = dir.join(format!("Literata-{style}.ttf"));
        let bytes = map_file(&path).with_context(|| format!("can't map {}", path.display()))?;
        *face = Some(
            fonts
                .add_face(FontData::Static(bytes))
                .with_context(|| path.display().to_string())?,
        );
    }
    fonts
        .add_family("Literata", faces)
        .ok_or_else(|| format_err!("no regular Literata"))?;
    Ok(fonts)
}

/// Maps a whole file read-only for the rest of the process.
fn map_file(path: &Path) -> std::io::Result<&'static [u8]> {
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len() as usize;
    if len == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "empty file",
        ));
    }
    // SAFETY: a private read-only mapping of the whole file, never unmapped,
    // so the slice is valid for 'static. The data directory is not changed
    // while the app runs (bundles are unpacked with the app stopped).
    unsafe {
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            file.as_raw_fd(),
            0,
        );
        if ptr == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        Ok(std::slice::from_raw_parts(ptr as *const u8, len))
    }
}

/// The quit target: the library title, hit-tested at least 9 mm high.
fn quit_zone(ui: &Ui, title: NodeId) -> Rect {
    ui.tree.rect(title).expand_to(MIN_TARGET)
}

/// The index of `id` in `first..first + n`, if it is there.
fn in_range(id: u32, first: u32, n: usize) -> Option<usize> {
    (first..first + n as u32)
        .contains(&id)
        .then(|| (id - first) as usize)
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

    fn save_prefs(&self) {
        if let Err(e) = self.prefs.save(self.lib.data_dir()) {
            eprintln!("aristotle: can't save preferences: {e}");
        }
    }

    // --- Night mode and light -------------------------------------------

    /// Bring the panel and the UI in line with `prefs.night`; the next
    /// render is a full flash that shows the switch everywhere at once.
    fn apply_night(&mut self) {
        let method = self.prefs.night.then_some(self.prefs.night_method);
        NIGHT.with(|n| n.set(method.is_some()));
        self.panel.set_night(method);
        self.ui.set_night(method == Some(NightMethod::Software));
        self.ui.request_full_refresh();
    }

    fn set_night(&mut self, on: bool) {
        self.prefs.night = on;
        self.save_prefs();
        self.apply_night();
        perf!("night mode {}", if on { "on" } else { "off" });
    }

    fn light_step(&self) -> u32 {
        self.light.as_ref().and_then(|l| l.step().ok()).unwrap_or(0)
    }

    fn set_light_step(&self, step: u32) {
        if let Some(l) = &self.light {
            if let Err(e) = l.set_step(step.min(LIGHT_STEPS)) {
                eprintln!("aristotle: frontlight: {e}");
            }
        }
    }

    /// The light as the reader menu's slider shows it, 0 to 1.
    fn light_level(&self) -> f32 {
        self.light_step() as f32 / LIGHT_STEPS as f32
    }

    // --- Library ----------------------------------------------------------

    fn show_library(&mut self, refresh: Refresh) -> Result<(), Error> {
        self.show_library_at(refresh, 0)
    }

    /// Rebuild the library screen for `self.nav`, on list page `page`.
    fn show_library_at(&mut self, refresh: Refresh, page: usize) -> Result<(), Error> {
        let mode = self.prefs.covers.mode();
        let model = ShelfModel::build(&self.lib, &self.nav, mode, &mut self.covers);
        let facts = Facts::status(&self.battery);
        self.clock = facts.clock.clone();
        let (tree, view) = ui_lib::Library::build_shelf(
            &mut self.ui.text,
            &self.styles,
            &model.shelf(facts.status_line(), mode),
        );
        self.ui.set_tree_with(tree, refresh);
        self.ui.layout();
        if page > 0 {
            self.ui.tree.set_page(view.list, page);
            self.ui.layout();
        }
        view.sync(&mut self.ui.tree);
        // Quitting is drastic: ask for a full second's hold.
        self.ui.gesture_config_mut().long_press_us = 1_000_000;
        let old = std::mem::replace(&mut self.screen, Screen::Library { model, view });
        let submitted = self.render_ui();
        // A closed book is freed only now: the display lists of a laid-out
        // chapter are many allocations, and the library should not wait.
        drop(old);
        submitted
    }

    fn list_page(&self) -> usize {
        match &self.screen {
            Screen::Library { view, .. } => self.ui.tree.page(view.list).0,
            Screen::Settings { ids: Some(ids), .. } => self.ui.tree.page(ids.list).0,
            _ => 0,
        }
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
        let lib = &self.lib;
        let pruned = snapshot::prune(lib.data_dir(), |id| lib.book(id).is_some());
        if pruned > 0 {
            perf!("scan: {pruned} snapshots of books no longer in the library deleted");
        }
        if r.added + r.updated + r.moved + r.removed + r.restored + r.sidecars_imported > 0 {
            self.saver.save(&self.lib);
            if matches!(self.screen, Screen::Library { .. }) {
                let page = self.list_page();
                self.show_library_at(Refresh::Auto, page)?;
            }
        }
        Ok(())
    }

    // --- Settings ---------------------------------------------------------

    fn show_settings(&mut self, refresh: Refresh, page: usize) -> Result<(), Error> {
        let facts = Facts::read(&self.battery, &self.root);
        self.clock = facts.clock.clone();
        let light = self
            .light
            .as_ref()
            .map(|l| (l.step().unwrap_or(0), LIGHT_STEPS));
        let view = SystemView {
            status: facts.status_line(),
            light,
            night: self.prefs.night,
            covers: self.prefs.covers.mode(),
            sleep_minutes: self.prefs.sleep(),
            clippings: self.prefs.clippings,
            battery: facts.battery_info(),
            storage: facts.storage,
            date: &facts.date,
            time: &facts.clock,
            usb: facts.usb,
            about: &facts.about,
        };
        let (tree, ids) = set_ui::Settings::build(&mut self.ui.text, &self.styles, &view);
        self.ui.set_tree_with(tree, refresh);
        self.ui.layout();
        if page > 0 {
            self.ui.tree.set_page(ids.list, page);
            self.ui.layout();
        }
        ids.sync(&mut self.ui.tree);
        self.ui.gesture_config_mut().long_press_us = 500_000;
        self.screen = Screen::Settings {
            ids: Some(ids),
            page,
        };
        self.render_ui()
    }

    fn on_settings_event(&mut self, ev: UiEvent) -> Result<Flow, Error> {
        use set_ui::action as a;
        let UiEvent::Action { id, node, at } = ev else {
            return Ok(Flow::Continue);
        };
        let Screen::Settings { ids: Some(ids), .. } = &self.screen else {
            return Ok(Flow::Continue);
        };
        let list = ids.list;
        let page = self.ui.tree.page(list).0;
        let turned = ids.handle(&mut self.ui.tree, id);
        if turned {
            self.ui.layout();
            ids.sync(&mut self.ui.tree);
            let now = self.ui.tree.page(list).0;
            if let Screen::Settings { page: p, .. } = &mut self.screen {
                *p = now;
            }
            return Ok(Flow::Continue);
        }
        match id {
            a::BACK => return self.show_library(Refresh::Full).map(|_| Flow::Continue),
            a::LIGHT_DOWN | a::LIGHT_UP | a::LIGHT_SET => {
                let cur = self.light_step();
                let step = match id {
                    a::LIGHT_SET => Slider::step_at(self.ui.tree.rect(node), at.x, LIGHT_STEPS),
                    a::LIGHT_DOWN => cur.saturating_sub(1),
                    _ => (cur + 1).min(LIGHT_STEPS),
                };
                self.set_light_step(step);
            }
            a::NIGHT => self.set_night(!self.prefs.night),
            a::CLIPPINGS => {
                self.prefs.clippings = !self.prefs.clippings;
                self.save_prefs();
                perf!(
                    "clippings: {}",
                    if self.prefs.clippings { "on" } else { "off" }
                );
            }
            id if in_range(id, a::LANGUAGE, aristotle_i18n::locales().len()).is_some() => {
                let i = (id - a::LANGUAGE) as usize;
                let code = aristotle_i18n::locales()[i].code;
                if code != self.prefs.language {
                    self.prefs.language = code.to_string();
                    self.save_prefs();
                    aristotle_i18n::set_language(code);
                    perf!("language: {code}");
                    // Every word changes: one flash, not a diffed update.
                    return self
                        .show_settings(Refresh::Full, page)
                        .map(|_| Flow::Continue);
                }
            }
            id if in_range(id, a::USB, 2).is_some() => {
                let storage = id == a::USB + 1;
                let res = system::set_usb_mode(storage.then_some(10));
                perf!(
                    "usb: {} requested: {res:?}",
                    if storage { "drive" } else { "network" }
                );
                if let Err(e) = res {
                    eprintln!("aristotle: usb mode: {e}");
                }
            }
            id => {
                if let Some(i) = in_range(id, a::LIGHT_PRESET, LIGHT_PRESETS.len()) {
                    self.set_light_step(LIGHT_PRESETS[i].1);
                } else if let Some(i) = in_range(id, a::COVERS, CoverMode::ALL.len()) {
                    self.prefs.covers = CoverPref::from_mode(CoverMode::ALL[i]);
                    self.save_prefs();
                } else if let Some(i) = in_range(id, a::SLEEP, SLEEP_MINUTES.len()) {
                    self.prefs.set_sleep(SLEEP_MINUTES[i]);
                    self.save_prefs();
                } else {
                    return Ok(Flow::Continue);
                }
            }
        }
        // Rebuild with only the changed pixels sent; a night switch makes
        // that render a full flash.
        self.show_settings(Refresh::Auto, page)?;
        Ok(Flow::Continue)
    }

    // --- Screensaver ------------------------------------------------------

    fn sleep_deadline(&self) -> Option<Instant> {
        if self.asleep.is_some() {
            return None;
        }
        self.prefs
            .sleep()
            .map(|m| self.last_touch + Duration::from_secs(m as u64 * 60))
    }

    /// Nobody has touched the screen for the chosen time: save, put the
    /// light out and show the screensaver.
    fn fall_asleep(&mut self) -> Result<(), Error> {
        // The screensaver replaces the screen: no redraw for a book still
        // opening, a book opening without a snapshot is given up, and no
        // snapshots are made ahead of time while asleep.
        self.finish_opening(true)?;
        self.cancel_pending();
        self.prewarm.input();
        if let Screen::Reader(r) = &self.screen {
            let now = aristotle_library::library::now();
            self.lib
                .set_progress(&r.session.id, r.session.progress(now));
        }
        self.save_snapshot();
        self.saver.save(&self.lib);
        self.debounce.saved();
        self.asleep = Some(self.light_step());
        self.set_light_step(0);
        let photos = self.prefs.covers.mode() == CoverMode::Photos;
        let book = shelf::current_book(&self.lib)
            .cloned()
            .map(|rec| shelf::screen_book(&self.lib, &rec, &mut self.covers, photos));
        let tree = sleep::build(&self.styles, book.as_ref(), photos);
        self.ui.set_tree(tree);
        perf!("sleep: screensaver after {} min", self.prefs.sleep_minutes);
        self.render_ui()
    }

    /// A touch on the screensaver: the light and the screen come back.
    fn wake(&mut self) -> Result<(), Error> {
        if let Some(step) = self.asleep.take() {
            self.set_light_step(step);
        }
        self.last_touch = Instant::now();
        self.prewarm.input();
        perf!("sleep: woken");
        match &self.screen {
            Screen::Library { .. } => self.show_library(Refresh::Full),
            Screen::Settings { page, .. } => {
                let page = *page;
                self.show_settings(Refresh::Full, page)
            }
            // (A book still opening was finished before the screensaver.)
            Screen::Reader(_) | Screen::Opening => self.reader_after_sleep(),
        }
    }

    // --- Event loop -------------------------------------------------------

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
            if let Some(d) = self.sleep_deadline() {
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
            if self.opening.is_some() || self.pending.is_some() {
                timeout = timeout.min(Duration::from_millis(10));
            }
            if let Some(d) = self.prewarm.deadline().filter(|_| self.asleep.is_none()) {
                timeout = timeout.min(d.saturating_duration_since(Instant::now()));
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
            if self.asleep.is_none() {
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
            }
            self.panel.poll();
            self.background()?;
            if self.sleep_deadline().is_some_and(|d| Instant::now() >= d) {
                self.fall_asleep()?;
            }
        }
    }

    /// Whether the UI tree (not the page-turn path) owns input right now.
    fn ui_driven(&self) -> bool {
        match &self.screen {
            Screen::Library { .. } | Screen::Settings { .. } => true,
            Screen::Reader(r) => r.menu_open || !r.overlay.is_none(),
            Screen::Opening => self.opening.as_ref().is_some_and(|o| o.menu_open),
        }
    }

    /// Work done between events: adopting page numbers, saving progress,
    /// the clock in the status line.
    fn background(&mut self) -> Result<(), Error> {
        if let Some(result) = self.opening.as_ref().and_then(|o| o.worker.poll()) {
            self.adopt(result, false)?;
        }
        if let Some(result) = self.pending.as_ref().and_then(|p| p.worker.poll()) {
            self.finish_pending(result)?;
        }
        self.poll_lookup()?;
        // Snapshots ahead of time: only on the library screen (not the
        // settings), with no book opening and no screensaver.
        if matches!(self.screen, Screen::Library { .. })
            && self.pending.is_none()
            && self.asleep.is_none()
        {
            self.prewarm.tick(
                &self.lib,
                &self.prefs.style(),
                &self.mobi_cache,
                self.speed.secs_per_page(),
            );
        }
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
        if self.asleep.is_none() {
            let (clock, _) = device::clock_and_date();
            if clock != self.clock {
                let page = self.list_page();
                match &self.screen {
                    Screen::Library { .. } => self.show_library_at(Refresh::Auto, page)?,
                    Screen::Settings { .. } => self.show_settings(Refresh::Auto, page)?,
                    Screen::Reader(_) | Screen::Opening => self.clock = clock,
                }
            }
        }
        Ok(())
    }

    fn on_touch(&mut self, ev: &aristotle_hal::input::TouchEvent) -> Result<Flow, Error> {
        if self.asleep.is_some() {
            // Any touch wakes; the rest of that touch does nothing.
            if ev.phase == HalPhase::Down {
                self.wake()?;
                self.ui.reset_gestures();
                if let Screen::Reader(r) = &mut self.screen {
                    r.gestures.reset();
                }
                self.swallow = true;
            }
            return Ok(Flow::Continue);
        }
        if self.swallow {
            if ev.phase == HalPhase::Up {
                self.swallow = false;
            }
            return Ok(Flow::Continue);
        }
        self.last_touch = Instant::now();
        let ev = ui_event(ev);
        self.prewarm.input();
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
            Screen::Settings { .. } => self.on_settings_event(ev),
            Screen::Reader(r) => {
                let reading_action =
                    matches!(ev, UiEvent::Action { id, .. } if id >= overlay::FIRST_ACTION);
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
        use ui_lib::action as a;
        // Anything but opening a book gives up a book still opening.
        let opens_book = match (&self.screen, &ev) {
            (Screen::Library { model, .. }, UiEvent::Action { id, .. }) => {
                *id == a::RESUME
                    || (*id >= a::ITEM
                        && matches!(
                            model.target((*id - a::ITEM) as usize),
                            Some(Target::Book(_))
                        ))
            }
            _ => false,
        };
        if !opens_book {
            self.cancel_pending();
        }
        let Screen::Library { model, view } = &mut self.screen else {
            return Ok(Flow::Continue);
        };
        match ev {
            UiEvent::LongPress(p)
                if self.nav.at_home() && quit_zone(&self.ui, view.title).contains(p) =>
            {
                perf!("quit: title held");
                return Ok(Flow::Quit);
            }
            UiEvent::Action { id, node, .. } => match id {
                a::RESUME => {
                    if let Some(book) = model.hero_id().map(String::from) {
                        self.open_from_library(&book, node)?;
                    }
                }
                a::SETTINGS => self.show_settings(Refresh::Full, 0)?,
                a::SORT => {
                    self.nav.sort = (self.nav.sort + 1) % SORTS.len();
                    self.prefs.sort = self.nav.sort;
                    self.save_prefs();
                    self.show_library(Refresh::Auto)?;
                }
                a::BACK => {
                    self.nav.up();
                    self.show_library(Refresh::Full)?;
                }
                id if id >= a::ITEM => match model.target((id - a::ITEM) as usize).cloned() {
                    Some(Target::Book(book)) => self.open_from_library(&book, node)?,
                    Some(Target::Folder(name)) => {
                        self.nav.folder.push(name);
                        self.show_library(Refresh::Full)?;
                    }
                    Some(Target::Series(name)) => {
                        self.nav.series = Some(name);
                        self.show_library(Refresh::Full)?;
                    }
                    None => {}
                },
                id => {
                    if let Some(i) = in_range(id, a::VIEW, View::ALL.len()) {
                        self.nav.view = View::ALL[i];
                        self.prefs.library_view = i;
                        self.save_prefs();
                        self.show_library(Refresh::Auto)?;
                    } else if let Some(i) = in_range(id, a::FILTER, Filter::ALL.len()) {
                        self.nav.filter = Filter::ALL[i];
                        self.show_library(Refresh::Auto)?;
                    } else if view.handle(&mut self.ui.tree, id) {
                        self.ui.layout();
                        view.sync(&mut self.ui.tree);
                        // A text-only list does not ghost: a quiet partial
                        // update. Cards and cover photos (large grays) do:
                        // their page turns are one clean flash each, never
                        // a flash by count.
                        if self.prefs.covers.mode() != CoverMode::None {
                            self.ui.request_full_refresh();
                        }
                    }
                }
            },
            _ => {}
        }
        Ok(Flow::Continue)
    }

    // --- Reader -----------------------------------------------------------

    /// Open a book from the library. The pressed card already shows the
    /// tap landed. With a snapshot the open is a single flash. Without one
    /// the book opens on a worker while the card stays inverted; a first
    /// open of a MOBI or AZW3 file also converts it (seconds on the
    /// device), so for those the busy mark shows too, in the same update.
    fn open_from_library(&mut self, id: &str, card: NodeId) -> Result<(), Error> {
        let slow = self.lib.book(id).is_some_and(|r| {
            matches!(
                r.format,
                aristotle_library::Format::Mobi | aristotle_library::Format::Azw3
            ) && r.last_opened.is_none()
        });
        if slow {
            if let Screen::Library { view, .. } = &self.screen {
                self.ui.tree.set_hidden(view.busy, false);
            }
        }
        self.open_book(id, Some(card))
    }

    /// Open the first book whose title contains `title` (case and
    /// diacritics ignored), as `--open` asks.
    fn open_by_title(&mut self, title: &str) -> Result<(), Error> {
        let want = aristotle_library::text::fold(title);
        let id = self
            .lib
            .books()
            .iter()
            .find(|b| aristotle_library::text::fold(&b.meta.title).contains(&want))
            .map(|b| b.id.clone());
        match id {
            Some(id) => self.open_book(&id, None),
            None => {
                eprintln!("aristotle: no book titled {title:?}");
                Ok(())
            }
        }
    }

    fn open_book(&mut self, id: &str, card: Option<NodeId>) -> Result<(), Error> {
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            return Ok(());
        }
        self.cancel_pending();
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
        // Nothing to show before the layout: the tapped card stays inverted
        // as the acknowledgement, the book opens on a worker, and the
        // library keeps taking input (another card, a filter) meanwhile.
        if let Some(card) = card {
            self.ui.tree.set_inverted(card, true);
        }
        perf!(
            "open: \"{}\": no snapshot, opening in the background",
            rec.meta.title
        );
        self.pending = Some(Box::new(Pending {
            id: id.to_string(),
            title: rec.meta.title.clone(),
            worker: open::spawn_with(src, style, false),
            card,
            tapped: t0,
        }));
        Ok(())
    }

    /// Forget a book opening without a snapshot (the reader tapped
    /// something else): its worker drops to idle priority, its card is
    /// released.
    fn cancel_pending(&mut self) {
        let Some(p) = self.pending.take() else {
            return;
        };
        self.release_card(p.card);
        perf!(
            "open: \"{}\" given up {:.0} ms after the tap",
            p.title,
            ms(p.tapped.elapsed())
        );
    }

    /// The library card of a book no longer opening: not inverted, and no
    /// busy mark.
    fn release_card(&mut self, card: Option<NodeId>) {
        if let Some(card) = card {
            self.ui.tree.set_inverted(card, false);
        }
        if let Screen::Library { view, .. } = &self.screen {
            self.ui.tree.set_hidden(view.busy, true);
        }
    }

    /// Show a book opened without a snapshot: one mode-8 flash to its page.
    fn finish_pending(&mut self, result: Result<Prepared, Error>) -> Result<(), Error> {
        let Some(p) = self.pending.take() else {
            return Ok(());
        };
        let prepared = match result {
            Ok(prepared) => prepared,
            Err(e) => {
                eprintln!("aristotle: {}: {e:#}", p.title);
                self.release_card(p.card);
                return self.render_ui();
            }
        };
        let times = prepared.times;
        let mut session = Session::from_prepared(prepared)?;
        session.set_speed(self.speed.secs_per_page());
        let page = session.page(session.current)?;
        let light = self.light_level();
        let settings = self.prefs.menu_settings(light);
        let (tree, menu, page_node, nav) =
            reader_tree(&self.styles, &session.title, &settings, page);
        // A screen switch: the stock UI's flashing full refresh (mode 8).
        self.ui.set_tree_with(tree, Refresh::Full);
        self.ui.gesture_config_mut().long_press_us = 500_000;
        self.render_ui()?;
        perf!(
            "open: \"{}\": book {:.0} ms, position {:.0} ms, first page {:.0} ms, submitted {:.0} ms after the tap; page numbers {}; no snapshot",
            session.title,
            ms(times.open),
            ms(times.place),
            ms(times.render),
            ms(p.tapped.elapsed()),
            if times.pages_cached { "from cache" } else { "being computed in the background" }
        );
        self.lib
            .mark_opened(&p.id, aristotle_library::library::now());
        self.debounce.changed(Instant::now());
        let gestures = TurnGestures::new(self.ui.bounds());
        let old = std::mem::replace(&mut self.screen, Screen::Opening);
        self.enter_reader(session, menu, page_node, nav, settings, gestures);
        drop(old);
        if let Screen::Reader(r) = &mut self.screen {
            let (pn, pp) = r.session.prerender();
            perf!(
                "open: prerender next {:.0} ms, previous {:.0} ms",
                ms(pn),
                ms(pp)
            );
        }
        Ok(())
    }

    /// Show the page the book was left at from its snapshot, and open the
    /// book itself on a worker thread (see open.rs).
    fn open_from_snapshot(
        &mut self,
        mut src: Source,
        style: Style,
        mut snap: snapshot::Loaded,
        t0: Instant,
    ) -> Result<(), Error> {
        let light = self.light_level();
        let settings = self.prefs.menu_settings(light);
        let page = snap.page.clone();
        let (tree, menu, page_node, nav) =
            reader_tree(&self.styles, &src.title, &settings, page.clone());
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
        // Open exactly where the snapshot is, not where its locator maps.
        src.exact = Some(snap.info.location);
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
            let before = self.panel.full_updates();
            let stale = self.finish_opening(true)?;
            self.on_intent(intent)?;
            self.redraw_if_stale(stale, before)?;
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
            // Selecting, footnotes and the bookmark need the book: while it
            // opens they do nothing (no wait, nothing on the panel).
            Intent::Hold { .. } | Intent::Drag(_) | Intent::Release(_) | Intent::Tap(_) => Ok(true),
            Intent::OpenMenu => {
                let light = self.light_level();
                let night = self.prefs.night;
                let Some(o) = self.opening.as_mut() else {
                    return Ok(false);
                };
                o.menu_open = true;
                o.settings.light = light;
                o.settings.night = night;
                let (info, page) = o.shown().clone();
                fit_menu(
                    &mut self.ui,
                    &self.styles,
                    &o.title,
                    &o.settings,
                    &page,
                    &mut o.menu,
                    &mut o.page_node,
                    &mut o.nav,
                );
                self.ui.tree.set_text(o.menu.chapter_label, &info.chapter);
                self.ui.tree.set_text(o.menu.page_label, &info.label);
                self.ui.tree.set_progress(o.menu.progress, info.fraction);
                if let Some(w) = self.ui.tree.widget_mut::<Slider>(o.menu.light) {
                    w.value = light;
                }
                self.ui.tree.invalidate(o.menu.light, Refresh::Auto);
                self.ui.reset_gestures();
                self.ui.layout();
                let parts = menu_parts(&self.ui, &o.menu, o.nav);
                self.ui.render();
                self.panel.present_overlay(&self.ui, &parts);
                Ok(true)
            }
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
                self.ui.tree.set_hidden(o.nav, true);
                self.ui.render();
                let page = o.shown().1.clone();
                self.panel.show_page(&page, false, "menu closed")?;
            }
            UiEvent::Action {
                id: menu_action::BACK,
                ..
            } => self.abandon_opening()?,
            UiEvent::Action { id, .. } => {
                let before = self.panel.full_updates();
                let stale = self.finish_opening(true)?;
                if id >= overlay::FIRST_ACTION {
                    self.on_overlay_event(ev)?;
                } else {
                    self.on_menu_event(ev)?;
                }
                self.redraw_if_stale(stale, before)?;
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    /// After an action that needed the book: if the adopted page differs
    /// from what the panel shows and the action did not redraw the whole
    /// screen itself (a turn at the first or last page, a light change),
    /// one full mode-9 update now.
    fn redraw_if_stale(&mut self, stale: bool, full_updates_before: u64) -> Result<(), Error> {
        if !stale || self.panel.full_updates() != full_updates_before {
            return Ok(());
        }
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        self.ui.tree.invalidate(r.page_node, Refresh::Auto);
        if r.menu_open {
            self.render_ui_with(Content::Page)
        } else {
            let page = r.session.page(r.session.current)?;
            self.panel.show_page(&page, false, "corrected page")?;
            self.ui.render();
            Ok(())
        }
    }

    /// Keep the page shown from the snapshot as the progress when the book
    /// will not be adopted (back to the library, or the open failed).
    fn keep_shown_progress(&mut self, o: &Opening) {
        if !o.turned {
            return;
        }
        let (info, page) = o.shown().clone();
        let now = aristotle_library::library::now();
        let progress = progress::progress_for(info.locator.clone(), info.fraction, now);
        self.lib.set_progress(&o.id, progress);
        if info.last {
            self.lib.set_status(&o.id, Status::Finished);
        }
        let key = snapshot::Key::new(&o.id, o.stamp, &o.style, info.locator.clone());
        self.snapshots.save(key, (info, page), None);
    }

    /// Back to the library before the book finished opening: drop the
    /// worker and keep the page shown as the progress (and its snapshot).
    fn abandon_opening(&mut self) -> Result<(), Error> {
        let Some(o) = self.opening.take() else {
            return Ok(());
        };
        self.keep_shown_progress(&o);
        perf!(
            "open: \"{}\": back to the library {:.0} ms after the tap, before the book was ready",
            o.title,
            ms(o.tapped.elapsed())
        );
        // Dropping the worker cancels it between its steps, at idle priority.
        drop(o);
        self.show_library(Refresh::Full)?;
        self.snapshots.forget();
        self.saver.save(&self.lib);
        self.debounce.saved();
        Ok(())
    }

    /// Wait for the book being opened and adopt it. With `deferred`, a
    /// page that differs from the snapshot is not sent: the caller redraws
    /// next. Returns whether the panel then still shows the snapshot's page
    /// where the book's differs.
    fn finish_opening(&mut self, deferred: bool) -> Result<bool, Error> {
        let Some(o) = self.opening.as_ref() else {
            return Ok(false);
        };
        let t = Instant::now();
        let result = o.worker.wait();
        perf!("open: waited {:.0} ms for the book", ms(t.elapsed()));
        self.adopt(result, deferred)
    }

    /// Take over the session the worker made, and make sure the panel shows
    /// its page: no update if the snapshot showed exactly that page, else
    /// one full mode-9 update (none with `deferred`; see
    /// [`App::finish_opening`]).
    fn adopt(&mut self, result: Result<Prepared, Error>, deferred: bool) -> Result<bool, Error> {
        let Some(o) = self.opening.take() else {
            return Ok(false);
        };
        let prepared = match result {
            Ok(p) => p,
            Err(e) => {
                eprintln!("aristotle: {}: {e:#}", o.title);
                self.keep_shown_progress(&o);
                self.show_library(Refresh::Full)?;
                self.saver.save(&self.lib);
                return Ok(false);
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
        let mut page = session.page(at)?;
        let verdict = if right {
            open::compare(&o.shown().1, &page, &session.style)
        } else {
            open::Verdict::Page
        };
        if verdict == open::Verdict::Chrome {
            // Same text, but the header or footer moved on since the
            // snapshot was saved (page numbers that were not known then, a
            // new reading speed). The page on the panel stays until the
            // next turn: the open remains one update.
            page = o.shown().1.clone();
            session.keep_page(at, page.clone());
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
        let same = verdict != open::Verdict::Page;
        if same && !o.turned {
            // The snapshot on disk is exactly this page: don't write it back.
            let key = snapshot::Key::new(&o.id, o.stamp, &o.style, o.current.0.locator.clone());
            let next = o.next.as_ref().map(|(i, _)| i.location);
            self.snapshots.already_saved(key, page.clone(), next);
        }
        if o.turned || !right {
            let now = aristotle_library::library::now();
            self.lib.set_progress(&session.id, session.progress(now));
            self.debounce.changed(Instant::now());
        }
        let o = *o;
        let menu_open = o.menu_open;
        let page_node = o.page_node;
        if let Some(w) = self.ui.tree.widget_mut::<PageView>(page_node) {
            w.page = page.clone();
        }
        self.enter_reader(session, o.menu, page_node, o.nav, o.settings, o.gestures);
        if let Screen::Reader(r) = &mut self.screen {
            if menu_open {
                // The labels go out in the same frame as the page, if any.
                r.menu_open = true;
                update_menu_labels(&mut self.ui, r);
            }
        }
        match (same, deferred, menu_open) {
            (true, _, true) => self.render_ui()?,
            // The UI's copy of the screen already has these pixels: no
            // update at all.
            (true, _, false) => {}
            // The caller redraws the whole screen next, or asks us to.
            (false, true, _) => {}
            // Never a partial fix-up: one full mode-9 update, like a page
            // turn (behind an open menu, like a chapter jump).
            (false, false, true) => {
                self.ui.tree.invalidate(page_node, Refresh::Auto);
                self.render_ui_with(Content::Page)?;
            }
            (false, false, false) => {
                self.panel.show_page(&page, false, "corrected page")?;
                self.ui.tree.invalidate(page_node, Refresh::Auto);
                // The panel already shows the page; this only updates the UI's copy.
                self.ui.render();
            }
        }
        if let Screen::Reader(r) = &mut self.screen {
            r.session.prerender();
        }
        Ok(!same && deferred)
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
        if let Some((key, current, next)) = self.snapshot_parts() {
            self.snapshots.save(key, current, next);
        }
    }

    /// What [`App::save_snapshot`] stores, without copying any pixels yet;
    /// sets the progress it belongs to.
    #[allow(clippy::type_complexity)]
    fn snapshot_parts(
        &mut self,
    ) -> Option<(
        snapshot::Key,
        (snapshot::PageInfo, Rc<Page>),
        Option<(snapshot::PageInfo, Rc<Page>)>,
    )> {
        let Screen::Reader(r) = &mut self.screen else {
            return None;
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
        let current = s.page(here).ok()?;
        let next = s
            .known_next(here)
            .and_then(|l| s.rendered(l).map(|p| (open::page_info(s, l), p)));
        let info = open::page_info(s, here);
        Some((key, (info, current), next))
    }

    /// The reader screen built again around the current page, menu and
    /// overlays closed (after the screensaver): one flash.
    fn reader_after_sleep(&mut self) -> Result<(), Error> {
        let settings = self.prefs.menu_settings(self.light_level());
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        r.settings = settings;
        r.menu_open = false;
        r.overlay = Overlay::None;
        r.gestures.reset();
        self.rebuild_reader(Refresh::Full)?;
        self.render_ui()
    }

    fn on_intent(&mut self, intent: Intent) -> Result<(), Error> {
        match intent {
            Intent::Turn { dir, at_us } => self.turn(dir, at_us),
            Intent::OpenMenu => self.open_menu(),
            Intent::Hold { at } => self.hold(at),
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
        let night = self.prefs.night;
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        r.menu_open = true;
        r.settings.light = light;
        r.settings.night = night;
        let page = r.session.page(r.session.current)?;
        fit_menu(
            &mut self.ui,
            &self.styles,
            &r.session.title,
            &r.settings,
            &page,
            &mut r.menu,
            &mut r.page_node,
            &mut r.nav,
        );
        update_menu_labels(&mut self.ui, r);
        if let Some(w) = self.ui.tree.widget_mut::<Slider>(r.menu.light) {
            w.value = light;
        }
        self.ui.tree.invalidate(r.menu.light, Refresh::Auto);
        self.ui.reset_gestures();
        self.ui.layout();
        let parts = menu_parts(&self.ui, &r.menu, r.nav);
        self.ui.render();
        self.panel.present_overlay(&self.ui, &parts);
        Ok(())
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
                    menu_action::NIGHT => {
                        let on = !self.prefs.night;
                        s.night = on;
                        r.settings = s;
                        if let Some(t) = r.menu.night_toggle {
                            if let Some(w) = self.ui.tree.widget_mut::<Toggle>(t) {
                                w.on = on;
                            }
                            self.ui.tree.invalidate(t, Refresh::Auto);
                        }
                        self.set_night(on);
                        self.render_ui()
                    }
                    menu_action::LIGHT_SET | menu_action::LIGHT_DOWN | menu_action::LIGHT_UP => {
                        let cur = self.light.as_ref().and_then(|l| l.step().ok()).unwrap_or(0);
                        let step = match id {
                            menu_action::LIGHT_SET => {
                                Slider::step_at(self.ui.tree.rect(node), at.x, LIGHT_STEPS)
                            }
                            menu_action::LIGHT_DOWN => cur.saturating_sub(1),
                            _ => (cur + 1).min(LIGHT_STEPS),
                        };
                        if let Some(l) = &self.light {
                            if let Err(e) = l.set_step(step) {
                                eprintln!("aristotle: frontlight: {e}");
                            }
                        }
                        s.light = step as f32 / LIGHT_STEPS as f32;
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
            w.page = page.clone();
        }
        self.ui.tree.invalidate(r.page_node, Refresh::Auto);
        fit_menu(
            &mut self.ui,
            &self.styles,
            &r.session.title,
            &r.settings,
            &page,
            &mut r.menu,
            &mut r.page_node,
            &mut r.nav,
        );
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
        // A long relayout: the busy mark goes out with the button's release
        // (one DU update), the new page after it (one full mode-9 update).
        if r.session.slow_relayout() {
            self.ui.tree.set_hidden(r.menu.busy, false);
            self.render_ui()?;
        }
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        let t = Instant::now();
        r.session.restyle(self.prefs.style())?;
        let page = r.session.page(r.session.current)?;
        let t_layout = t.elapsed();
        r.settings = settings;
        let (tree, menu, page_node, nav) =
            reader_tree(&self.styles, &r.session.title, &settings, page.clone());
        r.menu = menu;
        r.page_node = page_node;
        r.nav = nav;
        self.ui.set_tree_with(tree, Refresh::Auto);
        fit_menu(
            &mut self.ui,
            &self.styles,
            &r.session.title,
            &settings,
            &page,
            &mut r.menu,
            &mut r.page_node,
            &mut r.nav,
        );
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
        let snapshot = self.snapshot_parts();
        // The library goes out first; copying the pages and serialising the
        // index wait until it is submitted.
        self.show_library(Refresh::Full)?;
        if let Some((key, current, next)) = snapshot {
            self.snapshots.save(key, current, next);
        }
        // The closed book's page needn't stay in memory for the skip test.
        self.snapshots.forget();
        self.saver.save(&self.lib);
        self.debounce.saved();
        Ok(())
    }

    /// Save progress, give back the light and the stock screensaver, and
    /// leave a white day-mode screen for the stock UI.
    fn leave(self) -> Result<(), Error> {
        let mut app = self;
        // A forward turn made while the book was opening counts.
        app.finish_opening(true)?;
        if let Screen::Reader(r) = &app.screen {
            let now = aristotle_library::library::now();
            app.lib.set_progress(&r.session.id, r.session.progress(now));
        }
        if let Some(step) = app.asleep.take() {
            app.set_light_step(step);
        }
        if app.screensaver_was == Some(false) {
            let _ = power::prevent_screensaver(false);
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

/// The reader screen: the page with the (hidden) menu and its row of
/// reading actions over it. Returns the tree, the menu, the page node and
/// the row.
fn reader_tree(
    styles: &Styles,
    title: &str,
    settings: &Settings,
    page: Rc<Page>,
) -> (Tree, Menu, NodeId, NodeId) {
    reader_tree_with(styles, title, settings, page, (0, 0), &mut |_| Vec::new())
}

/// [`reader_tree`] with the menu's edges moved by `extents` (see
/// [`fit_menu`]) and the layers `extra` adds between the page and the menu
/// (the selection, a popup; see reading.rs).
fn reader_tree_with(
    styles: &Styles,
    title: &str,
    settings: &Settings,
    page: Rc<Page>,
    extents: (i32, i32),
    extra: &mut dyn FnMut(&mut Tree) -> Vec<NodeId>,
) -> (Tree, Menu, NodeId, NodeId) {
    let mut t = Tree::new();
    let page_node = t.custom(PageView { page });
    let mut layers = vec![page_node];
    layers.extend(extra(&mut t));
    let spec = MenuSpec {
        title,
        // Only Literata ships with the renderer, so there is no font row.
        families: None,
        night: true,
        top_extra: extents.0,
        bottom_extra: extents.1,
    };
    let menu = build_menu(&mut t, styles, &spec, settings);
    let nav = overlay::nav_bar(&mut t, styles, extents.0);
    layers.push(menu.root);
    layers.push(nav);
    let root = t.add(Node::new(Kind::Stack), layers);
    t.set_root(root);
    (t, menu, page_node, nav)
}

/// Show the menu over `page` with its edges in the white between two lines
/// of text, never through them: the bars hide whole lines. The top edge is
/// the bottom of the row of reading actions under the top bar. Rebuilds the
/// reader tree when the edges have to move (the menu's labels are then
/// fresh: set them after this).
#[allow(clippy::too_many_arguments)]
fn fit_menu(
    ui: &mut Ui,
    styles: &Styles,
    title: &str,
    settings: &Settings,
    page: &Rc<Page>,
    menu: &mut Menu,
    page_node: &mut NodeId,
    nav: &mut NodeId,
) {
    ui.tree.set_hidden(menu.root, false);
    ui.tree.set_hidden(*nav, false);
    ui.layout();
    let top = ui.tree.rect(menu.top).bottom() - menu.top_extra + overlay::NAV_H;
    let sheet = ui.tree.rect(menu.sheet).y + menu.bottom_extra;
    let extents = mask_extents(
        &page.data,
        page.stride,
        page.width as usize,
        page.height as usize,
        top,
        sheet,
        120,
    );
    if extents != (menu.top_extra, menu.bottom_extra) {
        let (tree, m, pn, nv) =
            reader_tree_with(styles, title, settings, page.clone(), extents, &mut |_| {
                Vec::new()
            });
        ui.set_tree_with(tree, Refresh::Auto);
        ui.tree.set_hidden(m.root, false);
        ui.tree.set_hidden(nv, false);
        ui.layout();
        *menu = m;
        *page_node = pn;
        *nav = nv;
    }
}

/// The opaque parts of the menu, for [`Panel::present_overlay`]: the top
/// bar with the row of reading actions under it, and the sheet.
fn menu_parts(ui: &Ui, menu: &Menu, nav: NodeId) -> [Rect; 2] {
    let top = ui.tree.rect(menu.top);
    let row = ui.tree.rect(overlay::nav_row(&ui.tree, nav));
    [top.union(&row), ui.tree.rect(menu.sheet)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_fonts_map_literata() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../aristotle-render/assets/fonts");
        let fonts = ui_fonts(&dir).unwrap();
        assert_eq!(fonts.family_by_name("Literata"), Some(FamilyId(0)));
        assert!(ui_fonts(&dir.join("missing")).is_err());
    }

    /// The runtime data of `tools/build-render.sh host`, found as the
    /// renderer's tests find it.
    fn render_data() -> Option<PathBuf> {
        let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
        let dir = var("ARISTOTLE_RENDER_DATA")
            .map(PathBuf::from)
            .or_else(|| {
                var("ARISTOTLE_CACHE")
                    .map(PathBuf::from)
                    .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".cache/aristotle")))
                    .map(|c| c.join("render/host/data"))
            })?;
        dir.join("serif").is_dir().then_some(dir)
    }

    /// The app on an in-memory panel, as `--preview` runs it.
    fn memory_app(library: &Path, root: &Path) -> App {
        let args = Args {
            data: PathBuf::new(),
            library: library.to_path_buf(),
            root: root.to_path_buf(),
            turn_mode: None,
            night_method: None,
            scan: true,
            bench_open: None,
            dicts: Vec::new(),
            preview: None,
            open: None,
            index_dicts: false,
            test_clippings: None,
            lang: None,
        };
        let panel = Panel::memory(1072, 1448, prefs::STOCK_TURN);
        build_app(
            &args,
            panel,
            Prefs::default(),
            start_dictionaries(&args),
            None,
        )
        .unwrap()
    }

    fn updates(app: &App) -> Vec<String> {
        match &app.panel {
            Panel::Memory(m) => m.updates.iter().map(|u| u.label.clone()).collect(),
            Panel::Device(_) => Vec::new(),
        }
    }

    /// Run the loop's background work until `done`, for at most 20 s.
    fn wait(app: &mut App, done: impl Fn(&App) -> bool) {
        let t = Instant::now();
        while !done(app) {
            assert!(t.elapsed() < Duration::from_secs(20), "timed out");
            std::thread::sleep(Duration::from_millis(10));
            app.background().unwrap();
        }
    }

    /// Open the book from its snapshot and return the panel updates that
    /// took, the snapshot's flash included.
    fn reopen(app: &mut App, title: &str) -> Vec<String> {
        let before = updates(app).len();
        app.open_by_title(title).unwrap();
        assert!(matches!(app.screen, Screen::Opening), "no snapshot open");
        wait(app, |a| matches!(a.screen, Screen::Reader(_)));
        updates(app)[before..].to_vec()
    }

    /// Reopening a book from its snapshot is one update, also when the
    /// snapshot's footer is out of date: made ahead of time before the page
    /// numbers were known (or at another reading speed).
    /// An EPUB titled "Reopened" with two chapters in its contents (the
    /// footer's progress line needs chapters).
    fn write_book(path: &Path) {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let mut put = |name: &str, data: String| {
            zip.start_file(name, opts).unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        };
        put("mimetype", "application/epub+zip".into());
        put(
            "META-INF/container.xml",
            "<?xml version=\"1.0\"?><container version=\"1.0\" \
             xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles>\
             <rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/>\
             </rootfiles></container>"
                .into(),
        );
        for c in 0..2 {
            let mut body = format!("<h1>Chapter {c}</h1>");
            for p in 0..40 {
                body.push_str(&format!(
                    "<p>Paragraph {p}. Every art, and every science reduced to a \
                     teachable form, aims, it is thought, at some good.</p>"
                ));
            }
            put(
                &format!("c{c}.xhtml"),
                format!(
                    "<?xml version=\"1.0\" encoding=\"utf-8\"?><html \
                     xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"en\"><head><title>{c}\
                     </title></head><body>{body}</body></html>"
                ),
            );
        }
        put(
            "nav.xhtml",
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><html xmlns=\"http://www.w3.org/1999/xhtml\" \
             xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>Contents</title></head>\
             <body><nav epub:type=\"toc\"><ol><li><a href=\"c0.xhtml\">Chapter 0</a></li>\
             <li><a href=\"c1.xhtml\">Chapter 1</a></li></ol></nav></body></html>"
                .into(),
        );
        put(
            "content.opf",
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><package \
             xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"id\">\
             <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:identifier \
             id=\"id\">reopened</dc:identifier><dc:title>Reopened</dc:title>\
             <dc:language>en</dc:language></metadata><manifest>\
             <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" \
             properties=\"nav\"/>\
             <item id=\"c0\" href=\"c0.xhtml\" media-type=\"application/xhtml+xml\"/>\
             <item id=\"c1\" href=\"c1.xhtml\" media-type=\"application/xhtml+xml\"/>\
             </manifest><spine><itemref idref=\"c0\"/><itemref idref=\"c1\"/></spine></package>"
                .into(),
        );
        zip.finish().unwrap();
    }

    #[test]
    fn reopening_from_a_snapshot_is_one_update() {
        let Some(data) = render_data() else {
            eprintln!("skipped: no render data directory (run tools/build-render.sh host)");
            return;
        };
        aristotle_render::use_data_dir(&data).unwrap();
        let dir = testdir("reopen");
        let (root, library) = (dir.join("books"), dir.join("library"));
        std::fs::create_dir_all(&root).unwrap();
        let book = root.join("reopened.epub");
        write_book(&book);
        let mut lib = Library::open(&library).unwrap();
        lib.scan(&root, &ScanOptions::default()).unwrap();
        let id = lib.books()[0].id.clone();
        let at = progress::progress_at(&aristotle_render::book::Position::Fraction(0.4), 0.4, 1);
        lib.set_progress(&id, at);
        lib.save().unwrap();
        // A reading speed of its own: the footer's minutes depend on it.
        std::fs::write(
            library.join("reading.json"),
            r#"{"secs_per_page": 10.0, "samples": 2}"#,
        )
        .unwrap();
        let snap = snapshot::path(&library, &id);
        let prewarm = |app: &mut App| {
            std::thread::sleep(prewarm::QUIET + Duration::from_millis(100));
            wait(app, |_| snap.exists());
            // Let the job finish and be reaped.
            wait(app, |a| a.prewarm.deadline().is_none());
        };

        // 1. A snapshot made ahead of time, with no page numbers known yet;
        //    then the book is paginated (as by an earlier read) and opened.
        let mut app = memory_app(&library, &root);
        prewarm(&mut app);
        let style = app.prefs.style();
        let stamp = snapshot::file_stamp(&book).unwrap();
        let mut b = aristotle_render::book::Book::open(&book, &style, None).unwrap();
        b.paginate();
        let cache = pagecache::cache_path(&library, &id, stamp.0, stamp.1, &style);
        pagecache::store(&cache, b.pagination().unwrap()).unwrap();
        assert_eq!(reopen(&mut app, "Reopened"), ["flash"]);
        app.leave().unwrap();

        // 2. Made ahead of time with the page numbers known: the footer is
        //    the one the open renders, at the reader's own speed.
        std::fs::remove_file(&snap).unwrap();
        let mut app = memory_app(&library, &root);
        prewarm(&mut app);
        let rec = app.lib.book(&id).unwrap().clone();
        let mut src = Source::new(&app.lib, &rec, &app.mobi_cache).unwrap();
        src.secs_per_page = app.speed.secs_per_page();
        let shown = open::find_snapshot(&src, &style).expect("a valid snapshot");
        src.exact = Some(shown.info.location);
        let mut s =
            Session::from_prepared(Session::prepare(src, style.clone(), false).unwrap()).unwrap();
        let real = s.page(s.current).unwrap();
        assert_eq!(
            open::compare(&shown.page, &real, &style),
            open::Verdict::Same
        );
        assert_eq!(reopen(&mut app, "Reopened"), ["flash"]);
        app.leave().unwrap();
    }
}

/// A fresh, empty directory for one test, under `$TMPDIR` (the build
/// script points it at `$ARISTOTLE_CACHE/tmp`, not the RAM-backed /tmp).
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
        .join("aristotle-tests")
        .join(format!("{name}-{}-{n}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
