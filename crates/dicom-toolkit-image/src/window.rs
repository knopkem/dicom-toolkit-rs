//! Window/level (VOI LUT) transformation.
//!
//! Implements the DICOM linear windowing function per PS 3.3 §C.7.6.3.1.5.

// ── WindowLevel ───────────────────────────────────────────────────────────────

/// A window/level (VOI LUT) transformation.
///
/// Maps a linear range of input values to an output range, clamping values
/// that fall outside the window.  Follows the DICOM linear windowing formula
/// defined in PS 3.3 §C.7.6.3.1.5.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowLevel {
    /// Window center (center of the input range mapped to the output midpoint).
    pub center: f64,
    /// Window width (total extent of the input range linearly mapped to output).
    pub width: f64,
}

impl WindowLevel {
    /// Create a new `WindowLevel` with the given center and width.
    pub fn new(center: f64, width: f64) -> Self {
        Self { center, width }
    }

    /// Apply the VOI LUT function to a single pixel value.
    ///
    /// Returns a value in `[output_min, output_max]`.
    ///
    /// Formula (PS 3.3 §C.7.6.3.1.5):
    /// - if `x ≤ c − 0.5 − (w−1)/2`: output = `output_min`
    /// - if `x > c − 0.5 + (w−1)/2`: output = `output_max`
    /// - else: `((x − (c − 0.5)) / (w − 1) + 0.5) × (output_max − output_min) + output_min`
    pub fn apply(&self, value: f64, output_min: f64, output_max: f64) -> f64 {
        let c = self.center;
        // Clamp width to ≥ 1 to avoid a division-by-zero; width ≤ 1 gives a step function.
        let w = self.width.max(1.0);

        let lower = c - 0.5 - (w - 1.0) / 2.0;
        let upper = c - 0.5 + (w - 1.0) / 2.0;

        if value <= lower {
            output_min
        } else if value > upper {
            output_max
        } else {
            let t = (value - (c - 0.5)) / (w - 1.0) + 0.5;
            t * (output_max - output_min) + output_min
        }
    }

    /// Apply the window/level transformation to a slice of f64 pixel values,
    /// returning 8-bit output values (0–255).
    pub fn apply_to_frame(&self, pixels: &[f64]) -> Vec<u8> {
        pixels
            .iter()
            .map(|&v| self.apply(v, 0.0, 255.0).round().clamp(0.0, 255.0) as u8)
            .collect()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_level_clamps_below() {
        // center=128, width=256 → lower = 128 - 0.5 - 127.5 = 0
        let wl = WindowLevel::new(128.0, 256.0);
        assert_eq!(wl.apply(-1.0, 0.0, 255.0) as u8, 0);
        assert_eq!(wl.apply(0.0, 0.0, 255.0) as u8, 0);
    }

    #[test]
    fn window_level_clamps_above() {
        // center=128, width=256 → upper = 128 - 0.5 + 127.5 = 255
        let wl = WindowLevel::new(128.0, 256.0);
        assert_eq!(wl.apply(256.0, 0.0, 255.0) as u8, 255);
        assert_eq!(wl.apply(300.0, 0.0, 255.0) as u8, 255);
    }

    #[test]
    fn window_level_linear() {
        // center=128, width=256 → at x=128: t = (128 - 127.5)/255 + 0.5 ≈ 0.502
        let wl = WindowLevel::new(128.0, 256.0);
        let out = wl.apply(128.0, 0.0, 255.0);
        assert!((out - 128.0).abs() < 1.0, "expected ~128, got {out}");
    }
}
