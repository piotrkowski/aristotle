//! Text: fonts, shaping, rasterization, hyphenation and paragraph layout.

pub mod engine;
pub mod font;
pub mod hyphen;
pub mod layout;

pub use engine::{LineMetrics, ShapedRun, TextEngine, TextStyle};
pub use font::{FaceId, FamilyId, FontData, FontError, Fonts};
pub use hyphen::{ui_hyphenator, Hyphenator, NoHyphenation, PolishSyllables};
pub use layout::{layout_paragraph, Align, Paragraph, ParagraphLayout, ParagraphStyle, Span};
