// SPDX-License-Identifier: AGPL-3.0-or-later
//! MOBI (Mobipocket 7) books, converted to EPUB so Plato's own engine lays
//! them out.
//!
//! Plato reads MOBI through MuPDF, whose HTML layout has no hyphenation
//! patterns for Polish, ignores our font and justifies poorly. Converting the
//! book once to a plain EPUB in a cache directory gives MOBI books the same
//! typesetting as EPUB: Literata, Knuth–Plass justification and Liang
//! hyphenation in the language from the book's EXTH header.
//!
//! What is handled: PalmDOC-compressed or uncompressed text, UTF-8 or
//! CP1252, trailing record entries, EXTH title/author/language, images
//! (`recindex`), chapter splits at `<mbp:pagebreak/>`, MOBI's presentational
//! paragraph attributes, and the HTML-isms Plato's XML parser can't take
//! (void elements, unquoted attributes, unclosed paragraphs).
//!
//! Not handled, so [`crate::book::Book`] falls back to MuPDF: DRM,
//! HUFF/CDIC compression (Amazon store books), KF8-only files (AZW3). Joint
//! MOBI/KF8 files use their MOBI 7 part. Not converted yet: the table of
//! contents (NCX index).
//!
//! `filepos` links (`<a filepos=12345>`, a byte offset into the text: how
//! MOBI books link footnotes and their back links) become EPUB links: an
//! anchor `<a id="filepos12345"/>` is put where the offset falls, in
//! whichever chapter file that is, and the link gets
//! `href="ch0003.xhtml#filepos12345"`. calibre names its anchors the same
//! way, and the reader's footnote popups know them
//! ([`crate::notes::note_id`]).
//!
//! Format references: MobileRead wiki ("MOBI", "PalmDOC") and calibre's
//! `mobi/reader` (trailing entries).

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::position::PosMap;
use plato_core::anyhow::{bail, format_err, Context, Error};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Bump when the conversion output changes, to invalidate cached EPUBs.
const CONVERTER_VERSION: u32 = 3;

const COMPRESSION_NONE: u16 = 1;
const COMPRESSION_PALMDOC: u16 = 2;
const COMPRESSION_HUFFCDIC: u16 = 17480;

/// What the MOBI headers say; shown by `aristotle-render info`.
#[derive(Clone, Debug, Default)]
pub struct MobiInfo {
    pub title: Option<String>,
    pub author: Option<String>,
    pub language: Option<String>,
    pub compression: u16,
    pub encoding: u32,
    pub version: u32,
    pub encrypted: bool,
    /// Record index where a KF8 part starts, in joint MOBI/KF8 files.
    pub kf8_boundary: Option<u32>,
}

/// A decoded MOBI 7 book.
pub struct Mobi {
    pub info: MobiInfo,
    pub html: String,
    /// (recindex, extension, bytes)
    pub images: Vec<(u32, &'static str, Vec<u8>)>,
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

/// The records of a Palm database.
fn records(data: &[u8]) -> Result<Vec<&[u8]>, Error> {
    let n = u16_at(data, 76).ok_or_else(|| format_err!("truncated PalmDB header"))? as usize;
    let mut offsets = Vec::with_capacity(n + 1);
    for i in 0..n {
        let off = u32_at(data, 78 + 8 * i).ok_or_else(|| format_err!("truncated record list"))?;
        offsets.push(off as usize);
    }
    offsets.push(data.len());
    let mut out = Vec::with_capacity(n);
    for w in offsets.windows(2) {
        let (a, b) = (w[0].min(data.len()), w[1].min(data.len()));
        out.push(&data[a..b.max(a)]);
    }
    Ok(out)
}

/// Decompress PalmDOC (LZ77 variant) `src`, appending to `out`.
pub fn palmdoc_decompress(src: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < src.len() {
        let c = src[i];
        i += 1;
        match c {
            0x00 | 0x09..=0x7f => out.push(c),
            0x01..=0x08 => {
                let end = (i + c as usize).min(src.len());
                out.extend_from_slice(&src[i..end]);
                i = end;
            }
            0x80..=0xbf => {
                let Some(&next) = src.get(i) else { break };
                i += 1;
                let pair = ((c as usize) << 8) | next as usize;
                let distance = (pair >> 3) & 0x7ff;
                let length = (pair & 7) + 3;
                if distance == 0 || distance > out.len() {
                    continue;
                }
                let start = out.len() - distance;
                // Byte by byte: the source may overlap what is being written.
                for k in 0..length {
                    let b = out[start + k];
                    out.push(b);
                }
            }
            0xc0..=0xff => {
                out.push(b' ');
                out.push(c ^ 0x80);
            }
        }
    }
}

/// Size of the trailing entries at the end of a text record, per the
/// MOBI header's extra data flags: bits 1..15 each announce an entry whose
/// size is stored backwards as a varint at the very end; bit 0 a multibyte
/// overlap entry before them.
pub fn trailing_size(rec: &[u8], flags: u16) -> usize {
    let mut num = 0usize;
    let mut bits = flags >> 1;
    while bits != 0 {
        if bits & 1 != 0 {
            let end = rec.len().saturating_sub(num);
            let (mut value, mut shift, mut pos) = (0usize, 0, end);
            while pos > 0 {
                let v = rec[pos - 1];
                pos -= 1;
                value |= ((v & 0x7f) as usize) << shift;
                shift += 7;
                if v & 0x80 != 0 || shift >= 28 {
                    break;
                }
            }
            num += value;
        }
        bits >>= 1;
    }
    if flags & 1 != 0 {
        if let Some(&b) = rec.len().checked_sub(num + 1).and_then(|i| rec.get(i)) {
            num += (b & 3) as usize + 1;
        }
    }
    num.min(rec.len())
}

/// Windows-1252 to Unicode for 0x80..=0x9f; other bytes are Latin-1.
fn cp1252(b: u8) -> char {
    const HIGH: [u16; 32] = [
        0x20ac, 0x81, 0x201a, 0x192, 0x201e, 0x2026, 0x2020, 0x2021, 0x2c6, 0x2030, 0x160, 0x2039,
        0x152, 0x8d, 0x17d, 0x8f, 0x90, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022, 0x2013, 0x2014,
        0x2dc, 0x2122, 0x161, 0x203a, 0x153, 0x9d, 0x17e, 0x178,
    ];
    match b {
        0x80..=0x9f => char::from_u32(HIGH[(b - 0x80) as usize] as u32).unwrap_or('?'),
        _ => b as char,
    }
}

/// Primary language from a Windows LANGID, the MOBI header's fallback when
/// EXTH 524 is missing.
fn locale_language(locale: u32) -> Option<&'static str> {
    Some(match locale & 0xff {
        0x05 => "cs",
        0x07 => "de",
        0x09 => "en",
        0x0a => "es",
        0x0c => "fr",
        0x10 => "it",
        0x15 => "pl",
        0x16 => "pt",
        0x19 => "ru",
        0x1b => "sk",
        _ => return None,
    })
}

fn image_extension(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if data.starts_with(b"\x89PNG") {
        Some("png")
    } else if data.starts_with(b"GIF8") {
        Some("gif")
    } else if data.starts_with(b"BM") {
        Some("bmp")
    } else {
        None
    }
}

/// Parse and decode a MOBI file.
pub fn parse(data: &[u8]) -> Result<Mobi, Error> {
    parse_with(data, true)
}

/// Parse a MOBI file; without `text`, only the headers (the returned book
/// has no text and no images).
pub fn parse_with(data: &[u8], text: bool) -> Result<Mobi, Error> {
    if data.get(60..68) != Some(b"BOOKMOBI".as_slice()) {
        bail!("not a Mobipocket book");
    }
    let recs = records(data)?;
    let r0 = *recs.first().ok_or_else(|| format_err!("no records"))?;
    let short = || format_err!("truncated MOBI header");
    let mut info = MobiInfo {
        compression: u16_at(r0, 0).ok_or_else(short)?,
        encrypted: u16_at(r0, 12).ok_or_else(short)? != 0,
        encoding: 1252,
        ..Default::default()
    };
    let text_length = u32_at(r0, 4).ok_or_else(short)? as usize;
    let text_records = u16_at(r0, 8).ok_or_else(short)? as usize;
    let (mut extra_flags, mut first_image) = (0u16, None);
    if r0.get(16..20) == Some(b"MOBI".as_slice()) {
        let header_len = u32_at(r0, 20).ok_or_else(short)? as usize;
        info.encoding = u32_at(r0, 28).ok_or_else(short)?;
        info.version = u32_at(r0, 36).unwrap_or(0);
        if let (Some(off), Some(len)) = (u32_at(r0, 84), u32_at(r0, 88)) {
            info.title = r0
                .get(off as usize..(off + len) as usize)
                .map(|b| String::from_utf8_lossy(b).into_owned());
        }
        info.language = u32_at(r0, 92).and_then(locale_language).map(String::from);
        first_image = u32_at(r0, 108).filter(|&i| i != 0xffff_ffff && i != 0);
        if header_len >= 0xe4 {
            extra_flags = u16_at(r0, 0xf2).unwrap_or(0);
        }
        let exth_flags = u32_at(r0, 128).unwrap_or(0);
        if exth_flags & 0x40 != 0 {
            parse_exth(r0.get(16 + header_len..).unwrap_or(&[]), &mut info);
        }
    }
    if info.encrypted {
        bail!("DRM-protected MOBI");
    }
    if info.version >= 8 {
        bail!("KF8-only book (AZW3), not MOBI 7");
    }
    if !text {
        return Ok(Mobi {
            info,
            html: String::new(),
            images: Vec::new(),
        });
    }

    let mut raw = Vec::with_capacity(text_length);
    for rec in recs.iter().skip(1).take(text_records) {
        let body = &rec[..rec.len() - trailing_size(rec, extra_flags)];
        match info.compression {
            COMPRESSION_NONE => raw.extend_from_slice(body),
            COMPRESSION_PALMDOC => palmdoc_decompress(body, &mut raw),
            COMPRESSION_HUFFCDIC => bail!("HUFF/CDIC compression (store books) is not supported"),
            other => bail!("unknown MOBI compression {other}"),
        }
    }
    raw.truncate(text_length);
    let html = if info.encoding == 65001 {
        String::from_utf8_lossy(&raw).into_owned()
    } else {
        raw.iter().map(|&b| cp1252(b)).collect()
    };

    let mut images = Vec::new();
    if let Some(first) = first_image {
        let last = info.kf8_boundary.map_or(recs.len(), |b| b as usize);
        for (i, rec) in recs.iter().enumerate().take(last).skip(first as usize) {
            if let Some(ext) = image_extension(rec) {
                images.push((i as u32 - first + 1, ext, rec.to_vec()));
            }
        }
    }
    Ok(Mobi { info, html, images })
}

fn parse_exth(exth: &[u8], info: &mut MobiInfo) {
    if exth.get(0..4) != Some(b"EXTH".as_slice()) {
        return;
    }
    let count = u32_at(exth, 8).unwrap_or(0);
    let mut off = 12;
    for _ in 0..count {
        let (Some(kind), Some(len)) = (u32_at(exth, off), u32_at(exth, off + 4)) else {
            break;
        };
        let Some(value) = exth.get(off + 8..off + len as usize) else {
            break;
        };
        let text = || String::from_utf8_lossy(value).trim().to_string();
        match kind {
            100 => info.author = Some(text()),
            503 => info.title = Some(text()),
            524 => info.language = Some(text()).filter(|l| !l.is_empty()),
            121 => info.kf8_boundary = u32_at(value, 0).filter(|&i| i != u32::MAX),
            _ => {}
        }
        off += len.max(8) as usize;
    }
}

const VOID: &[&str] = &[
    "br", "hr", "img", "meta", "link", "input", "col", "area", "base", "wbr", "param", "source",
];
/// Elements that end an open `<p>`, as in HTML.
const CLOSES_P: &[&str] = &[
    "p",
    "div",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "ul",
    "ol",
    "table",
    "pre",
    "hr",
];

fn escape_attr(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// Parse a start or end tag's inside (`<` and `>` stripped) into
/// (name, attributes, self-closing). Attribute values may be quoted with
/// either quote or not at all.
fn parse_tag(inner: &str) -> (String, Vec<(String, String)>, bool) {
    let self_closing = inner.ends_with('/');
    let inner = inner.trim_end_matches('/');
    let name_end = inner
        .find(|c: char| c.is_whitespace())
        .unwrap_or(inner.len());
    let name = inner[..name_end].to_ascii_lowercase();
    let mut attrs = Vec::new();
    let rest = &inner[name_end..];
    let b = rest.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i] != b'=' && !(b[i] as char).is_whitespace() {
            i += 1;
        }
        if start == i {
            i += 1;
            continue;
        }
        let key = rest[start..i].to_ascii_lowercase();
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && (b[i] as char).is_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let vs = i + 1;
                i = vs;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                value = rest[vs..i.min(b.len())].to_string();
                i += 1;
            } else {
                let vs = i;
                while i < b.len() && !(b[i] as char).is_whitespace() {
                    i += 1;
                }
                value = rest[vs..i].to_string();
            }
        }
        attrs.push((key, value));
    }
    (name, attrs, self_closing)
}

/// MOBI's presentational paragraph attributes as CSS: `width` is the first
/// line indent, `height` the space above, `align` the alignment.
fn mobi_style(attrs: &mut Vec<(String, String)>) {
    let mut css = String::new();
    attrs.retain(|(k, v)| {
        let prop = match k.as_str() {
            "width" => "text-indent",
            "height" => "margin-top",
            _ => return true,
        };
        let v = if v.chars().all(|c| c.is_ascii_digit()) {
            format!("{v}px")
        } else {
            v.clone()
        };
        css.push_str(&format!("{prop}: {v}; "));
        false
    });
    if !css.is_empty() {
        attrs.push(("style".into(), css.trim_end().to_string()));
    }
}

/// Turn MOBI 7 HTML into well-formed XHTML body fragments, one per chapter
/// (split at `<mbp:pagebreak/>`).
pub fn chapters(html: &str, images: &[(u32, &'static str, Vec<u8>)]) -> Vec<String> {
    chapters_mapped(html, images, true).0
}

/// The name of chapter `n`'s file in the EPUB.
fn chapter_file(n: usize) -> String {
    format!("ch{n:04}.xhtml")
}

/// The targets of `filepos` links, as (offset in `html`, filepos), sorted.
/// A filepos counts bytes of the MOBI text: in a UTF-8 book the same bytes
/// as `html`, in a CP1252 book one character of `html` each.
fn filepos_targets(html: &str, utf8: bool) -> Vec<(usize, u64)> {
    let mut raw: Vec<u64> = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("filepos") {
        rest = &rest[i + 7..];
        let v = rest
            .trim_start_matches(|c: char| c == '=' || c == '"' || c == '\'' || c.is_whitespace());
        let digits: String = v.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<u64>() {
            raw.push(n);
        }
    }
    raw.sort_unstable();
    raw.dedup();
    if utf8 {
        return raw
            .into_iter()
            .filter(|&r| (r as usize) <= html.len())
            .map(|r| (r as usize, r))
            .collect();
    }
    let mut out = Vec::with_capacity(raw.len());
    let mut it = raw.into_iter().peekable();
    for (n, (byte, _)) in html.char_indices().enumerate() {
        while it.peek().is_some_and(|&r| r as usize <= n) {
            out.push((byte, it.next().expect("peeked")));
        }
    }
    out.extend(it.map(|r| (html.len(), r)));
    out
}

/// Minimum distance in source bytes between two anchors of the position
/// map: about three lines of text, plenty to land on the right page.
const ANCHOR_STEP: usize = 200;

/// Chapter bodies as in [`chapters`], plus for each chapter the anchors
/// `(offset in html, offset in the chapter body)` where a run of text was
/// copied, so that positions in the MOBI text can be found in the EPUB.
pub fn chapters_mapped(
    html: &str,
    images: &[(u32, &'static str, Vec<u8>)],
    utf8: bool,
) -> (Vec<String>, Vec<Vec<(usize, usize)>>) {
    let body = html
        .find("<body")
        .and_then(|i| html[i..].find('>').map(|j| i + j + 1))
        .unwrap_or(0);
    let end = html
        .rfind("</body>")
        .filter(|&e| e >= body)
        .unwrap_or(html.len());
    let src = &html[body..end];

    let mut out = Vec::new();
    let mut cur = String::new();
    let mut maps = Vec::new();
    let mut map: Vec<(usize, usize)> = Vec::new();
    let anchor = |map: &mut Vec<(usize, usize)>, src_off: usize, out_off: usize| {
        if map
            .last()
            .is_none_or(|&(s, _)| body + src_off >= s + ANCHOR_STEP)
        {
            map.push((body + src_off, out_off));
        }
    };
    // Link targets still to place, and the chapter each one landed in.
    let targets = filepos_targets(html, utf8);
    let mut next_target = 0;
    let mut target_chapter: Vec<(u64, usize)> = Vec::new();
    // Place the anchors of targets at or before `at` (an offset in `src`).
    let place = |at: usize,
                 next: &mut usize,
                 cur: &mut String,
                 placed: &mut Vec<(u64, usize)>,
                 chapter: usize| {
        while *next < targets.len() && targets[*next].0 <= body + at {
            let raw = targets[*next].1;
            cur.push_str(&format!("<a id=\"filepos{raw}\"></a>"));
            placed.push((raw, chapter));
            *next += 1;
        }
    };
    // Open elements: (name, start tag as emitted).
    let mut stack: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while i < src.len() {
        place(
            i,
            &mut next_target,
            &mut cur,
            &mut target_chapter,
            out.len(),
        );
        let Some(lt) = src[i..].find('<').map(|k| i + k) else {
            anchor(&mut map, i, cur.len());
            cur.push_str(&src[i..]);
            break;
        };
        if lt > i {
            anchor(&mut map, i, cur.len());
        }
        cur.push_str(&src[i..lt]);
        if src[lt..].starts_with("<!--") {
            i = src[lt..].find("-->").map_or(src.len(), |k| lt + k + 3);
            continue;
        }
        let Some(gt) = src[lt..].find('>').map(|k| lt + k) else {
            break;
        };
        let inner = src[lt + 1..gt].trim();
        i = gt + 1;
        if inner.starts_with('!') || inner.starts_with('?') {
            continue;
        }
        if let Some(name) = inner.strip_prefix('/') {
            let name = name.trim().to_ascii_lowercase();
            if let Some(pos) = stack.iter().rposition(|(n, _)| *n == name) {
                while stack.len() > pos {
                    let (n, _) = stack.pop().expect("non-empty");
                    cur.push_str(&format!("</{n}>"));
                }
            }
            continue;
        }
        let (name, mut attrs, self_closing) = parse_tag(inner);
        if name == "mbp:pagebreak" {
            for (n, _) in stack.iter().rev() {
                cur.push_str(&format!("</{n}>"));
            }
            out.push(std::mem::take(&mut cur));
            maps.push(std::mem::take(&mut map));
            for (_, open) in &stack {
                cur.push_str(open);
            }
            continue;
        }
        if name.starts_with("mbp:") || name == "guide" || name == "reference" || name.is_empty() {
            continue;
        }
        if CLOSES_P.contains(&name.as_str()) && stack.last().is_some_and(|(n, _)| n == "p") {
            stack.pop();
            cur.push_str("</p>");
        }
        if name == "img" {
            if let Some(idx) = attrs
                .iter()
                .find(|(k, _)| k == "recindex")
                .and_then(|(_, v)| v.trim().parse::<u32>().ok())
            {
                if let Some((_, ext, _)) = images.iter().find(|(r, _, _)| *r == idx) {
                    attrs.push(("src".into(), format!("images/{idx:05}.{ext}")));
                }
            }
            attrs.retain(|(k, _)| !k.ends_with("recindex"));
        }
        if name == "a" {
            // A link to a text offset: its target file is known only once
            // every chapter is cut (see below).
            if let Some(fp) = attrs
                .iter()
                .find(|(k, _)| k == "filepos")
                .and_then(|(_, v)| v.trim().parse::<u64>().ok())
            {
                attrs.push(("href".into(), format!("@@filepos{fp}@@")));
            }
        }
        attrs.retain(|(k, _)| k != "filepos");
        if matches!(name.as_str(), "p" | "div" | "blockquote") {
            mobi_style(&mut attrs);
        }
        let mut tag = format!("<{name}");
        for (k, v) in &attrs {
            tag.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
        }
        if VOID.contains(&name.as_str()) || self_closing {
            tag.push_str("/>");
            cur.push_str(&tag);
        } else {
            tag.push('>');
            cur.push_str(&tag);
            stack.push((name, tag));
        }
    }
    for (n, _) in stack.iter().rev() {
        cur.push_str(&format!("</{n}>"));
    }
    out.push(cur);
    maps.push(map);
    let keep: Vec<bool> = out
        .iter()
        .map(|c| {
            // Keep chapters with text or images.
            let mut in_tag = false;
            let text = c.chars().any(|ch| match ch {
                '<' => {
                    in_tag = true;
                    false
                }
                '>' => {
                    in_tag = false;
                    false
                }
                _ => !in_tag && !ch.is_whitespace(),
            });
            text || c.contains("<img")
        })
        .collect();
    // Chapters keep their order, so a kept chapter's file number is the
    // number of kept chapters before it.
    let file_of: Vec<Option<usize>> = keep
        .iter()
        .scan(0usize, |n, &k| {
            let f = k.then_some(*n);
            *n += k as usize;
            Some(f)
        })
        .collect();
    let mut k = keep.iter();
    out.retain(|_| *k.next().expect("one flag per chapter"));
    let mut k = keep.iter();
    maps.retain(|_| *k.next().expect("one flag per chapter"));
    if out.iter().any(|c| c.contains("@@filepos")) {
        for c in &mut out {
            let mut fixed = String::with_capacity(c.len());
            let mut rest = c.as_str();
            while let Some(i) = rest.find("@@filepos") {
                fixed.push_str(&rest[..i]);
                let after = &rest[i + 9..];
                let end = after.find("@@").unwrap_or(after.len());
                let raw = after[..end].parse::<u64>().unwrap_or(u64::MAX);
                let file = target_chapter
                    .iter()
                    .find(|(r, _)| *r == raw)
                    .and_then(|&(_, ch)| file_of.get(ch).copied().flatten());
                match file {
                    Some(f) => fixed.push_str(&format!("{}#filepos{raw}", chapter_file(f))),
                    // The target fell in a chapter with nothing in it.
                    None => fixed.push_str(&format!("#filepos{raw}")),
                }
                rest = &after[(end + 2).min(after.len())..];
            }
            fixed.push_str(rest);
            *c = fixed;
        }
    }
    (out, maps)
}

fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;")
}

/// Style for converted books: MOBI 7 books are typeset with indented
/// paragraphs and no space between them, which Plato's default stylesheet
/// (block margins) would not reproduce.
const MOBI_CSS: &str = "p { margin: 0; text-indent: 1.5em; }\n\
                        img { display: block; margin: 0 auto; }\n";

/// Write `mobi` as an EPUB at `path`. Returns the map from MOBI positions
/// (byte offsets into the uncompressed text, the stock reader's unit) to
/// Plato locations in the EPUB.
pub fn write_epub(mobi: &Mobi, path: &Path) -> Result<PosMap, Error> {
    let lang = mobi.info.language.clone().unwrap_or_else(|| "en".into());
    let title = mobi.info.title.clone().unwrap_or_else(|| "Untitled".into());
    // The html is the MOBI text decoded: UTF-8 books keep their byte
    // offsets, CP1252 books turned every byte into one char.
    let utf8 = mobi.info.encoding == 65001;
    let (chapters, maps) = chapters_mapped(&mobi.html, &mobi.images, utf8);
    let (mut seen_bytes, mut seen_chars) = (0usize, 0usize);
    let mut raw_offset = |html_offset: usize| -> u64 {
        if utf8 {
            return html_offset as u64;
        }
        seen_chars += mobi.html[seen_bytes..html_offset].chars().count();
        seen_bytes = html_offset;
        seen_chars as u64
    };
    let mut posmap = PosMap::default();
    // Plato's location of a byte in spine item n is the sum of the sizes of
    // items 0..n plus its offset in item n.
    let mut location = 0u64;

    let mut zip = ZipWriter::new(File::create(path)?);
    // Stored, not deflated: faster to open on the device, and the text was
    // already decompressed once.
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", opts)?;
    zip.write_all(b"application/epub+zip")?;
    zip.start_file("META-INF/container.xml", opts)?;
    zip.write_all(
        b"<?xml version=\"1.0\"?>\n<container version=\"1.0\" \
          xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles>\
          <rootfile full-path=\"content.opf\" media-type=\"application/oebps-package+xml\"/>\
          </rootfiles></container>\n",
    )?;
    zip.start_file("mobi.css", opts)?;
    zip.write_all(MOBI_CSS.as_bytes())?;

    let mut manifest =
        String::from("<item id=\"css\" href=\"mobi.css\" media-type=\"text/css\"/>\n");
    let mut spine = String::new();
    for (n, (body, map)) in chapters.iter().zip(&maps).enumerate() {
        let name = chapter_file(n);
        zip.start_file(&name, opts)?;
        let head = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"{l}\" lang=\"{l}\"><head>\
             <title>{t}</title><link rel=\"stylesheet\" type=\"text/css\" href=\"mobi.css\"/>\
             </head><body>",
            l = escape_attr(&lang),
            t = xml_text(&title),
        );
        const TAIL: &str = "</body></html>\n";
        zip.write_all(head.as_bytes())?;
        zip.write_all(body.as_bytes())?;
        zip.write_all(TAIL.as_bytes())?;
        for &(src, out) in map {
            posmap.push(raw_offset(src), location + (head.len() + out) as u64);
        }
        location += (head.len() + body.len() + TAIL.len()) as u64;
        manifest.push_str(&format!(
            "<item id=\"c{n}\" href=\"{name}\" media-type=\"application/xhtml+xml\"/>\n"
        ));
        spine.push_str(&format!("<itemref idref=\"c{n}\"/>\n"));
    }
    for (idx, ext, data) in &mobi.images {
        let name = format!("images/{idx:05}.{ext}");
        zip.start_file(&name, opts)?;
        zip.write_all(data)?;
        let mime = match *ext {
            "jpg" => "image/jpeg",
            "png" => "image/png",
            "gif" => "image/gif",
            _ => "image/bmp",
        };
        manifest.push_str(&format!(
            "<item id=\"i{idx}\" href=\"{name}\" media-type=\"{mime}\"/>\n"
        ));
    }
    zip.start_file("content.opf", opts)?;
    write!(
        zip,
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <package xmlns=\"http://www.idpf.org/2007/opf\" version=\"2.0\" unique-identifier=\"id\">\n\
         <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n\
         <dc:identifier id=\"id\">aristotle-mobi</dc:identifier>\n\
         <dc:title>{t}</dc:title>\n<dc:creator>{a}</dc:creator>\n<dc:language>{l}</dc:language>\n\
         </metadata>\n<manifest>\n{manifest}</manifest>\n<spine>\n{spine}</spine>\n</package>\n",
        t = xml_text(&title),
        a = xml_text(mobi.info.author.as_deref().unwrap_or("")),
        l = xml_text(&lang),
    )?;
    zip.finish()?;
    Ok(posmap)
}

/// Where the position map of a cached EPUB is kept.
pub fn posmap_path(epub: &Path) -> PathBuf {
    epub.with_extension("posmap")
}

/// The position map written next to a cached EPUB, if any.
pub fn load_posmap(epub: &Path) -> Option<PosMap> {
    fs::read_to_string(posmap_path(epub))
        .ok()
        .and_then(|t| PosMap::from_text(&t).ok())
}

/// The PalmDB header and record 0 of the MOBI at `src`: everything
/// [`parse_with`] reads without the text. A few kilobytes instead of the
/// whole file, which can be megabytes of images.
fn read_head(src: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut f = File::open(src)?;
    let mut head = Vec::with_capacity(8192);
    (&mut f).take(4096).read_to_end(&mut head)?;
    let records = u16_at(&head, 76).unwrap_or(0);
    // Record 1 starts where record 0 ends; one record means read it all.
    let end = match (records, u32_at(&head, 78 + 8)) {
        (2.., Some(off)) => off as u64,
        _ => u64::MAX,
    };
    if end > head.len() as u64 {
        (&mut f)
            .take(end - head.len() as u64)
            .read_to_end(&mut head)?;
    }
    Ok(head)
}

/// The cached EPUB for the MOBI at `src`, converting it if the cache has
/// none for this file's path, size and modification time. Returns the EPUB
/// path, the book's MOBI header information, and whether it was converted
/// now.
pub fn cached_epub(src: &Path, cache_dir: &Path) -> Result<(PathBuf, MobiInfo, bool), Error> {
    let meta = fs::metadata(src).with_context(|| format!("can't read {}", src.display()))?;
    let mut h = DefaultHasher::new();
    CONVERTER_VERSION.hash(&mut h);
    src.hash(&mut h);
    meta.len().hash(&mut h);
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .hash(&mut h);
    let stem: String = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
        .take(40)
        .collect();
    let out = cache_dir.join(format!("{}-{:016x}.epub", stem.trim(), h.finish()));
    if out.exists() && posmap_path(&out).exists() {
        // Only the headers: reading the whole file and decompressing the
        // text again would cost a noticeable part of opening the book on
        // the device.
        let head = read_head(src).with_context(|| format!("can't read {}", src.display()))?;
        return Ok((out, parse_with(&head, false)?.info, false));
    }
    let data = fs::read(src).with_context(|| format!("can't read {}", src.display()))?;
    let mobi = parse(&data)?;
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("can't create {}", cache_dir.display()))?;
    // Two threads may convert the same book at once (the reader's open and
    // a snapshot made ahead of time): each writes its own temporary files,
    // and the renames, of identical results, are atomic.
    static CONVERSIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = CONVERSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let part = format!("{}-{n}.part", std::process::id());
    let tmp = out.with_extension(format!("epub.{part}"));
    let posmap = write_epub(&mobi, &tmp)?;
    let map_tmp = out.with_extension(format!("posmap.{part}"));
    fs::write(&map_tmp, posmap.to_text())?;
    fs::rename(&map_tmp, posmap_path(&out))?;
    fs::rename(&tmp, &out)?;
    Ok((out, mobi.info, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palmdoc_literals_and_back_references() {
        // "abcabcabc": three literals, then a copy of distance 3, length 6.
        let pair: u16 = (3 << 3) | (6 - 3);
        let src = [b'a', b'b', b'c', 0x80 | (pair >> 8) as u8, pair as u8];
        let mut out = Vec::new();
        palmdoc_decompress(&src, &mut out);
        assert_eq!(out, b"abcabcabc");
        // 0xc0..: space plus a letter; 1..8: raw bytes.
        let mut out = Vec::new();
        palmdoc_decompress(&[b'a', b'b' ^ 0x80, 2, 0xc4, 0x85], &mut out);
        assert_eq!(out, "a bą".as_bytes());
    }

    #[test]
    fn trailing_entries() {
        // Text, one multibyte entry (no overlap), one 3-byte TBS entry.
        let rec = [b'x', b'y', 0x00, 0xaa, 0xbb, 0x83];
        assert_eq!(trailing_size(&rec, 0b11), 4);
        assert_eq!(trailing_size(&rec, 0b10), 3);
        assert_eq!(trailing_size(&rec, 0), 0);
    }

    #[test]
    fn html_is_balanced_and_split() {
        let html = "<html><head><guide></guide></head><body>\
                    <p width=\"1em\" align=justify>Jeden<br>dwa<p>trzy</p>\
                    <mbp:pagebreak/><div><p>cztery <img recindex=\"00001\"></div>\
                    </body></html>";
        let images = [(1, "jpg", vec![])];
        let ch = chapters(html, &images);
        assert_eq!(ch.len(), 2);
        assert_eq!(
            ch[0],
            "<p align=\"justify\" style=\"text-indent: 1em;\">Jeden<br/>dwa</p><p>trzy</p>"
        );
        assert_eq!(
            ch[1],
            "<div><p>cztery <img src=\"images/00001.jpg\"/></p></div>"
        );
    }

    #[test]
    fn filepos_links_become_epub_links() {
        // A note reference in the first chapter, the notes in the second.
        let text = "Tekst<a filepos=0000000NNN><sup>1</sup></a>.<mbp:pagebreak/><p>";
        let note = "<p>1. Przypis.</p>";
        let html0 = format!("<html><body>{text}{note}</body></html>");
        let at = html0.find(note).unwrap();
        let html = html0.replace("NNN", &format!("{at:03}"));
        let ch = chapters(&html, &[]);
        assert_eq!(ch.len(), 2);
        let fp = format!("filepos{at}");
        assert!(
            ch[0].contains(&format!("<a href=\"ch0001.xhtml#{fp}\"><sup>1</sup></a>")),
            "{}",
            ch[0]
        );
        assert!(ch[1].contains(&format!("<a id=\"{fp}\"></a>")), "{}", ch[1]);
        // CP1252: offsets count characters.
        let pl = "Zażółć<a filepos=NN>*</a><mbp:pagebreak/>";
        let at = "<html><body>".len() + pl.chars().count();
        let html =
            format!("<html><body>{pl}<p>Uwaga.</p></body></html>").replace("NN", &at.to_string());
        let (ch, _) = chapters_mapped(&html, &[], false);
        assert!(
            ch[0].contains(&format!("ch0001.xhtml#filepos{at}")),
            "{}",
            ch[0]
        );
        assert!(ch[1].contains(&format!("id=\"filepos{at}\"")), "{}", ch[1]);
    }

    /// A fresh directory under `$TMPDIR` (the build script points it at
    /// `$ARISTOTLE_CACHE/tmp`, not the RAM-backed /tmp).
    pub(crate) fn testdir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = std::env::temp_dir()
            .join("aristotle-render-tests")
            .join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn book(encoding: u32) -> Mobi {
        let mut html = String::from("<html><head><guide></guide></head><body>");
        for ch in 0..3 {
            for p in 0..40 {
                html.push_str(&format!(
                    "<p width=\"2em\">Akapit {ch}-{p}: café, tekst do wypełnienia strony.<br>"
                ));
            }
            html.push_str("<mbp:pagebreak/>");
        }
        html.push_str("</body></html>");
        Mobi {
            info: MobiInfo {
                encoding,
                language: Some("pl".into()),
                ..Default::default()
            },
            html,
            images: Vec::new(),
        }
    }

    /// A PalmDB file: a MOBI header with a title in record 0, then
    /// uncompressed text records and a large fake image record.
    fn mobi_file(title: &str, text: &[u8], boundary: Option<u32>) -> Vec<u8> {
        let mut r0 = vec![0u8; 16 + 0xe8];
        r0[0..2].copy_from_slice(&COMPRESSION_NONE.to_be_bytes());
        r0[4..8].copy_from_slice(&(text.len() as u32).to_be_bytes());
        r0[8..10].copy_from_slice(&1u16.to_be_bytes());
        r0[16..20].copy_from_slice(b"MOBI");
        r0[20..24].copy_from_slice(&0xe8u32.to_be_bytes());
        r0[28..32].copy_from_slice(&65001u32.to_be_bytes());
        r0[36..40].copy_from_slice(&6u32.to_be_bytes());
        if let Some(boundary) = boundary {
            r0[128..132].copy_from_slice(&0x40u32.to_be_bytes());
            r0.extend_from_slice(b"EXTH");
            for value in [24u32, 1, 121, 12, boundary] {
                r0.extend_from_slice(&value.to_be_bytes());
            }
        }
        let title_at = r0.len() as u32;
        r0[84..88].copy_from_slice(&title_at.to_be_bytes());
        r0[88..92].copy_from_slice(&(title.len() as u32).to_be_bytes());
        r0.extend_from_slice(title.as_bytes());
        let records = [r0, text.to_vec(), vec![0xff; 100_000]];
        let mut out = vec![0u8; 78 + 8 * records.len() + 2];
        out[60..68].copy_from_slice(b"BOOKMOBI");
        out[76..78].copy_from_slice(&(records.len() as u16).to_be_bytes());
        for (i, r) in records.iter().enumerate() {
            let at = 78 + 8 * i;
            let offset = out.len() as u32;
            out[at..at + 4].copy_from_slice(&offset.to_be_bytes());
            out.extend_from_slice(r);
        }
        out
    }

    #[test]
    fn header_only_read_matches_the_whole_file() {
        let data = mobi_file("Lalka", b"<p>Tekst.</p>", None);
        let path = testdir("head").join("book.mobi");
        fs::write(&path, &data).unwrap();
        let head = read_head(&path).unwrap();
        assert!(head.len() < 5000, "{} bytes", head.len());
        let (a, b) = (
            parse_with(&head, false).unwrap().info,
            parse_with(&data, false).unwrap().info,
        );
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        assert_eq!(a.title.as_deref(), Some("Lalka"));
        // And the cache path is the same with or without the conversion.
        let cache = testdir("head-cache");
        let (epub, _, now) = cached_epub(&path, &cache).unwrap();
        assert!(now);
        let (again, info, now) = cached_epub(&path, &cache).unwrap();
        assert_eq!((again, now), (epub, false));
        assert_eq!(info.title.as_deref(), Some("Lalka"));
    }

    #[test]
    fn cached_metadata_normalizes_only_the_no_kf8_sentinel() {
        let dir = testdir("kf8-boundary");
        for boundary in [None, Some(u32::MAX), Some(0), Some(3)] {
            let path = dir.join(format!("book-{boundary:?}.mobi"));
            let data = mobi_file("Original test", b"<p>Original test text.</p>", boundary);
            fs::write(&path, &data).unwrap();
            let expected = boundary.filter(|&i| i != u32::MAX);
            assert_eq!(parse(&data).unwrap().info.kf8_boundary, expected);
            let cache = dir.join("cache");
            let (epub, cold, converted) = cached_epub(&path, &cache).unwrap();
            assert!(converted);
            assert_eq!(cold.kf8_boundary, expected);
            let (again, warm, converted) = cached_epub(&path, &cache).unwrap();
            assert!(!converted);
            assert_eq!(again, epub);
            assert_eq!(warm.kf8_boundary, expected);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    /// Stock Kindle positions (bytes of MOBI text) must land on the same
    /// words in the converted EPUB, and the spine must match Plato's.
    #[test]
    fn positions_land_on_the_same_text() {
        use crate::position::Spine;
        use plato_core::document::epub::EpubDocument;
        use plato_core::document::Document;
        use std::io::Read;

        for encoding in [65001, 1252] {
            let mobi = book(encoding);
            let path = testdir("posmap").join("book.epub");
            let map = write_epub(&mobi, &path).unwrap();
            let spine = Spine::read(&path).unwrap();
            let doc = EpubDocument::new(&path).unwrap();
            assert_eq!(doc.pages_count(), spine.total());
            assert_eq!(spine.sizes.len(), 3);

            let mut zip = zip::ZipArchive::new(File::open(&path).unwrap()).unwrap();
            for marker in ["Akapit 0-0:", "Akapit 0-7:", "Akapit 1-17:", "Akapit 2-39:"] {
                let byte = mobi.html.find(marker).unwrap();
                let position = if encoding == 65001 {
                    byte
                } else {
                    mobi.html[..byte].chars().count()
                };
                let (item, offset) = spine.coordinates(map.location(position as u64) as usize);
                let mut text = String::new();
                zip.by_name(&format!("ch{item:04}.xhtml"))
                    .unwrap()
                    .read_to_string(&mut text)
                    .unwrap();
                let near = &text[offset.min(text.len())..(offset + 80).min(text.len())];
                assert!(
                    near.contains(marker),
                    "{encoding}: {marker} -> {item}:{offset} {near:?}"
                );
                let back = map.position(map.location(position as u64)) as usize;
                assert!(back.abs_diff(position) < 8, "{position} -> {back}");
            }
        }
    }
}
