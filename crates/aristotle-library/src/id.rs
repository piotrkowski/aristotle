// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Aristotle contributors
// SPDX-FileCopyrightText: KOReader contributors
//! Book identity.
//!
//! A book's ID is KOReader's "partial MD5": the MD5 of up to twelve 1 KiB
//! samples taken at offsets 0 and 1024 << 2i for i = 0..=10 (1 KiB, 4 KiB,
//! 16 KiB, ... 1 GiB), stopping at the end of the file. Reasons:
//!
//! - It costs at most twelve small reads, so even a 300 MB PDF is identified
//!   in milliseconds on the device, unlike a full hash.
//! - It depends on content only, so progress survives renames and moves
//!   between author folders, which a path or inode key would lose.
//! - It is the exact document hash KOReader uses for its sidecar files and
//!   for progress sync (kosync), so we can later exchange progress with it.
//!
//! The first offset comes from KOReader's `util.partialMD5` loop starting at
//! i = -1: LuaJIT's `bit.lshift(1024, -2)` masks the shift count to 30 and
//! the 32-bit result is 0. (Borrowed from KOReader `frontend/util.lua`.)

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use md5::{Digest, Md5};

use crate::error::Result;

pub fn partial_md5(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut hasher = Md5::new();
    let mut buf = [0u8; 1024];
    let offsets = std::iter::once(0u64).chain((0..=10).map(|i| 1024u64 << (2 * i)));
    for offset in offsets {
        if offset >= len {
            break;
        }
        file.seek(SeekFrom::Start(offset))?;
        let want = (len - offset).min(1024) as usize;
        file.read_exact(&mut buf[..want])?;
        hasher.update(&buf[..want]);
    }
    Ok(hex(&hasher.finalize()))
}

/// MD5 of a whole buffer, used as the content key of cover images.
pub fn md5_hex(data: &[u8]) -> String {
    hex(&Md5::digest(data))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_file_hashes_whole_content() {
        let dir = crate::testdir("id");
        let p = dir.join("a.txt");
        std::fs::write(&p, b"hello").unwrap();
        // Only offset 0 is inside the file, so this is plain MD5("hello").
        assert_eq!(partial_md5(&p).unwrap(), "5d41402abc4b2a76b9719d911017c592");
    }

    #[test]
    fn samples_are_taken_at_koreader_offsets() {
        let dir = crate::testdir("id");
        let p = dir.join("b.bin");
        let data: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let mut h = Md5::new();
        h.update(&data[0..1024]);
        h.update(&data[1024..2048]);
        h.update(&data[4096..5120]);
        h.update(&data[16384..17408]);
        assert_eq!(partial_md5(&p).unwrap(), hex(&h.finalize()));
    }
}
