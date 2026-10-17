//! Opening a book without making the reader wait for its layout.
//!
//! 1. If the book has a matching [snapshot](crate::snapshot), its page goes
//!    on the panel at once (a screen switch: mode 8, full) and the book is
//!    opened, placed and rendered on a worker thread ([`spawn`]), with the
//!    pages on either side.
//! 2. Until the worker is done ([`Opening`]), the reader can use what the
//!    snapshot holds: a forward turn shows its next page and a back turn
//!    returns from there (normal mode-9 turns), the menu opens with the
//!    labels stored with the page, and "back to the library" abandons the
//!    worker (its thread drops to idle priority and its result is thrown
//!    away), keeping the page shown as the progress. Anything that needs the
//!    book (a turn beyond the snapshot, a menu setting) waits for the worker.
//! 3. When the worker is done, the main loop adopts the session
//!    ([`reconcile`], [`compare`]). If the book places the saved position on
//!    the page the snapshot showed and renders it to the same pixels,
//!    nothing is sent: the open was one mode-8 flash. Otherwise (another
//!    location, or a header or footer that changed, say page numbers that
//!    were not known when the snapshot was taken) the real page replaces it
//!    with one full mode-9 update, never a partial fix-up.
//!
//! Without a snapshot ([`Pending`]) there is nothing to show early: the
//! tapped card stays inverted, the worker opens the book, and the library
//! keeps taking input; tapping anything else gives the book up. When the
//! worker is done, its page goes out with one mode-8 flash. Snapshots made
//! ahead of time (`prewarm.rs`) keep this case rare.

use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use aristotle_library::{Library, ScanOptions};
use aristotle_render::book::Position;
use aristotle_render::page::Page;
use aristotle_render::plato_core::anyhow::{format_err, Error};
use aristotle_render::plato_core::document::html::layout::HYPHENATION_PATTERNS;
use aristotle_render::style::Style;
use aristotle_ui::screens::library::{self as ui_lib, Filter};
use aristotle_ui::screens::reader::{Menu, Settings};
use aristotle_ui::text::{FamilyId, Fonts, TextEngine};
use aristotle_ui::theme::Styles;
use aristotle_ui::tree::{NodeId, Refresh};
use aristotle_ui::ui::Ui;

use crate::prefs::Prefs;
use crate::session::{Prepared, Session, Source, OPEN_GATE};
use crate::shelf::{Covers, ShelfModel};
use crate::snapshot::PageInfo;
use crate::turns::TurnGestures;
use crate::{ms, progress, snapshot};

// The worker hands its result to the main thread.
const _: fn() = || {
    fn send<T: Send>() {}
    send::<Prepared>();
};

/// A book being opened on a worker thread. Dropping it before the result
/// arrived abandons the work: the thread finishes at idle priority, so a
/// book opened next is not slowed down, and its result is dropped.
pub struct Worker {
    rx: Receiver<Result<Prepared, Error>>,
    pub started: Instant,
    /// The thread's id, once it runs; 0 before.
    tid: Arc<AtomicI32>,
    abandoned: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

/// Open `src` with `style` on a thread, with the pages around the saved
/// position rendered too.
pub fn spawn(src: Source, style: Style) -> Worker {
    spawn_with(src, style, true)
}

/// [`spawn`], rendering the pages on either side only with `neighbours`.
pub fn spawn_with(mut src: Source, style: Style, neighbours: bool) -> Worker {
    let (tx, rx) = mpsc::channel();
    let tid = Arc::new(AtomicI32::new(0));
    let abandoned = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    // Dropping the worker cancels the open between its steps.
    src.cancel = Some(abandoned.clone());
    let work = {
        let (tid, done) = (tid.clone(), done.clone());
        move || {
            // SAFETY: a plain system call.
            tid.store(
                unsafe { libc::syscall(libc::SYS_gettid) } as i32,
                Ordering::SeqCst,
            );
            // One background open at a time: an abandoned one finishes its
            // current step and drops its book first.
            let _gate = OPEN_GATE.lock().unwrap_or_else(|e| e.into_inner());
            let result = Session::prepare(src, style, neighbours);
            done.store(true, Ordering::SeqCst);
            // Nobody waiting any more: the book is dropped here, in the gate.
            let _ = tx.send(result);
        }
    };
    // If the thread can't start, the work (and its sender) is dropped and
    // the worker reports an error.
    if let Err(e) = thread::Builder::new().name("open".into()).spawn(work) {
        eprintln!("aristotle: can't start the open thread: {e}");
    }
    Worker {
        rx,
        started: Instant::now(),
        tid,
        abandoned,
        done,
    }
}

impl Worker {
    /// The result, if the worker is done. The channel only closes without
    /// a result when the thread could not be started (a panic aborts the
    /// whole app: release builds use panic=abort).
    pub fn poll(&self) -> Option<Result<Prepared, Error>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(format_err!("the open thread did not run")))
            }
        }
    }

    /// Block until the worker is done.
    pub fn wait(&self) -> Result<Prepared, Error> {
        self.rx
            .recv()
            .unwrap_or_else(|_| Err(format_err!("the open thread did not run")))
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.abandoned.store(true, Ordering::SeqCst);
        let tid = self.tid.load(Ordering::SeqCst);
        if tid != 0 && !self.done.load(Ordering::SeqCst) {
            // Linux takes a thread id where POSIX says process (see
            // pagecache.rs on why the system call is made directly).
            const SCHED_IDLE: libc::c_int = 5;
            // SAFETY: sched_param is plain data; the pointer is valid.
            unsafe {
                let param: libc::sched_param = std::mem::zeroed();
                libc::syscall(
                    libc::SYS_sched_setscheduler,
                    tid,
                    SCHED_IDLE,
                    &param as *const libc::sched_param,
                );
            }
        }
    }
}

/// Where the reader is once the real session exists, and whether the page
/// on the panel is the right one for it.
///
/// The snapshot showed the page at `snap_current` (and then, if `turned`,
/// the one at `snap_next`). The book placed the saved position at
/// `real_current`, followed by `real_next`.
pub fn reconcile(
    snap_current: usize,
    snap_next: Option<usize>,
    turned: bool,
    real_current: usize,
    real_next: Option<usize>,
) -> (usize, bool) {
    if real_current != snap_current {
        // Never keep a wrong page: show the saved position's real page.
        return (real_current, false);
    }
    if !turned {
        return (real_current, true);
    }
    match (snap_next, real_next) {
        (Some(s), Some(r)) if s == r => (r, true),
        // The reader turned forward from the right page: stay one page on.
        (_, Some(r)) => (r, false),
        (_, None) => (real_current, false),
    }
}

/// How the page on the panel differs from the real one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Same,
    /// Only the header or footer bands differ (say, page numbers that were
    /// not known when the snapshot was taken).
    Chrome,
    /// The text differs.
    Page,
}

pub fn compare(shown: &Page, real: &Page, style: &Style) -> Verdict {
    if shown.data == real.data {
        return Verdict::Same;
    }
    if (shown.width, shown.height, shown.stride) != (real.width, real.height, real.stride) {
        return Verdict::Page;
    }
    let top = style.text_top as usize * shown.stride;
    let bottom = (shown.height as usize).saturating_sub(style.text_bottom as usize) * shown.stride;
    if top <= bottom && shown.data[top..bottom] == real.data[top..bottom] {
        Verdict::Chrome
    } else {
        Verdict::Page
    }
}

/// A book without a snapshot opening on a worker while the library stays
/// on screen, its card inverted.
pub struct Pending {
    pub id: String,
    pub title: String,
    pub worker: Worker,
    pub card: Option<NodeId>,
    pub tapped: Instant,
}

/// A book shown from its snapshot while the worker opens it.
pub struct Opening {
    pub title: String,
    pub id: String,
    pub stamp: (u64, i64),
    pub style: Style,
    pub worker: Worker,
    /// When the book was tapped in the library.
    pub tapped: Instant,
    /// The snapshot's page, and its next page if one was stored.
    pub current: (PageInfo, Rc<Page>),
    pub next: Option<(PageInfo, Rc<Page>)>,
    /// Whether the next page is the one on the panel.
    pub turned: bool,
    pub menu_open: bool,
    pub menu: Menu,
    pub page_node: NodeId,
    pub settings: Settings,
    pub gestures: TurnGestures,
}

impl Opening {
    /// The stored page on the panel.
    pub fn shown(&self) -> &(PageInfo, Rc<Page>) {
        match (&self.next, self.turned) {
            (Some(next), true) => next,
            _ => &self.current,
        }
    }
}

/// The snapshot to show for `src` opened with `style`, if it has a valid
/// one: saved with the progress the index holds now.
pub fn find_snapshot(src: &Source, style: &Style) -> Option<snapshot::Loaded> {
    let locator = src.saved.as_ref()?.locator.clone();
    let key = snapshot::Key::new(&src.id, src.stamp, style, locator);
    snapshot::load(&src.data_dir, &key, style)
}

/// Load what every first open needs and no book owns, at idle priority
/// while the library is on screen: Plato's hyphenation patterns (every
/// file in `hyphenation-patterns/`, deserialised on first use; 5 to 9 ms
/// on the host, so tens of milliseconds on the device).
pub fn warm_up() {
    let _ = thread::Builder::new().name("warm-up".into()).spawn(|| {
        crate::pagecache::lower_priority();
        let t = Instant::now();
        let n = HYPHENATION_PATTERNS.len();
        crate::perf!(
            "warm-up: {n} hyphenation pattern sets loaded in {:.0} ms",
            ms(t.elapsed())
        );
    });
}

/// What a snapshot stores about the page at `location` besides its pixels.
pub fn page_info(session: &mut Session, location: usize) -> PageInfo {
    let here = session.current;
    session.current = location;
    let (chapter, label, fraction) = session.labels();
    let locator = session.progress(0).locator;
    session.current = here;
    PageInfo {
        location,
        locator,
        fraction,
        chapter,
        label,
        last: session.known_last(location),
    }
}

/// `aristotle --library DIR [--root DIR] --bench-open TITLE`: the open path
/// of the first book whose title contains TITLE, without panel or
/// touchscreen. Run it on the host or on the device with the stock GUI up.
///
/// 1. The old path, twice: open, place, render and the reader's first UI
///    frame, all before the submit.
/// 2. Save a snapshot as the app does when progress is saved.
/// 3. The new path, twice: the snapshot read, decoded and drawn into the
///    first UI frame (the submit), then the worker's open and the adoption.
///
/// It writes to the library directory like the app: page numbers, the
/// snapshot, the progress (set to the middle of the book if there is none).
pub fn bench(library: &Path, root: Option<&Path>, title: &str) -> Result<(), Error> {
    let mut lib = Library::open(library)?;
    if let Some(root) = root {
        lib.scan(root, &ScanOptions::default())?;
    }
    let needle = title.to_lowercase();
    let id = lib
        .books()
        .iter()
        .find(|b| b.meta.title.to_lowercase().contains(&needle))
        .map(|b| b.id.clone())
        .ok_or_else(|| format_err!("no book titled like {title:?} in {}", library.display()))?;
    let prefs = Prefs::load(library);
    let style = prefs.style();
    let settings = prefs.menu_settings(0.0);
    let mobi_cache = library.join("epub-cache");
    if lib.book(&id).is_some_and(|b| b.progress.is_none()) {
        let p = progress::progress_at(&Position::Fraction(0.5), 0.5, 0);
        lib.set_progress(&id, p);
    }
    let rec = lib.book(&id).cloned().expect("found above");
    println!("book        {} ({})", rec.meta.title, rec.id);

    let styles = Styles::new(FamilyId(0));
    let blank = Rc::new(Page::white(&style));
    let (tree, _, _) = crate::reader_tree(&styles, &rec.meta.title, &settings, blank);
    let mut ui = Ui::new(
        tree,
        TextEngine::new(Fonts::embedded()),
        style.width,
        style.height,
    );
    ui.render();
    // The reader's first frame, as App::open_book draws it: returns the time.
    let frame = |ui: &mut Ui, page: Rc<Page>| {
        let t = Instant::now();
        let (tree, _, _) = crate::reader_tree(&styles, &rec.meta.title, &settings, page);
        ui.set_tree_with(tree, Refresh::Full);
        let damage = ui.render();
        assert!(!damage.is_empty());
        t.elapsed()
    };

    // The app does this while the library is on screen (warm_up).
    let t = Instant::now();
    let sets = HYPHENATION_PATTERNS.len();
    println!(
        "warm-up     {sets} hyphenation pattern sets in {:.1} ms (at startup, off the open path)",
        ms(t.elapsed())
    );

    println!("\nold path: everything before the first submit");
    let mut kept = None;
    for run in 1..=2 {
        let t0 = Instant::now();
        let (mut session, times) = Session::open(&lib, &rec, style.clone(), &mobi_cache)?;
        let page = session.page(session.current)?;
        let t_frame = frame(&mut ui, page);
        println!(
            "  run {run}: book {:6.1} ms, position {:6.1} ms, first page {:5.1} ms, UI frame {:4.1} ms -> submit after {:6.1} ms",
            ms(times.open),
            ms(times.place),
            ms(times.render),
            ms(t_frame),
            ms(t0.elapsed())
        );
        kept = Some(session);
    }
    let mut session = kept.expect("two runs");
    // Like a reader who stays on the page: the page numbers arrive first.
    let t = Instant::now();
    while session.paginating() && t.elapsed().as_secs() < 60 {
        session.poll_pagination();
        thread::sleep(std::time::Duration::from_millis(20));
    }
    let (pn, pp) = session.prerender();
    println!(
        "  prerender next {:.1} ms, previous {:.1} ms",
        ms(pn),
        ms(pp)
    );

    // Save, as App::save_snapshot does on a pause, back to the library or quit.
    let progress = session.progress(aristotle_library::library::now());
    lib.set_progress(&id, progress.clone());
    let key = snapshot::Key::new(&id, session.stamp(), &style, progress.locator);
    let here = session.current;
    let current = session.page(here)?;
    let next = session
        .known_next(session.current)
        .and_then(|l| session.rendered(l).map(|p| (l, p)));
    let mut pages = vec![(page_info(&mut session, here), (*current).clone())];
    if let Some((l, p)) = next {
        pages.push((page_info(&mut session, l), (*p).clone()));
    }
    let snap = snapshot::Snapshot { key, pages };
    let t = Instant::now();
    let encoded = snapshot::encode(&snap)?;
    let t_encode = t.elapsed();
    let t = Instant::now();
    let n = snapshot::store(lib.data_dir(), &snap)?;
    println!(
        "\nsnapshot saved: {} pages, {} KB ({:.0} KB per page raw), encode {:.1} ms, encode and write {:.1} ms",
        snap.pages.len(),
        n / 1024,
        current.data.len() as f64 / 1024.0,
        ms(t_encode),
        ms(t.elapsed())
    );
    debug_assert_eq!(n, encoded.len());
    drop(session);

    println!("\nnew path: snapshot on the panel first, the book opened by a worker");
    let rec = lib.book(&id).cloned().expect("found above");
    for run in 1..=2 {
        let t0 = Instant::now();
        let mut src = Source::new(&lib, &rec, &mobi_cache)?;
        let mut snap = find_snapshot(&src, &style)
            .ok_or_else(|| format_err!("the snapshot just saved does not match"))?;
        src.exact = Some(snap.info.location);
        let (read, decode) = (snap.read, snap.decode);
        let t_frame = frame(&mut ui, snap.page.clone());
        let t_submit = t0.elapsed();
        let worker = spawn(src, style.clone());
        let t = Instant::now();
        let next = snap.take_next();
        let t_next = t.elapsed();
        let prepared = worker.wait()?;
        let t_worker = worker.started.elapsed();
        let times = prepared.times;
        let t = Instant::now();
        let (at, right) = reconcile(
            snap.info.location,
            next.as_ref().map(|(i, _)| i.location),
            false,
            prepared.current,
            prepared.next(),
        );
        let mut session = Session::from_prepared(prepared)?;
        let page = session.page(at)?;
        let verdict = if right {
            compare(&snap.page, &page, &style)
        } else {
            Verdict::Page
        };
        let next_verdict = match (&next, session.known_next(at)) {
            (Some((i, p)), Some(real)) if i.location == real => {
                session.rendered(real).map_or("not rendered".into(), |r| {
                    format!("{:?}", compare(p, &r, &style))
                })
            }
            (Some(_), _) => "at another location".into(),
            (None, _) => "not stored".into(),
        };
        println!(
            "  run {run}: read {:.1} ms ({} KB), decode {:.1} ms, UI frame {:.1} ms -> submit after {:.1} ms",
            ms(read),
            snap.bytes / 1024,
            ms(decode),
            ms(t_frame),
            ms(t_submit)
        );
        println!(
            "         next page decoded in {:.1} ms; worker {:.1} ms (book {:.1}, position {:.1}, first page {:.1}, neighbours {:.1}); adopted in {:.1} ms; real page after {:.1} ms; snapshot {:?}, its next page {}",
            ms(t_next),
            ms(t_worker),
            ms(times.open),
            ms(times.place),
            ms(times.render),
            ms(times.neighbours),
            ms(t.elapsed()),
            ms(t0.elapsed()),
            verdict,
            next_verdict
        );
    }

    println!("\nback to the library: building its frame before the flash");
    let mut covers = Covers::default();
    for run in 1..=3 {
        let t = Instant::now();
        let model = ShelfModel::build(&lib, Filter::All, 0, &mut covers);
        let t_model = t.elapsed();
        let t = Instant::now();
        let (tree, _view) =
            ui_lib::Library::build_shelf(&mut ui.text, &styles, &model.shelf("12:00", None));
        let t_tree = t.elapsed();
        let t = Instant::now();
        ui.set_tree_with(tree, Refresh::Full);
        let t_layout = t.elapsed();
        let t = Instant::now();
        ui.render();
        println!(
            "  run {run}: {} books; model and covers {:.1} ms, tree {:.1} ms, layout {:.1} ms, paint {:.1} ms",
            lib.books().len(),
            ms(t_model),
            ms(t_tree),
            ms(t_layout),
            ms(t.elapsed())
        );
    }

    println!("\nprewarm: snapshots made ahead of time (one thread, idle priority in the app)");
    let never = AtomicBool::new(false);
    // Every book a third of the way in, if not opened yet.
    let unread: Vec<String> = lib
        .books()
        .iter()
        .filter(|b| b.progress.is_none())
        .map(|b| b.id.clone())
        .collect();
    for (i, b) in unread.iter().enumerate() {
        let p = progress::progress_at(&Position::Fraction(0.3), 0.3, 1 + i as i64);
        lib.set_progress(b, p);
    }
    for rec in crate::prewarm::candidates(&lib) {
        let Some((src, key)) = crate::prewarm::job_for(&lib, rec, &style, &mobi_cache) else {
            continue;
        };
        let t = Instant::now();
        let made = crate::prewarm::make(src, style.clone(), key, &never);
        let t_make = t.elapsed();
        let n = match made {
            Ok(Some(n)) => n,
            Ok(None) => continue,
            Err(e) => {
                println!("  {}: {e:#}", rec.meta.title);
                continue;
            }
        };
        // Then open it as the app would and check the snapshot.
        let mut src = Source::new(&lib, rec, &mobi_cache)?;
        let snap = find_snapshot(&src, &style)
            .ok_or_else(|| format_err!("the prewarmed snapshot does not match"))?;
        src.exact = Some(snap.info.location);
        let prepared = spawn(src, style.clone()).wait()?;
        let (at, right) = reconcile(snap.info.location, None, false, prepared.current, None);
        let mut session = Session::from_prepared(prepared)?;
        let page = session.page(at)?;
        let verdict = if right {
            compare(&snap.page, &page, &style)
        } else {
            Verdict::Page
        };
        println!(
            "  {:40} made in {:6.1} ms, {:3} KB; opened from it: snapshot {:?}",
            rec.meta.title.chars().take(40).collect::<String>(),
            ms(t_make),
            n / 1024,
            verdict
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_never_keeps_a_wrong_page() {
        // Placed where the snapshot was: keep it.
        assert_eq!(
            reconcile(100, Some(200), false, 100, Some(200)),
            (100, true)
        );
        // Turned to the snapshot's next page, which is the real next one.
        assert_eq!(reconcile(100, Some(200), true, 100, Some(200)), (200, true));
        // The book places the position elsewhere: redraw there.
        assert_eq!(reconcile(100, Some(200), false, 90, Some(150)), (90, false));
        assert_eq!(reconcile(100, Some(200), true, 90, Some(150)), (90, false));
        // Right page, but its successor moved: one page on, redrawn.
        assert_eq!(
            reconcile(100, Some(200), true, 100, Some(210)),
            (210, false)
        );
        // Turned past what turns out to be the last page.
        assert_eq!(reconcile(100, Some(200), true, 100, None), (100, false));
    }

    #[test]
    fn compare_tells_chrome_from_text() {
        let s = Style::pw4();
        let a = Page::white(&s);
        assert_eq!(compare(&a, &a.clone(), &s), Verdict::Same);
        let mut footer = a.clone();
        let y = s.height as usize - 40;
        footer.data[y * a.stride + 500] = 0;
        assert_eq!(compare(&a, &footer, &s), Verdict::Chrome);
        let mut header = a.clone();
        header.data[20 * a.stride + 500] = 0;
        assert_eq!(compare(&a, &header, &s), Verdict::Chrome);
        let mut text = a.clone();
        text.data[700 * a.stride + 500] = 0;
        assert_eq!(compare(&a, &text, &s), Verdict::Page);
        let mut both = footer.clone();
        both.data[700 * a.stride + 500] = 0;
        assert_eq!(compare(&a, &both, &s), Verdict::Page);
    }
}
