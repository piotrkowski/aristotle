// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page numbers without laying out the whole book on every open.
//!
//! "x / y" needs the start of every page, and finding them means laying out
//! the whole book: 4 to 5 s on the device. The reader opens at the saved
//! position without it and gets the pages in two ways:
//!
//! - **On disk**, per book and style: `pages/<book id>-<key>.txt` in the
//!   library data directory, one page start per line. The key hashes
//!   everything that moves page breaks (the typography, the screen, the
//!   file's size and mtime, and [`LAYOUT_VERSION`]), so a stale file is
//!   never used; changing the font size simply looks up another file.
//! - **In the background** when there is no file: a thread opens its own
//!   copy of the book (Plato documents are independent, each with its own
//!   FreeType and MuPDF state), paginates it at the lowest CPU priority
//!   (`SCHED_IDLE`: on the PW4's single core it runs only while the reader
//!   waits for input, never during a page turn or prerender), sends the
//!   result back and writes the file.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use aristotle_render::book::Book;
use aristotle_render::style::Style;

/// Bump when an aristotle-render or Plato change can move page breaks for
/// the same style; old files are then ignored (and new ones written).
pub const LAYOUT_VERSION: u32 = 2;

pub const PAGES_DIR: &str = "pages";

/// The cache file for a book (by library ID and file stamp) and style.
pub fn cache_path(data_dir: &Path, book_id: &str, size: u64, mtime: i64, style: &Style) -> PathBuf {
    let mut h = DefaultHasher::new();
    LAYOUT_VERSION.hash(&mut h);
    size.hash(&mut h);
    mtime.hash(&mut h);
    style_key(style).hash(&mut h);
    data_dir
        .join(PAGES_DIR)
        .join(format!("{book_id}-{:016x}.txt", h.finish()))
}

/// Every style field that can move a page break, as text (floats by their
/// bits, so 10.0 and 10.000001 differ as they do for the layout).
fn style_key(s: &Style) -> String {
    format!(
        "{}x{}@{} {} {} {:08x} {:08x} m{} t{} b{} {:?}",
        s.width,
        s.height,
        s.dpi,
        s.font_family,
        s.font_dir,
        s.font_size.to_bits(),
        s.line_height.to_bits(),
        s.margin_mm,
        s.text_top,
        s.text_bottom,
        s.language
    )
}

pub fn load(path: &Path) -> Option<Vec<usize>> {
    let text = std::fs::read_to_string(path).ok()?;
    let pages: Vec<usize> = text
        .lines()
        .map(|l| l.trim().parse().ok())
        .collect::<Option<_>>()?;
    // Strictly increasing and non-empty, or it is not ours.
    (!pages.is_empty() && pages.windows(2).all(|w| w[0] < w[1])).then_some(pages)
}

pub fn store(path: &Path, pages: &[usize]) -> std::io::Result<()> {
    let mut text = String::with_capacity(pages.len() * 8);
    for p in pages {
        text.push_str(&p.to_string());
        text.push('\n');
    }
    aristotle_library::index::write_atomic(path, text.as_bytes())
        .map_err(|e| std::io::Error::other(e.to_string()))
}

/// A background pagination in progress.
pub struct Paginator {
    rx: Receiver<Vec<usize>>,
    cancel: Arc<AtomicBool>,
}

impl Paginator {
    /// Paginate `book` with `style` on a thread and store the result at
    /// `cache`. The data directory must already be the current directory
    /// (Plato's fonts are found relative to it).
    pub fn start(book: PathBuf, style: Style, mobi_cache: PathBuf, cache: PathBuf) -> Paginator {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancel);
        let spawned = thread::Builder::new()
            .name("paginate".into())
            .spawn(move || {
                lower_priority();
                let t = Instant::now();
                let mut b = match Book::open(&book, &style, Some(&mobi_cache)) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("aristotle: background pagination: {e:#}");
                        return;
                    }
                };
                let mut n = 0;
                while b.locate(n).is_some() {
                    if stop.load(Ordering::Relaxed) {
                        crate::perf!("paginate: cancelled after {n} pages");
                        return;
                    }
                    n += 1;
                }
                let pages = b.pagination().map(<[usize]>::to_vec).unwrap_or_default();
                crate::perf!(
                    "paginate: {} pages in {:.0} ms (background, idle priority)",
                    pages.len(),
                    crate::ms(t.elapsed())
                );
                if pages.is_empty() {
                    return;
                }
                if let Err(e) = store(&cache, &pages) {
                    eprintln!("aristotle: can't store {}: {e}", cache.display());
                }
                let _ = tx.send(pages);
            });
        if let Err(e) = spawned {
            eprintln!("aristotle: can't start the pagination thread: {e}");
        }
        Paginator { rx, cancel }
    }

    /// The pages, once the thread is done; `Some(None)` if it gave up.
    pub fn poll(&self) -> Option<Option<Vec<usize>>> {
        match self.rx.try_recv() {
            Ok(p) => Some(Some(p)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(None),
        }
    }
}

impl Drop for Paginator {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Run the calling thread only when nothing else wants the CPU. musl's
/// `sched_setscheduler` is a stub that fails with ENOSYS (POSIX gives it
/// process-wide semantics Linux does not have), so the system call is made
/// directly; with pid 0 Linux applies it to the calling thread. Nice 19 as a
/// fallback.
pub(crate) fn lower_priority() {
    const SCHED_IDLE: libc::c_int = 5;
    // SAFETY: sched_param is plain data; priority 0 is what SCHED_IDLE takes.
    let param: libc::sched_param = unsafe { std::mem::zeroed() };
    // SAFETY: plain system calls on the calling thread with a valid pointer.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_sched_setscheduler,
            0,
            SCHED_IDLE,
            &param as *const libc::sched_param,
        )
    };
    if rc != 0 {
        let tid = unsafe { libc::syscall(libc::SYS_gettid) } as libc::id_t;
        unsafe { libc::setpriority(libc::PRIO_PROCESS, tid, 19) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_changes_with_anything_that_moves_pages() {
        let dir = Path::new("/data");
        let s = Style::pw4();
        let base = cache_path(dir, "abc", 100, 5, &s);
        assert_eq!(base, cache_path(dir, "abc", 100, 5, &s.clone()));
        assert!(base.starts_with("/data/pages"));
        let mut bigger = s.clone();
        bigger.font_size = 11.0;
        let mut looser = s.clone();
        looser.line_height = 1.5;
        let mut wider = s.clone();
        wider.margin_mm = 8;
        let mut german = s.clone();
        german.language = Some("de".into());
        for other in [
            cache_path(dir, "abc", 100, 5, &bigger),
            cache_path(dir, "abc", 100, 5, &looser),
            cache_path(dir, "abc", 100, 5, &wider),
            cache_path(dir, "abc", 100, 5, &german),
            cache_path(dir, "abc", 101, 5, &s),
            cache_path(dir, "abc", 100, 6, &s),
            cache_path(dir, "abd", 100, 5, &s),
        ] {
            assert_ne!(other, base);
        }
    }

    #[test]
    fn store_and_load() {
        let dir = crate::testdir("pages");
        let path = cache_path(&dir, "id", 1, 2, &Style::pw4());
        assert_eq!(load(&path), None);
        store(&path, &[0, 1200, 2500, 9000]).unwrap();
        assert_eq!(load(&path), Some(vec![0, 1200, 2500, 9000]));
        // Damaged or foreign files are ignored, not trusted.
        std::fs::write(&path, "0\n5\n3\n").unwrap();
        assert_eq!(load(&path), None);
        std::fs::write(&path, "0\nx\n").unwrap();
        assert_eq!(load(&path), None);
        std::fs::write(&path, "").unwrap();
        assert_eq!(load(&path), None);
    }
}
