// SPDX-License-Identifier: AGPL-3.0-or-later
//! E-ink waveform hints.
//!
//! The numbers are Lab126's *Zelda* numbering used by the PW4 EPDC driver,
//! the same values as `aristotle_hal::framebuffer::Waveform`. They are kept
//! here as a small enum so this crate builds and tests on the host without
//! the HAL; the device glue converts with `Waveform(hint as u32)`.
//!
//! Measured costs on our panel (`docs/ui-framework.md`, "E-ink rendering", 120 × 120 region):
//! A2 120–144 ms, DU 309 ms, GL16 and GC16 544 ms. Because the driver runs
//! one update at a time, choosing a slower waveform than needed delays every
//! update queued behind it, so the cheapest correct waveform matters.

/// Which waveform a damaged rectangle should be shown with.
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Waveform {
    /// Direct update: any gray to pure black or white, no flash, about
    /// 300 ms. Right for black-and-white UI chrome (icons, rules, inverted
    /// selections) whose new content has no gray pixels.
    DU = 1,
    /// Full 16-level update with a flash. Clears the ghosting that
    /// accumulates from non-flashing updates; used periodically on page turns
    /// and for big context switches.
    GC16 = 2,
    /// 16 gray levels without a flash. The default for anti-aliased text and
    /// images.
    GL16 = 3,
    /// Fastest, black and white only, leaves ghosting. Used only for the
    /// touch-down acknowledgement of a control (and the live selection
    /// while a finger drags); the UI always follows it with a [`GLKW16`]
    /// update of the same pixels once the action settles.
    ///
    /// [`GLKW16`]: Waveform::GLKW16
    A2 = 6,
    /// A cleaning 16-level update without a flash: mode 9, sent in full
    /// mode so the driver drives every pixel of the rectangle, not only
    /// the ones whose value changed. Mode 9 (FBInk's GLKW16) is what the
    /// stock reader uses for every page turn on the PW4 by day,
    /// and it runs the driver's ghost-reduction steps
    /// (`docs/benchmarks.md`, "Page-turn strategy experiment") [dev].
    /// Used for the final state of anything A2 touched, for paging and
    /// for removing overlays.
    GLKW16 = 9,
}

impl Waveform {
    /// The driver mode number (Zelda numbering).
    pub fn mode(self) -> u32 {
        self as u32
    }

    /// Order by fidelity, as the HAL's display thread does when it merges
    /// damage: a merged rectangle must use the most faithful of its parts,
    /// or the parts that needed grays or a flash would lose them.
    pub fn fidelity(self) -> u8 {
        match self {
            Waveform::A2 => 0,
            Waveform::DU => 1,
            Waveform::GL16 => 2,
            Waveform::GLKW16 => 3,
            Waveform::GC16 => 4,
        }
    }

    /// The more faithful of two waveforms.
    pub fn max(self, other: Waveform) -> Waveform {
        if other.fidelity() > self.fidelity() {
            other
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zelda_numbers() {
        assert_eq!(Waveform::A2.mode(), 6);
        assert_eq!(Waveform::DU.mode(), 1);
        assert_eq!(Waveform::GL16.mode(), 3);
        assert_eq!(Waveform::GC16.mode(), 2);
        assert_eq!(Waveform::GLKW16.mode(), 9);
    }

    #[test]
    fn max_prefers_fidelity() {
        assert_eq!(Waveform::A2.max(Waveform::DU), Waveform::DU);
        assert_eq!(Waveform::GC16.max(Waveform::GL16), Waveform::GC16);
        assert_eq!(Waveform::GL16.max(Waveform::A2), Waveform::GL16);
        assert_eq!(Waveform::GL16.max(Waveform::GLKW16), Waveform::GLKW16);
        assert_eq!(Waveform::GC16.max(Waveform::GLKW16), Waveform::GC16);
    }
}
