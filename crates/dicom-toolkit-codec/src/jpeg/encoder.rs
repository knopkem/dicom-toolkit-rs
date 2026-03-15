//! JPEG baseline encoder wrapping `jpeg-encoder`.
//!
//! Encodes raw pixel data to a JPEG fragment for DICOM encapsulated storage.

use dicom_toolkit_core::error::{DcmError, DcmResult};
use jpeg_encoder::{ColorType, Encoder};

use super::params::JpegParams;

/// Encode raw pixel bytes as JPEG baseline.
///
/// - `pixels`: interleaved pixel bytes (grayscale = 1 byte/px, RGB = 3 bytes/px)
/// - `width`, `height`: image dimensions in pixels
/// - `samples_per_pixel`: 1 (grayscale) or 3 (RGB/YCbCr)
/// - `params`: encoding parameters (quality, sampling factor)
///
/// Returns the JPEG-compressed fragment bytes.
pub fn encode_jpeg(
    pixels: &[u8],
    width: u16,
    height: u16,
    samples_per_pixel: u8,
    params: &JpegParams,
) -> DcmResult<Vec<u8>> {
    let color_type = match samples_per_pixel {
        1 => ColorType::Luma,
        3 => ColorType::Rgb,
        n => {
            return Err(DcmError::CompressionError {
                reason: format!("JPEG encoder: unsupported samples per pixel {n}"),
            })
        }
    };

    let mut buf = Vec::new();
    let mut enc = Encoder::new(&mut buf, params.quality);
    if samples_per_pixel == 3 {
        enc.set_sampling_factor(params.sampling_factor);
    }

    enc.encode(pixels, width, height, color_type)
        .map_err(|e| DcmError::CompressionError {
            reason: format!("JPEG encode error: {e}"),
        })?;

    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_grayscale_2x2() {
        let pixels = vec![128u8, 64, 192, 32];
        let result = encode_jpeg(&pixels, 2, 2, 1, &JpegParams::default());
        assert!(result.is_ok(), "encode failed: {:?}", result.err());
        let bytes = result.unwrap();
        // JPEG must start with SOI marker 0xFF 0xD8
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8]);
        // and end with EOI marker 0xFF 0xD9
        assert_eq!(&bytes[bytes.len() - 2..], &[0xFF, 0xD9]);
    }

    #[test]
    fn encode_rgb_2x2() {
        let pixels: Vec<u8> = vec![
            255, 0, 0, // red
            0, 255, 0, // green
            0, 0, 255, // blue
            128, 128, 128, // grey
        ];
        let result = encode_jpeg(&pixels, 2, 2, 3, &JpegParams::default());
        assert!(result.is_ok(), "encode failed: {:?}", result.err());
    }

    #[test]
    fn encode_decode_roundtrip_grayscale() {
        use crate::jpeg::decoder::decode_jpeg;

        let pixels: Vec<u8> = (0u8..=255).collect();
        let result = encode_jpeg(
            &pixels,
            16,
            16,
            1,
            &JpegParams {
                quality: 100,
                ..Default::default()
            },
        );
        let bytes = result.unwrap();

        let frame = decode_jpeg(&bytes).unwrap();
        assert_eq!(frame.width, 16);
        assert_eq!(frame.height, 16);
        assert_eq!(frame.samples_per_pixel, 1);
        assert_eq!(frame.data.len(), 16 * 16);
    }
}
