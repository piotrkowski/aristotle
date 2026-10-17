//! Font faces and families.
//!
//! A family maps (bold, italic) to a face. Missing styles are synthesized
//! rather than failing: a missing bold is emboldened outlines, a missing
//! italic is a slanted roman. That keeps the bundled reading fonts small
//! (Charis and Gentium ship only Regular and Italic here) while a book that
//! uses bold still renders with visible emphasis.

use std::fmt;
use std::path::Path;
use std::sync::Arc;
use swash::{CacheKey, FontRef, Metrics};

/// Font bytes, either compiled in or loaded at runtime.
#[derive(Clone)]
pub enum FontData {
    Static(&'static [u8]),
    Shared(Arc<Vec<u8>>),
}

impl FontData {
    pub fn bytes(&self) -> &[u8] {
        match self {
            FontData::Static(b) => b,
            FontData::Shared(v) => v,
        }
    }
}

#[derive(Debug)]
pub enum FontError {
    Io(std::io::Error),
    /// The bytes are not a TrueType or OpenType font.
    Invalid,
}

impl fmt::Display for FontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FontError::Io(e) => write!(f, "font file: {e}"),
            FontError::Invalid => write!(f, "not a usable font"),
        }
    }
}

impl std::error::Error for FontError {}

impl From<std::io::Error> for FontError {
    fn from(e: std::io::Error) -> Self {
        FontError::Io(e)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FaceId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FamilyId(pub u16);

pub(crate) struct Face {
    data: FontData,
    offset: u32,
    key: CacheKey,
    /// Unscaled design metrics, cached because every line layout needs them.
    pub metrics: Metrics,
}

impl Face {
    pub fn font_ref(&self) -> FontRef<'_> {
        FontRef {
            data: self.data.bytes(),
            offset: self.offset,
            key: self.key,
        }
    }
}

/// A face chosen for a style, with the synthesis needed to fake a missing
/// weight or slant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResolvedFace {
    pub face: FaceId,
    pub embolden: bool,
    pub slant: bool,
}

pub struct Family {
    pub name: String,
    /// Indexed by `bold as usize * 2 + italic as usize`.
    faces: [Option<FaceId>; 4],
}

#[derive(Default)]
pub struct Fonts {
    faces: Vec<Face>,
    families: Vec<Family>,
}

impl Fonts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Literata in four styles, compiled into the binary.
    ///
    /// Literata was designed for Google Play Books specifically for long
    /// reading on screens, and is the family the UI chrome uses too. Its
    /// family id is always `FamilyId(0)`.
    #[cfg(feature = "embedded-fonts")]
    pub fn embedded() -> Self {
        let mut f = Self::new();
        let mut add = |b: &'static [u8]| f.add_face(FontData::Static(b)).ok();
        let r = add(include_bytes!("../../assets/fonts/Literata-Regular.ttf"));
        let i = add(include_bytes!("../../assets/fonts/Literata-Italic.ttf"));
        let b = add(include_bytes!("../../assets/fonts/Literata-Bold.ttf"));
        let bi = add(include_bytes!("../../assets/fonts/Literata-BoldItalic.ttf"));
        f.add_family("Literata", [r, i, b, bi])
            .expect("embedded Literata is valid");
        f
    }

    pub fn add_face(&mut self, data: FontData) -> Result<FaceId, FontError> {
        let (offset, key, metrics) = {
            let font = FontRef::from_index(data.bytes(), 0).ok_or(FontError::Invalid)?;
            (font.offset, font.key, font.metrics(&[]))
        };
        if metrics.units_per_em == 0 {
            return Err(FontError::Invalid);
        }
        self.faces.push(Face {
            data,
            offset,
            key,
            metrics,
        });
        Ok(FaceId(self.faces.len() as u16 - 1))
    }

    /// Registers a family from up to four faces in the order regular,
    /// italic, bold, bold italic. A regular face is required.
    pub fn add_family(&mut self, name: &str, faces: [Option<FaceId>; 4]) -> Option<FamilyId> {
        faces[0]?;
        self.families.push(Family {
            name: name.to_string(),
            faces,
        });
        Some(FamilyId(self.families.len() as u16 - 1))
    }

    /// Loads `<prefix>-Regular.ttf`, `-Italic`, `-Bold` and `-BoldItalic`
    /// from a directory; any but the regular one may be missing.
    pub fn load_family(
        &mut self,
        name: &str,
        dir: &Path,
        prefix: &str,
    ) -> Result<FamilyId, FontError> {
        let mut faces = [None; 4];
        for (slot, style) in ["Regular", "Italic", "Bold", "BoldItalic"]
            .iter()
            .enumerate()
        {
            let path = dir.join(format!("{prefix}-{style}.ttf"));
            match std::fs::read(&path) {
                Ok(bytes) => faces[slot] = Some(self.add_face(FontData::Shared(Arc::new(bytes)))?),
                Err(e) if slot == 0 => return Err(e.into()),
                Err(_) => {}
            }
        }
        self.add_family(name, faces).ok_or(FontError::Invalid)
    }

    pub fn family(&self, id: FamilyId) -> &Family {
        &self.families[id.0 as usize]
    }

    pub fn family_by_name(&self, name: &str) -> Option<FamilyId> {
        self.families
            .iter()
            .position(|f| f.name.eq_ignore_ascii_case(name))
            .map(|i| FamilyId(i as u16))
    }

    pub(crate) fn face(&self, id: FaceId) -> &Face {
        &self.faces[id.0 as usize]
    }

    /// Picks the face for a style, synthesizing what the family lacks.
    pub fn resolve(&self, family: FamilyId, bold: bool, italic: bool) -> ResolvedFace {
        let faces = &self.family(family).faces;
        let exact = faces[bold as usize * 2 + italic as usize];
        if let Some(face) = exact {
            return ResolvedFace {
                face,
                embolden: false,
                slant: false,
            };
        }
        // Prefer keeping the real italic and faking the weight: a slanted
        // roman looks more wrong than a slightly heavier italic.
        if let Some(face) = faces[italic as usize] {
            return ResolvedFace {
                face,
                embolden: bold,
                slant: false,
            };
        }
        if let Some(face) = faces[bold as usize * 2] {
            return ResolvedFace {
                face,
                embolden: false,
                slant: italic,
            };
        }
        ResolvedFace {
            face: faces[0].expect("family has a regular face"),
            embolden: bold,
            slant: italic,
        }
    }
}
