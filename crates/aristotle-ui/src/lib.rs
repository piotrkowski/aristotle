// SPDX-License-Identifier: AGPL-3.0-or-later
//! Aristotle's UI toolkit: a small retained UI for e-ink.
//!
//! See `docs/ui-framework.md` for the architecture and design rules.

pub mod canvas;
pub mod geom;
pub mod icons;
pub mod input;
pub mod screens;
pub mod text;
pub mod theme;
pub mod tree;
pub mod ui;
pub mod waveform;
pub mod widgets;

pub use canvas::{Canvas, Painter};
pub use geom::{mm, pt, Insets, Point, Rect, Size};
pub use waveform::Waveform;
