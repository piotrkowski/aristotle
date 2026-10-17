//! The library: an index plus a cover cache in one data directory, and the
//! incremental scanner that keeps them in step with the books on disk.
//!
//! ## Scanning
//!
//! A scan walks the root, skipping hidden entries, `.sdr` sidecar folders and
//! configured folders (`dictionaries` by default), and stats every candidate
//! file. A file whose path, size and mtime match the index is not opened at
//! all, which makes a re-scan of an unchanged library cost one `readdir` per
//! folder and one `stat` per file. New and changed files are sniffed,
//! hashed (partial MD5, twelve 1 KiB reads), parsed, and get a thumbnail.
//!
//! Reading state belongs to the book, not the path. When a new file has the
//! ID of a book whose path disappeared, it is a move or rename and inherits
//! the old record's state; when a known path changes content, the record at
//! that path passes its state on; when a removed book comes back, its state
//! is restored from the `removed` list.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::clippings::{title_key, Clipping};
use crate::cover::{CoverCache, ThumbSpec};
use crate::error::Result;
use crate::format::{self, Format};
use crate::index::{
    Annotation, BookRecord, FileStamp, Index, Locator, Progress, Status, INDEX_FILE,
};
use crate::metadata;
use crate::query::{self, Query};
use crate::sidecar;
use crate::text;

pub const COVERS_DIR: &str = "covers";
/// Cap on remembered removed books; old entries are dropped first.
const MAX_REMOVED: usize = 500;

pub struct Library {
    data_dir: PathBuf,
    index: Index,
    covers: CoverCache,
}

#[derive(Clone, Debug)]
pub struct ScanOptions {
    /// Extract covers and build thumbnails for new and changed books.
    pub covers: bool,
    pub thumb: ThumbSpec,
    /// Folder names (case-insensitive) skipped at any depth.
    pub skip_dirs: Vec<String>,
    /// Import stock Kindle and KOReader sidecars for books new to the index.
    pub import_sidecars: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            covers: true,
            thumb: ThumbSpec::default(),
            skip_dirs: vec!["dictionaries".into()],
            import_sidecars: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct ScanReport {
    pub unchanged: usize,
    pub added: usize,
    pub updated: usize,
    pub moved: usize,
    pub restored: usize,
    pub removed: usize,
    pub ignored: usize,
    pub thumbnails: usize,
    pub sidecars_imported: usize,
    pub errors: Vec<(String, String)>,
    pub walk_time: Duration,
    pub parse_time: Duration,
    pub cover_time: Duration,
    pub total_time: Duration,
}

#[derive(Debug, Default)]
pub struct ClippingsImport {
    pub matched: usize,
    pub added: usize,
    pub duplicates: usize,
    /// Titles of clippings that matched no book.
    pub unmatched: Vec<String>,
}

struct Found {
    abs: PathBuf,
    stamp: FileStamp,
    author_dir: Option<String>,
}

pub(crate) fn unix_secs(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

pub fn now() -> i64 {
    unix_secs(SystemTime::now()).unwrap_or(0)
}

impl Library {
    /// Opens (or starts) the library stored in `data_dir`.
    pub fn open(data_dir: impl Into<PathBuf>) -> Result<Library> {
        Library::open_with(data_dir, ThumbSpec::default())
    }

    pub fn open_with(data_dir: impl Into<PathBuf>, thumb: ThumbSpec) -> Result<Library> {
        let data_dir = data_dir.into();
        let index = Index::load(&data_dir.join(INDEX_FILE))?;
        let covers = CoverCache::new(data_dir.join(COVERS_DIR), thumb);
        Ok(Library {
            data_dir,
            index,
            covers,
        })
    }

    pub fn save(&self) -> Result<()> {
        self.index.save(&self.data_dir.join(INDEX_FILE))
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn root(&self) -> Option<PathBuf> {
        self.index.root.as_ref().map(PathBuf::from)
    }

    pub fn books(&self) -> &[BookRecord] {
        &self.index.books
    }

    pub fn book(&self, id: &str) -> Option<&BookRecord> {
        self.index.books.iter().find(|b| b.id == id)
    }

    pub fn book_mut(&mut self, id: &str) -> Option<&mut BookRecord> {
        self.index.books.iter_mut().find(|b| b.id == id)
    }

    /// Looks a book up by full ID or by a unique ID prefix (handy in a shell).
    pub fn find(&self, id_or_prefix: &str) -> Option<&BookRecord> {
        if let Some(b) = self.book(id_or_prefix) {
            return Some(b);
        }
        let mut it = self
            .index
            .books
            .iter()
            .filter(|b| b.id.starts_with(id_or_prefix));
        match (it.next(), it.next()) {
            (Some(b), None) if !id_or_prefix.is_empty() => Some(b),
            _ => None,
        }
    }

    pub fn absolute_path(&self, rec: &BookRecord) -> Option<PathBuf> {
        self.root().map(|r| r.join(&rec.file.path))
    }

    pub fn cover_path(&self, rec: &BookRecord) -> Option<PathBuf> {
        rec.cover
            .as_ref()
            .map(|k| self.covers.path(k))
            .filter(|p| p.exists())
    }

    pub fn query(&self, q: &Query) -> Vec<&BookRecord> {
        query::run(&self.index.books, q)
    }

    pub fn mark_opened(&mut self, id: &str, when: i64) -> bool {
        let Some(b) = self.book_mut(id) else {
            return false;
        };
        b.last_opened = Some(when);
        if b.status == Status::Unread {
            b.status = Status::Reading;
        }
        true
    }

    pub fn set_progress(&mut self, id: &str, progress: Progress) -> bool {
        let Some(b) = self.book_mut(id) else {
            return false;
        };
        if b.status == Status::Unread {
            b.status = Status::Reading;
        }
        b.progress = Some(progress);
        true
    }

    /// Adds an annotation to a book unless an equal one is there already.
    /// Returns whether it was added.
    pub fn add_annotation(&mut self, id: &str, a: Annotation) -> bool {
        match self.book_mut(id) {
            Some(b) => merge_annotation(b, a),
            None => false,
        }
    }

    /// Removes the book's annotations for which `remove` is true. Returns
    /// how many went.
    pub fn remove_annotations(&mut self, id: &str, remove: impl Fn(&Annotation) -> bool) -> usize {
        let Some(b) = self.book_mut(id) else {
            return 0;
        };
        let before = b.annotations.len();
        b.annotations.retain(|a| !remove(a));
        before - b.annotations.len()
    }

    pub fn set_status(&mut self, id: &str, status: Status) -> bool {
        let Some(b) = self.book_mut(id) else {
            return false;
        };
        b.status = status;
        true
    }

    /// Brings the index in line with the files under `root`. See the module
    /// documentation for what is re-read and how state follows books.
    pub fn scan(&mut self, root: &Path, opts: &ScanOptions) -> Result<ScanReport> {
        let started = Instant::now();
        let mut report = ScanReport::default();
        let root = fs::canonicalize(root)?;
        let root_str = root.to_string_lossy().into_owned();
        if self.index.root.as_deref() != Some(&root_str) {
            // A different root makes every relative path meaningless; keep
            // the records as "removed" so their state can still be matched by ID.
            let old = std::mem::take(&mut self.index.books);
            self.index.removed.extend(old);
            self.index.ignored.clear();
            self.index.root = Some(root_str);
        }
        if self.covers.spec() != opts.thumb {
            self.covers = CoverCache::new(self.data_dir.join(COVERS_DIR), opts.thumb);
        }

        let mut found = Vec::new();
        let skip: HashSet<String> = opts.skip_dirs.iter().map(|s| s.to_lowercase()).collect();
        walk(&root, &root, &skip, &mut found, &mut report.errors);
        report.walk_time = started.elapsed();

        let mut old_books: HashMap<String, BookRecord> = std::mem::take(&mut self.index.books)
            .into_iter()
            .map(|b| (b.file.path.clone(), b))
            .collect();
        let old_ignored: HashMap<String, FileStamp> = std::mem::take(&mut self.index.ignored)
            .into_iter()
            .map(|s| (s.path.clone(), s))
            .collect();
        let mut books = Vec::new();
        let mut ignored = Vec::new();
        let mut pending: Vec<(Found, Option<BookRecord>)> = Vec::new();
        for f in found {
            if let Some(b) = old_books.remove(&f.stamp.path) {
                if b.file.same_file(&f.stamp) {
                    books.push(b);
                    report.unchanged += 1;
                } else {
                    pending.push((f, Some(b)));
                }
                continue;
            }
            if let Some(s) = old_ignored
                .get(&f.stamp.path)
                .filter(|s| s.same_file(&f.stamp))
            {
                ignored.push(s.clone());
                report.ignored += 1;
                continue;
            }
            pending.push((f, None));
        }
        // Whatever is left in old_books has vanished from its path: moved or deleted.
        let mut vanished: HashMap<String, BookRecord> =
            old_books.into_values().map(|b| (b.id.clone(), b)).collect();
        let mut taken_ids: HashSet<String> = books.iter().map(|b| b.id.clone()).collect();
        let now = now();

        for (f, prev) in pending {
            let t0 = Instant::now();
            let fmt = match format::sniff(&f.abs) {
                Ok(Some(fmt)) => fmt,
                Ok(None) => {
                    ignored.push(f.stamp);
                    report.ignored += 1;
                    continue;
                }
                Err(e) => {
                    report.errors.push((f.stamp.path.clone(), e.to_string()));
                    continue;
                }
            };
            let parsed = crate::id::partial_md5(&f.abs).and_then(|id| {
                Ok((
                    id,
                    metadata::extract(&f.abs, fmt, f.author_dir.as_deref(), opts.covers)?,
                ))
            });
            report.parse_time += t0.elapsed();
            let (mut id, ex) = match parsed {
                Ok(v) => v,
                Err(e) => {
                    report.errors.push((f.stamp.path.clone(), e.to_string()));
                    continue;
                }
            };
            if ex.dictionary {
                ignored.push(f.stamp);
                report.ignored += 1;
                continue;
            }
            let mut cover = None;
            if let Some(bytes) = &ex.cover {
                let t1 = Instant::now();
                let existed = self.covers.path(&self.covers.key(bytes)).exists();
                match self.covers.store(bytes) {
                    Ok(k) => {
                        cover = Some(k);
                        report.thumbnails += usize::from(!existed);
                    }
                    Err(e) => report
                        .errors
                        .push((f.stamp.path.clone(), format!("cover: {e}"))),
                }
                report.cover_time += t1.elapsed();
            }
            // Identical copies of one file share an ID; keep IDs unique.
            if taken_ids.contains(&id) {
                let base = id.clone();
                let mut n = 2;
                while taken_ids.contains(&id) {
                    id = format!("{base}-{n}");
                    n += 1;
                }
            }
            taken_ids.insert(id.clone());
            let mut rec = BookRecord {
                id: id.clone(),
                file: f.stamp,
                format: fmt,
                meta: ex.meta,
                cover,
                added: now,
                last_opened: None,
                status: Status::Unread,
                progress: None,
                settings: Default::default(),
                annotations: Vec::new(),
            };
            if let Some(p) = prev {
                rec.inherit_state(&p);
                report.updated += 1;
            } else if let Some(p) = vanished.remove(&id) {
                rec.inherit_state(&p);
                report.moved += 1;
            } else if let Some(i) = self.index.removed.iter().position(|r| r.id == id) {
                let p = self.index.removed.remove(i);
                rec.inherit_state(&p);
                report.restored += 1;
            } else {
                report.added += 1;
                if opts.import_sidecars && apply_sidecars(&mut rec, &f.abs).changed() {
                    report.sidecars_imported += 1;
                }
            }
            books.push(rec);
        }
        report.removed = vanished.len();
        self.index.removed.extend(vanished.into_values());
        if self.index.removed.len() > MAX_REMOVED {
            self.index
                .removed
                .sort_by_key(|r| std::cmp::Reverse(r.last_opened.unwrap_or(r.added)));
            self.index.removed.truncate(MAX_REMOVED);
        }
        books.sort_by(|a, b| a.file.path.cmp(&b.file.path));
        ignored.sort_by(|a, b| a.path.cmp(&b.path));
        self.index.books = books;
        self.index.ignored = ignored;
        report.total_time = started.elapsed();
        Ok(report)
    }

    /// Attaches clippings to books by title (diacritic-insensitive, then by
    /// prefix, since the Kindle sometimes shortens titles), using the author
    /// to break ties. Exact duplicates are skipped, so importing the same
    /// file twice is harmless.
    pub fn import_clippings(&mut self, clippings: &[Clipping]) -> ClippingsImport {
        let mut res = ClippingsImport::default();
        let keys: Vec<(String, String, String)> = self
            .index
            .books
            .iter()
            .map(|b| {
                let authors = text::fold(
                    &b.meta
                        .authors
                        .iter()
                        .map(|a| format!("{} {}", a.name, a.sort))
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                (
                    title_key(&b.meta.title),
                    title_key(&metadata::title_from_filename(Path::new(&b.file.path))),
                    authors,
                )
            })
            .collect();
        for c in clippings {
            let key = title_key(&c.title);
            let author = c.author.as_deref().map(text::fold);
            let exact: Vec<usize> = (0..keys.len())
                .filter(|&i| keys[i].0 == key || keys[i].1 == key)
                .collect();
            let candidates = if exact.is_empty() {
                (0..keys.len())
                    .filter(|&i| {
                        let t = &keys[i].0;
                        t.len().min(key.len()) >= 8
                            && (t.starts_with(&key) || key.starts_with(t.as_str()))
                    })
                    .collect()
            } else {
                exact
            };
            let pick = match candidates.len() {
                0 => None,
                1 => Some(candidates[0]),
                _ => candidates
                    .iter()
                    .copied()
                    .find(|&i| {
                        author.as_deref().is_some_and(|a| {
                            a.split(|c: char| !c.is_alphanumeric())
                                .filter(|w| w.len() > 2)
                                .any(|w| keys[i].2.contains(w))
                        })
                    })
                    .or(Some(candidates[0])),
            };
            let Some(i) = pick else {
                if !res.unmatched.contains(&c.title) {
                    res.unmatched.push(c.title.clone());
                }
                continue;
            };
            res.matched += 1;
            if merge_annotation(&mut self.index.books[i], c.to_annotation()) {
                res.added += 1;
            } else {
                res.duplicates += 1;
            }
        }
        res
    }

    /// Imports stock Kindle and KOReader sidecars for every book. Returns
    /// how many books changed.
    pub fn import_sidecars(&mut self) -> SidecarImport {
        let mut res = SidecarImport::default();
        let Some(root) = self.root() else {
            return res;
        };
        for b in &mut self.index.books {
            let outcome = apply_sidecars(b, &root.join(&b.file.path));
            res.changed += usize::from(outcome.changed());
            res.with_sidecars += usize::from(outcome.found);
            res.errors.extend(outcome.errors);
        }
        res
    }
}

/// Result of [`Library::import_sidecars`].
#[derive(Debug, Default)]
pub struct SidecarImport {
    /// Books whose progress or annotations changed.
    pub changed: usize,
    /// Books with a readable stock or KOReader sidecar, changed or not. A
    /// scan already imports the sidecars of new books, so a later import
    /// usually finds them up to date.
    pub with_sidecars: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Default)]
pub struct SidecarOutcome {
    /// A stock or KOReader sidecar was found and parsed.
    pub found: bool,
    pub progress: bool,
    pub annotations: usize,
    pub errors: Vec<String>,
}

impl SidecarOutcome {
    pub fn changed(&self) -> bool {
        self.progress || self.annotations > 0
    }
}

/// Merges what the `.sdr` folder knows into `rec`. Progress is taken only
/// if it is newer than what the record has, so a re-import never rewinds a
/// book that was read further in Aristotle.
pub fn apply_sidecars(rec: &mut BookRecord, abs: &Path) -> SidecarOutcome {
    let sc = sidecar::read_sidecars(abs);
    let mut out = SidecarOutcome {
        found: sc.kindle.is_some() || sc.koreader.is_some(),
        errors: sc.errors,
        ..Default::default()
    };
    let mut candidates: Vec<(Progress, Option<Status>)> = Vec::new();

    if let Some(k) = &sc.koreader {
        // KOReader stores the same partial MD5 we use; a mismatch means the
        // sidecar describes another version of the file.
        if k.partial_md5
            .as_deref()
            .is_none_or(|m| rec.id.starts_with(m))
        {
            let locator = match (&k.last_xpointer, k.last_page) {
                (Some(x), _) if rec.format == Format::Epub => Some(Locator::XPointer {
                    xpointer: x.clone(),
                }),
                (_, Some(p)) if matches!(rec.format, Format::Pdf | Format::Cbz) => {
                    Some(Locator::Page {
                        index: p.saturating_sub(1),
                    })
                }
                _ => None,
            };
            if let (Some(f), Some(locator)) = (k.percent_finished, locator) {
                let p = Progress {
                    fraction: f.clamp(0.0, 1.0) as f32,
                    locator,
                    updated: k.modified.unwrap_or(0),
                    source: Some("koreader".into()),
                    page: None,
                };
                candidates.push((p, k.status));
            }
        }
    }
    if let Some(st) = &sc.kindle {
        if let (Some((pos, ts)), Format::Mobi | Format::Azw3) = (&st.last_read, rec.format) {
            if let Some(position) = sidecar::kindle_position(pos) {
                let lens = crate::mobi::text_lengths(abs).ok();
                let kf8 = sc
                    .kindle_ext
                    .as_deref()
                    .is_some_and(|e| e.starts_with("azw3"));
                let len = lens
                    .and_then(|(m, k)| if kf8 { k.or(Some(m)) } else { Some(m) })
                    .filter(|&l| l > 0);
                // The text length is exact; the page map is the fallback
                // (its last page start is close to the end of the text).
                let fraction = match len {
                    Some(len) => Some(position as f64 / len as f64),
                    None => sc.page_map.as_ref().and_then(|m| m.fraction(position)),
                };
                if let Some(fraction) = fraction {
                    let page = sc
                        .page_map
                        .as_ref()
                        .and_then(|m| Some(m.label(m.page_index(position)?)));
                    let p = Progress {
                        fraction: fraction.clamp(0.0, 1.0) as f32,
                        locator: Locator::Kindle { position },
                        updated: ts.map(|t| t / 1000).unwrap_or(0),
                        source: Some("kindle".into()),
                        page,
                    };
                    candidates.push((p, None));
                }
            }
        }
        for a in &st.annotations {
            out.annotations += usize::from(merge_annotation(rec, a.clone()));
        }
    }
    if let Some((p, status)) = candidates.into_iter().max_by_key(|(p, _)| p.updated) {
        if rec
            .progress
            .as_ref()
            .is_none_or(|old| old.updated < p.updated)
        {
            rec.status = match status {
                Some(s) => s,
                None if p.fraction >= 0.99 => Status::Finished,
                None => Status::Reading,
            };
            rec.progress = Some(p);
            out.progress = true;
        }
    }
    out
}

fn merge_annotation(rec: &mut BookRecord, a: Annotation) -> bool {
    let dup = rec.annotations.iter().any(|x| {
        x.kind == a.kind
            && x.location == a.location
            && x.page == a.page
            && x.text == a.text
            && (x.position == a.position || x.position.is_none() || a.position.is_none())
    });
    if dup {
        return false;
    }
    rec.annotations.push(a);
    true
}

fn walk(
    root: &Path,
    dir: &Path,
    skip: &HashSet<String>,
    out: &mut Vec<Found>,
    errors: &mut Vec<(String, String)>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            errors.push((dir.to_string_lossy().into_owned(), e.to_string()));
            return;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            errors.push((
                entry.path().to_string_lossy().into_owned(),
                "file name is not UTF-8".into(),
            ));
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        let lower = name.to_lowercase();
        if ft.is_dir() {
            if lower.ends_with(".sdr") || skip.contains(&lower) {
                continue;
            }
            walk(root, &path, skip, out, errors);
            continue;
        }
        // Symlinks to files are followed, symlinks to folders are not (loops).
        let Ok(md) = fs::metadata(&path) else {
            continue;
        };
        if !md.is_file() || !format::is_candidate(&path) {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path);
        let rel_str = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        // On the device books live in documents/<Author>/; the first folder
        // below the root is the author fallback.
        let author_dir = if rel.components().count() > 1 {
            rel.components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
        } else {
            None
        };
        let (mtime, mtime_ns) = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| (d.as_secs() as i64, d.subsec_nanos()))
            .unwrap_or((0, 0));
        out.push(Found {
            abs: path,
            stamp: FileStamp {
                path: rel_str,
                size: md.len(),
                mtime,
                mtime_ns,
            },
            author_dir,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::AnnotationKind;

    /// The device case that once imported nothing: a MOBI under an author
    /// folder whose `.sdr` holds `.mbs`, `.mbp1` and `.apnx` in the layouts
    /// seen on firmware 5.13.6, with an `lpr` of "pos:pos:n:base64" form.
    #[test]
    fn device_layout_sidecars_import_position_and_percent() {
        let dir = crate::testdir("device-sdr");
        let root = dir.join("documents");
        let book = root.join("Adam Mickiewicz/Pan Tadeusz - Adam Mickiewicz.mobi");
        let sdr = root.join("Adam Mickiewicz/Pan Tadeusz - Adam Mickiewicz.sdr");
        fs::create_dir_all(&sdr).unwrap();
        // No text length in record 0, so the page map supplies the percent.
        fs::write(
            &book,
            crate::mobi::testutil::build("Pan Tadeusz", &[(100, "Adam Mickiewicz".as_bytes())], &[], 6),
        )
        .unwrap();
        let pages = sidecar::testutil::even_pages(180);
        let lpr = "7510:7510:11:REFUQQAAAHxFQkFSAAAAAQAAAABFQlZT";
        fs::write(
            sdr.join("Pan Tadeusz - Adam Mickiewicz.mbs"),
            sidecar::testutil::device_mbs(lpr, "84521", 1_792_229_945_807),
        )
        .unwrap();
        fs::write(
            sdr.join("Pan Tadeusz - Adam Mickiewicz.mbp1"),
            sidecar::testutil::device_mbp1(&pages),
        )
        .unwrap();
        fs::write(
            sdr.join("Pan Tadeusz - Adam Mickiewicz.apnx"),
            sidecar::testutil::apnx(&pages, "(1,a,1)"),
        )
        .unwrap();

        // Scan without the automatic import, then import explicitly, as the
        // CLI's import-sidecars does.
        let mut lib = Library::open(dir.join("data")).unwrap();
        let opts = ScanOptions {
            import_sidecars: false,
            covers: false,
            ..ScanOptions::default()
        };
        lib.scan(&root, &opts).unwrap();
        assert!(lib.books()[0].progress.is_none());
        let r = lib.import_sidecars();
        assert_eq!((r.changed, r.with_sidecars, r.errors.len()), (1, 1, 0));
        let b = &lib.books()[0];
        let p = b.progress.as_ref().unwrap();
        assert_eq!(p.locator, Locator::Kindle { position: 7510 });
        assert_eq!(
            (p.page.as_deref(), p.updated, b.status),
            (Some("4"), 1_792_229_945, Status::Reading)
        );
        assert_eq!((p.fraction * 100.0).round(), 2.0, "stock reader shows 2 %");

        // With the text length known, it is used instead (exact).
        let mut mobi = crate::mobi::testutil::build("Pan Tadeusz", &[], &[], 6);
        mobi[88 + 4..88 + 8].copy_from_slice(&375_500u32.to_be_bytes());
        fs::write(&book, mobi).unwrap();
        let mut lib = Library::open(dir.join("data2")).unwrap();
        lib.scan(
            &root,
            &ScanOptions {
                covers: false,
                ..ScanOptions::default()
            },
        )
        .unwrap();
        let p = lib.books()[0].progress.clone().unwrap();
        assert!((p.fraction - 0.02).abs() < 1e-6, "{}", p.fraction);
    }

    #[test]
    fn kindle_sidecar_progress_and_annotations() {
        let dir = crate::testdir("kindle-sdr");
        let root = dir.join("documents");
        fs::create_dir_all(root.join("Prus/Lalka.sdr")).unwrap();
        let mut mobi = crate::mobi::testutil::build("Lalka", &[], &[], 6);
        // Record 0 starts after the 78-byte header, one 8-byte entry and 2 pad
        // bytes; its bytes 4..8 hold the uncompressed text length.
        mobi[88 + 4..88 + 8].copy_from_slice(&30_000u32.to_be_bytes());
        fs::write(root.join("Prus/Lalka.mobi"), mobi).unwrap();
        fs::write(
            root.join("Prus/Lalka.sdr/Lalka.mbs"),
            sidecar::testutil::sample(),
        )
        .unwrap();

        let mut lib = Library::open(dir.join("data")).unwrap();
        let r = lib.scan(&root, &ScanOptions::default()).unwrap();
        assert_eq!((r.added, r.sidecars_imported), (1, 1));
        let b = &lib.books()[0];
        assert_eq!(b.status, Status::Reading);
        let p = b.progress.as_ref().unwrap();
        assert_eq!(
            (p.fraction, &p.locator, p.updated),
            (0.5, &Locator::Kindle { position: 15_000 }, 1_702_585_665)
        );
        assert_eq!(
            b.annotations
                .iter()
                .map(|a| a.kind.clone())
                .collect::<Vec<_>>(),
            [AnnotationKind::Highlight, AnnotationKind::Note]
        );

        // Importing again adds nothing and never rewinds newer progress.
        let id = b.id.clone();
        lib.set_progress(
            &id,
            Progress {
                fraction: 0.9,
                locator: Locator::Kindle { position: 27_000 },
                updated: 2_000_000_000,
                source: None,
                page: None,
            },
        );
        assert_eq!(lib.import_sidecars().changed, 0);
        assert_eq!(
            lib.book(&id).unwrap().progress.as_ref().unwrap().fraction,
            0.9
        );
    }
}
