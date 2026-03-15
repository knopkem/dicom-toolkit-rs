//! JPEG encoding parameters.

use jpeg_encoder::SamplingFactor;

/// JPEG codec encoding parameters, mirroring DCMTK's `DJ_RPLossless`/`DJ_RPLossy`.
#[derive(Debug, Clone)]
pub struct JpegParams {
    /// JPEG quality factor 1–100. 75 is a typical clinical default.
    pub quality: u8,
    /// Chroma sub-sampling factor for color images.
    pub sampling_factor: SamplingFactor,
    /// Whether to write a JFIF APP0 header (required for some viewers).
    pub write_jfif: bool,
}

impl Default for JpegParams {
    fn default() -> Self {
        Self {
            quality: 75,
            sampling_factor: SamplingFactor::F_2_2,
            write_jfif: true,
        }
    }
}

impl JpegParams {
    /// Lossless-quality parameters (quality=100, no sub-sampling).
    pub fn lossless() -> Self {
        Self {
            quality: 100,
            sampling_factor: SamplingFactor::F_1_1,
            write_jfif: true,
        }
    }
}
