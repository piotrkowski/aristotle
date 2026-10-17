//! Read-only ZIP access for EPUB and CBZ.
//!
//! The `zip` crate brings compression back ends, encryption and writers we do
//! not need. Metadata extraction only ever reads the central directory and two
//! or three small members (container.xml, the OPF, the cover), so a reader of
//! about two hundred lines over `miniz_oxide` is enough and keeps the device
//! binary small. Supported: stored and deflated members, ZIP64 sizes and
//! offsets. Not supported: encrypted members, multi-disk archives.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::{Error, Result};

/// Refuse to inflate members larger than this. Real covers and OPFs are far
/// smaller; the cap protects the 512 MB device against zip bombs.
pub const MAX_MEMBER_SIZE: u64 = 64 << 20;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    method: u16,
    flags: u16,
    compressed_size: u64,
    pub size: u64,
    header_offset: u64,
}

pub struct ZipArchive {
    file: File,
    entries: Vec<Entry>,
}

impl ZipArchive {
    pub fn open(path: &Path) -> Result<ZipArchive> {
        let file = File::open(path)?;
        ZipArchive::new(file)
    }

    pub fn new(mut file: File) -> Result<ZipArchive> {
        let len = file.seek(SeekFrom::End(0))?;
        // The end-of-central-directory record is 22 bytes plus a comment of up
        // to 64 KiB, so it sits in the last 65,557 bytes.
        let tail_len = len.min(22 + 0xFFFF);
        let tail = read_at(&mut file, len - tail_len, tail_len as usize)?;
        let eocd = tail
            .windows(4)
            .rposition(|w| w == b"PK\x05\x06")
            .ok_or_else(|| Error::format("zip: no end of central directory"))?;
        let e = &tail[eocd..];
        if e.len() < 22 {
            return Err(Error::format("zip: truncated end record"));
        }
        let mut count = le16(e, 10) as u64;
        let mut cd_size = le32(e, 12) as u64;
        let mut cd_offset = le32(e, 16) as u64;
        if count == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
            // ZIP64: a locator sits right before the classic record.
            let loc_at = (len - tail_len + eocd as u64)
                .checked_sub(20)
                .ok_or_else(|| Error::format("zip64: no locator"))?;
            let loc = read_at(&mut file, loc_at, 20)?;
            if &loc[..4] != b"PK\x06\x07" {
                return Err(Error::format("zip64: bad locator"));
            }
            let rec = read_at(&mut file, le64(&loc, 8), 56)?;
            if &rec[..4] != b"PK\x06\x06" {
                return Err(Error::format("zip64: bad end record"));
            }
            count = le64(&rec, 32);
            cd_size = le64(&rec, 40);
            cd_offset = le64(&rec, 48);
        }
        if cd_offset.saturating_add(cd_size) > len || cd_size > 32 << 20 {
            return Err(Error::format("zip: central directory out of range"));
        }
        let cd = read_at(&mut file, cd_offset, cd_size as usize)?;
        let mut entries = Vec::with_capacity(count.min(65_536) as usize);
        let mut p = 0usize;
        while p + 46 <= cd.len() && &cd[p..p + 4] == b"PK\x01\x02" {
            let flags = le16(&cd, p + 8);
            let method = le16(&cd, p + 10);
            let mut compressed_size = le32(&cd, p + 20) as u64;
            let mut size = le32(&cd, p + 24) as u64;
            let name_len = le16(&cd, p + 28) as usize;
            let extra_len = le16(&cd, p + 30) as usize;
            let comment_len = le16(&cd, p + 32) as usize;
            let mut header_offset = le32(&cd, p + 42) as u64;
            let name_end = p + 46 + name_len;
            let extra_end = name_end + extra_len;
            if extra_end + comment_len > cd.len() {
                return Err(Error::format("zip: truncated central directory"));
            }
            // EPUB requires UTF-8 names; anything else is decoded lossily.
            let name = String::from_utf8_lossy(&cd[p + 46..name_end]).into_owned();
            // ZIP64 extra field: only the fields saturated in the header are present, in order.
            let mut x = name_end;
            while x + 4 <= extra_end {
                let id = le16(&cd, x);
                let sz = le16(&cd, x + 2) as usize;
                if id == 1 {
                    let mut f = x + 4;
                    let fend = (x + 4 + sz).min(extra_end);
                    for v in [&mut size, &mut compressed_size, &mut header_offset] {
                        if *v == 0xFFFF_FFFF && f + 8 <= fend {
                            *v = le64(&cd, f);
                            f += 8;
                        }
                    }
                }
                x += 4 + sz;
            }
            entries.push(Entry {
                name,
                method,
                flags,
                compressed_size,
                size,
                header_offset,
            });
            p = extra_end + comment_len;
        }
        Ok(ZipArchive { file, entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Finds a member by exact name, falling back to a case-insensitive match
    /// because some EPUBs reference "Cover.JPG" as "cover.jpg".
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name).or_else(|| {
            self.entries
                .iter()
                .find(|e| e.name.eq_ignore_ascii_case(name))
        })
    }

    pub fn read(&mut self, name: &str) -> Result<Vec<u8>> {
        let entry = self
            .find(name)
            .cloned()
            .ok_or_else(|| Error::format(format!("zip: no member {name}")))?;
        self.read_entry(&entry)
    }

    pub fn read_entry(&mut self, entry: &Entry) -> Result<Vec<u8>> {
        if entry.flags & 1 != 0 {
            return Err(Error::format("zip: encrypted member"));
        }
        if entry.size > MAX_MEMBER_SIZE || entry.compressed_size > MAX_MEMBER_SIZE {
            return Err(Error::format(format!(
                "zip: member {} too large",
                entry.name
            )));
        }
        let lh = read_at(&mut self.file, entry.header_offset, 30)?;
        if &lh[..4] != b"PK\x03\x04" {
            return Err(Error::format("zip: bad local header"));
        }
        // Sizes come from the central directory: the local header may hold
        // zeros when a data descriptor follows the data.
        let data_at = entry.header_offset + 30 + le16(&lh, 26) as u64 + le16(&lh, 28) as u64;
        let data = read_at(&mut self.file, data_at, entry.compressed_size as usize)?;
        match entry.method {
            0 => Ok(data),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(
                &data,
                entry.size.max(1) as usize,
            )
            .map_err(|e| Error::format(format!("zip: inflate {}: {:?}", entry.name, e.status))),
            m => Err(Error::format(format!("zip: unsupported method {m}"))),
        }
    }
}

pub(crate) fn read_at(file: &mut File, offset: u64, len: usize) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0; len];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn le64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Minimal ZIP writer used by unit tests to build fixtures in memory.
#[cfg(test)]
pub(crate) mod testutil {
    pub fn build(members: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, data, deflate) in members {
            let offset = out.len() as u32;
            let (method, payload) = if *deflate {
                (8u16, miniz_oxide::deflate::compress_to_vec(data, 6))
            } else {
                (0u16, data.to_vec())
            };
            let crc = crc32(data);
            let mut h = Vec::new();
            h.extend_from_slice(b"PK\x03\x04");
            h.extend_from_slice(&20u16.to_le_bytes());
            h.extend_from_slice(&0x0800u16.to_le_bytes());
            h.extend_from_slice(&method.to_le_bytes());
            h.extend_from_slice(&[0; 4]);
            h.extend_from_slice(&crc.to_le_bytes());
            h.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&h);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&payload);
            cd.extend_from_slice(b"PK\x01\x02");
            cd.extend_from_slice(&20u16.to_le_bytes());
            cd.extend_from_slice(&h[4..30]);
            cd.extend_from_slice(&[0; 2 + 2 + 2 + 4]);
            cd.extend_from_slice(&offset.to_le_bytes());
            cd.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&cd);
        out.extend_from_slice(b"PK\x05\x06");
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(members.len() as u16).to_le_bytes());
        out.extend_from_slice(&(members.len() as u16).to_le_bytes());
        out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stored_and_deflated() {
        let dir = crate::testdir("zip");
        let path = dir.join("a.zip");
        let big = "Zażółć gęślą jaźń. ".repeat(500);
        std::fs::write(
            &path,
            testutil::build(&[
                ("mimetype", b"application/epub+zip", false),
                ("OEBPS/x.txt", big.as_bytes(), true),
            ]),
        )
        .unwrap();
        let mut z = ZipArchive::open(&path).unwrap();
        assert_eq!(z.entries().len(), 2);
        assert_eq!(z.read("mimetype").unwrap(), b"application/epub+zip");
        assert_eq!(z.read("oebps/X.TXT").unwrap(), big.as_bytes());
        assert!(z.read("missing").is_err());
    }

    #[test]
    fn rejects_garbage() {
        let dir = crate::testdir("zip");
        let path = dir.join("bad.zip");
        std::fs::write(&path, b"PK\x03\x04 not really a zip").unwrap();
        assert!(ZipArchive::open(&path).is_err());
    }
}
