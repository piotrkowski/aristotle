// SPDX-License-Identifier: AGPL-3.0-or-later
//! MOBI, AZW3 and PalmDOC metadata from the PalmDB header, record 0 and
//! the EXTH block.
//!
//! Only the PalmDB record table, record 0 (a few kilobytes) and the cover
//! record are read; the compressed text is never touched. Layout references:
//! the MobileRead wiki "MOBI" page and KindleUnpack; offsets below are
//! relative to the start of record 0.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::encoding::{decode, decode_guess, Encoding};
use crate::error::{Error, Result};
use crate::metadata::{Author, Extracted, Identifier};
use crate::text;

/// Record 0 offsets.
const ENCRYPTION: usize = 0x0C;
const MOBI_MAGIC: usize = 0x10;
const HEADER_LEN: usize = 0x14;
const TEXT_ENCODING: usize = 0x1C;
const FILE_VERSION: usize = 0x24;
const FULL_NAME_OFFSET: usize = 0x54;
const FULL_NAME_LEN: usize = 0x58;
const LOCALE: usize = 0x5C;
const FIRST_IMAGE: usize = 0x6C;
const EXTH_FLAGS: usize = 0x80;

/// EXTH record types we use.
const EXTH_AUTHOR: u32 = 100;
const EXTH_PUBLISHER: u32 = 101;
const EXTH_DESCRIPTION: u32 = 103;
const EXTH_ISBN: u32 = 104;
const EXTH_SUBJECT: u32 = 105;
const EXTH_PUBLISHED: u32 = 106;
const EXTH_ASIN: u32 = 113;
const EXTH_KF8_BOUNDARY: u32 = 121;
const EXTH_COVER_OFFSET: u32 = 201;
const EXTH_UPDATED_TITLE: u32 = 503;
const EXTH_ASIN2: u32 = 504;
const EXTH_LANGUAGE: u32 = 524;
const EXTH_DICT_IN: u32 = 531;

struct PalmDb {
    file: File,
    name: Vec<u8>,
    offsets: Vec<u64>,
    len: u64,
}

impl PalmDb {
    fn open(path: &Path) -> Result<PalmDb> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        let mut head = [0u8; 78];
        file.read_exact(&mut head)?;
        let count = u16::from_be_bytes([head[76], head[77]]) as usize;
        if count == 0 {
            return Err(Error::format("mobi: no records"));
        }
        let mut table = vec![0u8; count * 8];
        file.read_exact(&mut table)?;
        let offsets = table
            .as_chunks::<8>()
            .0
            .iter()
            .map(|r| u32::from_be_bytes(r[..4].try_into().unwrap()) as u64)
            .collect();
        let name_end = head[..32].iter().position(|&b| b == 0).unwrap_or(32);
        Ok(PalmDb {
            file,
            name: head[..name_end].to_vec(),
            offsets,
            len,
        })
    }

    fn record(&mut self, i: usize) -> Result<Vec<u8>> {
        let start = *self
            .offsets
            .get(i)
            .ok_or_else(|| Error::format("mobi: record out of range"))?;
        let end = self.offsets.get(i + 1).copied().unwrap_or(self.len);
        if end < start || end > self.len || end - start > 16 << 20 {
            return Err(Error::format("mobi: bad record bounds"));
        }
        self.file.seek(SeekFrom::Start(start))?;
        let mut buf = vec![0; (end - start) as usize];
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes(s.try_into().unwrap()))
}

/// MOBI header version: 8 for KF8-only (AZW3) files, 6 or less for MOBI 7,
/// 0 for plain PalmDOC.
pub fn header_version(path: &Path) -> Result<u32> {
    let mut db = PalmDb::open(path)?;
    let r0 = db.record(0)?;
    if r0.get(MOBI_MAGIC..MOBI_MAGIC + 4) != Some(b"MOBI") {
        return Ok(0);
    }
    Ok(be32(&r0, FILE_VERSION).unwrap_or(0))
}

fn parse_exth(r0: &[u8]) -> Vec<(u32, &[u8])> {
    let mut exth = Vec::new();
    let header_len = be32(r0, HEADER_LEN).unwrap_or(0) as usize;
    if be32(r0, EXTH_FLAGS).is_some_and(|f| f & 0x40 != 0) {
        let at = MOBI_MAGIC + header_len;
        if r0.get(at..at + 4) == Some(b"EXTH") {
            let count = be32(r0, at + 8).unwrap_or(0);
            let mut p = at + 12;
            for _ in 0..count {
                let (Some(t), Some(l)) = (be32(r0, p), be32(r0, p + 4)) else {
                    break;
                };
                let l = l as usize;
                if l < 8 || p + l > r0.len() {
                    break;
                }
                exth.push((t, &r0[p + 8..p + l]));
                p += l;
            }
        }
    }
    exth
}

/// Uncompressed text length of the MOBI 7 part and, in joint files, of the
/// KF8 part. Kindle positions are byte offsets into this text, so these turn
/// a stock sidecar position into a reading fraction.
pub fn text_lengths(path: &Path) -> Result<(u64, Option<u64>)> {
    let mut db = PalmDb::open(path)?;
    let r0 = db.record(0)?;
    let len = be32(&r0, 4).unwrap_or(0) as u64;
    let boundary = parse_exth(&r0)
        .iter()
        .find(|(t, _)| *t == EXTH_KF8_BOUNDARY)
        .and_then(|(_, v)| be32(v, 0));
    let mut kf8 = None;
    if let Some(b) = boundary.filter(|&b| b != u32::MAX && b > 0) {
        // EXTH 121 points at the KF8 record 0, right after a "BOUNDARY" record.
        if let Ok(r) = db.record(b as usize) {
            kf8 = be32(&r, 4).map(|l| l as u64);
        }
    }
    Ok((len, kf8))
}

pub fn extract(path: &Path, want_cover: bool) -> Result<Extracted> {
    let mut db = PalmDb::open(path)?;
    let r0 = db.record(0)?;
    let mut ex = Extracted::default();
    let m = &mut ex.meta;
    // Encryption type 1 or 2 is Mobipocket DRM; we list such books but cannot open them.
    m.drm = r0
        .get(ENCRYPTION..ENCRYPTION + 2)
        .is_some_and(|b| b != [0, 0]);

    if r0.get(MOBI_MAGIC..MOBI_MAGIC + 4) != Some(b"MOBI") {
        // PalmDOC: only the database name, Latin-1 with underscores.
        m.title = decode(&db.name, Encoding::Windows1252).replace('_', " ");
        return Ok(ex);
    }
    let enc = match be32(&r0, TEXT_ENCODING) {
        Some(65001) => Some(Encoding::Utf8),
        Some(1252) => Some(Encoding::Windows1252),
        _ => None,
    };
    let dec = |b: &[u8]| text::clean(&enc.map(|e| decode(b, e)).unwrap_or_else(|| decode_guess(b)));

    let exth = parse_exth(&r0);
    let first = |t: u32| exth.iter().find(|(k, _)| *k == t).map(|(_, v)| *v);
    let all = |t: u32| exth.iter().filter(move |(k, _)| *k == t).map(|(_, v)| *v);

    m.title = match first(EXTH_UPDATED_TITLE) {
        Some(t) => dec(t),
        None => {
            let off = be32(&r0, FULL_NAME_OFFSET).unwrap_or(0) as usize;
            let len = be32(&r0, FULL_NAME_LEN).unwrap_or(0) as usize;
            r0.get(off..off.saturating_add(len))
                .map(dec)
                .unwrap_or_default()
        }
    };
    if m.title.is_empty() {
        m.title = decode(&db.name, Encoding::Windows1252).replace('_', " ");
    }
    for a in all(EXTH_AUTHOR) {
        for name in dec(a).split(['&', ';']) {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            // Some tools write "Surname, Given"; show it the natural way and keep it as the sort name.
            match name.split_once(", ") {
                Some((last, given)) if !given.contains(',') => {
                    m.authors
                        .push(Author::new(&format!("{given} {last}"), Some(name)));
                }
                _ => m.authors.push(Author::new(name, None)),
            }
        }
    }
    m.publisher = first(EXTH_PUBLISHER).map(dec);
    m.description = first(EXTH_DESCRIPTION)
        .map(|d| text::strip_html(&enc.map(|e| decode(d, e)).unwrap_or_else(|| decode_guess(d))));
    m.published = first(EXTH_PUBLISHED).map(dec);
    m.subjects = all(EXTH_SUBJECT)
        .map(dec)
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(isbn) = first(EXTH_ISBN) {
        m.identifiers.push(Identifier {
            scheme: "isbn".into(),
            value: dec(isbn),
        });
    }
    if let Some(asin) = first(EXTH_ASIN).or_else(|| first(EXTH_ASIN2)) {
        let value = dec(asin);
        // calibre stores the book's UUID in the ASIN field of sideloaded files.
        let is_uuid = value.len() == 36 && value.bytes().filter(|&b| b == b'-').count() == 4;
        m.identifiers.push(Identifier {
            scheme: if is_uuid { "uuid" } else { "asin" }.into(),
            value,
        });
    }
    m.identifiers.retain(|i| !i.value.is_empty());
    m.language = first(EXTH_LANGUAGE).map(dec).or_else(|| {
        be32(&r0, LOCALE)
            .and_then(|l| locale_language(l & 0xFF))
            .map(str::to_owned)
    });
    ex.dictionary = first(EXTH_DICT_IN).is_some();

    if want_cover {
        let first_image = be32(&r0, FIRST_IMAGE).unwrap_or(u32::MAX);
        if let (Some(off), true) = (
            first(EXTH_COVER_OFFSET).and_then(|v| be32(v, 0)),
            first_image != u32::MAX,
        ) {
            if off != u32::MAX {
                if let Ok(img) = db.record(first_image as usize + off as usize) {
                    if crate::cover::sniff_image(&img).is_some() {
                        ex.cover = Some(img);
                    }
                }
            }
        }
    }
    Ok(ex)
}

/// Windows LANGID primary language codes used in the MOBI locale field.
fn locale_language(primary: u32) -> Option<&'static str> {
    Some(match primary {
        0x04 => "zh",
        0x05 => "cs",
        0x07 => "de",
        0x09 => "en",
        0x0A => "es",
        0x0C => "fr",
        0x0E => "hu",
        0x10 => "it",
        0x11 => "ja",
        0x13 => "nl",
        0x15 => "pl",
        0x16 => "pt",
        0x19 => "ru",
        0x1D => "sv",
        0x22 => "uk",
        _ => return None,
    })
}

/// Builds a minimal MOBI file in memory for tests.
#[cfg(test)]
pub(crate) mod testutil {
    pub fn build(title: &str, exth: &[(u32, &[u8])], images: &[&[u8]], version: u32) -> Vec<u8> {
        let mut ex = Vec::new();
        for (t, v) in exth {
            ex.extend_from_slice(&t.to_be_bytes());
            ex.extend_from_slice(&((v.len() + 8) as u32).to_be_bytes());
            ex.extend_from_slice(v);
        }
        let mut exth_block = b"EXTH".to_vec();
        exth_block.extend_from_slice(&((ex.len() + 12) as u32).to_be_bytes());
        exth_block.extend_from_slice(&(exth.len() as u32).to_be_bytes());
        exth_block.extend_from_slice(&ex);

        let header_len = 0xE8u32;
        let mut r0 = vec![0u8; 16 + header_len as usize];
        r0[0x10..0x14].copy_from_slice(b"MOBI");
        r0[0x14..0x18].copy_from_slice(&header_len.to_be_bytes());
        r0[0x1C..0x20].copy_from_slice(&65001u32.to_be_bytes());
        r0[0x24..0x28].copy_from_slice(&version.to_be_bytes());
        r0[0x5C..0x60].copy_from_slice(&0x0415u32.to_be_bytes());
        r0[0x6C..0x70].copy_from_slice(&1u32.to_be_bytes());
        r0[0x80..0x84].copy_from_slice(&0x40u32.to_be_bytes());
        r0.extend_from_slice(&exth_block);
        let name_off = r0.len() as u32;
        r0.extend_from_slice(title.as_bytes());
        r0.extend_from_slice(&[0, 0]);
        r0[0x54..0x58].copy_from_slice(&name_off.to_be_bytes());
        r0[0x58..0x5C].copy_from_slice(&(title.len() as u32).to_be_bytes());

        let mut records: Vec<&[u8]> = vec![&r0];
        records.extend_from_slice(images);
        let mut out = vec![0u8; 78];
        out[..8].copy_from_slice(b"test_db\0");
        out[60..68].copy_from_slice(b"BOOKMOBI");
        out[76..78].copy_from_slice(&(records.len() as u16).to_be_bytes());
        let mut off = 78 + records.len() * 8 + 2;
        for (i, r) in records.iter().enumerate() {
            out.extend_from_slice(&(off as u32).to_be_bytes());
            out.extend_from_slice(&(i as u32).to_be_bytes());
            off += r.len();
        }
        out.extend_from_slice(&[0, 0]);
        for r in records {
            out.extend_from_slice(r);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exth_metadata_and_cover() {
        let dir = crate::testdir("mobi");
        let p = dir.join("x.azw3");
        let png = b"\x89PNG\r\n\x1a\nrest";
        let data = testutil::build(
            "Full Name",
            &[
                (100, "Schopenhauer, Arthur".as_bytes()),
                (503, "Erystyka, czyli sztuka prowadzenia sporów".as_bytes()),
                (103, "<p>O sztuce &amp; sporze</p>".as_bytes()),
                (201, &0u32.to_be_bytes()),
            ],
            &[png],
            8,
        );
        std::fs::write(&p, data).unwrap();
        assert_eq!(header_version(&p).unwrap(), 8);
        let ex = extract(&p, true).unwrap();
        assert_eq!(ex.meta.title, "Erystyka, czyli sztuka prowadzenia sporów");
        assert_eq!(
            ex.meta.authors,
            vec![Author {
                name: "Arthur Schopenhauer".into(),
                sort: "Schopenhauer, Arthur".into()
            }]
        );
        assert_eq!(ex.meta.description.as_deref(), Some("O sztuce & sporze"));
        assert_eq!(ex.meta.language.as_deref(), Some("pl"));
        assert_eq!(ex.cover.as_deref(), Some(&png[..]));
        assert!(!ex.meta.drm);
    }

    #[test]
    fn full_name_when_no_updated_title() {
        let dir = crate::testdir("mobi");
        let p = dir.join("y.mobi");
        std::fs::write(&p, testutil::build("Książę", &[], &[], 6)).unwrap();
        let ex = extract(&p, true).unwrap();
        assert_eq!(ex.meta.title, "Książę");
        assert!(ex.cover.is_none());
    }
}
