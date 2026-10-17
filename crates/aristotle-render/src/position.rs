//! Mapping between reading positions in the library index and locations in
//! Plato's engine.
//!
//! Plato's EPUB location is a byte offset into the concatenation of the
//! book's spine items (each item counted with its uncompressed size). The
//! library index stores positions in units that survive a change of
//! renderer:
//!
//! - EPUB: spine item and byte offset inside it ([`Spine`]);
//! - MOBI: the stock Kindle "position", a byte offset into the book's
//!   uncompressed MOBI text. Books imported from the stock reader carry
//!   these. MOBI books are read through an EPUB converted by
//!   [`crate::mobi`], whose markup differs from the MOBI's, so the
//!   converter records anchors pairing MOBI positions with locations in its
//!   EPUB ([`PosMap`]).
//!
//! Nothing here touches Plato's layout, so the mapping is exact for the text
//! and independent of font size and margins.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use percent_encoding::percent_decode_str;
use plato_core::anyhow::{format_err, Context, Error};
use plato_core::document::html::xml::XmlParser;
use plato_core::helpers::decode_entities;
use zip::ZipArchive;

/// The sizes of an EPUB's spine items, in the order and with the sizes
/// Plato's `EpubDocument` uses for its locations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spine {
    pub sizes: Vec<usize>,
    /// Archive path of each item, as Plato resolves links against it.
    pub paths: Vec<String>,
}

impl Spine {
    /// Read the spine of `epub` the way Plato does (`EpubDocument::new`):
    /// manifest items in spine order, hrefs entity- and percent-decoded and
    /// joined to the OPF's folder, items missing from the archive skipped.
    /// Plato keeps this private, so it is repeated here; a test checks that
    /// both agree.
    pub fn read(epub: &Path) -> Result<Spine, Error> {
        let mut zip = ZipArchive::new(File::open(epub)?)?;
        let container = read_entry(&mut zip, "META-INF/container.xml")?;
        let opf_path = XmlParser::new(&container)
            .parse()
            .root()
            .find("rootfile")
            .and_then(|e| e.attribute("full-path"))
            .map(String::from)
            .ok_or_else(|| format_err!("can't get the OPF path"))?;
        let parent = Path::new(&opf_path)
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        let opf = read_entry(&mut zip, &opf_path)?;
        let info = XmlParser::new(&opf).parse();
        let root = info.root();
        let manifest = root
            .find("manifest")
            .ok_or_else(|| format_err!("the manifest is missing"))?;
        let spine = root
            .find("spine")
            .ok_or_else(|| format_err!("the spine is missing"))?;
        let mut sizes = Vec::new();
        let mut paths = Vec::new();
        for child in spine.children() {
            let Some(href) = child
                .attribute("idref")
                .and_then(|idref| manifest.find_by_id(idref))
                .and_then(|entry| entry.attribute("href"))
            else {
                continue;
            };
            let href = decode_entities(href);
            let href = percent_decode_str(&href).decode_utf8_lossy();
            let path = parent.join::<&str>(href.as_ref());
            let Some(name) = path.to_str() else {
                continue;
            };
            if let Some(size) = zip.by_name(name).ok().map(|f| f.size() as usize) {
                sizes.push(size);
                paths.push(name.to_string());
            }
        }
        if sizes.is_empty() {
            return Err(format_err!("the spine is empty"));
        }
        Ok(Spine { sizes, paths })
    }

    /// Plato location where spine item `item` starts.
    pub fn start(&self, item: usize) -> Option<usize> {
        (item < self.sizes.len()).then(|| self.sizes[..item].iter().sum())
    }

    /// Spine item of an archive path.
    pub fn item_of(&self, path: &str) -> Option<usize> {
        self.paths.iter().position(|p| p == path)
    }

    /// Total size: Plato's location space is `0..total()`.
    pub fn total(&self) -> usize {
        self.sizes.iter().sum()
    }

    /// The Plato location of byte `offset` in spine item `item`, clamped to
    /// that item.
    pub fn location(&self, item: usize, offset: usize) -> Option<usize> {
        let size = *self.sizes.get(item)?;
        let start: usize = self.sizes[..item].iter().sum();
        Some(start + offset.min(size.saturating_sub(1)))
    }

    /// Spine item and offset inside it for a Plato location.
    pub fn coordinates(&self, location: usize) -> (usize, usize) {
        let mut start = 0;
        for (i, &size) in self.sizes.iter().enumerate() {
            if location < start + size {
                return (i, location - start);
            }
            start += size;
        }
        let last = self.sizes.len().saturating_sub(1);
        (
            last,
            self.sizes.get(last).map_or(0, |s| s.saturating_sub(1)),
        )
    }
}

fn read_entry(zip: &mut ZipArchive<File>, name: &str) -> Result<String, Error> {
    let mut text = String::new();
    zip.by_name(name)
        .with_context(|| format!("no {name}"))?
        .read_to_string(&mut text)?;
    Ok(text)
}

/// Anchors pairing positions in a MOBI book's text with Plato locations in
/// its converted EPUB, both strictly increasing. Positions between anchors
/// are interpolated; the converter places an anchor at every run of text it
/// copies (at most every few hundred bytes), so the error stays within a
/// line or two of text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PosMap {
    /// `(MOBI position, EPUB location)`.
    pub anchors: Vec<(u64, u64)>,
}

impl PosMap {
    /// Add an anchor, dropping any that would break monotonicity (markup the
    /// converter removes or adds can make two text runs meet).
    pub fn push(&mut self, position: u64, location: u64) {
        match self.anchors.last() {
            Some(&(p, l)) if position <= p || location <= l => {}
            _ => self.anchors.push((position, location)),
        }
    }

    /// The EPUB location of a MOBI position.
    pub fn location(&self, position: u64) -> u64 {
        interpolate(&self.anchors, position, |a| a.0, |a| a.1)
    }

    /// The MOBI position of an EPUB location.
    pub fn position(&self, location: u64) -> u64 {
        interpolate(&self.anchors, location, |a| a.1, |a| a.0)
    }

    /// The smallest MOBI position whose location is at or after `location`.
    /// [`PosMap::position`] rounds down, and where the EPUB has more bytes
    /// than the MOBI text (rewritten markup), mapping it back lands before
    /// `location`: on a page start, that is the previous page. Saving this
    /// position instead makes a page start come back to its own page.
    pub fn position_at_or_after(&self, location: u64) -> u64 {
        let mut lo = self.position(location);
        if self.location(lo) >= location {
            return lo;
        }
        let end = self.anchors.last().map_or(lo, |a| a.0);
        let mut hi = lo + 1;
        while hi < end && self.location(hi) < location {
            hi = lo + (hi - lo) * 2;
        }
        let mut hi = hi.min(end);
        if self.location(hi) < location {
            return hi;
        }
        // location() never decreases with the position: bisect (lo, hi].
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if self.location(mid) >= location {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }

    /// One anchor per line, `position location`, so the file can be read in
    /// a shell.
    pub fn to_text(&self) -> String {
        let mut s = String::with_capacity(self.anchors.len() * 16);
        for (p, l) in &self.anchors {
            s.push_str(&format!("{p} {l}\n"));
        }
        s
    }

    pub fn from_text(text: &str) -> Result<PosMap, Error> {
        let mut map = PosMap::default();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let mut it = line.split_whitespace().map(str::parse::<u64>);
            match (it.next(), it.next()) {
                (Some(Ok(p)), Some(Ok(l))) => map.push(p, l),
                _ => return Err(format_err!("bad position map line {line:?}")),
            }
        }
        Ok(map)
    }
}

/// Piecewise linear map from `key` to `value` over sorted anchors, clamped
/// at both ends.
fn interpolate<T>(
    anchors: &[T],
    x: u64,
    key: impl Fn(&T) -> u64,
    value: impl Fn(&T) -> u64,
) -> u64 {
    let Some(first) = anchors.first() else {
        return x;
    };
    let i = anchors.partition_point(|a| key(a) <= x);
    if i == 0 {
        return value(first);
    }
    let a = &anchors[i - 1];
    let Some(b) = anchors.get(i) else {
        return value(a);
    };
    let (k0, k1, v0, v1) = (key(a), key(b), value(a), value(b));
    let t = (x - k0) as f64 / (k1 - k0).max(1) as f64;
    (v0 + ((v1 - v0) as f64 * t) as u64).min(v1.saturating_sub(1).max(v0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spine_coordinates_round_trip() {
        let s = Spine {
            sizes: vec![100, 50, 200],
            paths: vec!["a".into(), "b".into(), "c".into()],
        };
        assert_eq!(s.total(), 350);
        assert_eq!(s.location(0, 0), Some(0));
        assert_eq!(s.location(1, 10), Some(110));
        assert_eq!(s.location(2, 1000), Some(349), "clamped to the item");
        assert_eq!(s.location(3, 0), None);
        for loc in [0, 99, 100, 149, 150, 349] {
            let (i, o) = s.coordinates(loc);
            assert_eq!(s.location(i, o), Some(loc));
        }
        assert_eq!(s.coordinates(10_000), (2, 199));
    }

    #[test]
    fn posmap_interpolates_and_inverts() {
        let mut m = PosMap::default();
        m.push(0, 300);
        m.push(1000, 2300);
        m.push(1000, 2400); // not increasing: dropped
        m.push(2000, 2800);
        assert_eq!(m.anchors.len(), 3);
        assert_eq!(m.location(0), 300);
        assert_eq!(m.location(500), 1300);
        assert_eq!(m.location(1500), 2550);
        assert_eq!(m.location(99_999), 2800);
        assert_eq!(m.position(1300), 500);
        assert_eq!(m.position(100), 0);
        for p in [0u64, 10, 999, 1000, 1700] {
            let back = m.position(m.location(p));
            assert!(back.abs_diff(p) <= 1, "{p} -> {back}");
        }
        // Where locations outnumber positions, rounding down loses ground;
        // position_at_or_after never lands before the location.
        for l in [300u64, 301, 1301, 1302, 2299, 2301, 2799] {
            let p = m.position_at_or_after(l);
            assert!(m.location(p) >= l, "{l} -> {p} -> {}", m.location(p));
            assert!(
                p == 0 || m.location(p - 1) < l,
                "{l} -> {p} is not the smallest"
            );
        }
        let text = m.to_text();
        assert_eq!(PosMap::from_text(&text).unwrap(), m);
        assert!(PosMap::from_text("1 x").is_err());
    }
}
