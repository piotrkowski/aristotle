// SPDX-License-Identifier: AGPL-3.0-or-later
//! Screen geometry and density, independent of drivers and rendering.

pub const REFERENCE_DPI: u16 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Density(u16);

impl Default for Density {
    fn default() -> Self {
        Self(REFERENCE_DPI)
    }
}

impl Density {
    pub fn new(dpi: u16) -> Result<Self, &'static str> {
        if !(72..=1200).contains(&dpi) {
            return Err("DPI must be between 72 and 1200");
        }
        Ok(Self(dpi))
    }

    pub fn dpi(self) -> u16 {
        self.0
    }

    pub fn scale(self, reference: f32) -> f32 {
        reference * self.0 as f32 / REFERENCE_DPI as f32
    }

    /// Converts reference pixels to native pixels, preserving nonzero rules.
    pub fn px(self, reference: i32) -> i32 {
        let scaled = self.scale(reference as f32).round() as i32;
        if reference > 0 {
            scaled.max(1)
        } else if reference < 0 {
            scaled.min(-1)
        } else {
            0
        }
    }

    pub fn reference(self, pixels: i32) -> i32 {
        (pixels as f32 * REFERENCE_DPI as f32 / self.0 as f32).round() as i32
    }

    pub fn mm(self, value: f32) -> i32 {
        (value * self.0 as f32 / 25.4).round() as i32
    }

    pub fn pt(self, value: f32) -> f32 {
        value * self.0 as f32 / 72.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    pub width: u32,
    pub height: u32,
    pub density: Density,
}

impl Screen {
    pub fn new(width: u32, height: u32, dpi: u16) -> Result<Self, &'static str> {
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err("screen dimensions must be between 1 and 8192 pixels");
        }
        Ok(Self {
            width,
            height,
            density: Density::new(dpi)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_units_and_reference_pixels_agree() {
        for dpi in [167, 212, 300] {
            let d = Density::new(dpi).unwrap();
            assert_eq!(d.pt(72.0), dpi as f32);
            assert!((d.mm(25.4) - dpi as i32).abs() <= 1);
            assert!((d.px(106) as f32 / dpi as f32 * 25.4 - 9.0).abs() < 0.15);
            assert_eq!(d.px(1), 1);
            assert_eq!(d.px(-1), -1);
        }
        assert!(Screen::new(0, 800, 167).is_err());
        assert!(Screen::new(600, 800, 0).is_err());
    }
}
