//! Aristotle's UI toolkit: a small retained UI for e-ink.
//!
//! See `docs/ui-framework.md` for the architecture and design rules.

pub mod canvas;
pub mod geom;
pub mod icons;
pub mod input;
pub mod text;
pub mod tree;
pub mod ui;
pub mod waveform;

pub use canvas::{Canvas, Painter};
pub use geom::{mm, pt, Insets, Point, Rect, Size};
pub use waveform::Waveform;
