//! `aristotle-library-cli`: drive the library from a shell, on the host or
//! over SSH on the device. Argument parsing is hand-rolled to keep the
//! static ARM binary small.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use aristotle_library::clippings;
use aristotle_library::cover::{read_pgm, write_png, ThumbSpec};
use aristotle_library::library::{Library, ScanOptions};
use aristotle_library::{BookRecord, Format, Query, SortKey, StatusFilter};

const USAGE: &str = "\
usage: aristotle-library-cli [--data DIR] COMMAND

commands:
  scan ROOT [--no-covers] [--width N]   index the books under ROOT
  list [--sort recent|title|author|series|added] [--reverse]
       [--format epub,mobi,azw3,pdf,txt,cbz] [--status unread|reading|finished|unfinished]
       [--search TEXT] [--json]
  show ID [--json]                      one book; ID may be a unique prefix
  clippings FILE [--import] [--json]    parse a Kindle 'My Clippings.txt'
  import-sidecars                       import stock Kindle and KOReader .sdr state
  thumb ID OUT.png                      export a book's thumbnail as PNG

The data directory holds library.json and covers/. Default: $ARISTOTLE_DATA,
else /mnt/us/.aristotle on the Kindle, else ~/.local/share/aristotle.";

fn default_data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("ARISTOTLE_DATA") {
        return PathBuf::from(d);
    }
    if Path::new("/mnt/us").is_dir() {
        return PathBuf::from("/mnt/us/.aristotle");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("aristotle")
}

struct Args {
    rest: Vec<String>,
}

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        match self.rest.iter().position(|a| a == name) {
            Some(i) => {
                self.rest.remove(i);
                true
            }
            None => false,
        }
    }

    fn option(&mut self, name: &str) -> Result<Option<String>, String> {
        match self.rest.iter().position(|a| a == name) {
            Some(i) if i + 1 < self.rest.len() => {
                let v = self.rest.remove(i + 1);
                self.rest.remove(i);
                Ok(Some(v))
            }
            Some(_) => Err(format!("{name} needs a value")),
            None => Ok(None),
        }
    }

    fn positional(&mut self, what: &str) -> Result<String, String> {
        if self.rest.is_empty() || self.rest[0].starts_with("--") {
            return Err(format!("missing {what}"));
        }
        Ok(self.rest.remove(0))
    }

    fn done(&self) -> Result<(), String> {
        match self.rest.first() {
            Some(a) => Err(format!("unexpected argument: {a}")),
            None => Ok(()),
        }
    }
}

fn main() -> ExitCode {
    let mut args = Args {
        rest: std::env::args().skip(1).collect(),
    };
    if args.rest.is_empty() || args.flag("--help") || args.flag("-h") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&mut args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &mut Args) -> Result<(), String> {
    let data = args
        .option("--data")?
        .map(PathBuf::from)
        .unwrap_or_else(default_data_dir);
    let cmd = args.positional("command")?;
    let err = |e: aristotle_library::Error| e.to_string();
    match cmd.as_str() {
        "scan" => {
            let root = PathBuf::from(args.positional("ROOT")?);
            let mut opts = ScanOptions::default();
            if args.flag("--no-covers") {
                opts.covers = false;
            }
            if let Some(w) = args.option("--width")? {
                let width: u32 = w.parse().map_err(|_| "bad --width")?;
                opts.thumb = ThumbSpec {
                    width,
                    max_height: width * 8 / 5,
                };
            }
            args.done()?;
            let mut lib = Library::open_with(&data, opts.thumb).map_err(err)?;
            let r = lib.scan(&root, &opts).map_err(err)?;
            lib.save().map_err(err)?;
            println!(
                "{} books: {} new, {} updated, {} moved, {} restored, {} unchanged, {} removed; {} non-book files ignored",
                lib.books().len(),
                r.added,
                r.updated,
                r.moved,
                r.restored,
                r.unchanged,
                r.removed,
                r.ignored
            );
            println!(
                "{} thumbnails made, {} books got sidecar state",
                r.thumbnails, r.sidecars_imported
            );
            println!(
                "time: total {:.1} ms (walk {:.1}, parse {:.1}, covers {:.1})",
                ms(r.total_time),
                ms(r.walk_time),
                ms(r.parse_time),
                ms(r.cover_time)
            );
            for (p, e) in &r.errors {
                eprintln!("warning: {p}: {e}");
            }
            println!("index: {}", data.join("library.json").display());
        }
        "list" => {
            let mut q = Query::default();
            if let Some(s) = args.option("--sort")? {
                q.sort = SortKey::parse(&s).ok_or("unknown --sort")?;
            }
            q.reverse = args.flag("--reverse");
            if let Some(f) = args.option("--format")? {
                q.formats = f
                    .split(',')
                    .map(|s| Format::parse(s).ok_or(format!("unknown format {s}")))
                    .collect::<Result<_, _>>()?;
            }
            if let Some(s) = args.option("--status")? {
                q.status = Some(StatusFilter::parse(&s).ok_or("unknown --status")?);
            }
            q.search = args.option("--search")?;
            let json = args.flag("--json");
            args.done()?;
            let lib = Library::open(&data).map_err(err)?;
            let books = lib.query(&q);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&books).map_err(|e| e.to_string())?
                );
            } else {
                for b in &books {
                    println!("{}", line(b));
                }
                eprintln!("{} of {} books", books.len(), lib.books().len());
            }
        }
        "show" => {
            let id = args.positional("ID")?;
            let json = args.flag("--json");
            args.done()?;
            let lib = Library::open(&data).map_err(err)?;
            let b = lib
                .find(&id)
                .ok_or("no such book (or the prefix is ambiguous)")?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(b).map_err(|e| e.to_string())?
                );
            } else {
                show(&lib, b);
            }
        }
        "clippings" => {
            let file = args.positional("FILE")?;
            let import = args.flag("--import");
            let json = args.flag("--json");
            args.done()?;
            let bytes = std::fs::read(&file).map_err(|e| format!("{file}: {e}"))?;
            let parsed = clippings::parse(&bytes);
            if json {
                let v: Vec<_> = parsed
                    .clippings
                    .iter()
                    .map(|c| serde_json::json!({"title": c.title, "author": c.author, "annotation": c.to_annotation()}))
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
                );
            } else {
                let mut by_book: Vec<(String, usize, usize, usize)> = Vec::new();
                for c in &parsed.clippings {
                    let name = match &c.author {
                        Some(a) => format!("{} ({a})", c.title),
                        None => c.title.clone(),
                    };
                    let i = match by_book.iter().position(|e| e.0 == name) {
                        Some(i) => i,
                        None => {
                            by_book.push((name, 0, 0, 0));
                            by_book.len() - 1
                        }
                    };
                    match c.kind {
                        aristotle_library::index::AnnotationKind::Highlight => by_book[i].1 += 1,
                        aristotle_library::index::AnnotationKind::Note => by_book[i].2 += 1,
                        aristotle_library::index::AnnotationKind::Bookmark => by_book[i].3 += 1,
                    }
                }
                for (name, h, n, b) in &by_book {
                    println!("{h:4} highlights {n:3} notes {b:3} bookmarks  {name}");
                }
                println!(
                    "{} clippings in {} books, {} unparsable entries",
                    parsed.clippings.len(),
                    by_book.len(),
                    parsed.skipped
                );
            }
            if import {
                let mut lib = Library::open(&data).map_err(err)?;
                let r = lib.import_clippings(&parsed.clippings);
                lib.save().map_err(err)?;
                println!(
                    "imported: {} matched, {} new, {} already present, {} titles without a book",
                    r.matched,
                    r.added,
                    r.duplicates,
                    r.unmatched.len()
                );
                for t in &r.unmatched {
                    println!("  no book: {t}");
                }
            }
        }
        "import-sidecars" => {
            args.done()?;
            let mut lib = Library::open(&data).map_err(err)?;
            let r = lib.import_sidecars();
            lib.save().map_err(err)?;
            for e in &r.errors {
                eprintln!("warning: {e}");
            }
            println!(
                "{} books have stock or KOReader sidecars; {} updated now, the rest were already imported",
                r.with_sidecars, r.changed
            );
            for b in lib.books() {
                let Some(p) = &b.progress else { continue };
                if matches!(p.source.as_deref(), Some("kindle" | "koreader")) {
                    let page = p
                        .page
                        .as_deref()
                        .map(|l| format!(", page {l}"))
                        .unwrap_or_default();
                    println!(
                        "  {:5.1}%{page}  {:?}  [{}]  {}",
                        p.fraction * 100.0,
                        p.locator,
                        p.source.as_deref().unwrap_or(""),
                        b.meta.title
                    );
                }
            }
        }
        "thumb" => {
            let id = args.positional("ID")?;
            let out = args.positional("OUT.png")?;
            args.done()?;
            let lib = Library::open(&data).map_err(err)?;
            let b = lib.find(&id).ok_or("no such book")?;
            let p = lib.cover_path(b).ok_or("book has no thumbnail")?;
            let img = read_pgm(&p).map_err(err)?;
            write_png(Path::new(&out), &img).map_err(err)?;
            println!("{} x {} -> {out}", img.width, img.height);
        }
        other => return Err(format!("unknown command {other}\n\n{USAGE}")),
    }
    Ok(())
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn authors(b: &BookRecord) -> String {
    b.meta
        .authors
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn line(b: &BookRecord) -> String {
    let progress = b
        .progress
        .as_ref()
        .map(|p| format!("{:3.0}%", p.fraction * 100.0))
        .unwrap_or_else(|| "   -".into());
    let series = b
        .meta
        .series
        .as_ref()
        .map(|s| match s.index {
            Some(i) => format!("  [{} #{}]", s.name, i),
            None => format!("  [{}]", s.name),
        })
        .unwrap_or_default();
    format!(
        "{}  {:4}  {:8}  {}  {} — {}{}",
        &b.id[..8.min(b.id.len())],
        b.format.name(),
        format!("{:?}", b.status).to_lowercase(),
        progress,
        authors(b),
        b.meta.title,
        series
    )
}

fn show(lib: &Library, b: &BookRecord) {
    let m = &b.meta;
    println!("id:          {}", b.id);
    println!("path:        {}", b.file.path);
    println!("format:      {}  ({} bytes)", b.format.name(), b.file.size);
    println!(
        "title:       {}{}",
        m.title,
        if m.title_source == aristotle_library::metadata::TitleSource::Filename {
            "  (from file name)"
        } else {
            ""
        }
    );
    for a in &m.authors {
        println!("author:      {}  [sort: {}]", a.name, a.sort);
    }
    if let Some(s) = &m.series {
        println!(
            "series:      {}{}",
            s.name,
            s.index.map(|i| format!(" #{i}")).unwrap_or_default()
        );
    }
    for (k, v) in [
        ("language", &m.language),
        ("publisher", &m.publisher),
        ("published", &m.published),
    ] {
        if let Some(v) = v {
            println!("{:13}{v}", format!("{k}:"));
        }
    }
    for i in &m.identifiers {
        println!(
            "identifier:  {}{}",
            if i.scheme.is_empty() {
                String::new()
            } else {
                format!("{}:", i.scheme)
            },
            i.value
        );
    }
    if !m.subjects.is_empty() {
        println!("subjects:    {}", m.subjects.join("; "));
    }
    if let Some(p) = m.pages {
        println!("pages:       {p}");
    }
    if m.drm {
        println!("drm:         yes (cannot be opened)");
    }
    match lib.cover_path(b) {
        Some(p) => println!("thumbnail:   {}", p.display()),
        None => println!("thumbnail:   none"),
    }
    println!("status:      {:?}", b.status);
    if let Some(p) = &b.progress {
        println!(
            "progress:    {:.1}% {:?} ({}){}",
            p.fraction * 100.0,
            p.locator,
            p.source.as_deref().unwrap_or("aristotle"),
            p.page
                .as_deref()
                .map(|l| format!(", page {l}"))
                .unwrap_or_default()
        );
    }
    println!("annotations: {}", b.annotations.len());
    if let Some(d) = &m.description {
        let short: String = d.chars().take(400).collect();
        println!(
            "\n{short}{}",
            if d.chars().count() > 400 { "…" } else { "" }
        );
    }
}
