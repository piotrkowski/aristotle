//! Reading StarDict dictionaries: `.ifo`, `.idx`, `.syn` and `.dict(.dz)`.
//!
//! Plato's own dictionary code reads the dictd format, not StarDict, so this
//! is ours. Everything is read with positioned reads on demand: the owner's
//! Polish dictionary has a 27 MB synonym file and the Greek-English one 48
//! MB, far too much to hold in memory on the PW4. Lookups go through the
//! fold index ([`super::fold`]), which points into these files.
//!
//! Format (StarDict 3.0.0, `doc/StarDictFileFormat` of the StarDict
//! project):
//!
//! - `.ifo`: `key=value` lines after a fixed first line.
//! - `.idx`: entries `word\0` + offset + size (big-endian, 32 or 64-bit
//!   offsets per `idxoffsetbits`), sorted.
//! - `.syn`: entries `word\0` + u32 index of the `.idx` entry.
//! - `.dict.dz`: dictzip, a gzip file whose `RA` extra field lists the
//!   compressed sizes of chunks that can be inflated independently.

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use flate2::{Decompress, FlushDecompress};

/// What the `.ifo` file says.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub bookname: String,
    pub wordcount: u32,
    pub synwordcount: u32,
    /// 32 or 64.
    pub idxoffsetbits: u32,
    /// `h` for HTML, `m` for plain text, ...; empty if entries carry types.
    pub sametypesequence: String,
}

impl Info {
    pub fn parse(text: &str) -> Info {
        let mut info = Info {
            idxoffsetbits: 32,
            ..Info::default()
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "bookname" => info.bookname = v.to_string(),
                "wordcount" => info.wordcount = v.parse().unwrap_or(0),
                "synwordcount" => info.synwordcount = v.parse().unwrap_or(0),
                "idxoffsetbits" => info.idxoffsetbits = v.parse().unwrap_or(32),
                "sametypesequence" => info.sametypesequence = v.to_string(),
                _ => {}
            }
        }
        info
    }
}

/// The files of one dictionary, found from its `.ifo`.
#[derive(Clone, Debug)]
pub struct Paths {
    pub ifo: PathBuf,
    pub idx: PathBuf,
    pub syn: Option<PathBuf>,
    pub dict: PathBuf,
}

impl Paths {
    pub fn from_ifo(ifo: &Path) -> Option<Paths> {
        let stem = ifo.with_extension("");
        let with = |ext: &str| {
            let mut p = stem.clone().into_os_string();
            p.push(ext);
            PathBuf::from(p)
        };
        let idx = with(".idx");
        let dict = [with(".dict.dz"), with(".dict")]
            .into_iter()
            .find(|p| p.is_file())?;
        if !idx.is_file() {
            return None;
        }
        let syn = Some(with(".syn")).filter(|p| p.is_file());
        Some(Paths {
            ifo: ifo.to_path_buf(),
            idx,
            syn,
            dict,
        })
    }

    /// The name StarDict files share, e.g. "grc-pl".
    pub fn stem(&self) -> String {
        self.ifo
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// An `.idx` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdxEntry {
    pub word: String,
    pub offset: u64,
    pub size: u32,
}

/// Longest word we read in one go; StarDict limits words to 255 bytes.
const MAX_WORD: usize = 256;

/// Read the NUL-terminated word at `pos`, and `extra` bytes after it.
fn read_word(f: &File, pos: u64, extra: usize) -> io::Result<(String, Vec<u8>)> {
    let mut buf = vec![0u8; MAX_WORD + extra];
    let n = read_at_most(f, &mut buf, pos)?;
    buf.truncate(n);
    let end = buf
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unterminated word"))?;
    if buf.len() < end + 1 + extra {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short entry"));
    }
    let word = String::from_utf8_lossy(&buf[..end]).into_owned();
    Ok((word, buf[end + 1..end + 1 + extra].to_vec()))
}

/// Like `read_exact_at`, but a short read at the end of the file is fine.
fn read_at_most(f: &File, buf: &mut [u8], pos: u64) -> io::Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        match f.read_at(&mut buf[done..], pos + done as u64) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(done)
}

/// Positioned access to the `.idx` and `.syn` files.
pub struct Index {
    idx: File,
    syn: Option<File>,
    wide: bool,
}

impl Index {
    pub fn open(paths: &Paths, info: &Info) -> io::Result<Index> {
        Ok(Index {
            idx: File::open(&paths.idx)?,
            syn: paths.syn.as_ref().map(File::open).transpose()?,
            wide: info.idxoffsetbits == 64,
        })
    }

    fn entry_tail(&self) -> usize {
        if self.wide {
            12
        } else {
            8
        }
    }

    /// The `.idx` entry starting at byte `pos`.
    pub fn idx_at(&self, pos: u64) -> io::Result<IdxEntry> {
        let (word, tail) = read_word(&self.idx, pos, self.entry_tail())?;
        let (offset, size) = if self.wide {
            (
                u64::from_be_bytes(tail[..8].try_into().unwrap()),
                u32::from_be_bytes(tail[8..12].try_into().unwrap()),
            )
        } else {
            (
                u32::from_be_bytes(tail[..4].try_into().unwrap()) as u64,
                u32::from_be_bytes(tail[4..8].try_into().unwrap()),
            )
        };
        Ok(IdxEntry { word, offset, size })
    }

    /// The `.syn` entry at byte `pos`: the synonym and the number of the
    /// `.idx` entry it stands for.
    pub fn syn_at(&self, pos: u64) -> io::Result<(String, u32)> {
        let f = self
            .syn
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no .syn file"))?;
        let (word, tail) = read_word(f, pos, 4)?;
        Ok((word, u32::from_be_bytes(tail[..4].try_into().unwrap())))
    }

    /// Walk every `.idx` entry: (byte position, word). Streams the file in
    /// blocks, for building the fold index.
    pub fn walk_idx(&self, mut f: impl FnMut(u64, &str)) -> io::Result<()> {
        walk(&self.idx, self.entry_tail(), |pos, w, _| f(pos, w))
    }

    /// Walk every `.syn` entry: (byte position, word, `.idx` number).
    pub fn walk_syn(&self, mut f: impl FnMut(u64, &str, u32)) -> io::Result<()> {
        match &self.syn {
            Some(file) => walk(file, 4, |pos, w, tail| {
                f(pos, w, u32::from_be_bytes(tail[..4].try_into().unwrap()))
            }),
            None => Ok(()),
        }
    }
}

fn walk(file: &File, tail: usize, mut f: impl FnMut(u64, &str, &[u8])) -> io::Result<()> {
    const BLOCK: usize = 1 << 20;
    let mut buf = vec![0u8; BLOCK];
    let mut base = 0u64; // file position of buf[0]
    let mut have = read_at_most(file, &mut buf, 0)?;
    let mut eof = have < buf.len();
    let mut i = 0usize;
    loop {
        // A whole entry in the buffer, or refill (or stop at the end; a
        // partial entry there means a truncated file and is dropped).
        let complete = buf[i..have]
            .iter()
            .position(|&b| b == 0)
            .filter(|&z| i + z + 1 + tail <= have);
        let Some(z) = complete else {
            if eof {
                return Ok(());
            }
            base += i as u64;
            buf.copy_within(i..have, 0);
            let kept = have - i;
            let n = read_at_most(file, &mut buf[kept..], base + kept as u64)?;
            have = kept + n;
            eof = have < buf.len();
            i = 0;
            continue;
        };
        let word = String::from_utf8_lossy(&buf[i..i + z]);
        f(base + i as u64, &word, &buf[i + z + 1..i + z + 1 + tail]);
        i += z + 1 + tail;
    }
}

/// The definitions file, plain or dictzip.
pub struct Data {
    file: File,
    zip: Option<Dictzip>,
    /// The last chunk inflated: (number, bytes). Neighbouring entries share
    /// chunks, and a popup often reads several entries in a row.
    cache: Option<(usize, Vec<u8>)>,
}

struct Dictzip {
    chunk_len: usize,
    /// File offset of each chunk, plus the end of the last.
    starts: Vec<u64>,
}

impl Data {
    pub fn open(path: &Path) -> io::Result<Data> {
        let file = File::open(path)?;
        let zip = if path.extension().is_some_and(|e| e == "dz") {
            Some(Dictzip::read_header(&file)?)
        } else {
            None
        };
        Ok(Data {
            file,
            zip,
            cache: None,
        })
    }

    /// The `size` bytes at `offset` of the uncompressed text.
    pub fn read(&mut self, offset: u64, size: u32) -> io::Result<Vec<u8>> {
        let Some(zip) = &self.zip else {
            let mut buf = vec![0u8; size as usize];
            self.file.read_exact_at(&mut buf, offset)?;
            return Ok(buf);
        };
        let chunk_len = zip.chunk_len as u64;
        let mut out = Vec::with_capacity(size as usize);
        let mut pos = offset;
        let end = offset + size as u64;
        while pos < end {
            let n = (pos / chunk_len) as usize;
            let chunk = self.chunk(n)?;
            let from = (pos - n as u64 * chunk_len) as usize;
            if from >= chunk.len() {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "past the end"));
            }
            let to = chunk.len().min(from + (end - pos) as usize);
            out.extend_from_slice(&chunk[from..to]);
            pos += (to - from) as u64;
        }
        Ok(out)
    }

    fn chunk(&mut self, n: usize) -> io::Result<&[u8]> {
        if self.cache.as_ref().map(|c| c.0) != Some(n) {
            let zip = self.zip.as_ref().expect("dictzip");
            let (a, b) = match (zip.starts.get(n), zip.starts.get(n + 1)) {
                (Some(&a), Some(&b)) => (a, b),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "no such chunk",
                    ))
                }
            };
            let mut packed = vec![0u8; (b - a) as usize];
            self.file.read_exact_at(&mut packed, a)?;
            let mut out = Vec::with_capacity(zip.chunk_len);
            let mut d = Decompress::new(false);
            d.decompress_vec(&packed, &mut out, FlushDecompress::Sync)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            self.cache = Some((n, out));
        }
        Ok(&self.cache.as_ref().expect("filled").1)
    }
}

impl Dictzip {
    fn read_header(f: &File) -> io::Result<Dictzip> {
        let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
        let mut head = [0u8; 12];
        f.read_exact_at(&mut head, 0)?;
        if head[0] != 0x1f || head[1] != 0x8b || head[2] != 8 {
            return Err(bad("not gzip"));
        }
        let flags = head[3];
        if flags & 4 == 0 {
            return Err(bad("gzip without dictzip header"));
        }
        let xlen = u16::from_le_bytes([head[10], head[11]]) as usize;
        let mut extra = vec![0u8; xlen];
        f.read_exact_at(&mut extra, 12)?;
        let mut pos = 12 + xlen as u64;
        // Subfields: id (2), len (2 LE), data.
        let mut i = 0;
        let mut ra = None;
        while i + 4 <= extra.len() {
            let len = u16::from_le_bytes([extra[i + 2], extra[i + 3]]) as usize;
            if &extra[i..i + 2] == b"RA" {
                ra = extra.get(i + 4..i + 4 + len);
            }
            i += 4 + len;
        }
        let ra = ra.ok_or_else(|| bad("no RA field"))?;
        if ra.len() < 6 {
            return Err(bad("short RA field"));
        }
        let chunk_len = u16::from_le_bytes([ra[2], ra[3]]) as usize;
        let count = u16::from_le_bytes([ra[4], ra[5]]) as usize;
        if ra.len() < 6 + 2 * count {
            return Err(bad("short RA table"));
        }
        // Skip the zero-terminated name and comment, and the header CRC.
        for flag in [8u8, 16] {
            if flags & flag != 0 {
                let mut b = [0u8; 1];
                loop {
                    f.read_exact_at(&mut b, pos)?;
                    pos += 1;
                    if b[0] == 0 {
                        break;
                    }
                }
            }
        }
        if flags & 2 != 0 {
            pos += 2;
        }
        let mut starts = Vec::with_capacity(count + 1);
        starts.push(pos);
        for k in 0..count {
            pos += u16::from_le_bytes([ra[6 + 2 * k], ra[7 + 2 * k]]) as u64;
            starts.push(pos);
        }
        Ok(Dictzip { chunk_len, starts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ifo_fields() {
        let i = Info::parse(
            "StarDict's dict ifo file\nversion=3.0.0\nbookname=Wikisłownik\nwordcount=2108\nsynwordcount=5472\nidxoffsetbits=32\nsametypesequence=h\n",
        );
        assert_eq!(i.bookname, "Wikisłownik");
        assert_eq!(i.wordcount, 2108);
        assert_eq!(i.synwordcount, 5472);
        assert_eq!(i.sametypesequence, "h");
    }
}
