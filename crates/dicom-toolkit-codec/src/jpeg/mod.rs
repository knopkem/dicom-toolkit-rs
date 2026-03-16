//! JPEG codec support — JPEG baseline/extended plus classic JPEG Lossless.
//!
//! Wraps the `jpeg-decoder` and `jpeg-encoder` crates, plus the in-workspace
//! classic JPEG Lossless encoder, and adapts them to the DICOM encapsulated
//! pixel data model.

pub mod decoder;
pub mod encoder;
pub mod lossless_encoder;
pub mod params;

use dicom_toolkit_core::error::DcmResult;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use decoder::{decode_jpeg, JpegFrame};
pub use encoder::encode_jpeg;
pub use lossless_encoder::encode_jpeg_lossless;
pub use params::JpegParams;

// ── JpegDecoder ───────────────────────────────────────────────────────────────

/// A decoded JPEG frame with pixel data and dimensions.
#[derive(Debug)]
pub struct DecodedFrame {
    /// Raw pixel bytes (grayscale L8, RGB24, or CMYK32).
    pub pixels: Vec<u8>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Number of colour components (1 = grayscale, 3 = RGB, 4 = CMYK).
    pub components: u8,
}

/// JPEG codec for DICOM — decodes JPEG Baseline, Extended, and Lossless
/// transfer syntaxes.
///
/// JPEG Lossless (PS3.4 Process 14) decoding depends on `jpeg-decoder`
/// support; it may return [`dicom_toolkit_core::error::DcmError::DecompressionError`]
/// if the lossless process is not supported by the library.
pub struct JpegDecoder {
    /// Transfer syntax UID this instance is associated with.
    pub transfer_syntax_uid: &'static str,
}

impl JpegDecoder {
    /// Decode a JPEG-compressed fragment to raw pixel bytes.
    ///
    /// Works for JPEG Baseline (1.2.840.10008.1.2.4.50),
    /// JPEG Extended (…4.51), JPEG Lossless (…4.57, …4.70).
    pub fn decode_frame(data: &[u8]) -> DcmResult<DecodedFrame> {
        let frame = decode_jpeg(data)?;
        Ok(DecodedFrame {
            pixels: frame.data,
            width: frame.width as u32,
            height: frame.height as u32,
            components: frame.samples_per_pixel,
        })
    }
}

// ── JPEG transfer syntax UIDs ─────────────────────────────────────────────────

/// JPEG Baseline (Process 1) transfer syntax UID.
pub const TS_JPEG_BASELINE: &str = "1.2.840.10008.1.2.4.50";
/// JPEG Extended (Process 2 & 4) transfer syntax UID.
pub const TS_JPEG_EXTENDED: &str = "1.2.840.10008.1.2.4.51";
/// JPEG Lossless, Non-Hierarchical (Process 14) transfer syntax UID.
pub const TS_JPEG_LOSSLESS: &str = "1.2.840.10008.1.2.4.57";
/// JPEG Lossless, Selection Value 1 transfer syntax UID.
pub const TS_JPEG_LOSSLESS_SV1: &str = "1.2.840.10008.1.2.4.70";

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jpeg::encoder::encode_jpeg;
    use crate::jpeg::params::JpegParams;

    #[test]
    fn jpeg_decode_baseline() {
        // Encode a small image and immediately decode it to verify the
        // JpegDecoder::decode_frame surface.
        let width = 8u16;
        let height = 8u16;
        let pixels: Vec<u8> = (0u8..64).collect();
        let params = JpegParams {
            quality: 90,
            ..Default::default()
        };
        let compressed = encode_jpeg(&pixels, width, height, 1, &params).unwrap();

        let frame = JpegDecoder::decode_frame(&compressed).unwrap();
        assert_eq!(frame.width, width as u32);
        assert_eq!(frame.height, height as u32);
        assert_eq!(frame.components, 1);
        assert_eq!(frame.pixels.len(), (width as usize) * (height as usize));
    }
}
