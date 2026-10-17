//! `aristotle-render`: render, show and benchmark e-book pages on the PW4.
//!
//! ```text
//! aristotle-render [OPTIONS] info   BOOK
//! aristotle-render [OPTIONS] render BOOK PAGE OUT.raw
//! aristotle-render [OPTIONS] show   BOOK PAGE
//! aristotle-render [OPTIONS] bench  BOOK [PAGE] [COUNT]
//! aristotle-render [OPTIONS] turn   BOOK START COUNT PAUSE_MS
//! aristotle-render [OPTIONS] pages  BOOK [CHECKS]
//! aristotle-render [OPTIONS] place  BOOK FRACTION...
//!
//! OPTIONS
//!   --data DIR         runtime data (fonts/, css/, hyphenation-patterns/,
//!                      serif/); default: data/ next to the binary
//!   --cache DIR        where MOBI books converted to EPUB are kept;
//!                      default: cache/ next to the binary
//!   --no-convert       read MOBI with MuPDF instead of converting it
//!   --font-size PT     body size in points (default 10)
//!   --line-height EM   (default 1.3)
//!   --margin MM        whole millimetres (default 5)
//!   --lang TAG         hyphenate EPUB text in this language (default: the
//!                      book's, guessed from the text if it declares several)
//!   --percent          don't paginate the whole book; the footer shows the
//!                      page number and percentage only
//!   --bw               render: write the black and white (A2) frame instead
//!   --force            show/turn: run even if the stock GUI is up
//! ```
//!
//! Pages are 1-based. `render` works anywhere, including on the PC with a
//! host build; `show` and `turn` need the PW4 framebuffer with the stock GUI
//! stopped (`stop lab126_gui`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use aristotle_render::book::Book;
use aristotle_render::page::Chrome;
use aristotle_render::screen::{stock_gui_running, Screen, TurnStrategy};
use aristotle_render::style::Style;
use plato_core::anyhow::{bail, format_err, Context, Error};

struct Args {
    command: String,
    positional: Vec<String>,
    data: Option<PathBuf>,
    /// Where converted MOBI books are kept; `None` reads MOBI with MuPDF.
    cache: Option<PathBuf>,
    no_convert: bool,
    percent: bool,
    bw: bool,
    force: bool,
    /// Page turns show an A2 preview first (experimental, may ghost).
    a2_preview: bool,
    /// Page turns use one full update with this waveform mode instead.
    turn_mode: Option<u32>,
    style: Style,
}

fn parse_args() -> Result<Args, Error> {
    let mut style = Style::pw4();
    let mut positional = Vec::new();
    let (mut data, mut percent, mut bw, mut force) = (None, false, false, false);
    let mut a2_preview = false;
    let mut turn_mode = None;
    let (mut cache, mut no_convert) = (None, false);
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format_err!("{name} needs a value"));
        match arg.as_str() {
            "--data" => data = Some(PathBuf::from(value("--data")?)),
            "--cache" => cache = Some(PathBuf::from(value("--cache")?)),
            "--no-convert" => no_convert = true,
            "--font-size" => style.font_size = value("--font-size")?.parse()?,
            "--line-height" => style.line_height = value("--line-height")?.parse()?,
            "--margin" => style.margin_mm = value("--margin")?.parse()?,
            "--lang" => style.language = Some(value("--lang")?),
            "--percent" => percent = true,
            "--bw" => bw = true,
            "--force" => force = true,
            "--a2-preview" => a2_preview = true,
            "--turn-mode" => turn_mode = Some(value("--turn-mode")?.parse()?),
            "-h" | "--help" => bail!("{}", usage()),
            s if s.starts_with("--") => bail!("unknown option {s}\n{}", usage()),
            _ => positional.push(arg),
        }
    }
    if positional.is_empty() {
        bail!("{}", usage());
    }
    let command = positional.remove(0);
    Ok(Args {
        command,
        positional,
        data,
        cache,
        no_convert,
        percent,
        bw,
        force,
        a2_preview,
        turn_mode,
        style,
    })
}

fn turn_strategy(args: &Args) -> TurnStrategy {
    if args.a2_preview {
        TurnStrategy::A2Preview
    } else if let Some(mode) = args.turn_mode {
        TurnStrategy::Single(mode)
    } else {
        TurnStrategy::Stock
    }
}

fn usage() -> &'static str {
    "usage: aristotle-render [--data DIR] [--font-size PT] [--line-height EM] [--margin MM] [--lang TAG]\n\
     \x20                       [--cache DIR] [--no-convert] [--percent] [--bw] [--force] [--a2-preview] [--turn-mode N] COMMAND\n\
     commands:\n\
     \x20 info   BOOK\n\
     \x20 render BOOK PAGE OUT.raw\n\
     \x20 show   BOOK PAGE\n\
     \x20 bench  BOOK [PAGE] [COUNT]\n\
     \x20 turn   BOOK START COUNT PAUSE_MS\n\
     \x20 pages  BOOK [CHECKS]\n\
     \x20 place  BOOK FRACTION..."
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Run `f` and return its result with the wall-clock time it took.
fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = f();
    (out, start.elapsed())
}

fn arg<T: std::str::FromStr>(args: &Args, index: usize, name: &str) -> Result<T, Error>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    args.positional
        .get(index)
        .ok_or_else(|| format_err!("missing {name}\n{}", usage()))?
        .parse()
        .with_context(|| format!("bad {name}"))
}

fn page_index(page: usize) -> Result<usize, Error> {
    page.checked_sub(1)
        .ok_or_else(|| format_err!("pages are numbered from 1"))
}

/// Peak resident memory of this process in MiB (`ru_maxrss`).
fn max_rss_mib() -> f64 {
    // SAFETY: getrusage only writes into the zeroed struct we pass.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    usage.ru_maxrss as f64 / 1024.0
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aristotle-render: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Error> {
    let mut args = parse_args()?;
    let book_path: PathBuf = arg::<String>(&args, 0, "BOOK")?.into();
    let book_path = book_path
        .canonicalize()
        .with_context(|| format!("can't find {}", book_path.display()))?;
    // Output paths are relative to where we were started, not to the data
    // directory we are about to change into.
    let cwd = std::env::current_dir()?;
    let exe_dir = std::env::current_exe()?
        .parent()
        .ok_or_else(|| format_err!("binary has no parent directory"))?
        .to_path_buf();
    let data = args
        .data
        .as_ref()
        .map_or_else(|| exe_dir.join("data"), |d| cwd.join(d));
    args.cache = if args.no_convert {
        None
    } else {
        Some(
            args.cache
                .as_ref()
                .map_or_else(|| exe_dir.join("cache"), |c| cwd.join(c)),
        )
    };
    aristotle_render::use_data_dir(&data)?;

    match args.command.as_str() {
        "info" => info(&args, &book_path),
        "render" => render(&args, &book_path, &cwd),
        "show" => show(&args, &book_path),
        "bench" => bench(&args, &book_path),
        "turn" => turn(&args, &book_path),
        "pages" => pages(&args, &book_path),
        "place" => place(&args, &book_path),
        other => bail!("unknown command {other}\n{}", usage()),
    }
}

/// Print the start of every page, one per line, then place CHECKS locations
/// spread over the book, each in a fresh copy as a reader does when it
/// opens a book, and print `@location page next` for each. Comparing the
/// output of two builds shows whether a layout change moved any page break
/// or placement. Placements that disagree with the pagination are reported:
/// Plato places a location between a spine item's start and its first text
/// on that item's first page.
fn pages(args: &Args, path: &Path) -> Result<(), Error> {
    let checks: usize = if args.positional.len() > 1 {
        arg(args, 1, "CHECKS")?
    } else {
        0
    };
    let (mut book, _) = open(args, path)?;
    let (_, t) = timed(|| book.paginate());
    let starts = book.pagination().map(<[usize]>::to_vec).unwrap_or_default();
    for s in &starts {
        println!("{s}");
    }
    eprintln!("{} pages, paginated in {:.0} ms", starts.len(), ms(t));
    let mut wrong = 0;
    let mut slowest = Duration::ZERO;
    for i in 0..checks {
        // Spread over the book, not on page starts.
        let loc = (book.len() as u64 * (2 * i as u64 + 1) / (2 * checks as u64)) as usize;
        let (mut fresh, _) = open(args, path)?;
        let (found, t) = timed(|| fresh.page_start(loc));
        slowest = slowest.max(t);
        let expected = starts[starts.partition_point(|&s| s <= loc).saturating_sub(1)];
        let next = found.and_then(|f| fresh.next_page(f));
        let expected_next = starts.iter().copied().find(|&s| s > expected);
        println!(
            "@{loc} {} {}",
            found.map_or(-1, |f| f as i64),
            next.map_or(-1, |n| n as i64)
        );
        if found != Some(expected) || next != expected_next {
            wrong += 1;
        }
    }
    if checks > 0 {
        eprintln!(
            "{checks} placements in fresh copies, {wrong} off the pagination, slowest {:.0} ms",
            ms(slowest)
        );
    }
    Ok(())
}

/// Time what opening a book at a saved position costs, for each FRACTION of
/// the book (0 to 1), in a fresh copy: open, place, render that page, then
/// the next page (as the reader prerenders it) and a restyle at that place.
fn place(args: &Args, path: &Path) -> Result<(), Error> {
    for f in args.positional.iter().skip(1) {
        let f: f32 = f.parse().context("FRACTION")?;
        let (mut book, mut chrome) = open(args, path)?;
        let loc = book
            .location_of(&aristotle_render::book::Position::Fraction(f))
            .unwrap_or(0);
        let (start, t_place) = timed(|| book.page_start(loc));
        let start = start.ok_or_else(|| format_err!("no page at {loc}"))?;
        let (_, t_render) = timed(|| book.render_location(start, &mut chrome));
        let (next, t_next) = timed(|| {
            book.next_page(start)
                .map(|n| book.render_location(n, &mut chrome))
        });
        let mut bigger = args.style.clone();
        bigger.font_size += 1.0;
        let (_, t_restyle) = timed(|| -> Result<_, Error> {
            let mut b = Book::open(path, &bigger, args.cache.as_deref())?;
            let at = b.location_of(&book.position_at(start)).unwrap_or(0);
            Ok(b.page_start(at))
        });
        println!(
            "{:4.0}%: place {:7.1} ms, render {:5.1} ms, next page {:5.1} ms{}, restyle (+1 pt) and place {:7.1} ms",
            f * 100.0,
            ms(t_place),
            ms(t_render),
            ms(t_next),
            if next.is_some() { "" } else { " (none)" },
            ms(t_restyle)
        );
    }
    Ok(())
}

fn open(args: &Args, path: &Path) -> Result<(Book, Chrome), Error> {
    let book = Book::open(path, &args.style, args.cache.as_deref())?;
    let chrome = Chrome::load(&args.style).context("can't load the chrome fonts")?;
    Ok((book, chrome))
}

fn print_mobi(book: &Book) {
    if let Some(m) = book.mobi() {
        let i = &m.info;
        println!(
            "mobi          version {}, compression {}, encoding {}, kf8 boundary {:?}",
            i.version, i.compression, i.encoding, i.kf8_boundary
        );
        println!(
            "converted     {} ({})",
            m.epub.display(),
            if m.converted_now { "now" } else { "cached" }
        );
    }
}

fn info(args: &Args, path: &Path) -> Result<(), Error> {
    let (opened, t_open) = timed(|| open(args, path));
    let (mut book, _) = opened?;
    println!("file          {}", path.display());
    print_mobi(&book);
    println!("engine        {:?}", book.engine());
    println!("title         {}", book.title().unwrap_or("-"));
    println!("author        {}", book.author().as_deref().unwrap_or("-"));
    println!(
        "language      {}",
        book.language().unwrap_or("- (none declared)")
    );
    let h = book.hyphenation();
    println!(
        "hyphenation   {} -> {} ({})",
        h.tag,
        h.language
            .as_deref()
            .unwrap_or("no patterns for this language"),
        if h.loaded {
            "patterns loaded"
        } else {
            "patterns missing"
        }
    );
    println!("toc entries   {}", book.toc().len());
    let (pages, t_pag) = timed(|| book.paginate());
    println!("pages         {pages}");
    println!(
        "open {:.1} ms, paginate {:.1} ms, max RSS {:.1} MiB",
        ms(t_open),
        ms(t_pag),
        max_rss_mib()
    );
    Ok(())
}

fn render(args: &Args, path: &Path, cwd: &Path) -> Result<(), Error> {
    let index = page_index(arg(args, 1, "PAGE")?)?;
    let out = cwd.join(arg::<String>(args, 2, "OUT.raw")?);
    let (mut book, mut chrome) = open(args, path)?;
    if !args.percent {
        book.paginate();
    }
    let (page, t) = timed(|| book.render_page(index, &mut chrome));
    let page = page?;
    let page = if args.bw { page.threshold() } else { page };
    page.write_raw(&out)
        .with_context(|| format!("can't write {}", out.display()))?;
    println!(
        "page {} of {} -> {} ({}x{}, stride {}) in {:.1} ms",
        index + 1,
        book.known_page_count()
            .map_or("?".to_string(), |n| n.to_string()),
        out.display(),
        page.width,
        page.height,
        page.stride,
        ms(t)
    );
    Ok(())
}

fn check_gui(args: &Args) -> Result<(), Error> {
    if stock_gui_running() && !args.force {
        bail!("the stock GUI (cvm) is running and would draw over us; run `stop lab126_gui` first, or pass --force");
    }
    Ok(())
}

fn show(args: &Args, path: &Path) -> Result<(), Error> {
    check_gui(args)?;
    let index = page_index(arg(args, 1, "PAGE")?)?;
    let (mut book, mut chrome) = open(args, path)?;
    if !args.percent {
        book.paginate();
    }
    let (page, t_render) = timed(|| book.render_page(index, &mut chrome));
    let page = page?;
    let bw = page.threshold();
    let mut screen = Screen::open().context("can't open /dev/fb0")?;
    let t = screen.turn(&page, &bw, turn_strategy(args))?;
    println!(
        "page {}: render {:.1} ms, visible {:.1} ms, settled {:.1} ms (panel times from blit)",
        index + 1,
        ms(t_render),
        ms(t.visible),
        ms(t.settled)
    );
    Ok(())
}

fn stats(label: &str, mut v: Vec<f64>) {
    if v.is_empty() {
        return;
    }
    v.sort_by(f64::total_cmp);
    let p = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    println!(
        "{label:<10} median {:>7.1}  p90 {:>7.1}  max {:>7.1}  min {:>7.1}",
        p(0.5),
        p(0.9),
        v[v.len() - 1],
        v[0]
    );
}

fn bench(args: &Args, path: &Path) -> Result<(), Error> {
    let first: usize = if args.positional.len() > 1 {
        arg(args, 1, "PAGE")?
    } else {
        1
    };
    let count: usize = if args.positional.len() > 2 {
        arg(args, 2, "COUNT")?
    } else {
        21
    };
    let start = page_index(first)?;

    let (opened, t_open) = timed(|| Book::open(path, &args.style, args.cache.as_deref()));
    let mut book = opened?;
    let (chrome, t_chrome) = timed(|| Chrome::load(&args.style));
    let mut chrome = chrome?;
    let (hyph, t_hyph) = timed(|| book.hyphenation());
    let (first_loc, t_first) = timed(|| book.locate(0));
    first_loc.ok_or_else(|| format_err!("the book has no pages"))?;
    let (_, t_seek) = timed(|| book.locate(start));

    println!("book       {}", path.display());
    print_mobi(&book);
    println!(
        "engine     {:?}, language {}, hyphenation {} ({})",
        book.engine(),
        book.language().unwrap_or("-"),
        hyph.language.as_deref().unwrap_or("none"),
        if hyph.loaded { "loaded" } else { "missing" }
    );
    println!(
        "open                {:>8.1} ms  (parse container, load body fonts; MOBI: read, and convert unless cached)",
        ms(t_open)
    );
    println!("chrome fonts        {:>8.1} ms", ms(t_chrome));
    println!(
        "hyphenation load    {:>8.1} ms  (all pattern files in hyphenation-patterns/)",
        ms(t_hyph)
    );
    println!(
        "first layout        {:>8.1} ms  (locate page 1: lays out its spine item)",
        ms(t_first)
    );
    println!("seek to page {first:<6} {:>8.1} ms", ms(t_seek));
    println!();
    println!(
        "{:>6} {:>9} {:>9} {:>9} {:>9} {:>9}   (ms; layout = locating the page, 0 unless a new chapter is laid out)",
        "page", "layout", "render", "compose", "bw", "total"
    );

    let (mut tot, mut ren) = (Vec::new(), Vec::new());
    for index in start..start + count {
        let (loc, t_loc) = timed(|| book.locate(index));
        let Some(loc) = loc else {
            println!("{:>6} end of book", index + 1);
            break;
        };
        let (body, t_body) = timed(|| book.render_body(loc));
        let body = body.ok_or_else(|| format_err!("can't render page {}", index + 1))?;
        let (page, t_comp) = timed(|| {
            let folio = book.folio(index, loc);
            book.compose(&body, &folio, &mut chrome)
        });
        let (_bw, t_bw) = timed(|| page.threshold());
        let total = t_loc + t_body + t_comp + t_bw;
        println!(
            "{:>6} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1}",
            index + 1,
            ms(t_loc),
            ms(t_body),
            ms(t_comp),
            ms(t_bw),
            ms(total)
        );
        tot.push(ms(total));
        ren.push(ms(t_body));
    }
    println!();
    stats("total", tot);
    stats("render", ren);

    if !args.percent {
        let (pages, t_pag) = timed(|| book.paginate());
        println!("paginate whole book {:>8.1} ms  ({pages} pages)", ms(t_pag));
    }
    println!("max RSS             {:>8.1} MiB", max_rss_mib());
    Ok(())
}

/// Page turns as a reader app should do them: the next page is rendered
/// while the panel is busy with the current one, so a turn costs only the
/// panel's time. Reports the time from the (simulated) tap to the page being
/// on the panel, which is what the reader waits for.
fn turn_pipelined(
    book: &mut Book,
    chrome: &mut Chrome,
    screen: &mut Screen,
    start: usize,
    count: usize,
    pause: Duration,
    strategy: TurnStrategy,
) -> Result<(), Error> {
    println!(
        "{:>6} {:>12} {:>12}   (ms; pipelined)",
        "page", "tap->shown", "prerender"
    );
    let mut next = Some(book.render_page(start + 1, chrome)?);
    let (mut shown, mut prerender) = (Vec::new(), Vec::new());
    for index in start + 1..=start + count {
        std::thread::sleep(pause);
        let Some(page) = next.take() else { break };
        let tap = Instant::now();
        let marker = screen.submit(&page, strategy)?;
        // The CPU is free while the panel refreshes: prepare the next page.
        let t = Instant::now();
        next = book.render_page(index + 1, chrome).ok();
        let t_pre = t.elapsed();
        screen.finish(marker)?;
        let t_shown = tap.elapsed();
        println!("{:>6} {:>12.1} {:>12.1}", index + 1, ms(t_shown), ms(t_pre));
        shown.push(ms(t_shown));
        prerender.push(ms(t_pre));
    }
    println!();
    stats("tap->shown", shown);
    stats("prerender", prerender);
    println!("max RSS {:.1} MiB", max_rss_mib());
    Ok(())
}

fn turn(args: &Args, path: &Path) -> Result<(), Error> {
    check_gui(args)?;
    let start = page_index(arg(args, 1, "START")?)?;
    let count: usize = arg(args, 2, "COUNT")?;
    let pause = Duration::from_millis(arg(args, 3, "PAUSE_MS")?);

    let (mut book, mut chrome) = open(args, path)?;
    if !args.percent {
        let (pages, t) = timed(|| book.paginate());
        println!("paginated {pages} pages in {:.1} ms", ms(t));
    }
    let mut screen = Screen::open().context("can't open /dev/fb0")?;
    let first = book.render_page(start, &mut chrome)?;
    let t = screen.show_clean(&first)?;
    println!("page {} shown with GC16 in {:.1} ms", start + 1, ms(t));
    println!(
        "{:>6} {:>9} {:>9} {:>9} {:>11} {:>11}   (ms; *_total include render)",
        "page", "render", "visible", "settled", "vis_total", "set_total"
    );

    let strategy = turn_strategy(args);
    if strategy != TurnStrategy::A2Preview {
        return turn_pipelined(
            &mut book,
            &mut chrome,
            &mut screen,
            start,
            count,
            pause,
            strategy,
        );
    }
    let (mut vis, mut set, mut vis_t, mut ren) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for index in start + 1..=start + count {
        std::thread::sleep(pause);
        let t0 = Instant::now();
        let page = match book.render_page(index, &mut chrome) {
            Ok(p) => p,
            Err(e) => {
                println!("{:>6} stop: {e:#}", index + 1);
                break;
            }
        };
        let bw = page.threshold();
        let t_render = t0.elapsed();
        let t = screen.turn(&page, &bw, turn_strategy(args))?;
        println!(
            "{:>6} {:>9.1} {:>9.1} {:>9.1} {:>11.1} {:>11.1}",
            index + 1,
            ms(t_render),
            ms(t.visible),
            ms(t.settled),
            ms(t_render + t.visible),
            ms(t_render + t.settled)
        );
        ren.push(ms(t_render));
        vis.push(ms(t.visible));
        set.push(ms(t.settled));
        vis_t.push(ms(t_render + t.visible));
    }
    println!();
    stats("render", ren);
    stats("visible", vis);
    stats("settled", set);
    stats("vis_total", vis_t);
    println!("max RSS {:.1} MiB", max_rss_mib());
    Ok(())
}
