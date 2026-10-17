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
//! The KRDS layout below comes from the firmware's `ObjectReader`,
//! `ObjectWriter` and `State` classes (`ReaderContentSDK.jar`), confirmed by
//! decompiling 5.13.6; it has not yet been checked against a sidecar from
//! our device. Everything is big-endian (`java.io.DataOutputStream`):
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
//! MOBI and AZW3 positions are decimal byte offsets into the book text.

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
        });
    }
}

/// Kindle "location" for a MOBI/AZW3 position: 150 bytes per location,
/// counted from 1. The constant is calibre's (`MAGIC_MOBI_CONSTANT`); the
/// firmware computes it in native code we have not traced.
pub fn kindle_location(position: &str) -> Option<u32> {
    position
        .trim()
        .parse::<u64>()
        .ok()
        .map(|p| (p / 150 + 1) as u32)
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
