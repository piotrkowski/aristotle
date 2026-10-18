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
//! aristotle [--data DIR] [--library DIR] [--root DIR] [--fonts DIR] [--turn-mode N]
//!           [--night-method software|hardware] [--no-scan] [--bench-open TITLE]
//!           [--open TITLE] [--dict DIR]... [--index-dicts] [--test-clippings PATH]
//!           [--preview OUT BOOK] [--preview-layout OUT BOOK WIDTH HEIGHT]
//!
//!   --data DIR      renderer data (fonts/, css/, hyphenation-patterns/,
//!                   serif/); default: data/ next to the binary
//!   --library DIR   library index, covers, page cache and preferences;
//!                   default: $ARISTOTLE_DATA or /mnt/us/.aristotle
//!   --root DIR      where the books are; default: /mnt/us/documents
//!   --fonts DIR     the reader's own TTF and OTF fonts; default: /mnt/us/fonts
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
//!   --preview-layout OUT BOOK WIDTH HEIGHT  check reading at a pixel size (default: 300 DPI)
//!   --dpi DPI       density for --preview-layout (default: 300)
//! ```
//!
//! Run it with the stock GUI stopped: `device/app/run-aristotle.sh`.
//! Hold the "Biblioteka" title for a second to quit. Timings go to stderr
//! as `[perf ...]` lines; see docs/reader.md.

mod catalogs;
mod device;
mod dict;
mod fonts;
mod launcher;
mod marks;
mod net;
mod open;
mod overlay;
mod pagecache;
mod panel;
mod powerd;
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
mod sync;
mod turns;

use std::cell::Cell;
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use aristotle_hal::input::{monotonic_us, Phase as HalPhase, TouchScreen, Transform};
use aristotle_hal::power::{Battery, Frontlight, LIGHT_STEPS};
use aristotle_hal::system;
use aristotle_library::{Library, ScanOptions, Status};
use aristotle_render::page::Page;
use aristotle_render::plato_core::anyhow::{bail, format_err, Context, Error};
use aristotle_render::style::Style;
use aristotle_ui::geom::{Density, Rect, Screen as ScreenProfile, Size};
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
    preview_size: Option<Size>,
    preview_density: Density,
    open: Option<String>,
    index_dicts: bool,
    test_clippings: Option<PathBuf>,
    /// `--lang CODE`: the UI language, stored in the preferences.
    lang: Option<String>,
    /// The folder of the binary, where `sync.json` and `catalogs.json`
    /// are read from.
    bundle: PathBuf,
    /// The private folder for passwords (`net::accounts_dir`).
    accounts: PathBuf,
    /// The user's fonts folder (`fonts.rs`).
    fonts: PathBuf,
}

fn usage() -> &'static str {
    "usage: aristotle [--data DIR] [--library DIR] [--root DIR] [--fonts DIR] [--turn-mode 9|7] \
     [--night-method software|hardware] [--no-scan] [--bench-open TITLE] [--open TITLE] \
     [--dict DIR]... [--index-dicts] [--test-clippings PATH] [--lang en|pl] \
     [--preview OUT BOOK] [--preview-layout OUT BOOK WIDTH HEIGHT] [--dpi DPI]"
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
        preview_size: None,
        preview_density: Density::default(),
        open: None,
        index_dicts: false,
        test_clippings: None,
        lang: None,
        bundle: exe_dir.clone(),
        accounts: net::accounts_dir(),
        fonts: PathBuf::from(fonts::DEFAULT_DIR),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format_err!("{name} needs a value"));
        match arg.as_str() {
            "--data" => a.data = cwd.join(value("--data")?),
            "--library" => a.library = cwd.join(value("--library")?),
            "--root" => a.root = cwd.join(value("--root")?),
            "--fonts" => a.fonts = cwd.join(value("--fonts")?),
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
            "--preview" | "--preview-layout" => {
                let out = cwd.join(value(&arg)?);
                let book = cwd.join(value(&arg)?);
                a.preview_size = if arg == "--preview-layout" {
                    let w = value(&arg)?.parse::<i32>()?;
                    let h = value(&arg)?.parse::<i32>()?;
                    if !(600..=4096).contains(&w) || !(800..=4096).contains(&h) {
                        bail!("preview width must be 600..4096, height must be 800..4096");
                    }
                    Some(Size::new(w, h))
                } else {
                    None
                };
                a.preview = Some((out, book));
            }
            "--lang" => {
                let code = value("--lang")?;
                if !aristotle_i18n::locales().iter().any(|l| l.code == code) {
                    bail!("unknown language {code}\n{}", usage());
                }
                a.lang = Some(code);
            }
            "--dpi" => {
                a.preview_density =
                    Density::new(value("--dpi")?.parse()?).map_err(|e| format_err!("{e}"))?
            }
            "-h" | "--help" => bail!("{}", usage()),
            other => bail!("unknown argument {other}\n{}", usage()),
        }
    }
    if a.preview_density != Density::default() && a.preview_size.is_none() {
        bail!("--dpi requires --preview-layout; device density stays at 300 DPI");
    }
    Ok(a)
}

fn main() -> ExitCode {
    START.get_or_init(Instant::now);
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("aristotle: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<u8, Error> {
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
        return open::bench(&args.library, root, title).map(|()| 0);
    }
    if let Some((out, book)) = &args.preview {
        return preview::run(&args, out, book).map(|()| 0);
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
        return Ok(0);
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
    // Our screensaver replaces the stock one, and powerd's events drive it
    // (powerd.rs). Put back as found on exit.
    let sleep_ctl = powerd::Sleep::start().context("can't acquire the screensaver")?;
    let mut app = build_app(&args, panel, prefs, start_dictionaries(&args), sleep_ctl)?;
    perf!("startup: first frame submitted");
    open::warm_up();

    if args.scan {
        app.scan(&args.root)?;
    }
    if let Some(title) = &args.open {
        app.open_by_title(title)?;
    }
    let result = app.run(&mut touch);
    let code = app.exit_target.exit_code();
    app.leave()?;
    result.map(|()| code)
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
    sleep_ctl: powerd::Sleep,
) -> Result<App, Error> {
    let (w, h) = panel.size();
    aristotle_i18n::set_language(&prefs.language);
    let profile =
        ScreenProfile::new(w, h, args.preview_density.dpi()).map_err(|e| format_err!("{e}"))?;
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
    let ui = Ui::with_screen(
        Tree::new(),
        TextEngine::new(ui_fonts(Path::new("serif"))?),
        profile,
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
        styles: Styles::new(FamilyId(0)).with_density(profile.density),
        covers: Covers::default(),
        screen: Screen::Settings { ids: None, page: 0 },
        nav,
        asleep: None,
        last_touch: Instant::now(),
        clock: String::new(),
        sleep_ctl,
        swallow: false,
        dict: Some(dict),
        speed: Speed::load(&args.library),
        exports: export_paths(args),
        net: net::Net::new(net::Paths {
            state: args.accounts.clone(),
            import: args.bundle.clone(),
        }),
        sync: sync::SyncState::default(),
        catalogs: catalogs::CatalogState::default(),
        fonts_dir: args.fonts.clone(),
        font_faces: HashMap::new(),
        exit_target: launcher::Target::Kindle,
        settings_message: None,
        kual_available: launcher::kual_available(),
    };
    app.load_font();
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
    /// Progress sync and the catalogs (catalogs.rs).
    Network(Box<catalogs::NetScreen>),
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
    /// The saved light step while asleep.
    asleep: Option<u32>,
    last_touch: Instant,
    /// The time the status line shows, to refresh it each minute.
    clock: String,
    /// powerd: its screensaver and suspend events, the idle timer.
    sleep_ctl: powerd::Sleep,
    /// The touch that woke the screensaver: ignored until it lifts.
    swallow: bool,
    /// Dictionary lookups (a thread of their own).
    dict: Option<Service>,
    speed: Speed,
    /// Where highlights are exported.
    exports: ExportPaths,
    /// Network settings and requests (net.rs).
    net: net::Net,
    /// Progress sync (sync.rs).
    sync: sync::SyncState,
    /// The catalog screens' requests (catalogs.rs).
    catalogs: catalogs::CatalogState,
    /// The user's fonts folder (fonts.rs).
    fonts_dir: PathBuf,
    /// UI families made for showing font names, by regular face file.
    font_faces: HashMap<PathBuf, FamilyId>,
    exit_target: launcher::Target,
    settings_message: Option<String>,
    kual_available: bool,
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
    ui.tree.rect(title).expand_to(ui.profile().density.mm(9.0))
}

/// The index of `id` in `first..first + n`, if it is there.
fn in_range(id: u32, first: u32, n: usize) -> Option<usize> {
    (first..first + n as u32)
        .contains(&id)
        .then(|| (id - first) as usize)
}

impl App {
    fn page_style(&self) -> Style {
        self.prefs
            .style()
            .with_screen(self.ui.profile(), self.panel.stride())
    }

    /// Render and send UI chrome changes.
    fn render_ui(&mut self) -> Result<(), Error> {
        self.render_ui_with(Content::Chrome)
    }

    fn render_ui_with(&mut self, content: Content) -> Result<(), Error> {
        if self.asleep.is_some() {
            return Ok(());
        }
        self.submit_ui(content)
    }

    fn submit_ui(&mut self, content: Content) -> Result<(), Error> {
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
        self.rebuild_library_at(refresh, page, false)
    }

    fn rebuild_library_at(
        &mut self,
        refresh: Refresh,
        page: usize,
        deferred: bool,
    ) -> Result<(), Error> {
        let mode = self.prefs.covers.mode();
        let model = ShelfModel::build(&self.lib, &self.nav, mode, &mut self.covers);
        let facts = Facts::status(&self.battery);
        self.clock = facts.clock.clone();
        let width = self.ui.bounds().w;
        let (tree, view) = ui_lib::Library::build_shelf_for_width(
            &mut self.ui.text,
            &self.styles,
            &model.shelf(facts.status_line(), mode),
            width,
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
        let submitted = if deferred { Ok(()) } else { self.render_ui() };
        // A closed book is freed only now: the display lists of a laid-out
        // chapter are many allocations, and the library should not wait.
        drop(old);
        submitted
    }

    fn list_page(&self) -> usize {
        match &self.screen {
            Screen::Library { view, .. } => self.ui.tree.page(view.list).0,
            Screen::Settings { ids: Some(ids), .. } => self.ui.tree.page(ids.list).0,
            Screen::Network(ns) => ns.frame.as_ref().map_or(0, |f| self.ui.tree.page(f.list).0),
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
        let sync_account = self
            .net
            .accounts
            .sync
            .as_ref()
            .filter(|s| !s.server.is_empty())
            .map(|s| {
                aristotle_i18n::tf!(
                    aristotle_i18n::Msg::SettingsSyncOn,
                    user = s.username,
                    server = s.server
                )
            });
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
            sleep_screen_cover: self.prefs.sleep_screen == prefs::SleepScreen::BookCover,
            clippings: self.prefs.clippings,
            battery: facts.battery_info(),
            storage: facts.storage,
            date: &facts.date,
            time: &facts.clock,
            usb: facts.usb,
            sync: sync_account.as_deref(),
            catalogs: self.net.accounts.catalogs.len(),
            about: &facts.about,
            exit_message: self.settings_message.as_deref(),
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
            if now != page {
                // A new page of settings: clean, without a flash.
                self.ui.request_clean_refresh();
            }
            if let Screen::Settings { page: p, .. } = &mut self.screen {
                *p = now;
            }
            return Ok(Flow::Continue);
        }
        match id {
            a::BACK => return self.show_library(Refresh::Full).map(|_| Flow::Continue),
            a::RETURN_KINDLE => {
                self.exit_target = launcher::Target::Kindle;
                return Ok(Flow::Quit);
            }
            a::OPEN_KUAL => {
                if self.kual_available {
                    self.exit_target = launcher::Target::Kual;
                    return Ok(Flow::Quit);
                }
                self.settings_message = Some(
                    aristotle_i18n::tr(aristotle_i18n::Msg::SettingsKualUnavailable).to_string(),
                );
            }
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
            a::SYNC => {
                return self
                    .open_network(catalogs::NetView::Sync, page)
                    .map(|_| Flow::Continue)
            }
            a::CATALOGS => {
                return self
                    .open_network(catalogs::NetView::Catalogs, page)
                    .map(|_| Flow::Continue)
            }
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
                    self.settings_message = Some(
                        aristotle_i18n::tr(aristotle_i18n::Msg::SettingsUsbFailed).to_string(),
                    );
                } else {
                    self.settings_message = None;
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
                } else if let Some(i) = in_range(id, a::SLEEP_SCREEN, 2) {
                    self.prefs.sleep_screen = if i == 0 {
                        prefs::SleepScreen::LastScreen
                    } else {
                        prefs::SleepScreen::BookCover
                    };
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
        // Asleep, or powerd was asked and has a few seconds to answer.
        if self.asleep.is_some() || self.sleep_ctl.sleep_pending() {
            return None;
        }
        self.prefs
            .sleep()
            .map(|m| self.last_touch + Duration::from_secs(m as u64 * 60))
    }

    /// Save progress and switch the light off; powerd controls CPU suspend.
    fn fall_asleep(&mut self, by_powerd: bool) -> Result<(), Error> {
        // Finish an opening without repainting; cancel work without a snapshot.
        self.finish_opening(true)?;
        self.cancel_pending();
        self.prewarm.input();
        self.ui.reset_gestures();
        if let Screen::Reader(r) = &mut self.screen {
            r.gestures.reset();
        }
        if let Screen::Reader(r) = &self.screen {
            let now = aristotle_library::library::now();
            self.lib
                .set_progress(&r.session.id, r.session.progress(now));
        }
        self.sync_push(false, false);
        self.save_snapshot();
        self.saver.save(&self.lib);
        self.debounce.saved();
        self.asleep = Some(self.light_step());
        self.set_light_step(0);
        self.sleep_ctl.asleep(by_powerd);
        if self.prefs.sleep_screen == prefs::SleepScreen::LastScreen {
            perf!(
                "sleep: last screen ({})",
                if by_powerd { "powerd" } else { "own" }
            );
            return Ok(());
        }
        let photos = self.prefs.covers.mode() == CoverMode::Photos;
        let book = shelf::current_book(&self.lib)
            .cloned()
            .map(|rec| shelf::screen_book(&self.lib, &rec, &mut self.covers, photos));
        let tree = sleep::build(&self.styles, book.as_ref(), photos);
        self.ui.set_tree(tree);
        perf!(
            "sleep: screensaver ({})",
            if by_powerd { "powerd" } else { "own" }
        );
        self.submit_ui(Content::Chrome)
    }

    /// powerd left its screensaver, or a touch woke ours: the light and
    /// the screen come back.
    fn wake(&mut self, by_powerd: bool) -> Result<(), Error> {
        self.ui.reset_gestures();
        if let Some(step) = self.asleep.take() {
            self.set_light_step(step);
        }
        self.last_touch = Instant::now();
        self.sleep_ctl.awake(by_powerd, self.last_touch);
        self.prewarm.input();
        perf!(
            "sleep: woken ({})",
            if by_powerd { "powerd" } else { "touch" }
        );
        match &self.screen {
            Screen::Library { .. } => self.show_library(Refresh::Full),
            Screen::Settings { page, .. } => {
                let page = *page;
                self.show_settings(Refresh::Full, page)
            }
            Screen::Network(_) => self.show_network(Refresh::Full),
            // (A book still opening was finished before the screensaver.)
            Screen::Reader(_) | Screen::Opening => self.reader_after_sleep(),
        }
    }

    // --- Event loop -------------------------------------------------------

    fn poll_timeout(&self, now_us: u64, now: Instant) -> Duration {
        let mut timeout = Duration::from_secs(1);
        if let Some(t) = self.ui.next_deadline(now_us) {
            timeout = timeout.min(Duration::from_micros(t.saturating_sub(now_us).max(1000)));
        }
        if let Some(d) = self.debounce.deadline() {
            timeout = timeout.min(d.saturating_duration_since(now));
        }
        if let Some(d) = self.sleep_deadline() {
            timeout = timeout.min(d.saturating_duration_since(now));
        }
        if self.asleep.is_none() {
            if let Screen::Reader(r) = &self.screen {
                if r.session.paginating() {
                    timeout = timeout.min(Duration::from_millis(250));
                }
                if let Some(t) = r.gestures.next_deadline(now_us) {
                    timeout = timeout.min(Duration::from_micros(t.max(1000)));
                }
                if matches!(r.overlay, Overlay::Waiting { .. }) {
                    timeout = timeout.min(Duration::from_millis(20));
                }
            }
        }
        if self.opening.is_some() || self.pending.is_some() {
            timeout = timeout.min(Duration::from_millis(10));
        }
        if self.net.busy() {
            timeout = timeout.min(Duration::from_millis(100));
        }
        if let Some(d) = self.prewarm.deadline().filter(|_| self.asleep.is_none()) {
            timeout = timeout.min(d.saturating_duration_since(now));
        }
        if let Some(d) = self.sleep_ctl.deadline(self.asleep.is_some()) {
            timeout = timeout.min(d.saturating_duration_since(now));
        }
        timeout
    }

    fn run(&mut self, touch: &mut TouchScreen) -> Result<(), Error> {
        loop {
            if STOP.load(Ordering::Relaxed) {
                return Ok(());
            }
            let timeout = self.poll_timeout(monotonic_us(), Instant::now());
            let (events, _) =
                match touch.poll_with(timeout.max(Duration::from_millis(1)), self.sleep_ctl.fd()) {
                    Ok(e) => e,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e.into()),
                };
            for ev in &events {
                if self.on_touch(ev)? == Flow::Quit {
                    return Ok(());
                }
            }
            self.on_power()?;
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
            let now = Instant::now();
            if self.sleep_deadline().is_some_and(|d| now >= d) && self.sleep_ctl.request_sleep(now)
            {
                self.fall_asleep(false)?;
            }
            match self.sleep_ctl.tick(Instant::now(), self.asleep.is_some()) {
                powerd::Action::Sleep if self.asleep.is_none() => self.fall_asleep(false)?,
                powerd::Action::Wake if self.asleep.is_some() => self.wake(false)?,
                _ => {}
            }
        }
    }

    /// powerd's events: its screensaver, waking, suspending.
    fn on_power(&mut self) -> Result<(), Error> {
        use aristotle_hal::powerd::PowerEvent as P;
        for ev in self.sleep_ctl.events() {
            match ev {
                P::GoingToScreenSaver(src) => {
                    perf!("power: goingToScreenSaver {src}");
                    if self.asleep.is_none() {
                        self.fall_asleep(true)?;
                    } else {
                        self.sleep_ctl.powerd_took_over();
                    }
                }
                P::OutOfScreenSaver(src) => {
                    perf!("power: outOfScreenSaver {src}");
                    if self.asleep.is_some() {
                        self.wake(true)?;
                        self.ui.reset_gestures();
                        if let Screen::Reader(r) = &mut self.screen {
                            r.gestures.reset();
                        }
                    }
                }
                P::ReadyToSuspend(delay) => {
                    perf!("power: readyToSuspend {delay}");
                    if self.asleep.is_none() {
                        self.sleep_ctl.resync();
                    }
                }
                other => perf!("power: {other:?}"),
            }
        }
        Ok(())
    }

    /// Whether the UI tree (not the page-turn path) owns input right now.
    fn ui_driven(&self) -> bool {
        match &self.screen {
            Screen::Library { .. } | Screen::Settings { .. } | Screen::Network(_) => true,
            Screen::Reader(r) => r.menu_open || !r.overlay.is_none(),
            Screen::Opening => self.opening.as_ref().is_some_and(|o| o.menu_open),
        }
    }

    /// Work done between events: adopting page numbers, saving progress,
    /// the clock in the status line.
    fn background(&mut self) -> Result<(), Error> {
        if self.asleep.is_some() {
            self.poll_net()?;
            return Ok(());
        }
        if let Some(result) = self.opening.as_ref().and_then(|o| o.worker.poll()) {
            self.adopt(result, false)?;
        }
        if let Some(result) = self.pending.as_ref().and_then(|p| p.worker.poll()) {
            self.finish_pending(result)?;
        }
        self.poll_lookup()?;
        self.poll_net()?;
        // Snapshots ahead of time: only on the library screen (not the
        // settings), with no book opening and no screensaver.
        if matches!(self.screen, Screen::Library { .. })
            && self.pending.is_none()
            && self.asleep.is_none()
        {
            let style = self.page_style();
            self.prewarm.tick(
                &self.lib,
                &style,
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
                    Screen::Network(_) => self.show_network(Refresh::Auto)?,
                    Screen::Reader(_) | Screen::Opening => self.clock = clock,
                }
            }
        }
        Ok(())
    }

    fn on_touch(&mut self, ev: &aristotle_hal::input::TouchEvent) -> Result<Flow, Error> {
        if self.asleep.is_some() {
            // Any touch wakes (through powerd when it is powerd's
            // screensaver); the rest of that touch does nothing.
            if ev.phase == HalPhase::Down {
                if self.sleep_ctl.touch_asleep(Instant::now()) {
                    self.wake(false)?;
                }
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
        self.sleep_ctl.touched();
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
            Screen::Network(_) => self.on_network_event(ev),
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
                        // A new page of the list is a new screen of text:
                        // one clean full-screen mode-9 update, like a book
                        // page. Cards and cover photos (large grays) need
                        // the flash, as on the stock home screen.
                        if self.prefs.covers.mode() != CoverMode::None {
                            self.ui.request_full_refresh();
                        } else {
                            self.ui.request_clean_refresh();
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
        self.sync_before_open(id);
        let t0 = Instant::now();
        let style = self.page_style();
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
        let gestures = TurnGestures::with_density(self.ui.bounds(), self.ui.profile().density);
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
            gestures: TurnGestures::with_density(self.ui.bounds(), self.ui.profile().density),
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
                self.rebuild_library_at(Refresh::Full, 0, deferred)?;
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
            (_, true, _) => {}
            (true, false, true) => self.render_ui()?,
            // The UI's copy of the screen already has these pixels: no
            // update at all.
            (true, false, false) => {}
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
        self.sync_book_opened();
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
                    menu_action::FONT_LIST => self.open_fonts(),
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
        self.relayout(settings)
    }

    /// Lay the book out again in the style of the preferences and show it
    /// with the menu open: one full mode-9 update.
    pub(crate) fn relayout(&mut self, settings: Settings) -> Result<(), Error> {
        let style = self.page_style();
        let Screen::Reader(r) = &mut self.screen else {
            return Ok(());
        };
        let t = Instant::now();
        r.session.restyle(style)?;
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
            "restyle: {} {} pt, line height {}, margin {} mm: layout and page {:.0} ms",
            r.session.style.font_family,
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
        self.sync_push(true, false);
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
        app.sync_push(true, true);
        if let Some(step) = app.asleep.take() {
            app.set_light_step(step);
        }
        app.sleep_ctl.restore();
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
        // The reading font's row opens the list of fonts (fonts.rs).
        families: None,
        font_list: true,
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
    let top =
        ui.tree.rect(menu.top).bottom() - menu.top_extra + ui.profile().density.px(overlay::NAV_H);
    let sheet = ui.tree.rect(menu.sheet).y + menu.bottom_extra;
    let extents = mask_extents(
        &page.data,
        page.stride,
        page.width as usize,
        page.height as usize,
        top,
        sheet,
        ui.profile().density.px(120),
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
    if let Some(l) = r.menu.font_label {
        ui.tree.set_text(l, &r.session.style.font_family);
    }
    if let Some(w) = ui.tree.widget_mut::<Steps>(r.menu.size_steps) {
        w.value = r.settings.size;
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
        memory_app_sized(library, root, 1072, 1448)
    }

    fn memory_app_sized(library: &Path, root: &Path, w: u32, h: u32) -> App {
        memory_app_profile(library, root, ScreenProfile::new(w, h, 300).unwrap())
    }

    fn memory_app_profile(library: &Path, root: &Path, profile: ScreenProfile) -> App {
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
            preview_size: None,
            preview_density: profile.density,
            open: None,
            index_dicts: false,
            test_clippings: None,
            lang: None,
            bundle: library.to_path_buf(),
            accounts: library.join("accounts"),
            fonts: library.join("fonts"),
        };
        let panel = Panel::memory(profile.width, profile.height, prefs::STOCK_TURN);
        build_app(
            &args,
            panel,
            Prefs::default(),
            start_dictionaries(&args),
            powerd::Sleep::none(),
        )
        .unwrap()
    }

    #[test]
    fn settings_can_leave_without_an_agent_and_report_unavailable_kual() {
        let library = testdir("settings-exit");
        let root = library.join("documents");
        std::fs::create_dir_all(&root).unwrap();
        let mut app = memory_app(&library, &root);
        app.show_settings(Refresh::Full, 0).unwrap();
        let event = |app: &App, action| {
            let node = (0..app.ui.tree.len())
                .map(|i| NodeId(i as u32))
                .find(|id| app.ui.tree.node(*id).action == Some(action))
                .unwrap();
            UiEvent::Action {
                id: action,
                node,
                at: aristotle_ui::geom::Point { x: 200, y: 240 },
            }
        };
        let kual = event(&app, set_ui::action::OPEN_KUAL);
        assert!(app.on_settings_event(kual).unwrap() == Flow::Continue);
        assert!(app.settings_message.is_some());
        assert!(app.exit_target == launcher::Target::Kindle);
        app.kual_available = true;
        let kual = event(&app, set_ui::action::OPEN_KUAL);
        assert!(app.on_settings_event(kual).unwrap() == Flow::Quit);
        assert_eq!(app.exit_target.exit_code(), launcher::KUAL_EXIT_CODE);
        let stock = event(&app, set_ui::action::RETURN_KINDLE);
        assert!(app.on_settings_event(stock).unwrap() == Flow::Quit);
        assert_eq!(app.exit_target.exit_code(), 0);
        app.leave().unwrap();
        std::fs::remove_dir_all(&library).unwrap();
    }

    fn updates(app: &App) -> Vec<String> {
        match &app.panel {
            Panel::Memory(m) => m.updates.iter().map(|u| u.label.clone()).collect(),
            Panel::Device(_) => Vec::new(),
        }
    }

    fn pixels(app: &App) -> &[u8] {
        match &app.panel {
            Panel::Memory(panel) => &panel.pixels,
            Panel::Device(_) => panic!("memory panel required"),
        }
    }

    #[test]
    fn last_screen_sleep_preserves_library_and_settings_pixels() {
        for settings in [false, true] {
            for mode in [
                prefs::SleepScreen::LastScreen,
                prefs::SleepScreen::BookCover,
            ] {
                let dir = testdir("sleep-screen");
                let mut app = memory_app(&dir.join("library"), &dir.join("books"));
                app.show_library(Refresh::Full).unwrap();
                if settings {
                    app.show_settings(Refresh::Full, 0).unwrap();
                    let action = set_ui::action::SLEEP_SCREEN
                        + u32::from(mode == prefs::SleepScreen::BookCover);
                    let node = (0..app.ui.tree.len())
                        .map(|n| NodeId(n as u32))
                        .find(|&n| app.ui.tree.node(n).action == Some(action))
                        .unwrap();
                    app.on_settings_event(UiEvent::Action {
                        id: action,
                        node,
                        at: aristotle_ui::geom::Point::default(),
                    })
                    .unwrap();
                    assert_eq!(Prefs::load(&dir.join("library")).sleep_screen, mode);
                } else {
                    app.prefs.sleep_screen = mode;
                }
                let held = if settings && mode == prefs::SleepScreen::LastScreen {
                    let night = (0..app.ui.tree.len())
                        .map(|n| NodeId(n as u32))
                        .find(|&n| app.ui.tree.node(n).action == Some(set_ui::action::NIGHT))
                        .unwrap();
                    let at = app.ui.tree.rect(night).center();
                    touch(
                        &mut app,
                        HalPhase::Down,
                        at.x as u32,
                        at.y as u32,
                        monotonic_us(),
                    );
                    Some(at)
                } else {
                    None
                };
                let before = updates(&app).len();
                let shown = pixels(&app).to_vec();
                app.fall_asleep(true).unwrap();
                assert!(app.asleep.is_some());
                assert!(app.debounce.deadline().is_none());
                if mode == prefs::SleepScreen::LastScreen {
                    assert_eq!(updates(&app).len(), before);
                    assert_eq!(pixels(&app), shown);
                } else {
                    assert_eq!(updates(&app)[before..], ["flash"]);
                    assert_ne!(pixels(&app), shown);
                }
                let asleep = updates(&app).len();
                app.background().unwrap();
                assert_eq!(updates(&app).len(), asleep);
                app.wake(true).unwrap();
                if let Some(at) = held {
                    touch(
                        &mut app,
                        HalPhase::Up,
                        at.x as u32,
                        at.y as u32,
                        monotonic_us(),
                    );
                    assert!(!app.prefs.night, "a pre-sleep press must not fire on wake");
                }
                assert!(app.asleep.is_none());
                assert!(updates(&app).len() > asleep);
                assert_eq!(matches!(app.screen, Screen::Settings { .. }), settings);
            }
        }
    }

    #[test]
    fn last_screen_sleep_finishes_snapshot_open_without_updates_and_saves_progress() {
        let Some((library, root, id)) = reopen_fixture(true) else {
            return;
        };
        let mut app = memory_app(&library, &root);
        app.open_by_title("Reopened").unwrap();
        assert!(matches!(app.screen, Screen::Opening));
        app.opening.as_mut().unwrap().menu_open = true;
        app.sync.before_open = Some((id.clone(), None));
        let before = updates(&app).len();
        let shown = pixels(&app).to_vec();
        app.fall_asleep(true).unwrap();
        assert!(app.opening.is_none());
        assert!(app.pending.is_none());
        assert!(app.asleep.is_some());
        assert_eq!(updates(&app).len(), before);
        assert_eq!(pixels(&app), shown);
        assert_eq!(app.sync.before_open.as_ref().map(|p| &p.0), Some(&id));
        let Screen::Reader(reader) = &app.screen else {
            panic!("reader adopted");
        };
        let locator = reader
            .session
            .progress(aristotle_library::library::now())
            .locator;
        app.wake(true).unwrap();
        let Screen::Reader(reader) = &app.screen else {
            panic!("reader restored");
        };
        assert!(!reader.menu_open);
        assert!(reader.overlay.is_none());
        app.saver.flush();
        let saved = Library::open(&library).unwrap();
        assert_eq!(
            saved.book(&id).unwrap().progress.as_ref().unwrap().locator,
            locator
        );
    }

    #[test]
    fn failed_snapshot_open_defers_library_repaint_until_wake() {
        let Some((library, root, _)) = reopen_fixture(true) else {
            return;
        };
        let mut app = memory_app(&library, &root);
        app.open_by_title("Reopened").unwrap();
        let before = updates(&app).len();
        let shown = pixels(&app).to_vec();
        app.adopt(Err(Error::msg("synthetic failed open")), true)
            .unwrap();
        app.fall_asleep(true).unwrap();
        assert_eq!(updates(&app).len(), before);
        assert_eq!(pixels(&app), shown);
        assert!(matches!(app.screen, Screen::Library { .. }));
        app.wake(true).unwrap();
        assert!(updates(&app).len() > before);
        assert!(matches!(app.screen, Screen::Library { .. }));
    }

    #[test]
    fn network_completions_update_state_without_repainting_or_retaining_busy_while_asleep() {
        for mode in [
            prefs::SleepScreen::LastScreen,
            prefs::SleepScreen::BookCover,
        ] {
            let dir = testdir("sleep-network");
            let mut app = memory_app(&dir.join("library"), &dir.join("books"));
            app.open_network(catalogs::NetView::Sync, 0).unwrap();
            app.prefs.sleep_screen = mode;
            app.fall_asleep(true).unwrap();
            let before = updates(&app).len();
            let shown = pixels(&app).to_vec();
            app.sync.checking = true;
            app.net.spawn("sleep-test", |_| net::Done::Login {
                register: false,
                result: Err(aristotle_net::kosync::Error::Unauthorized),
            });
            wait(&mut app, |app| !app.net.busy());
            assert!(!app.sync.checking);
            assert!(!app.sync.message.is_empty());
            assert!(app.asleep.is_some());
            assert_eq!(updates(&app).len(), before);
            assert_eq!(pixels(&app), shown);
            app.wake(true).unwrap();
            assert!(updates(&app).len() > before);
            assert!(matches!(app.screen, Screen::Network(_)));
        }
    }

    #[test]
    fn asleep_reader_workers_do_not_shorten_polling_but_network_completions_do() {
        let Some((library, root, id)) = reopen_fixture(true) else {
            return;
        };
        let mut app = memory_app(&library, &root);
        app.open_by_title("Reopened").unwrap();
        wait(&mut app, |app| matches!(app.screen, Screen::Reader(_)));
        let mut source =
            Source::new(&app.lib, app.lib.book(&id).unwrap(), &app.mobi_cache).unwrap();
        source.data_dir = testdir("sleep-paginator");
        let prepared = Session::prepare(source, app.prefs.style(), false).unwrap();
        let Screen::Reader(reader) = &mut app.screen else {
            panic!("reader required");
        };
        reader.session = Session::from_prepared(prepared).unwrap();
        assert!(reader.session.paginating());

        for waiting in [false, true] {
            for by_powerd in [false, true] {
                let Screen::Reader(reader) = &mut app.screen else {
                    panic!("reader required");
                };
                reader.overlay = if waiting {
                    Overlay::Waiting {
                        sel: selection::Selection {
                            anchor: (0, 0),
                            start: 0,
                            end: 0,
                        },
                        words: Rc::new(Vec::new()),
                    }
                } else {
                    Overlay::None
                };
                let now_us = monotonic_us();
                let now = Instant::now();
                app.prewarm.input();
                app.debounce.saved();
                app.last_touch = now;
                let awake = if waiting { 20 } else { 250 };
                assert_eq!(app.poll_timeout(now_us, now), Duration::from_millis(awake));

                app.fall_asleep(by_powerd).unwrap();
                assert_eq!(app.poll_timeout(now_us, now), Duration::from_secs(1));
                let (release, blocked) = std::sync::mpsc::channel();
                app.net.spawn("sleep-timeout", move |_| {
                    blocked.recv().unwrap();
                    net::Done::Login {
                        register: false,
                        result: Ok(()),
                    }
                });
                assert_eq!(app.poll_timeout(now_us, now), Duration::from_millis(100));
                release.send(()).unwrap();
                wait(&mut app, |app| !app.net.busy());
                assert_eq!(app.poll_timeout(now_us, now), Duration::from_secs(1));
                let Screen::Reader(reader) = &app.screen else {
                    panic!("reader retained");
                };
                assert!(reader.session.paginating());
                assert_eq!(matches!(reader.overlay, Overlay::Waiting { .. }), waiting);
                app.wake(by_powerd).unwrap();
            }
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

    /// A library in a fresh directory with the "Reopened" book at 40 %,
    /// its page numbers on disk and a snapshot made ahead of time: before
    /// the page numbers were known (its footer lacks "x / y", so the open
    /// finds [`open::Verdict::Chrome`]), or after (`Same`). Returns the
    /// library, the books' root and the book's id.
    fn reopen_fixture(numbers_first: bool) -> Option<(PathBuf, PathBuf, String)> {
        let Some(data) = render_data() else {
            assert!(
                std::env::var_os("CI").is_none(),
                "no render data directory in CI"
            );
            eprintln!("skipped: no render data directory (run tools/build-render.sh host)");
            return None;
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
        let mut app = memory_app(&library, &root);
        let style = app.prefs.style();
        // The page numbers, as an earlier read leaves them.
        let paginate = || {
            let stamp = snapshot::file_stamp(&book).unwrap();
            let mut b = aristotle_render::book::Book::open(&book, &style, None).unwrap();
            b.paginate();
            let cache = pagecache::cache_path(&library, &id, stamp.0, stamp.1, &style);
            pagecache::store(&cache, b.pagination().unwrap()).unwrap();
        };
        if numbers_first {
            paginate();
        }
        let snap = snapshot::path(&library, &id);
        std::thread::sleep(prewarm::QUIET + Duration::from_millis(100));
        wait(&mut app, |_| snap.exists());
        // Let the job finish and be reaped.
        wait(&mut app, |a| a.prewarm.deadline().is_none());
        if !numbers_first {
            paginate();
        }
        // What the open will find.
        let rec = app.lib.book(&id).unwrap().clone();
        let mut src = Source::new(&app.lib, &rec, &app.mobi_cache).unwrap();
        src.secs_per_page = app.speed.secs_per_page();
        let shown = open::find_snapshot(&src, &style).expect("a valid snapshot");
        src.exact = Some(shown.info.location);
        let mut s =
            Session::from_prepared(Session::prepare(src, style.clone(), false).unwrap()).unwrap();
        let real = s.page(s.current).unwrap();
        let expected = if numbers_first {
            open::Verdict::Same
        } else {
            open::Verdict::Chrome
        };
        assert_eq!(open::compare(&shown.page, &real, &style), expected);
        app.leave().unwrap();
        Some((library, root, id))
    }

    /// A copy of `library`: the fixture stays as it is for the next run.
    fn copy_library(library: &Path) -> PathBuf {
        fn copy(from: &Path, to: &Path) {
            std::fs::create_dir_all(to).unwrap();
            for e in std::fs::read_dir(from).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    copy(&e.path(), &to.join(e.file_name()));
                } else {
                    std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
                }
            }
        }
        let to = testdir("library");
        copy(library, &to);
        to
    }

    #[test]
    fn reading_and_relayout_use_the_panel_size_and_stride() {
        let Some(data) = render_data() else {
            assert!(
                std::env::var_os("CI").is_none(),
                "no render data directory in CI"
            );
            eprintln!("skipped: no render data directory (run tools/build-render.sh host)");
            return;
        };
        aristotle_render::use_data_dir(&data).unwrap();
        let dir = testdir("screen-sizes");
        let root = dir.join("books");
        let library = dir.join("library");
        std::fs::create_dir_all(&root).unwrap();
        write_book(&root.join("reopened.epub"));
        let mut lib = Library::open(&library).unwrap();
        lib.scan(&root, &ScanOptions::default()).unwrap();
        lib.save().unwrap();
        for (w, h, dpi) in [
            (800, 1200, 300),
            (1072, 1448, 300),
            (1236, 1648, 300),
            (600, 800, 167),
            (758, 1024, 212),
        ] {
            let profile = ScreenProfile::new(w, h, dpi).unwrap();
            let mut app = memory_app_profile(&library, &root, profile);
            app.open_by_title("Reopened").unwrap();
            wait(&mut app, |a| matches!(a.screen, Screen::Reader(_)));
            for size in [3, 2] {
                app.prefs.size = size;
                let settings = app.prefs.menu_settings(0.5);
                app.relayout(settings).unwrap();
                let stride = app.panel.stride();
                let Screen::Reader(r) = &mut app.screen else {
                    panic!("no book open")
                };
                assert_eq!((r.session.style.width, r.session.style.height), (w, h));
                assert_eq!(r.session.style.stride, stride);
                assert_eq!(r.session.style.dpi, dpi);
                assert_eq!(r.session.style.text_top, profile.density.px(104) as u32);
                let page = r.session.page(r.session.current).unwrap();
                assert_eq!((page.width, page.height, page.stride), (w, h, stride));
                let bounds = Rect::new(0, 0, w as i32, h as i32);
                if let Panel::Memory(m) = &app.panel {
                    assert!(m
                        .updates
                        .iter()
                        .all(|u| u.rect.intersect(&bounds) == Some(u.rect)));
                }
            }
            app.leave().unwrap();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Reopening from a snapshot takes one update, even with an outdated footer.
    #[test]
    fn reopening_from_a_snapshot_is_one_update() {
        // A snapshot made before the page numbers were known (Chrome), and
        // one made with them known (Same).
        for numbers_first in [false, true] {
            let Some((library, root, _)) = reopen_fixture(numbers_first) else {
                return;
            };
            let mut app = memory_app(&library, &root);
            assert_eq!(reopen(&mut app, "Reopened"), ["flash"]);
            app.leave().unwrap();
        }
    }

    #[test]
    fn imported_annotation_opens_read_only_without_changing_the_reading_position() {
        use aristotle_library::index::{Annotation, AnnotationKind};
        fn reader(app: &App) -> &Reading {
            match &app.screen {
                Screen::Reader(r) => r,
                _ => panic!("not reading"),
            }
        }
        let Some((library, root, id)) = reopen_fixture(false) else {
            return;
        };
        let mut app = memory_app(&library, &root);
        reopen(&mut app, "Reopened");
        let imported = Annotation {
            kind: AnnotationKind::Highlight,
            text: Some("A passage written for this test.".into()),
            page: Some("xii".into()),
            location: Some((10, 11)),
            position: None,
            created: Some("2026-01-02 03:04:05".into()),
            source: "kindle-clippings".into(),
            chapter: None,
            note: Some("An imported note.".into()),
        };
        app.lib.add_annotation(&id, imported.clone());
        let before = reader(&app).session.current;
        app.open_marks().unwrap();
        let Overlay::Marks { rows, .. } = &reader(&app).overlay else {
            panic!("no annotation list");
        };
        assert_eq!(rows.len(), 1);
        assert!(rows[0].target.is_none());
        app.on_overlay_event(UiEvent::Action {
            id: overlay::act::ITEM,
            node: NodeId(0),
            at: aristotle_ui::geom::Point { x: 0, y: 0 },
        })
        .unwrap();
        let Overlay::Annotation { text, target, .. } = &reader(&app).overlay else {
            panic!("no annotation detail");
        };
        assert!(text.text().contains("A passage written for this test."));
        assert!(text.text().contains("An imported note."));
        assert!(text.text().contains("xii"));
        assert!(target.is_none());
        assert_eq!(reader(&app).session.current, before);
        assert_eq!(app.lib.book(&id).unwrap().annotations, [imported]);
        app.leave().unwrap();
    }

    #[test]
    fn a_pull_from_a_previous_open_of_the_same_book_is_ignored() {
        let Some((library, root, id)) = reopen_fixture(false) else {
            return;
        };
        let mut app = memory_app(&library, &root);
        reopen(&mut app, "Reopened");
        let old_token = app.sync.opened.as_ref().unwrap().token;
        app.back_to_library().unwrap();
        app.open_by_title("Reopened").unwrap();
        wait(&mut app, |a| matches!(a.screen, Screen::Reader(_)));
        let current_token = app.sync.opened.as_ref().unwrap().token;
        assert_ne!(old_token, current_token);
        app.sync.message = "current status".into();
        app.on_pulled(
            id.clone(),
            old_token,
            Err(aristotle_net::kosync::Error::Invalid("stale")),
        )
        .unwrap();
        assert_eq!(app.sync.message, "current status");
        assert!(app.sync.offer.is_none());
        app.on_pulled(
            id,
            current_token,
            Err(aristotle_net::kosync::Error::Invalid("current")),
        )
        .unwrap();
        assert_ne!(app.sync.message, "current status");
        app.leave().unwrap();
    }

    #[test]
    fn a_failed_screensaver_push_retries_without_more_reading() {
        use crate::sync::{PushOutcome, PushReceipt};
        let Some((library, root, id)) = reopen_fixture(false) else {
            return;
        };
        let mut app = memory_app(&library, &root);
        reopen(&mut app, "Reopened");
        let token = app.sync.opened.as_ref().unwrap().token;
        let baseline = app.sync.opened.as_ref().unwrap().at_open.clone();
        let moved = aristotle_library::Locator::Epub {
            spine: 0,
            offset: 1,
        };
        assert_ne!(baseline.as_ref(), Some(&moved));
        let mut p = app.lib.book(&id).unwrap().progress.clone().unwrap();
        p.locator = moved.clone();
        p.fraction = 0.1;
        app.lib.set_progress(&id, p);
        app.net.accounts.sync = Some(aristotle_net::store::SyncSettings {
            server: "not a URL".into(),
            username: "test".into(),
            key: "test".into(),
            auto: true,
        });
        for _ in 0..2 {
            app.sync_push(false, false);
            assert!(app.net.busy(), "unchanged progress was not retried");
            wait(&mut app, |a| !a.net.busy());
            assert_eq!(app.sync.opened.as_ref().unwrap().at_open, baseline);
        }
        let receipt = |token, sequence, locator| PushReceipt {
            book: id.clone(),
            token,
            sequence,
            locator,
        };
        app.on_pushed(
            receipt(token, 10, moved.clone()),
            PushOutcome::Finished(Ok(())),
        );
        assert_eq!(
            app.sync.opened.as_ref().unwrap().at_open.as_ref(),
            Some(&moved)
        );
        let stale = aristotle_library::Locator::Epub {
            spine: 0,
            offset: 0,
        };
        app.on_pushed(
            receipt(token, 9, stale.clone()),
            PushOutcome::Finished(Ok(())),
        );
        app.on_pushed(
            receipt(token.wrapping_sub(1), 11, stale),
            PushOutcome::Finished(Ok(())),
        );
        assert_eq!(
            app.sync.opened.as_ref().unwrap().at_open.as_ref(),
            Some(&moved)
        );
        app.net.accounts.sync = None;
        app.leave().unwrap();
    }

    fn touch(app: &mut App, phase: HalPhase, x: u32, y: u32, time_us: u64) {
        let ev = aristotle_hal::input::TouchEvent {
            phase,
            id: 0,
            x,
            y,
            raw_x: x as i32,
            raw_y: y as i32,
            time_us,
        };
        app.on_touch(&ev).unwrap();
    }

    /// A 60 ms tap at (`x`, `y`).
    fn tap(app: &mut App, x: u32, y: u32) {
        let t = monotonic_us();
        touch(app, HalPhase::Down, x, y, t);
        touch(app, HalPhase::Up, x, y, t + 60_000);
    }

    /// A finger resting 600 ms at (`x`, `y`), then lifted.
    fn hold(app: &mut App, x: u32, y: u32) {
        let t = monotonic_us();
        touch(app, HalPhase::Down, x, y, t);
        app.tick_reading(t + 600_000).unwrap();
        touch(app, HalPhase::Up, x, y, t + 700_000);
    }

    /// What one step of input left: the panel's updates, its pixels after
    /// a full-screen update, the page, the menu and the overlay.
    #[derive(Debug, PartialEq)]
    struct After {
        updates: Vec<String>,
        pixels: Option<u64>,
        location: usize,
        menu_open: bool,
        overlay: String,
        bookmarked: bool,
    }

    fn after(app: &mut App, before: usize) -> After {
        use std::hash::{Hash, Hasher};
        let updates = updates(app)[before..].to_vec();
        let pixels = match &app.panel {
            Panel::Memory(m) if m.updates[before..].iter().any(|u| u.full_screen) => {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                m.pixels.hash(&mut h);
                Some(h.finish())
            }
            _ => None,
        };
        let Screen::Reader(r) = &mut app.screen else {
            panic!("not reading");
        };
        let overlay = match r.overlay {
            Overlay::None => "none",
            Overlay::Waiting { .. } => "waiting",
            Overlay::Dict { .. } => "dict",
            Overlay::Dragging { .. } => "dragging",
            Overlay::Actions { .. } => "actions",
            Overlay::Gloss { .. } => "gloss",
            Overlay::Note { .. } => "note",
            Overlay::Toc { .. } => "toc",
            Overlay::Marks { .. } => "marks",
            Overlay::Annotation { .. } => "annotation",
            Overlay::Fonts { .. } => "fonts",
            Overlay::GoTo { .. } => "go to",
            Overlay::Sync { .. } => "sync",
        }
        .to_string();
        let location = r.session.current;
        After {
            updates,
            pixels,
            location,
            menu_open: r.menu_open,
            overlay,
            bookmarked: r.session.bookmarked(location).is_some(),
        }
    }

    type Step<'a> = &'a dyn Fn(&mut App);

    /// Reopen a copy of `library` and run `steps` on it, recording each.
    /// Returns the page reopened and what each step left.
    fn reopen_and_run(library: &Path, root: &Path, steps: &[Step]) -> (usize, Vec<After>) {
        let library = copy_library(library);
        let mut app = memory_app(&library, root);
        assert_eq!(reopen(&mut app, "Reopened"), ["flash"]);
        let Screen::Reader(r) = &app.screen else {
            panic!("not reading");
        };
        let start = r.session.current;
        let mut out = Vec::new();
        for step in steps {
            let before = updates(&app).len();
            step(&mut app);
            out.push(after(&mut app, before));
        }
        app.leave().unwrap();
        (start, out)
    }

    /// The centre of a word in the middle of the page.
    fn some_word(app: &mut App) -> (u32, u32) {
        let Screen::Reader(r) = &mut app.screen else {
            panic!("not reading");
        };
        let words = r.session.words(r.session.current);
        let w = &words[words.len() / 2];
        (
            ((w.rect.x0 + w.rect.x1) / 2) as u32,
            ((w.rect.y0 + w.rect.y1) / 2) as u32,
        )
    }

    /// After a reopen that kept the snapshot's page (its footer out of
    /// date), every input does what it does after a reopen that showed the
    /// real page: taps turn and open the menu by their zones, a hold
    /// selects, the bookmark corner marks the page, night mode switches,
    /// and once a turn or a mark redrew the pages, every page shown has the
    /// same pixels (coming back to the reopened page too).
    #[test]
    fn input_after_a_kept_snapshot_page_is_as_after_an_exact_one() {
        let Some(chrome) = reopen_fixture(false) else {
            return;
        };
        let same = reopen_fixture(true).unwrap();
        let corner = turns::bookmark_corner(Rect::screen());
        let (cx, cy) = ((corner.x + corner.w / 2) as u32, (corner.y + 40) as u32);
        let forward: Step = &|a| tap(a, 950, 800);
        let back: Step = &|a| tap(a, 150, 800);
        let middle: Step = &|a| tap(a, 536, 724);
        let top: Step = &|a| tap(a, 536, 80);
        let mark: Step = &|a| tap(a, cx, cy);
        let select: Step = &|a| {
            let (x, y) = some_word(a);
            hold(a, x, y);
        };
        // The menu's night switch.
        let night: Step = &|a| {
            let ev = UiEvent::Action {
                id: menu_action::NIGHT,
                node: NodeId(0),
                at: aristotle_ui::geom::Point { x: 0, y: 0 },
            };
            a.on_menu_event(ev).unwrap();
        };
        let scripts: [&[Step]; 7] = [
            // The tap seen opening the menu on the device, then back onto
            // the reopened page.
            &[forward, back],
            &[back, forward, forward],
            // The middle of the page turns forward too.
            &[middle, back],
            // The top strip opens the menu; a tap on the page closes it.
            &[top, middle, forward],
            &[select],
            &[mark, forward, back, mark],
            &[top, night, middle, forward, back],
        ];
        for (i, script) in scripts.iter().enumerate() {
            let (start_c, mut kept) = reopen_and_run(&chrome.0, &chrome.1, script);
            let (start_s, mut exact) = reopen_and_run(&same.0, &same.1, script);
            assert_eq!(start_c, start_s, "script {i}: the same page reopened");
            // Until the first turn, a full redraw of the reopened page (a
            // popup, night mode) still has the snapshot's header and footer.
            let mut turned = false;
            for (k, e) in kept.iter_mut().zip(&mut exact) {
                turned |= k.location != start_c || k.bookmarked;
                if !turned {
                    k.pixels = None;
                    e.pixels = None;
                }
            }
            assert_eq!(kept, exact, "script {i}");
        }
        // What the first script does: one turn forward, one back.
        let (start, steps) = reopen_and_run(&chrome.0, &chrome.1, scripts[0]);
        assert_eq!(steps[0].updates, ["turn"]);
        assert!(steps[0].location > start && !steps[0].menu_open);
        assert_eq!(steps[1].updates, ["turn"]);
        assert_eq!(steps[1].location, start);
    }

    /// Dragging a selection over several words keeps the page on the
    /// panel, with the selection shaded over it: each step is at most one
    /// update, never a blank page, and the lift settles the screen with one
    /// update that cleans what the drag's A2 steps left.
    #[test]
    fn dragging_a_selection_keeps_the_page() {
        let Some((library, root, _)) = reopen_fixture(true) else {
            return;
        };
        let library = copy_library(&library);
        let mut app = memory_app(&library, &root);
        assert_eq!(reopen(&mut app, "Reopened"), ["flash"]);
        let Screen::Reader(r) = &mut app.screen else {
            panic!("not reading");
        };
        let words = r.session.words(r.session.current);
        let page = r.session.page(r.session.current).unwrap();
        let ink = |data: &[u8], stride: usize| {
            (0..1448usize)
                .map(|y| {
                    data[y * stride..y * stride + 1072]
                        .iter()
                        .filter(|&&p| p < 0x80)
                        .count()
                })
                .sum::<usize>()
        };
        let page_ink = ink(&page.data, page.stride);
        assert!(page_ink > 1000, "a page with text");
        let centre = |i: usize| {
            let w = &words[i];
            (
                ((w.rect.x0 + w.rect.x1) / 2) as u32,
                ((w.rect.y0 + w.rect.y1) / 2) as u32,
            )
        };
        let first = words.len() / 3;
        let (x, y) = centre(first);
        let t = monotonic_us();
        touch(&mut app, HalPhase::Down, x, y, t);
        app.tick_reading(t + 600_000).unwrap();
        let mut steps = Vec::new();
        for k in 1..=6u64 {
            let before = updates(&app).len();
            let (x, y) = centre(first + k as usize * 2);
            touch(&mut app, HalPhase::Move, x, y, t + 600_000 + k * 60_000);
            app.render_ui().unwrap();
            let Panel::Memory(m) = &app.panel else {
                unreachable!()
            };
            let shown = ink(&m.pixels, m.stride);
            assert!(
                shown * 2 > page_ink,
                "step {k}: the page is on the panel ({shown} of {page_ink} dark pixels)"
            );
            let new: Vec<String> = m.updates[before..]
                .iter()
                .map(|u| format!("{} {}", u.mode, u.label))
                .collect();
            assert!(new.len() <= 1, "step {k}: one update at most: {new:?}");
            steps.push(new);
        }
        let before = updates(&app).len();
        let (x, y) = centre(first + 12);
        touch(&mut app, HalPhase::Up, x, y, t + 1_100_000);
        let Panel::Memory(m) = &app.panel else {
            unreachable!()
        };
        let last: Vec<(u32, bool)> = m.updates[before..]
            .iter()
            .map(|u| (u.mode, u.full_screen))
            .collect();
        assert_eq!(last, [(9, true)], "the lift: one clean update ({steps:?})");
        assert!(ink(&m.pixels, m.stride) * 2 > page_ink);
        let Screen::Reader(r) = &app.screen else {
            panic!("not reading");
        };
        assert!(matches!(r.overlay, Overlay::Actions { .. }));
        app.leave().unwrap();
    }

    /// Back to the library at once after a reopen that kept the snapshot's
    /// page: the progress is saved as after an exact reopen, and the next
    /// open is again one update, on the same page.
    #[test]
    fn leaving_at_once_after_a_kept_snapshot_page() {
        let Some(chrome) = reopen_fixture(false) else {
            return;
        };
        let same = reopen_fixture(true).unwrap();
        let run = |(library, root, id): &(PathBuf, PathBuf, String)| {
            let library = copy_library(library);
            let mut app = memory_app(&library, root);
            assert_eq!(reopen(&mut app, "Reopened"), ["flash"]);
            app.back_to_library().unwrap();
            assert!(matches!(app.screen, Screen::Library { .. }));
            let progress = app.lib.book(id).unwrap().progress.clone().unwrap();
            app.leave().unwrap();
            let mut app = memory_app(&library, root);
            assert_eq!(reopen(&mut app, "Reopened"), ["flash"]);
            let Screen::Reader(r) = &app.screen else {
                panic!("not reading");
            };
            let at = r.session.current;
            app.leave().unwrap();
            (progress.locator, progress.fraction, at)
        };
        assert_eq!(run(&chrome), run(&same));
    }
}
