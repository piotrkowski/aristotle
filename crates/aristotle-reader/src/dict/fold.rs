//! The fold index: every headword and synonym of a dictionary under a
//! folded key (lower case, no diacritics), sorted, on disk.
//!
//! Why: StarDict files are sorted by their exact spelling, so a plain
//! binary search finds "ψυχή" but not "ψυχὴ" (grave accent, as Greek is
//! written mid-sentence), "ψυχη" or "ψῡχή" (the Greek-English dictionary
//! marks vowel length in its headwords). Folding makes lookups accent-,
//! breathing- and case-insensitive for Greek, and diacritic-insensitive for
//! Polish and Latin, with one binary search.
//!
//! The index is built once per dictionary (about 1 s per million entries on
//! the host; [`build`]) and stored in the library data directory. Lookups
//! keep only every 32nd key in memory and read the rest with positioned
//! reads, so a 1.8 million entry dictionary costs about 1 MB of RAM.
//!
//! File layout (all integers big-endian):
//!
//! ```text
//! header   "ARFOLD01", u32 records, u32 every, u64 records_len, u32 checkpoints, u32 idx_count
//! records  key bytes, 0, u32 ref       sorted by key, then ref
//! checks   u32 record offset, u16 key length, key bytes    every `every`-th record
//! idx      u32 byte offset of each .idx entry, by entry number
//! ```
//!
//! `ref` points at the entry the key came from: bit 31 set for a `.syn`
//! entry, clear for `.idx`; the low 31 bits are its byte offset in that
//! file.

use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::FileExt;
use std::path::Path;

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use super::stardict::Index;

const MAGIC: &[u8; 8] = b"ARFOLD01";
const HEADER: u64 = 32;
const EVERY: u32 = 32;
/// Marks a reference into the `.syn` file.
pub const SYN: u32 = 1 << 31;

/// The lookup key of a word: lower case, decomposed, combining marks
/// (accents, breathings, iota subscripts, Polish ogonki and kreski)
/// dropped, final sigma as sigma, and the apostrophes of Greek elision
/// unified. "Ἀρετὴ" and "αρετη" share a key; "łaska" and "laska" do not,
/// since "ł" is a letter of its own, not a decorated "l".
pub fn fold(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.nfd() {
        if is_combining_mark(c) || c == '\u{00AD}' {
            continue;
        }
        match c {
            'ς' | 'ϲ' => out.push('σ'),
            '\u{2019}' | '\u{02BC}' | '\u{1FBD}' | '\u{1FBF}' => out.push('\''),
            _ => out.extend(c.to_lowercase()),
        }
    }
    out
}

/// A loaded fold index.
pub struct FoldIndex {
    file: File,
    records_len: u64,
    /// (record offset, key) of every `EVERY`-th record.
    checks: Vec<(u32, Box<[u8]>)>,
    idx_table: u64,
    idx_count: u32,
}

impl FoldIndex {
    pub fn open(path: &Path) -> io::Result<FoldIndex> {
        let file = File::open(path)?;
        let mut h = [0u8; HEADER as usize];
        file.read_exact_at(&mut h, 0)?;
        if &h[..8] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a fold index",
            ));
        }
        let be32 = |i: usize| u32::from_be_bytes(h[i..i + 4].try_into().unwrap());
        let records_len = u64::from_be_bytes(h[16..24].try_into().unwrap());
        let n_checks = be32(24) as usize;
        let idx_count = be32(28);
        let checks_at = HEADER + records_len;
        let len = file.metadata()?.len();
        let idx_table = len
            .checked_sub(idx_count as u64 * 4)
            .filter(|&t| t >= checks_at)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "truncated fold index"))?;
        let mut raw = vec![0u8; (idx_table - checks_at) as usize];
        file.read_exact_at(&mut raw, checks_at)?;
        let mut checks = Vec::with_capacity(n_checks);
        let mut i = 0;
        while i + 6 <= raw.len() && checks.len() < n_checks {
            let off = u32::from_be_bytes(raw[i..i + 4].try_into().unwrap());
            let kl = u16::from_be_bytes(raw[i + 4..i + 6].try_into().unwrap()) as usize;
            let key = raw
                .get(i + 6..i + 6 + kl)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad checkpoint"))?;
            checks.push((off, key.into()));
            i += 6 + kl;
        }
        Ok(FoldIndex {
            file,
            records_len,
            checks,
            idx_table,
            idx_count,
        })
    }

    /// Byte offset of `.idx` entry number `n`.
    pub fn idx_offset(&self, n: u32) -> io::Result<u64> {
        if n >= self.idx_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bad .idx number",
            ));
        }
        let mut b = [0u8; 4];
        self.file
            .read_exact_at(&mut b, self.idx_table + n as u64 * 4)?;
        Ok(u32::from_be_bytes(b) as u64)
    }

    /// The records of block `j` (between checkpoints j and j+1).
    fn block(&self, j: usize) -> io::Result<Vec<(Vec<u8>, u32)>> {
        let start = self.checks[j].0 as u64;
        let end = self
            .checks
            .get(j + 1)
            .map_or(self.records_len, |c| c.0 as u64);
        let mut raw = vec![0u8; (end - start) as usize];
        self.file.read_exact_at(&mut raw, HEADER + start)?;
        let mut out = Vec::with_capacity(EVERY as usize);
        let mut i = 0;
        while i < raw.len() {
            let Some(z) = raw[i..].iter().position(|&b| b == 0) else {
                break;
            };
            if i + z + 5 > raw.len() {
                break;
            }
            let r = u32::from_be_bytes(raw[i + z + 1..i + z + 5].try_into().unwrap());
            out.push((raw[i..i + z].to_vec(), r));
            i += z + 5;
        }
        Ok(out)
    }

    /// The block where records with `key` would start.
    fn first_block(&self, key: &[u8]) -> usize {
        // Last checkpoint whose key is strictly smaller: equal keys may
        // begin in the block before a checkpoint that equals the key.
        self.checks
            .partition_point(|(_, k)| k.as_ref() < key)
            .saturating_sub(1)
    }

    /// References of every record whose key is exactly `key`.
    pub fn find(&self, key: &str) -> io::Result<Vec<u32>> {
        let key = key.as_bytes();
        let mut out = Vec::new();
        if self.checks.is_empty() {
            return Ok(out);
        }
        let mut j = self.first_block(key);
        while j < self.checks.len() {
            for (k, r) in self.block(j)? {
                match k.as_slice().cmp(key) {
                    std::cmp::Ordering::Less => {}
                    std::cmp::Ordering::Equal => out.push(r),
                    std::cmp::Ordering::Greater => return Ok(out),
                }
            }
            j += 1;
        }
        Ok(out)
    }

    /// Up to `n` records around where `key` would be, as (key, ref), for
    /// "did you mean" suggestions.
    pub fn around(&self, key: &str, n: usize) -> io::Result<Vec<(String, u32)>> {
        let key = key.as_bytes();
        if self.checks.is_empty() {
            return Ok(Vec::new());
        }
        let j = self.first_block(key);
        let mut recs = Vec::new();
        for b in j.saturating_sub(1)..(j + 2).min(self.checks.len()) {
            recs.extend(self.block(b)?);
        }
        let pos = recs.partition_point(|(k, _)| k.as_slice() < key);
        let from = pos.saturating_sub(n / 2);
        Ok(recs
            .into_iter()
            .skip(from)
            .take(n)
            .map(|(k, r)| (String::from_utf8_lossy(&k).into_owned(), r))
            .collect())
    }
}

/// Whether a synonym is worth indexing. The Wiktionary dictionaries carry
/// link targets such as "aegroto#la" and "dzień_świąteczny#pl" next to the
/// real forms; nobody reads those words in a book.
fn indexable(word: &str) -> bool {
    !word.is_empty() && !word.contains('#') && !word.contains('_')
}

/// What [`build`] found.
#[derive(Debug, Default)]
pub struct BuildStats {
    pub idx_entries: u32,
    pub syn_entries: u32,
    pub records: u32,
}

/// Build the fold index of a dictionary into `out` (written to a temporary
/// file and renamed, so a crash leaves no half index).
pub fn build(index: &Index, out: &Path) -> io::Result<BuildStats> {
    // Keys NUL-terminated in one arena; records as (key offset, ref,
    // target .idx number): 12 bytes each, about 30 MB for the largest of
    // the owner's dictionaries at the peak.
    let mut arena: Vec<u8> = Vec::new();
    let mut recs: Vec<(u32, u32, u32)> = Vec::new();
    let mut idx_offsets: Vec<u32> = Vec::new();
    let mut stats = BuildStats::default();
    let mut push = |arena: &mut Vec<u8>, key: &str, r: u32, target: u32| {
        if key.is_empty() || key.len() > 250 {
            return;
        }
        recs.push((arena.len() as u32, r, target));
        arena.extend_from_slice(key.as_bytes());
        arena.push(0);
    };
    index.walk_idx(|pos, word| {
        let n = idx_offsets.len() as u32;
        idx_offsets.push(pos as u32);
        push(&mut arena, &fold(word), pos as u32, n);
    })?;
    stats.idx_entries = idx_offsets.len() as u32;
    index.walk_syn(|pos, word, target| {
        stats.syn_entries += 1;
        if indexable(word) && pos < SYN as u64 {
            push(&mut arena, &fold(word), pos as u32 | SYN, target);
        }
    })?;
    let key = |o: u32| {
        let s = &arena[o as usize..];
        &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
    };
    // Headwords before synonyms for the same key and target, so the
    // dedup below keeps the headword.
    recs.sort_unstable_by(|a, b| {
        key(a.0)
            .cmp(key(b.0))
            .then(a.2.cmp(&b.2))
            .then((a.1 & SYN).cmp(&(b.1 & SYN)))
            .then(a.1.cmp(&b.1))
    });
    recs.dedup_by(|b, a| a.2 == b.2 && key(a.0) == key(b.0));
    stats.records = recs.len() as u32;

    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = out.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut w = BufWriter::new(File::create(&tmp)?);
        w.write_all(&[0u8; HEADER as usize])?;
        let mut checks = Vec::new();
        let mut pos = 0u64;
        for (i, &(o, r, _)) in recs.iter().enumerate() {
            let k = key(o);
            if (i as u32).is_multiple_of(EVERY) {
                checks.push((pos as u32, k));
            }
            w.write_all(k)?;
            w.write_all(&[0])?;
            w.write_all(&r.to_be_bytes())?;
            pos += k.len() as u64 + 5;
        }
        let records_len = pos;
        for (off, k) in &checks {
            w.write_all(&off.to_be_bytes())?;
            w.write_all(&(k.len() as u16).to_be_bytes())?;
            w.write_all(k)?;
        }
        for o in &idx_offsets {
            w.write_all(&o.to_be_bytes())?;
        }
        let mut h = Vec::with_capacity(HEADER as usize);
        h.extend_from_slice(MAGIC);
        h.extend_from_slice(&(recs.len() as u32).to_be_bytes());
        h.extend_from_slice(&EVERY.to_be_bytes());
        h.extend_from_slice(&records_len.to_be_bytes());
        h.extend_from_slice(&(checks.len() as u32).to_be_bytes());
        h.extend_from_slice(&(idx_offsets.len() as u32).to_be_bytes());
        let f = w.into_inner().map_err(|e| e.into_error())?;
        f.write_all_at(&h, 0)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, out)?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding() {
        // Greek: accents, breathings, iota subscript, length marks, case.
        assert_eq!(fold("Ἀρετὴ"), "αρετη");
        assert_eq!(fold("ψῡχῇ"), "ψυχη");
        assert_eq!(fold("ψυχή"), fold("ψῡ́χη"));
        assert_eq!(fold("λόγος"), "λογοσ");
        assert_eq!(fold("Σωκράτης"), "σωκρατησ");
        // Polish: diacritics go, ł stays a letter.
        assert_eq!(fold("Żółć"), "zołc");
        assert_ne!(fold("łaska"), fold("laska"));
        // Latin macrons.
        assert_eq!(fold("spīritus"), "spiritus");
        assert_eq!(fold("δ’"), "δ'");
    }
}
