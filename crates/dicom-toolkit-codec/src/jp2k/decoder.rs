//! JPEG 2000 decoder — wraps the forked `dicom-toolkit-jpeg2000` crate for DICOM use.
//!
//! Decodes raw J2K codestreams (as embedded in DICOM encapsulated pixel data)
//! at native bit depth, preserving the full dynamic range of 12-bit and 16-bit
//! medical images.

use dicom_toolkit_core::error::{DcmError, DcmResult};

/// Decoded JPEG 2000 frame with native bit-depth pixel data.
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// Raw pixel bytes. For bit_depth ≤ 8: one byte per sample.
    /// For bit_depth > 8: two bytes per sample in little-endian order.
    pub pixels: Vec<u8>,
    /// Width of the decoded image.
    pub width: u32,
    /// Height of the decoded image.
    pub height: u32,
    /// Bits per sample (e.g., 8, 12, 16).
    pub bits_per_sample: u8,
    /// Number of components (1 = grayscale, 3 = RGB).
    pub components: u8,
}

/// Decode a JPEG 2000 codestream into pixel data at native bit depth.
///
/// The input `data` should be a raw JPEG 2000 codestream (starting with
/// `FF 4F` SOC marker), as embedded in DICOM encapsulated pixel data fragments.
/// JP2 container format (starting with `00 00 00 0C 6A 50 20 20`) is also
/// accepted.
pub fn decode_jp2k(data: &[u8]) -> DcmResult<DecodedFrame> {
    use dicom_toolkit_jpeg2000::{DecodeSettings, Image};

    if data.is_empty() {
        return Err(DcmError::DecompressionError {
            reason: "empty JPEG 2000 codestream".into(),
        });
    }

    let settings = DecodeSettings {
        resolve_palette_indices: false,
        strict: false,
        target_resolution: None,
    };

    let image = Image::new(data, &settings).map_err(|e| DcmError::DecompressionError {
        reason: format!("JPEG 2000 parse error: {e}"),
    })?;

    let raw = image
        .decode_native()
        .map_err(|e| DcmError::DecompressionError {
            reason: format!("JPEG 2000 decode error: {e}"),
        })?;

    Ok(DecodedFrame {
        pixels: raw.data,
        width: raw.width,
        height: raw.height,
        bits_per_sample: raw.bit_depth,
        components: raw.num_components,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // SOC (FF4F) + SIZ marker for a minimal 2x2, 8-bit, 1-component image
    // This is a smoke test — real J2K testing requires actual encoded images.

    #[test]
    fn decode_empty_returns_error() {
        let result = decode_jp2k(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_garbage_returns_error() {
        let result = decode_jp2k(&[0x00, 0x01, 0x02, 0x03]);
        assert!(result.is_err());
    }
}
