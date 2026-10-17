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
//! contents (NCX index) and `filepos` links.
//!
//! Format references: MobileRead wiki ("MOBI", "PalmDOC") and calibre's
//! `mobi/reader` (trailing entries).

use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use plato_core::anyhow::{bail, format_err, Context, Error};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Bump when the conversion output changes, to invalidate cached EPUBs.
const CONVERTER_VERSION: u32 = 1;

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
            121 => info.kf8_boundary = u32_at(value, 0),
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
    // Open elements: (name, start tag as emitted).
    let mut stack: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let Some(lt) = src[i..].find('<').map(|k| i + k) else {
            cur.push_str(&src[i..]);
            break;
        };
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
    out.retain(|c| {
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
    });
    out
}

fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;")
}

/// Style for converted books: MOBI 7 books are typeset with indented
/// paragraphs and no space between them, which Plato's default stylesheet
/// (block margins) would not reproduce.
const MOBI_CSS: &str = "p { margin: 0; text-indent: 1.5em; }\n\
                        img { display: block; margin: 0 auto; }\n";

/// Write `mobi` as an EPUB at `path`.
pub fn write_epub(mobi: &Mobi, path: &Path) -> Result<(), Error> {
    let lang = mobi.info.language.clone().unwrap_or_else(|| "en".into());
    let title = mobi.info.title.clone().unwrap_or_else(|| "Untitled".into());
    let chapters = chapters(&mobi.html, &mobi.images);

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
    for (n, body) in chapters.iter().enumerate() {
        let name = format!("ch{n:04}.xhtml");
        zip.start_file(&name, opts)?;
        write!(
            zip,
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"{l}\" lang=\"{l}\"><head>\
             <title>{t}</title><link rel=\"stylesheet\" type=\"text/css\" href=\"mobi.css\"/>\
             </head><body>{body}</body></html>\n",
            l = escape_attr(&lang),
            t = xml_text(&title),
        )?;
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
    Ok(())
}

/// The cached EPUB for the MOBI at `src`, converting it if the cache has
/// none for this file's path, size and modification time. Returns the EPUB
/// path, the book's MOBI header information, and whether it was converted
/// now.
pub fn cached_epub(src: &Path, cache_dir: &Path) -> Result<(PathBuf, MobiInfo, bool), Error> {
    let data = fs::read(src).with_context(|| format!("can't read {}", src.display()))?;
    let meta = fs::metadata(src)?;
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
    let mobi = parse(&data)?;
    if out.exists() {
        return Ok((out, mobi.info, false));
    }
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("can't create {}", cache_dir.display()))?;
    let tmp = out.with_extension("epub.part");
    write_epub(&mobi, &tmp)?;
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
}
