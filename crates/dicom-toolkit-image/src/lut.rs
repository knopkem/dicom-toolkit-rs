//! Modality LUT and VOI LUT pipelines.
//!
//! Ports DCMTK's rescale/modality LUT functionality from `dcmimgle`.

// ── ModalityLut ───────────────────────────────────────────────────────────────

/// Modality LUT: converts stored pixel values to real-world (modality) values.
///
/// The linear transformation is:
/// `output = stored_pixel × slope + intercept`
///
/// Per PS 3.3 §C.7.6.3.1.2.
#[derive(Debug, Clone, PartialEq)]
pub struct ModalityLut {
    /// Additive offset applied after scaling (`RescaleIntercept`).
    pub intercept: f64,
    /// Multiplicative scale factor (`RescaleSlope`).
    pub slope: f64,
}

impl ModalityLut {
    /// Create a `ModalityLut` with the given parameters.
    pub fn new(intercept: f64, slope: f64) -> Self {
        Self { intercept, slope }
    }

    /// Identity LUT: slope = 1.0, intercept = 0.0.
    pub fn identity() -> Self {
        Self { intercept: 0.0, slope: 1.0 }
    }

    /// Returns `true` if this is an identity transformation.
    pub fn is_identity(&self) -> bool {
        (self.slope - 1.0).abs() < f64::EPSILON && self.intercept.abs() < f64::EPSILON
    }

    /// Apply the LUT to a single stored pixel value.
    pub fn apply(&self, stored: f64) -> f64 {
        stored * self.slope + self.intercept
    }

    /// Apply the LUT to a frame of 8-bit unsigned pixel data.
    pub fn apply_to_frame_u8(&self, data: &[u8]) -> Vec<f64> {
        data.iter().map(|&v| self.apply(v as f64)).collect()
    }

    /// Apply the LUT to a frame of 16-bit unsigned pixel data.
    pub fn apply_to_frame_u16(&self, data: &[u16]) -> Vec<f64> {
        data.iter().map(|&v| self.apply(v as f64)).collect()
    }

    /// Apply the LUT to a frame of 16-bit signed pixel data.
    pub fn apply_to_frame_i16(&self, data: &[i16]) -> Vec<f64> {
        data.iter().map(|&v| self.apply(v as f64)).collect()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modality_lut_identity() {
        let lut = ModalityLut::identity();
        assert!(lut.is_identity());
        let data: Vec<u16> = vec![0, 100, 1000, 65535];
        let result = lut.apply_to_frame_u16(&data);
        for (&expected, actual) in data.iter().zip(result.iter()) {
            assert!((expected as f64 - actual).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn modality_lut_ct_rescale() {
        // Typical CT: slope=1, intercept=-1024 (raw 1024 → 0 HU, raw 0 → -1024 HU)
        let lut = ModalityLut::new(-1024.0, 1.0);
        assert_eq!(lut.apply(1024.0), 0.0);
        assert_eq!(lut.apply(0.0), -1024.0);
    }

    #[test]
    fn modality_lut_non_identity() {
        let lut = ModalityLut::new(0.0, 2.0);
        assert!(!lut.is_identity());
    }
}
