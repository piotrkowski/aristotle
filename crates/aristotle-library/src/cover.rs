//! Cover thumbnails for the e-ink library view.
//!
//! Pipeline: raw cover bytes → decode to 8-bit luminance → area-average
//! downscale to the thumbnail width → Floyd–Steinberg dither to the 16 gray
//! levels the PW4 panel shows → binary PGM in the cache, named after the MD5
//! of the source image.
//!
//! Why these choices:
//!
//! - **Grayscale early.** The panel has no colour. JPEG decoding goes
//!   straight to luminance and uses the DCT scaling of `jpeg-decoder` (1/2,
//!   1/4 or 1/8 size during the IDCT), so a 1600 × 2560 cover is decoded at
//!   200 × 320 at a fraction of the cost. That matters on the 1 GHz A9.
//! - **16 levels, dithered.** GL16 and GC16 waveforms render 16 levels
//!   (0x00, 0x11, ... 0xFF). Values in between are rounded by the EPDC, which
//!   bands smooth gradients. Dithering ourselves keeps gradients smooth.
//!   Floyd–Steinberg beats ordered dithering for still images; the usual
//!   argument for ordered dither (stable patterns under partial refresh)
//!   does not apply to a thumbnail that is drawn once.
//! - **PGM.** Trivial to write and read, viewable on any desktop, and the
//!   pixel data is the exact byte layout of the 8 bpp framebuffer, so the UI
//!   can blit it without converting.
//! - **Content-keyed cache.** The key is the MD5 of the cover image, plus
//!   the thumbnail size, so re-scans, renames and duplicate books never
//!   redo the work, and a changed cover gets a new file automatically.

use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Thumbnail geometry. Width is fixed; height follows the aspect ratio but is
/// clamped so that a panorama or a strip does not produce an absurd tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThumbSpec {
    pub width: u32,
    pub max_height: u32,
}

impl Default for ThumbSpec {
    /// 200 px is about 1.7 cm on the 300 ppi panel: five covers per row in
    /// portrait with margins, readable titles on most covers.
    fn default() -> Self {
        ThumbSpec {
            width: 200,
            max_height: 320,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Jpeg,
    Png,
    Gif,
    Webp,
}

pub fn sniff_image(b: &[u8]) -> Option<ImageKind> {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageKind::Jpeg)
    } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageKind::Png)
    } else if b.starts_with(b"GIF8") {
        Some(ImageKind::Gif)
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(ImageKind::Webp)
    } else {
        None
    }
}

pub fn is_image_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".gif", ".webp"]
        .iter()
        .any(|e| l.ends_with(e))
        && !l.starts_with("__macosx/")
}

/// An 8-bit grayscale image, row-major, no padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gray {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Refuse to fully decode images larger than this (PNG has no scaled
/// decode); 40 MP of luminance is already 40 MB on a 512 MB device.
const MAX_PIXELS: u64 = 40_000_000;

/// Decodes a JPEG or PNG to luminance. JPEGs are decoded at the smallest DCT
/// scale that is still at least `min_width` wide.
pub fn decode_gray(data: &[u8], min_width: u32) -> Result<Gray> {
    match sniff_image(data) {
        Some(ImageKind::Jpeg) => decode_jpeg(data, min_width),
        Some(ImageKind::Png) => decode_png(data),
        Some(k) => Err(Error::format(format!("cover: {k:?} is not supported"))),
        None => Err(Error::format("cover: not an image")),
    }
}

fn decode_jpeg(data: &[u8], min_width: u32) -> Result<Gray> {
    let mut dec = jpeg_decoder::Decoder::new(Cursor::new(data));
    dec.read_info()
        .map_err(|e| Error::format(format!("jpeg: {e}")))?;
    let info = dec.info().ok_or_else(|| Error::format("jpeg: no header"))?;
    let (w, h) = (info.width as u32, info.height as u32);
    if w == 0 || h == 0 {
        return Err(Error::format("jpeg: empty image"));
    }
    let req_w = min_width.min(w);
    let req_h = ((req_w as u64 * h as u64).div_ceil(w as u64)) as u32;
    let (sw, sh) = dec
        .scale(
            req_w.min(u16::MAX as u32) as u16,
            req_h.clamp(1, u16::MAX as u32) as u16,
        )
        .map_err(|e| Error::format(format!("jpeg: {e}")))?;
    let px = dec
        .decode()
        .map_err(|e| Error::format(format!("jpeg: {e}")))?;
    let info = dec.info().ok_or_else(|| Error::format("jpeg: no header"))?;
    let (sw, sh) = (sw as u32, sh as u32);
    let n = (sw * sh) as usize;
    use jpeg_decoder::PixelFormat::*;
    let pixels: Vec<u8> = match info.pixel_format {
        L8 => px,
        L16 => px.as_chunks::<2>().0.iter().map(|c| c[0]).collect(),
        RGB24 => px
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| luma(c[0], c[1], c[2]))
            .collect(),
        // jpeg-decoder returns CMYK as stored; Adobe files store it inverted,
        // which this formula assumes, as most cover JPEGs in CMYK come from Adobe tools.
        CMYK32 => px
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| {
                let k = c[3] as u32;
                let l = luma(c[0], c[1], c[2]) as u32;
                (l * k / 255) as u8
            })
            .collect(),
    };
    if pixels.len() != n {
        return Err(Error::format("jpeg: unexpected pixel count"));
    }
    Ok(Gray {
        width: sw,
        height: sh,
        pixels,
    })
}

fn decode_png(data: &[u8]) -> Result<Gray> {
    let mut dec = png::Decoder::new(Cursor::new(data));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec
        .read_info()
        .map_err(|e| Error::format(format!("png: {e}")))?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err(Error::format("png: too large"));
    }
    let mut buf = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or_else(|| Error::format("png: too large"))?
    ];
    let frame = reader
        .next_frame(&mut buf)
        .map_err(|e| Error::format(format!("png: {e}")))?;
    let buf = &buf[..frame.buffer_size()];
    use png::ColorType::*;
    // Transparent areas are composited over white, the colour of the page.
    let over_white = |v: u8, a: u8| ((v as u32 * a as u32 + 255 * (255 - a as u32)) / 255) as u8;
    let pixels: Vec<u8> = match frame.color_type {
        Grayscale => buf.to_vec(),
        GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| over_white(c[0], c[1]))
            .collect(),
        Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| luma(c[0], c[1], c[2]))
            .collect(),
        Rgba => buf
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| over_white(luma(c[0], c[1], c[2]), c[3]))
            .collect(),
        Indexed => return Err(Error::format("png: palette not expanded")),
    };
    if pixels.len() != (frame.width * frame.height) as usize {
        return Err(Error::format("png: unexpected pixel count"));
    }
    Ok(Gray {
        width: frame.width,
        height: frame.height,
        pixels,
    })
}

/// Rec. 601 luma in integer arithmetic.
fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114 + 500) / 1000) as u8
}

/// Resizes with area averaging when shrinking (every source pixel
/// contributes in proportion to its overlap, so thin text on covers does not
/// alias away) and bilinear interpolation when enlarging.
pub fn resize(src: &Gray, w: u32, h: u32) -> Gray {
    let tmp = resize_axis(&src.pixels, src.width, src.height, w, true);
    let pixels = resize_axis(&tmp, w, src.height, h, false);
    Gray {
        width: w,
        height: h,
        pixels,
    }
}

/// Resamples one axis of an `sw` × `sh` image to length `d`: the width when
/// `horizontal`, else the height. Index closures let both passes share the
/// kernel.
fn resize_axis(px: &[u8], sw: u32, sh: u32, d: u32, horizontal: bool) -> Vec<u8> {
    let (s_len, lines) = if horizontal { (sw, sh) } else { (sh, sw) };
    let (out_w, out_h) = if horizontal { (d, sh) } else { (sw, d) };
    let mut out = vec![0u8; (out_w * out_h) as usize];
    let at = |line: u32, i: u32| -> usize {
        if horizontal {
            (line * sw + i) as usize
        } else {
            (i * sw + line) as usize
        }
    };
    let at_out = |line: u32, i: u32| -> usize {
        if horizontal {
            (line * out_w + i) as usize
        } else {
            (i * out_w + line) as usize
        }
    };
    let scale = s_len as f32 / d as f32;
    for line in 0..lines {
        for i in 0..d {
            let v = if scale > 1.0 {
                let start = i as f32 * scale;
                let end = start + scale;
                let mut acc = 0.0f32;
                let mut j = start.floor() as u32;
                while (j as f32) < end && j < s_len {
                    let lo = start.max(j as f32);
                    let hi = end.min(j as f32 + 1.0);
                    acc += px[at(line, j)] as f32 * (hi - lo);
                    j += 1;
                }
                acc / scale
            } else {
                let x = ((i as f32 + 0.5) * scale - 0.5).max(0.0);
                let j = (x.floor() as u32).min(s_len - 1);
                let k = (j + 1).min(s_len - 1);
                let t = x - j as f32;
                px[at(line, j)] as f32 * (1.0 - t) + px[at(line, k)] as f32 * t
            };
            out[at_out(line, i)] = v.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Serpentine Floyd–Steinberg error diffusion to 16 evenly spaced levels
/// (multiples of 17), the values the GL16/GC16 waveforms reproduce exactly.
pub fn dither16(img: &mut Gray) {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut cur = vec![0i32; w + 2];
    let mut next = vec![0i32; w + 2];
    for y in 0..h {
        let ltr = y % 2 == 0;
        for step in 0..w {
            let x = if ltr { step } else { w - 1 - step };
            let i = y * w + x;
            let old = (img.pixels[i] as i32 * 16 + cur[x + 1]).clamp(0, 255 * 16);
            let q = ((old + 8 * 17) / (17 * 16)).min(15) * 17;
            img.pixels[i] = q as u8;
            let err = old - q * 16;
            // Values and errors are kept in 1/16 gray steps, so the 7/3/5/1
            // sixteenths lose little to integer rounding.
            let (fwd, back) = if ltr { (x + 2, x) } else { (x, x + 2) };
            cur[fwd] += err * 7 / 16;
            next[back] += err * 3 / 16;
            next[x + 1] += err * 5 / 16;
            next[fwd] += err / 16;
        }
        std::mem::swap(&mut cur, &mut next);
        next.iter_mut().for_each(|e| *e = 0);
    }
}

/// Full pipeline from cover bytes to a dithered thumbnail.
pub fn make_thumbnail(data: &[u8], spec: ThumbSpec) -> Result<Gray> {
    let src = decode_gray(data, spec.width)?;
    let h = ((spec.width as u64 * src.height as u64 + src.width as u64 / 2) / src.width as u64)
        .clamp(1, spec.max_height as u64) as u32;
    let mut t = resize(&src, spec.width, h);
    dither16(&mut t);
    Ok(t)
}

pub fn write_pgm(path: &Path, img: &Gray) -> Result<()> {
    let mut data = format!("P5\n{} {}\n255\n", img.width, img.height).into_bytes();
    data.extend_from_slice(&img.pixels);
    crate::index::write_atomic(path, &data)
}

pub fn read_pgm(path: &Path) -> Result<Gray> {
    let data = fs::read(path)?;
    let mut fields = Vec::new();
    let mut p = 0;
    while fields.len() < 4 {
        while p < data.len() && data[p].is_ascii_whitespace() {
            p += 1;
        }
        let s = p;
        while p < data.len() && !data[p].is_ascii_whitespace() {
            p += 1;
        }
        if s == p {
            return Err(Error::format("pgm: truncated header"));
        }
        fields.push(String::from_utf8_lossy(&data[s..p]).into_owned());
    }
    p += 1;
    let num = |i: usize| {
        fields[i]
            .parse::<u32>()
            .map_err(|_| Error::format("pgm: bad header"))
    };
    if fields[0] != "P5" || num(3)? != 255 {
        return Err(Error::format("pgm: not an 8-bit P5 file"));
    }
    let (width, height) = (num(1)?, num(2)?);
    let pixels = data
        .get(p..p + (width * height) as usize)
        .ok_or_else(|| Error::format("pgm: truncated"))?
        .to_vec();
    Ok(Gray {
        width,
        height,
        pixels,
    })
}

/// The on-disk thumbnail cache.
pub struct CoverCache {
    dir: PathBuf,
    spec: ThumbSpec,
}

impl CoverCache {
    pub fn new(dir: PathBuf, spec: ThumbSpec) -> CoverCache {
        CoverCache { dir, spec }
    }

    pub fn spec(&self) -> ThumbSpec {
        self.spec
    }

    /// Cache key for a cover image: content hash plus geometry, so changing
    /// the thumbnail size never serves stale files.
    pub fn key(&self, cover: &[u8]) -> String {
        format!(
            "{}-{}x{}",
            crate::id::md5_hex(cover),
            self.spec.width,
            self.spec.max_height
        )
    }

    pub fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.pgm"))
    }

    /// Returns the key, creating the thumbnail unless it is already cached.
    pub fn store(&self, cover: &[u8]) -> Result<String> {
        let key = self.key(cover);
        let path = self.path(&key);
        if !path.exists() {
            let thumb = make_thumbnail(cover, self.spec)?;
            fs::create_dir_all(&self.dir)?;
            write_pgm(&path, &thumb)?;
        }
        Ok(key)
    }
}

/// Writes `img` as PNG (8-bit gray) for viewing on the host.
pub fn write_png(path: &Path, img: &Gray) -> Result<()> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc
            .write_header()
            .map_err(|e| Error::format(format!("png: {e}")))?;
        w.write_image_data(&img.pixels)
            .map_err(|e| Error::format(format!("png: {e}")))?;
    }
    let mut f = fs::File::create(path)?;
    f.write_all(&out)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod testutil {
    /// A gradient PNG, RGB, `w` × `h`.
    pub fn gradient_png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().unwrap();
            let mut data = Vec::new();
            for _y in 0..h {
                for x in 0..w {
                    let v = (x * 255 / (w - 1).max(1)) as u8;
                    data.extend_from_slice(&[v, v, v]);
                }
            }
            wr.write_image_data(&data).unwrap();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_has_16_levels_and_keeps_tone() {
        let png = testutil::gradient_png(600, 900);
        let t = make_thumbnail(&png, ThumbSpec::default()).unwrap();
        assert_eq!((t.width, t.height), (200, 300));
        assert!(t.pixels.iter().all(|&v| v % 17 == 0));
        // Dithering preserves mean brightness of each column band.
        let mean = |x0: u32, x1: u32| {
            let mut s = 0u64;
            let mut n = 0u64;
            for y in 0..t.height {
                for x in x0..x1 {
                    s += t.pixels[(y * t.width + x) as usize] as u64;
                    n += 1;
                }
            }
            s as f64 / n as f64
        };
        let expected = |x0: u32, x1: u32| (x0 + x1) as f64 / 2.0 / 199.0 * 255.0;
        for (a, b) in [(20, 40), (90, 110), (160, 180)] {
            assert!(
                (mean(a, b) - expected(a, b)).abs() < 4.0,
                "band {a}-{b}: {} vs {}",
                mean(a, b),
                expected(a, b)
            );
        }
        // A gradient must use many levels, not posterise to a few.
        let mut levels: Vec<u8> = t.pixels.clone();
        levels.sort();
        levels.dedup();
        assert!(levels.len() >= 14);
    }

    #[test]
    fn pgm_roundtrip_and_cache() {
        let dir = crate::testdir("cover");
        let cache = CoverCache::new(
            dir.join("covers"),
            ThumbSpec {
                width: 50,
                max_height: 60,
            },
        );
        let png = testutil::gradient_png(100, 400);
        let key = cache.store(&png).unwrap();
        let g = read_pgm(&cache.path(&key)).unwrap();
        assert_eq!((g.width, g.height), (50, 60));
        assert_eq!(cache.store(&png).unwrap(), key);
    }

    #[test]
    fn upscales_small_covers() {
        let png = testutil::gradient_png(40, 60);
        let t = make_thumbnail(&png, ThumbSpec::default()).unwrap();
        assert_eq!((t.width, t.height), (200, 300));
    }
}
