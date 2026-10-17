//! Page snapshots: the page a book was left at, kept on disk, so reopening
//! the book puts that page on the panel before any layout.
//!
//! Plato finds page breaks only by laying out the chapter that holds a
//! position from the chapter's start, and some books are one huge chapter
//! (a MOBI without page breaks converts to one): about 2 s on the device
//! before the first page could be drawn. But the page is already known: it
//! is the one the reader left. So whenever progress is saved (a pause in
//! reading, back to the library, quitting), the rendered current page, and
//! the next one when it is prerendered, go to `snapshots/<book id>.snap` in
//! the library data directory:
//!
//! - **Pixels:** the page buffer itself, 8-bit gray with the framebuffer's
//!   stride, each page compressed with zstd at level [`LEVEL`] (text pages
//!   shrink about tenfold; see docs/reader.md for sizes and decode times).
//! - **Key:** book id, the file's size and mtime, a hash of the look (every
//!   [`Style`] field, the layout version, the app build) and the progress
//!   locator saved with the page. A snapshot is shown only if all of these
//!   equal what the index and the preferences say now; a style change, a
//!   new build, a replaced file or a position imported from the stock
//!   reader all make it stale, and it is simply overwritten at the next save.
//!
//! What the snapshot shows is checked once the real page exists (see
//! `open.rs`): another location means a redraw, a different header or
//! footer means a partial update of just those pixels.
//!
//! The page chrome shows the chapter title, the page number and the
//! percentage, all functions of the location; it has no live fields (no
//! clock, no battery), so a snapshot cannot show a stale one. Anything live
//! added to the page chrome must be left out of saved pages, or redrawn
//! before showing them.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use aristotle_library::Locator;
use aristotle_render::page::Page;
use aristotle_render::style::Style;
use serde::{Deserialize, Serialize};

use crate::pagecache::LAYOUT_VERSION;

pub const DIR: &str = "snapshots";

const MAGIC: &[u8; 8] = b"ARSNAP01";

/// Bump when the file layout changes.
const FORMAT_VERSION: u32 = 1;

/// zstd level: 1 is the fastest to compress and decodes as fast as any;
/// higher levels gain little on anti-aliased text.
pub const LEVEL: i32 = 1;

/// Everything a snapshot must agree with to be shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    pub book: String,
    pub size: u64,
    pub mtime: i64,
    /// [`look`] of the style the page was rendered with.
    pub look: u64,
    /// The progress locator stored with the page.
    pub locator: Locator,
}

impl Key {
    pub fn new(book: &str, stamp: (u64, i64), style: &Style, locator: Locator) -> Key {
        Key {
            book: book.to_string(),
            size: stamp.0,
            mtime: stamp.1,
            look: look(style),
            locator,
        }
    }
}

/// A hash of everything that decides how a page looks, apart from the book
/// and the position: all style fields (by their `Debug` form, so a field
/// added later counts without anyone remembering this function), the
/// layout version and the build of the app.
pub fn look(style: &Style) -> u64 {
    let mut h = DefaultHasher::new();
    FORMAT_VERSION.hash(&mut h);
    LAYOUT_VERSION.hash(&mut h);
    format!("{style:?}").hash(&mut h);
    build_stamp().hash(&mut h);
    h.finish()
}

/// Size and mtime of the running binary: a new build may draw pages
/// differently, so its snapshots are not trusted.
fn build_stamp() -> (u64, i64) {
    static STAMP: OnceLock<(u64, i64)> = OnceLock::new();
    *STAMP.get_or_init(|| {
        use std::os::unix::fs::MetadataExt;
        std::env::current_exe()
            .and_then(std::fs::metadata)
            .map(|m| (m.len(), m.mtime()))
            .unwrap_or_default()
    })
}

pub fn path(data_dir: &Path, book: &str) -> PathBuf {
    data_dir.join(DIR).join(format!("{book}.snap"))
}

#[derive(Serialize, Deserialize)]
struct Header {
    key: Key,
    width: u32,
    height: u32,
    stride: usize,
    /// The stored pages: the current one, then the next one if known.
    pages: Vec<PageInfo>,
    /// Compressed size of each page, in the same order.
    sizes: Vec<usize>,
}

/// What the reader knows about a stored page without the book: enough to
/// fill the menu and to save the progress while the book is still opening.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageInfo {
    /// Page start (a Plato location).
    pub location: usize,
    /// The progress locator of this page.
    pub locator: Locator,
    pub fraction: f32,
    /// The menu's chapter title and page label ("Strona x z y" or a
    /// percentage).
    pub chapter: String,
    pub label: String,
}

/// A snapshot to store: the current page and, if known, the next one.
pub struct Snapshot {
    pub key: Key,
    pub pages: Vec<(PageInfo, Page)>,
}

/// Compress `s` into the file format.
pub fn encode(s: &Snapshot) -> io::Result<Vec<u8>> {
    let first = &s
        .pages
        .first()
        .ok_or_else(|| io::Error::other("no page"))?
        .1;
    let mut blobs = Vec::with_capacity(s.pages.len());
    for (_, p) in &s.pages {
        blobs.push(zstd::bulk::compress(&p.data, LEVEL)?);
    }
    let header = Header {
        key: s.key.clone(),
        width: first.width,
        height: first.height,
        stride: first.stride,
        pages: s.pages.iter().map(|(i, _)| i.clone()).collect(),
        sizes: blobs.iter().map(Vec::len).collect(),
    };
    let json = serde_json::to_vec(&header).map_err(io::Error::other)?;
    let mut out = Vec::with_capacity(16 + json.len() + blobs.iter().map(Vec::len).sum::<usize>());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    for b in &blobs {
        out.extend_from_slice(b);
    }
    Ok(out)
}

/// Write `s` for its book, atomically. Returns the file size.
pub fn store(data_dir: &Path, s: &Snapshot) -> io::Result<usize> {
    let data = encode(s)?;
    aristotle_library::index::write_atomic(&path(data_dir, &s.key.book), &data)
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(data.len())
}

/// A snapshot read back: the current page decoded, the next one still
/// compressed until it is needed.
pub struct Loaded {
    pub info: PageInfo,
    pub page: Rc<Page>,
    next: Option<(PageInfo, Vec<u8>)>,
    geometry: (u32, u32, usize),
    pub bytes: usize,
    pub read: Duration,
    pub decode: Duration,
}

impl Loaded {
    /// Decode the stored next page (once; `None` afterwards or if damaged).
    pub fn take_next(&mut self) -> Option<(PageInfo, Page)> {
        let (info, blob) = self.next.take()?;
        let (w, h, stride) = self.geometry;
        Some((info, decode_page(&blob, w, h, stride)?))
    }
}

fn decode_page(blob: &[u8], width: u32, height: u32, stride: usize) -> Option<Page> {
    let len = stride * height as usize;
    let data = zstd::bulk::decompress(blob, len).ok()?;
    (data.len() == len).then_some(Page {
        width,
        height,
        stride,
        data,
    })
}

/// The snapshot of `key.book`, if there is one, it matches `key` and has
/// the geometry of `style`.
pub fn load(data_dir: &Path, key: &Key, style: &Style) -> Option<Loaded> {
    let t = Instant::now();
    let data = std::fs::read(path(data_dir, &key.book)).ok()?;
    let read = t.elapsed();
    let t = Instant::now();
    let mut loaded = parse(&data, key, style)?;
    loaded.read = read;
    loaded.decode = t.elapsed();
    Some(loaded)
}

/// The header and the compressed pages, if `data` is a whole snapshot for
/// `key` with the geometry of `style`.
fn parse_header<'a>(data: &'a [u8], key: &Key, style: &Style) -> Option<(Header, Vec<&'a [u8]>)> {
    if data.get(..8)? != MAGIC {
        return None;
    }
    let n = u32::from_le_bytes(data.get(8..12)?.try_into().ok()?) as usize;
    let header: Header = serde_json::from_slice(data.get(12..12 + n)?).ok()?;
    let geometry = (style.width, style.height, style.stride);
    if header.key != *key
        || (header.width, header.height, header.stride) != geometry
        || header.pages.len() != header.sizes.len()
        || header.pages.is_empty()
        || header.pages[0].locator != key.locator
    {
        return None;
    }
    let mut blobs = Vec::new();
    let mut at = 12 + n;
    for size in &header.sizes {
        blobs.push(data.get(at..at + size)?);
        at += size;
    }
    Some((header, blobs))
}

/// Whether a snapshot for `key` exists, without decoding its pages.
pub fn is_valid(data_dir: &Path, key: &Key, style: &Style) -> bool {
    std::fs::read(path(data_dir, &key.book))
        .ok()
        .is_some_and(|data| parse_header(&data, key, style).is_some())
}

/// [`load`] on bytes already read.
fn parse(data: &[u8], key: &Key, style: &Style) -> Option<Loaded> {
    let (header, blobs) = parse_header(data, key, style)?;
    let geometry = (style.width, style.height, style.stride);
    let page = decode_page(blobs[0], style.width, style.height, style.stride)?;
    let mut infos = header.pages.into_iter();
    let info = infos.next()?;
    Some(Loaded {
        info,
        page: Rc::new(page),
        next: infos.next().map(|i| (i, blobs[1].to_vec())),
        geometry,
        bytes: data.len(),
        read: Duration::ZERO,
        decode: Duration::ZERO,
    })
}

/// Stores snapshots on a thread of its own at a low priority, so the
/// compression and the write (an fsync on the FAT userstore) never sit
/// between a tap and its page. The newest pending snapshot wins.
pub struct Writer {
    tx: Option<Sender<Snapshot>>,
    thread: Option<thread::JoinHandle<()>>,
    /// What was last queued, to skip saving the same pages again.
    last: Option<(Key, Rc<Page>, Option<usize>)>,
}

impl Writer {
    pub fn new(data_dir: &Path) -> Writer {
        let dir = data_dir.to_path_buf();
        let (tx, rx) = mpsc::channel::<Snapshot>();
        let thread = thread::Builder::new()
            .name("snapshot-writer".into())
            .spawn(move || {
                // SAFETY: plain system calls on the calling thread.
                unsafe {
                    let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
                    libc::setpriority(libc::PRIO_PROCESS, tid, 10);
                }
                while let Ok(mut snap) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        snap = newer;
                    }
                    let t = Instant::now();
                    match store(&dir, &snap) {
                        Ok(n) => crate::perf!(
                            "snapshot: {} page(s) saved, {} KB in {:.0} ms",
                            snap.pages.len(),
                            n / 1024,
                            crate::ms(t.elapsed())
                        ),
                        Err(e) => eprintln!("aristotle: can't save a page snapshot: {e}"),
                    }
                }
            })
            .ok();
        Writer {
            tx: Some(tx),
            thread,
            last: None,
        }
    }

    /// Queue the current page (and the next one, if rendered) under `key`,
    /// unless exactly these pages were queued last time.
    pub fn save(
        &mut self,
        key: Key,
        current: (PageInfo, Rc<Page>),
        next: Option<(PageInfo, Rc<Page>)>,
    ) {
        let next_loc = next.as_ref().map(|(i, _)| i.location);
        if let Some((k, p, n)) = &self.last {
            if *k == key && Rc::ptr_eq(p, &current.1) && *n == next_loc {
                return;
            }
        }
        self.last = Some((key.clone(), current.1.clone(), next_loc));
        let mut pages = vec![(current.0, (*current.1).clone())];
        if let Some((i, p)) = next {
            pages.push((i, (*p).clone()));
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(Snapshot { key, pages });
        }
    }

    /// Wait for queued snapshots, e.g. before quitting.
    pub fn flush(mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(style: &Style, seed: u8) -> Page {
        let mut p = Page::white(style);
        // Some text-like gray runs, different per seed.
        for y in (200..1200).step_by(40) {
            for x in 60..1000 {
                p.data[y * p.stride + x] = ((x * 7 + y + seed as usize) % 256) as u8;
            }
        }
        p
    }

    const AT: Locator = Locator::Kindle { position: 7510 };

    fn key(style: &Style) -> Key {
        Key::new("0123abcd", (1000, 42), style, AT)
    }

    fn info(location: usize, locator: Locator) -> PageInfo {
        PageInfo {
            location,
            locator,
            fraction: 0.25,
            chapter: "Rozdział".into(),
            label: "25%".into(),
        }
    }

    #[test]
    fn round_trip_with_the_next_page() {
        let dir = crate::testdir("snapshot");
        let s = Style::pw4();
        let (a, b) = (page(&s, 1), page(&s, 2));
        let next = info(6200, Locator::Kindle { position: 8000 });
        let snap = Snapshot {
            key: key(&s),
            pages: vec![(info(5000, AT), a.clone()), (next.clone(), b.clone())],
        };
        let n = store(&dir, &snap).unwrap();
        // A text page compresses well below its 1.5 MB.
        assert!(n < a.data.len() / 4, "{n} bytes");
        let mut l = load(&dir, &key(&s), &s).unwrap();
        assert_eq!(l.info, info(5000, AT));
        assert_eq!(l.page.data, a.data);
        let (i, p) = l.take_next().unwrap();
        assert_eq!((i, p.data == b.data), (next, true));
        assert!(l.take_next().is_none());
    }

    #[test]
    fn any_key_change_invalidates() {
        let dir = crate::testdir("snapshot");
        let s = Style::pw4();
        let k = key(&s);
        let snap = Snapshot {
            key: k.clone(),
            pages: vec![(info(5000, AT), page(&s, 1))],
        };
        store(&dir, &snap).unwrap();
        assert!(load(&dir, &k, &s).is_some_and(|mut l| l.take_next().is_none()));

        let mut bigger = s.clone();
        bigger.font_size = 11.0;
        let mut german = s.clone();
        german.language = Some("de".into());
        let others = [
            Key {
                size: 1001,
                ..k.clone()
            },
            Key {
                mtime: 43,
                ..k.clone()
            },
            Key {
                locator: Locator::Kindle { position: 7511 },
                ..k.clone()
            },
            Key {
                locator: Locator::Epub {
                    spine: 0,
                    offset: 7510,
                },
                ..k.clone()
            },
            Key::new("0123abcd", (1000, 42), &bigger, AT),
            Key::new("0123abcd", (1000, 42), &german, AT),
        ];
        for other in &others {
            assert_ne!(*other, k);
            assert!(load(&dir, other, &s).is_none(), "{other:?}");
        }
        // Another book has no snapshot at all.
        let elsewhere = Key {
            book: "ffff".into(),
            ..k.clone()
        };
        assert!(load(&dir, &elsewhere, &s).is_none());
        // Same key, other screen geometry.
        let mut wide = s.clone();
        wide.stride = 1100;
        assert!(load(&dir, &k, &wide).is_none());
    }

    #[test]
    fn damaged_files_are_ignored() {
        let dir = crate::testdir("snapshot");
        let s = Style::pw4();
        let k = key(&s);
        let snap = Snapshot {
            key: k.clone(),
            pages: vec![(info(1, AT), page(&s, 3))],
        };
        let good = encode(&snap).unwrap();
        let file = path(&dir, &k.book);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        for bad in [
            &good[..good.len() - 10],
            &good[..20],
            b"ARSNAP01\xff\xff\xff\xff".as_slice(),
            b"something else".as_slice(),
        ] {
            std::fs::write(&file, bad).unwrap();
            assert!(load(&dir, &k, &s).is_none());
        }
        std::fs::write(&file, &good).unwrap();
        assert!(load(&dir, &k, &s).is_some());
        assert!(is_valid(&dir, &k, &s));
        std::fs::write(&file, &good[..good.len() - 1]).unwrap();
        assert!(!is_valid(&dir, &k, &s));
        std::fs::write(&file, &good).unwrap();
        // A first page saved for another locator than its key is not ours.
        let odd = Snapshot {
            key: k.clone(),
            pages: vec![(info(1, Locator::Kindle { position: 1 }), page(&s, 3))],
        };
        store(&dir, &odd).unwrap();
        assert!(load(&dir, &k, &s).is_none());
    }

    #[test]
    fn writer_skips_unchanged_pages_and_flushes() {
        let dir = crate::testdir("snapshot");
        let s = Style::pw4();
        let k = key(&s);
        let p = Rc::new(page(&s, 4));
        let mut w = Writer::new(&dir);
        w.save(k.clone(), (info(10, AT), p.clone()), None);
        let first = w.last.clone().unwrap();
        // The same pages again are not queued twice.
        w.save(k.clone(), (info(10, AT), p.clone()), None);
        assert!(Rc::ptr_eq(&w.last.as_ref().unwrap().1, &first.1));
        w.flush();
        let l = load(&dir, &k, &s).unwrap();
        assert_eq!(l.info.location, 10);
        assert_eq!(l.page.data, p.data);
    }
}
