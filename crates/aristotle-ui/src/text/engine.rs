// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shaping and glyph rasterization with `swash`.
//!
//! Shaping runs OpenType GSUB and GPOS, so kerning pairs, ligatures and
//! mark positioning come from the font itself rather than from the legacy
//! `kern` table only. Rasterization uses swash's scaler with hinting on.
//! swash 0.2 hints through skrifa in its "smooth" mode, which snaps vertical
//! features (baseline, x-height, stems of horizontals) to the pixel grid but
//! keeps horizontal metrics linear. That is the right trade on a 300 DPI
//! panel: crisper x-height and baseline in 16 gray levels, while justified
//! lines keep exact widths and glyphs can sit at quarter-pixel positions.

use super::font::{FaceId, FamilyId, Fonts, ResolvedFace};
use crate::canvas::Painter;
use std::collections::HashMap;
use std::ops::Range;
use swash::scale::{Render, ScaleContext, Source};
use swash::shape::ShapeContext;
use swash::text::{Language, Script};
use swash::zeno::{Angle, Format, Transform, Vector};

/// How a piece of text looks.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    density: crate::geom::Density,
    pub family: FamilyId,
    /// Font size in native pixels after applying the screen density.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    /// Ink level, 0 black to 255 white.
    pub gray: u8,
    /// Line pitch as a multiple of `size`.
    pub line_height: f32,
    /// Extra space after every character, in pixels. Small caps and
    /// all-caps labels need a little to stay legible.
    pub tracking: f32,
    /// OpenType small capitals (`smcp`).
    pub small_caps: bool,
    /// Old-style (lowercase) figures (`onum`), which sit better in running
    /// text; UI numbers keep the default lining figures.
    pub oldstyle: bool,
}

impl TextStyle {
    pub fn new(family: FamilyId, size: f32) -> Self {
        Self {
            density: crate::geom::Density::default(),
            family,
            size,
            bold: false,
            italic: false,
            gray: 0,
            line_height: 1.35,
            tracking: 0.0,
            small_caps: false,
            oldstyle: false,
        }
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn gray(mut self, gray: u8) -> Self {
        self.gray = gray;
        self
    }

    pub fn line_height(mut self, lh: f32) -> Self {
        self.line_height = lh;
        self
    }

    pub fn tracking(mut self, t: f32) -> Self {
        self.tracking = self.density.scale(t);
        self
    }

    pub fn small_caps(mut self) -> Self {
        self.small_caps = true;
        self
    }

    pub fn oldstyle(mut self) -> Self {
        self.oldstyle = true;
        self
    }

    /// Sets the size in reference pixels at 300 DPI.
    pub fn size(mut self, size: f32) -> Self {
        self.size = self.density.scale(size);
        self
    }

    /// Sets an already measured native pixel size.
    pub fn size_pixels(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Changes density without scaling native metrics twice.
    pub fn at_density(mut self, density: crate::geom::Density) -> Self {
        let ratio = density.dpi() as f32 / self.density.dpi() as f32;
        self.size *= ratio;
        self.tracking *= ratio;
        self.density = density;
        self
    }
}

/// Vertical metrics of a style, in pixels.
#[derive(Clone, Copy, Debug)]
pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    /// Distance between baselines.
    pub pitch: f32,
    pub x_height: f32,
    pub cap_height: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct RunGlyph {
    pub id: u16,
    /// Pen position relative to the start of the run, plus GPOS offset.
    pub x: f32,
    pub y: f32,
}

/// One cluster: the smallest unit text can be split at. Ligatures are one
/// cluster, so a hyphenation point inside "ffi" is never taken.
#[derive(Clone, Debug)]
pub struct RunCluster {
    /// Byte range in the shaped string.
    pub source: Range<u32>,
    pub x: f32,
    pub advance: f32,
    pub glyphs: Range<u32>,
}

/// The result of shaping one string in one style.
#[derive(Clone, Debug)]
pub struct ShapedRun {
    pub face: ResolvedFace,
    pub size: f32,
    pub gray: u8,
    pub glyphs: Vec<RunGlyph>,
    pub clusters: Vec<RunCluster>,
    pub width: f32,
    /// The shaped string, kept by paragraph layout for ellipsis and
    /// debugging.
    pub text: Option<String>,
    /// Index of the span style this run came from, within its paragraph.
    pub style: usize,
}

impl ShapedRun {
    /// Index of the cluster starting at byte `offset`, if a cluster boundary
    /// is there.
    pub fn cluster_at(&self, offset: u32) -> Option<usize> {
        self.clusters
            .iter()
            .position(|c| c.source.start == offset)
            .or_else(|| {
                (self.clusters.last().map(|c| c.source.end) == Some(offset))
                    .then_some(self.clusters.len())
            })
    }

    /// Width of the clusters in `range`.
    pub fn width_of(&self, range: Range<usize>) -> f32 {
        if range.is_empty() {
            return 0.0;
        }
        let first = &self.clusters[range.start];
        let last = &self.clusters[range.end - 1];
        last.x + last.advance - first.x
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    face: FaceId,
    glyph: u16,
    /// Size in 1/16 px.
    size: u32,
    /// Horizontal subpixel phase in quarter pixels.
    phase: u8,
    embolden: bool,
    slant: bool,
}

struct GlyphImage {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    data: Vec<u8>,
}

/// Shapes, measures and draws text. Owns the fonts and the glyph cache.
pub struct TextEngine {
    pub fonts: Fonts,
    pub(crate) density: crate::geom::Density,
    shape: ShapeContext,
    scale: ScaleContext,
    cache: HashMap<GlyphKey, Option<GlyphImage>>,
    language: Option<Language>,
    /// Coverage curve applied when glyphs are rasterized; see
    /// [`TextEngine::set_darkening`].
    lut: [u8; 256],
}

impl TextEngine {
    pub fn new(fonts: Fonts) -> Self {
        let mut e = Self {
            fonts,
            density: crate::geom::Density::default(),
            shape: ShapeContext::new(),
            scale: ScaleContext::new(),
            cache: HashMap::new(),
            language: Language::parse("pl"),
            lut: [0; 256],
        };
        e.set_darkening(0.8);
        e
    }

    pub fn density(&self) -> crate::geom::Density {
        self.density
    }

    /// Sets the language used for `locl` shaping features (default Polish).
    pub fn set_language(&mut self, tag: &str) {
        self.language = Language::parse(tag);
        self.cache.clear();
    }

    /// Applies `coverage^gamma` to anti-aliased edges. Below 1.0 thickens
    /// strokes slightly. E-ink shows only 16 levels and its light grays
    /// wash out under the frontlight, so unadjusted text looks thin and
    /// gray next to the stock reader; 0.8 restores the weight without
    /// filling counters at small sizes.
    pub fn set_darkening(&mut self, gamma: f32) {
        for (i, v) in self.lut.iter_mut().enumerate() {
            *v = (255.0 * (i as f32 / 255.0).powf(gamma)).round() as u8;
        }
        self.cache.clear();
    }

    pub fn metrics(&self, style: &TextStyle) -> LineMetrics {
        let face = self.fonts.resolve(style.family, style.bold, style.italic);
        let m = self.fonts.face(face.face).metrics.scale(style.size);
        LineMetrics {
            ascent: m.ascent,
            descent: m.descent,
            pitch: (style.size * style.line_height).round(),
            x_height: m.x_height,
            cap_height: m.cap_height,
        }
    }

    /// Shapes `text` in `style`. Advances include `style.tracking`.
    pub fn shape(&mut self, text: &str, style: &TextStyle) -> ShapedRun {
        let face = self.fonts.resolve(style.family, style.bold, style.italic);
        let font = self.fonts.face(face.face).font_ref();
        let mut features: Vec<(&str, u16)> = vec![("kern", 1), ("liga", 1)];
        if style.small_caps {
            features.push(("smcp", 1));
        }
        if style.oldstyle {
            features.push(("onum", 1));
        }
        let mut shaper = self
            .shape
            .builder(font)
            .script(Script::Latin)
            .language(self.language)
            .size(style.size)
            .features(features.iter().copied())
            .build();
        shaper.add_str(text);
        let mut glyphs = Vec::new();
        let mut clusters = Vec::new();
        let mut pen = 0.0f32;
        // Synthetic bold widens every glyph a little; keep advances in step
        // so emboldened words do not collide.
        let extra = if face.embolden {
            style.size / 40.0
        } else {
            0.0
        };
        shaper.shape_with(|c| {
            let start = glyphs.len() as u32;
            let x = pen;
            for g in c.glyphs {
                glyphs.push(RunGlyph {
                    id: g.id,
                    x: pen + g.x,
                    y: g.y,
                });
                pen += g.advance;
            }
            if !c.glyphs.is_empty() {
                pen += style.tracking + extra;
            }
            clusters.push(RunCluster {
                source: c.source.start..c.source.end,
                x,
                advance: pen - x,
                glyphs: start..glyphs.len() as u32,
            });
        });
        ShapedRun {
            face,
            size: style.size,
            gray: style.gray,
            glyphs,
            clusters,
            width: pen,
            text: None,
            style: 0,
        }
    }

    /// Advance width of `text` in `style`.
    pub fn measure(&mut self, text: &str, style: &TextStyle) -> f32 {
        self.shape(text, style).width
    }

    /// Draws clusters `range` of `run` with the first one's pen position at
    /// `x` and the baseline at `baseline`.
    pub fn draw(
        &mut self,
        p: &mut Painter,
        run: &ShapedRun,
        range: Range<usize>,
        x: f32,
        baseline: f32,
    ) {
        if range.is_empty() {
            return;
        }
        let x0 = x - run.clusters[range.start].x;
        let g0 = run.clusters[range.start].glyphs.start;
        let g1 = run.clusters[range.end - 1].glyphs.end;
        let by = baseline.round() as i32;
        for g in &run.glyphs[g0 as usize..g1 as usize] {
            let gx = x0 + g.x;
            let ix = gx.floor();
            let phase = ((gx - ix) * 4.0).floor() as u8;
            let key = GlyphKey {
                face: run.face.face,
                glyph: g.id,
                size: (run.size * 16.0).round() as u32,
                phase,
                embolden: run.face.embolden,
                slant: run.face.slant,
            };
            let Some(img) = self.glyph(key) else { continue };
            p.blend_mask(
                ix as i32 + img.left,
                by - (g.y.round() as i32) - img.top,
                img.width,
                img.height,
                &img.data,
                run.gray,
            );
        }
    }

    fn glyph(&mut self, key: GlyphKey) -> Option<&GlyphImage> {
        if !self.cache.contains_key(&key) {
            let img = self.rasterize(key);
            self.cache.insert(key, img);
        }
        self.cache.get(&key).and_then(|g| g.as_ref())
    }

    fn rasterize(&mut self, key: GlyphKey) -> Option<GlyphImage> {
        let font = self.fonts.face(key.face).font_ref();
        let mut scaler = self
            .scale
            .builder(font)
            .size(key.size as f32 / 16.0)
            .hint(true)
            .build();
        let mut render = Render::new(&[Source::Outline]);
        render
            .format(Format::Alpha)
            .offset(Vector::new(key.phase as f32 / 4.0, 0.0));
        if key.embolden {
            render.embolden(key.size as f32 / 16.0 / 40.0);
        }
        if key.slant {
            render.transform(Some(Transform::skew(
                Angle::from_degrees(12.0),
                Angle::ZERO,
            )));
        }
        let img = render.render(&mut scaler, key.glyph)?;
        let data = img.data.iter().map(|&a| self.lut[a as usize]).collect();
        Some(GlyphImage {
            left: img.placement.left,
            top: img.placement.top,
            width: img.placement.width,
            height: img.placement.height,
            data,
        })
    }

    /// True if every character of `text` maps to a real glyph in the face
    /// used for `style`. Used to check diacritic coverage.
    pub fn covers(&self, text: &str, style: &TextStyle) -> bool {
        let face = self.fonts.resolve(style.family, style.bold, style.italic);
        let cmap = self.fonts.face(face.face).font_ref().charmap();
        text.chars().all(|c| cmap.map(c) != 0)
    }
}
