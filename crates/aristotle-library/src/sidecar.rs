// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reading state left in `.sdr` folders by earlier readers.
//!
//! Two readers write there:
//!
//! - **The stock Kindle reader** keeps two files per book in the "KRDS"
//!   (Kindle Reader Data Store) serialisation: a "freq" file with the last
//!   and furthest page read and a "rare" file with annotations and font
//!   settings. Names by book class, read from firmware 5.13.6
//!   (`ReaderSDK.jar`, `AbstractBook` and subclasses):
//!   MOBI 7 `.mbs` / `.mbp1`, KF8 `.azw3f` / `.azw3r`, KFX `.yjf` / `.yjr`,
//!   PDF `.pdt` / `.pds`. The legacy `.mbp` (PalmDB) is only read once to
//!   migrate it and then deleted by 5.x firmware, so we do not parse it.
//! - **KOReader** keeps `metadata.<ext>.lua`, a Lua table with
//!   `percent_finished`, `last_xpointer` or `last_page`, and a summary status.
//!
//! The KRDS format is publicly documented: jhowell's KRDS notes on
//! MobileRead, the krdsrw project (<https://github.com/midrare/krdsrw>) and
//! the `kindle_formats` crate. The layout below is described in our own
//! words from those sources and checked byte for byte against `.mbs` and
//! `.mbp1` files on our device. It is here for interoperability, so that
//! positions and annotations carry over between the stock reader and
//! Aristotle; this module is our own code and includes no Amazon code.
//! Everything is big-endian (`java.io.DataOutputStream`):
//!
//! ```text
//! 00 00 00 00 00 1A B1 26        magic, untagged
//! 02 <i64 = 1>                   tagged long: version
//! 01 <i32 n>                     tagged int: number of top-level objects
//! n × object
//! object := FE <null flag> <u16 len + modified UTF-8 name> value* FF
//! value  := 00 bool | 01 i32 | 02 i64 | 03 utf | 04 f64 | 05 i16 | 06 f32
//!         | 07 i8 | 09 char | object
//! utf    := 01 (null) | 00 <u16 len> <modified UTF-8>
//! ```
//!
//! Objects we use: `lpr` (byte version 2, utf position, long epoch ms),
//! `fpr` (utf position, long ms, ...), and `annotation.cache.object` (int n,
//! then n × (int type, `saved.avl.interval.tree` object of
//! `annotation.personal.{bookmark,highlight,note}` objects holding utf start,
//! utf end, long created ms, long modified ms, utf tags, and for notes a utf
//! text)). Highlighted text is not stored in sidecars; only positions are.
//! MOBI positions start with a decimal byte offset into the book text (see
//! [`kindle_position`] for the forms seen on the device). The annotation
//! layout is not yet checked on the device: both device samples had none.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::index::{Annotation, AnnotationKind, Status};

const MAGIC: [u8; 8] = [0, 0, 0, 0, 0, 0x1A, 0xB1, 0x26];

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    Long(i64),
    Utf(Option<String>),
    Double(f64),
    Short(i16),
    Float(f32),
    Byte(i8),
    Char(u16),
    Object(String, Vec<Value>),
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let s = self
            .b
            .get(self.p..self.p + n)
            .ok_or_else(|| Error::format("krds: truncated"))?;
        self.p += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn utf_body(&mut self) -> Result<Option<String>> {
        if self.u8()? != 0 {
            return Ok(None);
        }
        let len = u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize;
        Ok(Some(modified_utf8(self.take(len)?)))
    }

    fn value(&mut self, depth: u32) -> Result<Value> {
        if depth > 16 {
            return Err(Error::format("krds: nesting too deep"));
        }
        let tag = self.u8()?;
        Ok(match tag {
            0 => Value::Bool(self.u8()? != 0),
            1 => Value::Int(i32::from_be_bytes(self.take(4)?.try_into().unwrap())),
            2 => Value::Long(i64::from_be_bytes(self.take(8)?.try_into().unwrap())),
            3 => Value::Utf(self.utf_body()?),
            4 => Value::Double(f64::from_be_bytes(self.take(8)?.try_into().unwrap())),
            5 => Value::Short(i16::from_be_bytes(self.take(2)?.try_into().unwrap())),
            6 => Value::Float(f32::from_be_bytes(self.take(4)?.try_into().unwrap())),
            7 => Value::Byte(self.u8()? as i8),
            9 => Value::Char(u16::from_be_bytes(self.take(2)?.try_into().unwrap())),
            0xFE => {
                let name = self.utf_body()?.unwrap_or_default();
                let mut children = Vec::new();
                loop {
                    if self.b.get(self.p) == Some(&0xFF) {
                        self.p += 1;
                        break;
                    }
                    children.push(self.value(depth + 1)?);
                }
                Value::Object(name, children)
            }
            t => {
                return Err(Error::format(format!(
                    "krds: unknown tag {t:#x} at {}",
                    self.p - 1
                )))
            }
        })
    }
}

/// Java's modified UTF-8: NUL as C0 80 and characters above U+FFFF as two
/// 3-byte surrogates. Decoded by hand because `str::from_utf8` rejects both.
fn modified_utf8(b: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(b) {
        return s.to_owned();
    }
    let mut units: Vec<u16> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c < 0x80 {
            units.push(c as u16);
            i += 1;
        } else if c & 0xE0 == 0xC0 && i + 1 < b.len() {
            units.push(((c as u16 & 0x1F) << 6) | (b[i + 1] as u16 & 0x3F));
            i += 2;
        } else if c & 0xF0 == 0xE0 && i + 2 < b.len() {
            units.push(
                ((c as u16 & 0x0F) << 12)
                    | ((b[i + 1] as u16 & 0x3F) << 6)
                    | (b[i + 2] as u16 & 0x3F),
            );
            i += 3;
        } else {
            units.push(0xFFFD);
            i += 1;
        }
    }
    char::decode_utf16(units)
        .map(|r| r.unwrap_or('\u{FFFD}'))
        .collect()
}

/// Parses a KRDS file into its top-level objects.
pub fn parse_krds(b: &[u8]) -> Result<Vec<Value>> {
    if !b.starts_with(&MAGIC) {
        return Err(Error::format("krds: bad magic"));
    }
    let mut r = Reader { b, p: 8 };
    match r.value(0)? {
        Value::Long(1) => {}
        v => return Err(Error::format(format!("krds: unsupported version {v:?}"))),
    }
    let Value::Int(n) = r.value(0)? else {
        return Err(Error::format("krds: bad object count"));
    };
    let mut out = Vec::new();
    for _ in 0..n.max(0) {
        out.push(r.value(0)?);
    }
    Ok(out)
}

/// What a stock sidecar pair tells us about a book.
#[derive(Debug, Default, PartialEq)]
pub struct KindleState {
    /// Last page read: format-specific position string and epoch ms.
    pub last_read: Option<(String, Option<i64>)>,
    /// Furthest page read.
    pub furthest: Option<(String, Option<i64>)>,
    pub annotations: Vec<Annotation>,
}

fn utfs(children: &[Value]) -> impl Iterator<Item = &str> {
    children.iter().filter_map(|v| {
        if let Value::Utf(Some(s)) = v {
            Some(s.as_str())
        } else {
            None
        }
    })
}

fn longs(children: &[Value]) -> impl Iterator<Item = i64> + '_ {
    children.iter().filter_map(|v| {
        if let Value::Long(l) = v {
            Some(*l)
        } else {
            None
        }
    })
}

pub fn kindle_state(objects: &[Value]) -> KindleState {
    let mut st = KindleState::default();
    for o in objects {
        let Value::Object(name, ch) = o else { continue };
        match name.as_str() {
            "lpr" => {
                st.last_read = utfs(ch)
                    .next()
                    .map(|p| (p.to_owned(), longs(ch).next().filter(|&t| t > 0)))
            }
            "fpr" => {
                st.furthest = utfs(ch)
                    .next()
                    .map(|p| (p.to_owned(), longs(ch).next().filter(|&t| t > 0)))
            }
            "annotation.cache.object" => collect_annotations(ch, &mut st.annotations),
            _ => {}
        }
    }
    st
}

fn collect_annotations(values: &[Value], out: &mut Vec<Annotation>) {
    for v in values {
        let Value::Object(name, ch) = v else { continue };
        let kind = match name.strip_prefix("annotation.personal.") {
            Some("bookmark") => AnnotationKind::Bookmark,
            Some("highlight") => AnnotationKind::Highlight,
            Some("note") => AnnotationKind::Note,
            Some(_) => continue,
            None => {
                collect_annotations(ch, out);
                continue;
            }
        };
        let strings: Vec<&str> = utfs(ch).collect();
        let (Some(start), Some(end)) = (strings.first(), strings.get(1)) else {
            continue;
        };
        let created = longs(ch).next().filter(|&t| t > 0).map(format_epoch_ms);
        let text = if kind == AnnotationKind::Note {
            strings.get(3).map(|s| s.to_string())
        } else {
            None
        };
        out.push(Annotation {
            kind,
            text,
            page: None,
            location: kindle_location(start).zip(kindle_location(end)),
            position: Some((start.to_string(), end.to_string())),
            created,
            source: "kindle-sidecar".into(),
            chapter: None,
            note: None,
        });
    }
}

/// Kindle "location" for a MOBI/AZW3 position: 150 bytes per location,
/// counted from 1. The constant is calibre's (`MAGIC_MOBI_CONSTANT`); the
/// firmware computes it in native code we have not traced.
pub fn kindle_location(position: &str) -> Option<u32> {
    kindle_position(position).map(|p| (p / 150 + 1) as u32)
}

/// The byte position in a stock MOBI/AZW3 position string.
///
/// Verified on device sidecars (`.mbs`, firmware 5.13.6): `lpr` holds
/// `"7510:7510:11:REFUQQAA…"`, that is two equal decimal positions, a small
/// number whose meaning we do not know, and a base64 blob that decodes to
/// "DATA…EBAR…EBVS…" reader state. `fpr` holds a bare `"84521"`. The
/// firmware also accepts `pos:` prefixed strings migrated from `.mbp`
/// files (read from the 5.13.6 code, not seen on the device). In every
/// form the first decimal field is the position.
pub fn kindle_position(s: &str) -> Option<u64> {
    let s = s.trim();
    let s = s.strip_prefix("pos:").unwrap_or(s);
    s.split(':').next()?.trim().parse().ok()
}

/// A Kindle page map (`.apnx`): the text position where each printed page
/// starts, plus the label scheme.
///
/// Layout verified on device files generated by calibre (big-endian):
///
/// ```text
/// u16 1, u16 1, u32 offset of the page header, u32 length of JSON 1
/// JSON 1   {"contentGuid":…,"asin":…,"cdeType":"EBOK","fileRevisionId":"1"}
/// page header: u16 1, u16 length of JSON 2, u16 page count, u16 32
/// JSON 2   {"asin":…,"pageMap":"(1,a,1)"}
/// u32 × page count   start position of each page
/// ```
///
/// The stock reader caches the same data in the `.mbp1` file under
/// `apnx.key`; we read the `.apnx` itself.
#[derive(Clone, Debug, PartialEq)]
pub struct PageMap {
    /// The `pageMap` label scheme, e.g. `(1,r,1),(13,a,1)`: from page 1
    /// roman numerals starting at i, from page 13 arabic starting at 1.
    pub labels: String,
    pub offsets: Vec<u32>,
}

impl PageMap {
    /// Zero-based index of the page containing `position`.
    pub fn page_index(&self, position: u64) -> Option<usize> {
        if self.offsets.is_empty() {
            return None;
        }
        Some(
            self.offsets
                .partition_point(|&o| o as u64 <= position)
                .saturating_sub(1),
        )
    }

    /// Reading fraction by pages, interpolated inside the current page. The
    /// last page is assumed as long as the one before it.
    pub fn fraction(&self, position: u64) -> Option<f64> {
        let i = self.page_index(position)?;
        let n = self.offsets.len();
        let start = self.offsets[i] as u64;
        let end = match self.offsets.get(i + 1) {
            Some(&e) => e as u64,
            None if n >= 2 => start + (self.offsets[n - 1] - self.offsets[n - 2]) as u64,
            None => start + 1,
        };
        let within = if end > start {
            (position.saturating_sub(start)) as f64 / (end - start) as f64
        } else {
            0.0
        };
        Some(((i as f64 + within.min(1.0)) / n as f64).min(1.0))
    }

    /// Printed label of a zero-based page index under the `pageMap` scheme.
    pub fn label(&self, index: usize) -> String {
        let page = index + 1;
        let mut current: Option<(usize, char, usize)> = None;
        for group in self.labels.split(')') {
            let parts: Vec<&str> = group
                .trim_start_matches([',', '(', ' '])
                .split(',')
                .collect();
            if parts.len() < 3 {
                continue;
            }
            let (Ok(from), Some(kind), Ok(first)) = (
                parts[0].trim().parse::<usize>(),
                parts[1].trim().chars().next(),
                parts[2].trim().parse::<usize>(),
            ) else {
                continue;
            };
            if from <= page {
                current = Some((from, kind, first));
            }
        }
        match current {
            Some((from, 'r', first)) => roman(first + page - from),
            Some((from, _, first)) => (first + page - from).to_string(),
            None => page.to_string(),
        }
    }
}

fn roman(mut n: usize) -> String {
    const TABLE: [(usize, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, r) in TABLE {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

pub fn parse_apnx(b: &[u8]) -> Result<PageMap> {
    let be16 = |at: usize| {
        b.get(at..at + 2)
            .map(|s| u16::from_be_bytes([s[0], s[1]]) as usize)
    };
    let be32 = |at: usize| {
        b.get(at..at + 4)
            .map(|s| u32::from_be_bytes(s.try_into().unwrap()))
    };
    let bad = || Error::format("apnx: truncated or not a page map");
    if be16(0) != Some(1) {
        return Err(Error::format("apnx: unknown version"));
    }
    let header = be32(4).ok_or_else(bad)? as usize;
    let json_len = be16(header + 2).ok_or_else(bad)?;
    let count = be16(header + 4).ok_or_else(bad)?;
    let json = b.get(header + 8..header + 8 + json_len).ok_or_else(bad)?;
    let json = String::from_utf8_lossy(json);
    let labels = json
        .split("\"pageMap\":\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .unwrap_or("(1,a,1)")
        .to_owned();
    let start = header + 8 + json_len;
    let offsets = (0..count)
        .map(|i| be32(start + 4 * i))
        .collect::<Option<Vec<u32>>>()
        .ok_or_else(bad)?;
    Ok(PageMap { labels, offsets })
}

/// UTC "YYYY-MM-DD HH:MM:SS" from epoch milliseconds (no time zone database
/// on purpose; the device clock is UTC anyway).
pub fn format_epoch_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// The `.sdr` folder of a book: `Lalka.mobi` → `Lalka.sdr`.
pub fn sdr_dir(book: &Path) -> PathBuf {
    let stem = book
        .file_stem()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    let mut name = stem;
    name.push(".sdr");
    book.with_file_name(name)
}

/// Freq and rare sidecar extensions of the stock reader.
const KRDS_EXTENSIONS: &[&str] = &["mbs", "mbp1", "azw3f", "azw3r", "yjf", "yjr", "pdt", "pds"];

/// Everything found in a book's `.sdr` folder.
#[derive(Debug, Default)]
pub struct Sidecars {
    pub kindle: Option<KindleState>,
    /// Which stock file kind held the last-read position (its extension).
    pub kindle_ext: Option<String>,
    pub koreader: Option<KoreaderState>,
    /// Page map from `<name>.apnx`, used for page labels and as a fallback
    /// for the reading fraction.
    pub page_map: Option<PageMap>,
    pub errors: Vec<String>,
}

pub fn read_sidecars(book: &Path) -> Sidecars {
    let mut sc = Sidecars::default();
    let dir = sdr_dir(book);
    let Ok(entries) = fs::read_dir(&dir) else {
        return sc;
    };
    let mut files: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    files.sort();
    let mut state = KindleState::default();
    let mut any = false;
    for f in &files {
        let ext = f
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if KRDS_EXTENSIONS.contains(&ext.as_str()) {
            match fs::read(f)
                .map_err(Error::from)
                .and_then(|b| parse_krds(&b))
            {
                Ok(objs) => {
                    let s = kindle_state(&objs);
                    if s.last_read.is_some() {
                        sc.kindle_ext = Some(ext.clone());
                    }
                    state.last_read = state.last_read.or(s.last_read);
                    state.furthest = state.furthest.or(s.furthest);
                    state.annotations.extend(s.annotations);
                    any = true;
                }
                Err(e) => sc.errors.push(format!("{}: {e}", f.display())),
            }
        } else if ext == "apnx" {
            match fs::read(f)
                .map_err(Error::from)
                .and_then(|b| parse_apnx(&b))
            {
                Ok(m) => sc.page_map = Some(m),
                Err(e) => sc.errors.push(format!("{}: {e}", f.display())),
            }
        } else if name.starts_with("metadata.") && name.ends_with(".lua") {
            match fs::read_to_string(f) {
                Ok(s) => {
                    let mut k = parse_koreader(&s);
                    k.modified = fs::metadata(f)
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(crate::library::unix_secs);
                    sc.koreader = Some(k);
                }
                Err(e) => sc.errors.push(format!("{}: {e}", f.display())),
            }
        }
    }
    if any {
        sc.kindle = Some(state);
    }
    sc
}

/// The parts of KOReader's `metadata.<ext>.lua` we import.
#[derive(Debug, Default, PartialEq)]
pub struct KoreaderState {
    pub percent_finished: Option<f64>,
    pub last_xpointer: Option<String>,
    pub last_page: Option<u32>,
    pub status: Option<Status>,
    pub partial_md5: Option<String>,
    pub modified: Option<i64>,
}

/// Extracts a few scalar keys from KOReader's sidecar. The file is a Lua
/// table literal written by KOReader's dump(); a line scan is enough because
/// each key sits on its own line as `["key"] = value,`.
pub fn parse_koreader(src: &str) -> KoreaderState {
    let mut k = KoreaderState::default();
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("[\"") else {
            continue;
        };
        let Some((key, value)) = rest.split_once("\"] = ") else {
            continue;
        };
        let value = value.trim_end_matches(',').trim();
        match key {
            "percent_finished" => {
                k.percent_finished = value.parse().ok().filter(|v: &f64| v.is_finite())
            }
            "last_xpointer" => k.last_xpointer = lua_string(value),
            "last_page" => k.last_page = value.parse().ok(),
            "partial_md5_checksum" => k.partial_md5 = lua_string(value),
            "status" => {
                k.status = match lua_string(value).as_deref() {
                    Some("complete") => Some(Status::Finished),
                    Some("reading") | Some("abandoned") => Some(Status::Reading),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    k
}

fn lua_string(v: &str) -> Option<String> {
    let inner = v.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

#[cfg(test)]
pub(crate) mod testutil {
    pub struct KrdsWriter(pub Vec<u8>);

    impl KrdsWriter {
        pub fn new(objects: i32) -> Self {
            let mut w = KrdsWriter(super::MAGIC.to_vec());
            w.long(1);
            w.int(objects);
            w
        }
        pub fn int(&mut self, v: i32) {
            self.0.push(1);
            self.0.extend_from_slice(&v.to_be_bytes());
        }
        pub fn long(&mut self, v: i64) {
            self.0.push(2);
            self.0.extend_from_slice(&v.to_be_bytes());
        }
        pub fn byte(&mut self, v: i8) {
            self.0.push(7);
            self.0.push(v as u8);
        }
        pub fn utf(&mut self, s: &str) {
            self.0.extend_from_slice(&[3, 0]);
            self.0.extend_from_slice(&(s.len() as u16).to_be_bytes());
            self.0.extend_from_slice(s.as_bytes());
        }
        pub fn begin(&mut self, name: &str) {
            self.0.extend_from_slice(&[0xFE, 0]);
            self.0.extend_from_slice(&(name.len() as u16).to_be_bytes());
            self.0.extend_from_slice(name.as_bytes());
        }
        pub fn end(&mut self) {
            self.0.push(0xFF);
        }
        pub fn double(&mut self, v: f64) {
            self.0.push(4);
            self.0.extend_from_slice(&v.to_be_bytes());
        }
        pub fn bool(&mut self, v: bool) {
            self.0.extend_from_slice(&[0, v as u8]);
        }
        pub fn null_utf(&mut self) {
            self.0.extend_from_slice(&[3, 1]);
        }
    }

    /// A `.mbs` freq file with the object order, nesting and value types of
    /// the device samples (firmware 5.13.6), but made-up values.
    pub fn device_mbs(lpr: &str, fpr: &str, lpr_ms: i64) -> Vec<u8> {
        let mut w = KrdsWriter::new(6);
        w.begin("timer.model");
        w.long(0);
        w.long(7_001_749);
        w.long(19_877);
        w.double(0.4204);
        w.begin("timer.average.calculator");
        w.int(0);
        w.int(1);
        w.int(2);
        w.begin("timer.average.calculator.distribution.normal");
        w.long(90);
        w.double(13_100.5);
        w.double(2_000_000.25);
        w.end();
        w.int(1);
        w.begin("timer.average.calculator.outliers");
        w.int(0);
        w.end();
        w.end();
        w.end();
        w.begin("fpr");
        w.utf(fpr);
        w.long(-1);
        w.long(-1);
        w.null_utf();
        w.null_utf();
        w.end();
        w.begin("book.info.store");
        w.long(25_226);
        w.double(0.53);
        w.end();
        w.begin("page.history.store");
        w.int(0);
        w.end();
        w.begin("whisperstore.migration.status");
        w.bool(false);
        w.bool(false);
        w.end();
        w.begin("lpr");
        w.byte(2);
        w.utf(lpr);
        w.long(lpr_ms);
        w.end();
        w.0
    }

    /// A `.mbp1` rare file as on the device: no annotations, and the cached
    /// page map under `apnx.key`.
    pub fn device_mbp1(offsets: &[u32]) -> Vec<u8> {
        let mut w = KrdsWriter::new(4);
        w.begin("next.in.series.info.data");
        w.utf("{\"suppressed\":false}");
        w.end();
        w.begin("annotation.cache.object");
        w.int(0);
        w.end();
        w.begin("apnx.key");
        w.utf("1");
        w.utf("EBOK");
        w.bool(true);
        w.int(offsets.len() as i32 + 1);
        w.int(0);
        for &o in offsets {
            w.int(o as i32);
        }
        w.int(1);
        w.int(offsets.len() as i32);
        w.int(offsets.len() as i32);
        w.utf("(1,a,1)");
        w.end();
        w.begin("ReaderMetrics");
        w.int(1);
        w.utf("bookLaunchedBefore");
        w.utf("true");
        w.end();
        w.0
    }

    /// An `.apnx` page map in calibre's layout, as found on the device.
    pub fn apnx(offsets: &[u32], page_map: &str) -> Vec<u8> {
        let json1 =
            br#"{"contentGuid":"0000abcd","asin":"test","cdeType":"EBOK","fileRevisionId":"1"}"#;
        let json2 = format!("{{\"asin\":\"test\",\"pageMap\":\"{page_map}\"}}");
        let mut out = Vec::new();
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&((12 + json1.len()) as u32).to_be_bytes());
        out.extend_from_slice(&(json1.len() as u32).to_be_bytes());
        out.extend_from_slice(json1);
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&(json2.len() as u16).to_be_bytes());
        out.extend_from_slice(&(offsets.len() as u16).to_be_bytes());
        out.extend_from_slice(&32u16.to_be_bytes());
        out.extend_from_slice(json2.as_bytes());
        for o in offsets {
            out.extend_from_slice(&o.to_be_bytes());
        }
        out
    }

    /// Page starts every 2300 positions, as calibre's fast page mapping
    /// produced for the device samples.
    pub fn even_pages(n: u32) -> Vec<u32> {
        (0..n).map(|i| i * 2300).collect()
    }

    /// A freq+rare pair as one file: lpr, fpr and two annotations.
    pub fn sample() -> Vec<u8> {
        let mut w = KrdsWriter::new(3);
        w.begin("lpr");
        w.byte(2);
        w.utf("15000");
        w.long(1_702_585_665_000);
        w.end();
        w.begin("fpr");
        w.utf("30000");
        w.long(1_702_585_665_000);
        w.long(-1);
        w.0.extend_from_slice(&[3, 1, 3, 1]);
        w.end();
        w.begin("annotation.cache.object");
        w.int(2);
        w.int(1);
        w.begin("saved.avl.interval.tree");
        w.int(1);
        w.begin("annotation.personal.highlight");
        w.utf("9000");
        w.utf("9450");
        w.long(1_702_585_665_000);
        w.long(1_702_585_665_000);
        w.utf("0\u{FFFC}0");
        w.end();
        w.end();
        w.int(2);
        w.begin("saved.avl.interval.tree");
        w.int(1);
        w.begin("annotation.personal.note");
        w.utf("9450");
        w.utf("9450");
        w.long(1_702_585_665_000);
        w.long(1_702_585_665_000);
        w.utf("0\u{FFFC}0");
        w.utf("Zob. też Gorgiasz");
        w.end();
        w.end();
        w.end();
        w.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_layout_mbs_and_mbp1() {
        let b = testutil::device_mbs("7510:7510:11:REFUQQAAAHxFQkFS", "84521", 1_792_200_000_000);
        let st = kindle_state(&parse_krds(&b).unwrap());
        let (lpr, t) = st.last_read.clone().unwrap();
        assert_eq!(
            (kindle_position(&lpr), t),
            (Some(7510), Some(1_792_200_000_000))
        );
        assert_eq!(kindle_position(&st.furthest.unwrap().0), Some(84521));
        assert!(st.annotations.is_empty());
        let rare = parse_krds(&testutil::device_mbp1(&testutil::even_pages(180))).unwrap();
        assert_eq!(rare.len(), 4);
        assert_eq!(kindle_state(&rare), KindleState::default());
    }

    #[test]
    fn position_strings() {
        assert_eq!(kindle_position("49114:49114:29:REFU"), Some(49114));
        assert_eq!(kindle_position("84521"), Some(84521));
        assert_eq!(kindle_position("pos:120:4:QUJD"), Some(120));
        assert_eq!(kindle_position("abc"), None);
        assert_eq!(kindle_location("7510:7510:11:x"), Some(51));
    }

    #[test]
    fn apnx_page_map() {
        let m = parse_apnx(&testutil::apnx(&testutil::even_pages(180), "(1,a,1)")).unwrap();
        assert_eq!((m.offsets.len(), m.labels.as_str()), (180, "(1,a,1)"));
        assert_eq!(m.page_index(7510), Some(3));
        assert_eq!(m.label(3), "4");
        // Page 4 of 180, 610 of its 2300 positions in: about 1.8 %, which
        // the stock reader shows as 2 %.
        let f = m.fraction(7510).unwrap();
        assert!((f - (3.0 + 610.0 / 2300.0) / 180.0).abs() < 1e-9, "{f}");
        assert_eq!(m.fraction(10_000_000), Some(1.0));

        let m = PageMap {
            labels: "(1,r,1),(5,a,1)".into(),
            offsets: vec![0, 10, 20, 30, 40, 50],
        };
        assert_eq!(
            [m.label(0), m.label(3), m.label(4), m.label(5)],
            ["i", "iv", "1", "2"]
        );
        assert!(parse_apnx(b"\0\x02rest").is_err());
        assert!(parse_apnx(&testutil::apnx(&[0, 1], "(1,a,1)")[..40]).is_err());
    }

    /// Parses every sidecar under `$ARISTOTLE_SDR_SAMPLES` (copies of real
    /// `.sdr` folders, never committed) and prints what we would import.
    #[test]
    #[ignore = "needs real sidecars in $ARISTOTLE_SDR_SAMPLES"]
    fn device_samples() {
        let root = PathBuf::from(
            std::env::var_os("ARISTOTLE_SDR_SAMPLES").expect("set ARISTOTLE_SDR_SAMPLES"),
        );
        let mut seen = 0;
        for sdr in fs::read_dir(&root).unwrap().flatten() {
            for f in fs::read_dir(sdr.path()).unwrap().flatten() {
                let p = f.path();
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_owned();
                let b = fs::read(&p).unwrap();
                if KRDS_EXTENSIONS.contains(&ext.as_str()) {
                    let objs = parse_krds(&b).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                    let names: Vec<&str> = objs
                        .iter()
                        .filter_map(|o| {
                            if let Value::Object(n, _) = o {
                                Some(n.as_str())
                            } else {
                                None
                            }
                        })
                        .collect();
                    let st = kindle_state(&objs);
                    println!(
                        "{}: {names:?}\n  lpr {:?}\n  fpr {:?}\n  {} annotations",
                        p.display(),
                        st.last_read,
                        st.furthest,
                        st.annotations.len()
                    );
                    seen += 1;
                } else if ext == "apnx" {
                    let m = parse_apnx(&b).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
                    println!(
                        "{}: {} pages, map {}, last start {:?}",
                        p.display(),
                        m.offsets.len(),
                        m.labels,
                        m.offsets.last()
                    );
                    seen += 1;
                } else if ext == "lua" {
                    println!(
                        "{}: {:?}",
                        p.display(),
                        parse_koreader(&String::from_utf8_lossy(&b))
                    );
                    seen += 1;
                }
            }
        }
        assert!(seen > 0);
    }

    #[test]
    fn parses_krds_state() {
        let objs = parse_krds(&testutil::sample()).unwrap();
        let st = kindle_state(&objs);
        assert_eq!(
            st.last_read,
            Some(("15000".into(), Some(1_702_585_665_000)))
        );
        assert_eq!(st.furthest, Some(("30000".into(), Some(1_702_585_665_000))));
        assert_eq!(st.annotations.len(), 2);
        let h = &st.annotations[0];
        assert_eq!(
            (h.kind.clone(), h.location),
            (AnnotationKind::Highlight, Some((61, 64)))
        );
        assert_eq!(h.created.as_deref(), Some("2023-12-14 20:27:45"));
        let n = &st.annotations[1];
        assert_eq!(
            (n.kind.clone(), n.text.as_deref()),
            (AnnotationKind::Note, Some("Zob. też Gorgiasz"))
        );
    }

    #[test]
    fn rejects_bad_krds() {
        assert!(parse_krds(b"not a sidecar").is_err());
        let mut b = testutil::sample();
        b.truncate(b.len() - 10);
        assert!(parse_krds(&b).is_err());
    }

    #[test]
    fn modified_utf8_nul_and_surrogates() {
        assert_eq!(modified_utf8(&[b'a', 0xC0, 0x80, b'b']), "a\0b");
        // U+1F600 as CESU-8 surrogates.
        assert_eq!(modified_utf8(&[0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]), "😀");
    }

    #[test]
    fn koreader_metadata() {
        let lua = r#"-- we can read Lua syntax here!
return {
    ["cre_dom_version"] = 20240114,
    ["last_xpointer"] = "/body/DocFragment[12]/body/p[3]/text().0",
    ["partial_md5_checksum"] = "5d41402abc4b2a76b9719d911017c592",
    ["percent_finished"] = 0.4545,
    ["summary"] = {
        ["modified"] = "2024-01-01",
        ["status"] = "complete",
    },
}"#;
        let k = parse_koreader(lua);
        assert_eq!(k.percent_finished, Some(0.4545));
        assert_eq!(
            k.last_xpointer.as_deref(),
            Some("/body/DocFragment[12]/body/p[3]/text().0")
        );
        assert_eq!(k.status, Some(Status::Finished));
        assert_eq!(
            k.partial_md5.as_deref(),
            Some("5d41402abc4b2a76b9719d911017c592")
        );
    }

    #[test]
    fn sdr_path() {
        assert_eq!(
            sdr_dir(Path::new("/mnt/us/documents/Prus/Lalka.mobi")),
            PathBuf::from("/mnt/us/documents/Prus/Lalka.sdr")
        );
    }
}
