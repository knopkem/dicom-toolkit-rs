//! JPEG 2000 encoder — wraps the forked `dicom-toolkit-jpeg2000` encoder for DICOM use.
//!
//! Encodes raw pixel data into JPEG 2000 codestreams suitable for embedding
//! in DICOM encapsulated pixel data.

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_jpeg2000::{encode as j2k_encode, encode_htj2k as htj2k_encode, EncodeOptions};

/// Encode raw pixel data into a JPEG 2000 codestream.
///
/// # Arguments
/// * `pixels` — Raw pixel bytes. For ≤8-bit: one byte per sample.
///   For >8-bit: two bytes per sample in little-endian u16 layout.
/// * `width` — Image width in pixels.
/// * `height` — Image height in pixels.
/// * `bits_per_sample` — Actual sample precision written into the codestream
///   (for example 8, 12, or 16).
/// * `samples_per_pixel` — Number of components (1=grayscale, 3=RGB).
/// * `lossless` — If true, use reversible DWT 5-3 (lossless); if false, use DWT 9-7 (lossy).
///
/// # Returns
/// The encoded JPEG 2000 codestream bytes (`.j2c` format).
pub fn encode_jp2k(
    pixels: &[u8],
    width: u32,
    height: u32,
    bits_per_sample: u8,
    samples_per_pixel: u8,
    lossless: bool,
) -> DcmResult<Vec<u8>> {
    encode_with_mode(
        pixels,
        width,
        height,
        bits_per_sample,
        samples_per_pixel,
        lossless,
        false,
    )
}

/// Encode raw pixel data into an HTJ2K codestream.
pub fn encode_htj2k(
    pixels: &[u8],
    width: u32,
    height: u32,
    bits_per_sample: u8,
    samples_per_pixel: u8,
    lossless: bool,
) -> DcmResult<Vec<u8>> {
    encode_with_mode(
        pixels,
        width,
        height,
        bits_per_sample,
        samples_per_pixel,
        lossless,
        true,
    )
}

fn encode_with_mode(
    pixels: &[u8],
    width: u32,
    height: u32,
    bits_per_sample: u8,
    samples_per_pixel: u8,
    lossless: bool,
    use_ht_block_coding: bool,
) -> DcmResult<Vec<u8>> {
    let codec_name = if use_ht_block_coding {
        "HTJ2K"
    } else {
        "JPEG 2000"
    };

    if bits_per_sample == 0 || bits_per_sample > 16 {
        return Err(DcmError::CompressionError {
            reason: format!("{codec_name}: unsupported bit depth {bits_per_sample}"),
        });
    }

    let bit_depth = bits_per_sample;

    // Choose appropriate decomposition levels based on image size.
    // Cannot have more levels than log2(min(width, height)).
    let max_levels = (width.min(height) as f64).log2().floor() as u8;
    let num_levels = max_levels.min(5);

    let options = EncodeOptions {
        num_decomposition_levels: num_levels,
        reversible: lossless,
        guard_bits: if lossless { 1 } else { 2 },
        use_ht_block_coding,
        ..Default::default()
    };

    let encode_impl = if use_ht_block_coding {
        htj2k_encode
    } else {
        j2k_encode
    };

    encode_impl(
        pixels,
        width,
        height,
        samples_per_pixel,
        bit_depth,
        false, // DICOM pixel data is unsigned by convention
        &options,
    )
    .map_err(|e| DcmError::CompressionError {
        reason: format!("{codec_name} encode error: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_grayscale_8bit() {
        let pixels: Vec<u8> = (0..64).collect();
        let result = encode_jp2k(&pixels, 8, 8, 8, 1, true);
        assert!(result.is_ok(), "encode failed: {:?}", result.err());
        let encoded = result.unwrap();
        // J2K codestream starts with SOC marker (FF 4F)
        assert!(encoded.len() > 4);
        assert_eq!(encoded[0], 0xFF);
        assert_eq!(encoded[1], 0x4F);
    }

    #[test]
    fn encode_grayscale_16bit() {
        // 4×4 image, 16-bit: 32 bytes (16 samples × 2 bytes each)
        let mut pixels = Vec::with_capacity(32);
        for i in 0u16..16 {
            pixels.extend_from_slice(&(i * 256).to_le_bytes());
        }
        let result = encode_jp2k(&pixels, 4, 4, 16, 1, true);
        assert!(result.is_ok(), "encode failed: {:?}", result.err());
    }

    #[test]
    fn encode_rgb_8bit() {
        let pixels: Vec<u8> = (0..192).map(|i| (i & 0xFF) as u8).collect();
        let result = encode_jp2k(&pixels, 8, 8, 8, 3, true);
        assert!(result.is_ok(), "encode failed: {:?}", result.err());
    }

    #[test]
    fn encode_lossy() {
        let pixels: Vec<u8> = (0..64).collect();
        let result = encode_jp2k(&pixels, 8, 8, 8, 1, false);
        assert!(result.is_ok(), "encode failed: {:?}", result.err());
    }

    #[test]
    fn encode_roundtrip_8bit() {
        let original: Vec<u8> = (0..64).collect();
        let encoded = encode_jp2k(&original, 8, 8, 8, 1, true).unwrap();

        // Decode using the decoder
        let decoded = crate::jp2k::decoder::decode_jp2k(&encoded).unwrap();
        assert_eq!(decoded.width, 8);
        assert_eq!(decoded.height, 8);
        assert_eq!(decoded.bits_per_sample, 8);
        assert_eq!(decoded.components, 1);
        assert_eq!(decoded.pixels, original);
    }

    #[test]
    fn encode_roundtrip_16bit() {
        let mut original = Vec::with_capacity(32);
        for i in 0u16..16 {
            original.extend_from_slice(&(i * 257).to_le_bytes());
        }

        let encoded = encode_jp2k(&original, 4, 4, 16, 1, true).unwrap();
        let decoded = crate::jp2k::decoder::decode_jp2k(&encoded).unwrap();

        assert_eq!(decoded.width, 4);
        assert_eq!(decoded.height, 4);
        assert_eq!(decoded.bits_per_sample, 16);
        assert_eq!(decoded.components, 1);
        assert_eq!(decoded.pixels, original);
    }

    #[test]
    fn encode_roundtrip_12bit_in_u16_container() {
        let mut original = Vec::with_capacity(32);
        for i in 0u16..16 {
            original.extend_from_slice(&((i * 257) & 0x0FFF).to_le_bytes());
        }

        let encoded = encode_jp2k(&original, 4, 4, 12, 1, true).unwrap();
        let decoded = crate::jp2k::decoder::decode_jp2k(&encoded).unwrap();

        assert_eq!(decoded.width, 4);
        assert_eq!(decoded.height, 4);
        assert_eq!(decoded.bits_per_sample, 12);
        assert_eq!(decoded.components, 1);
        assert_eq!(decoded.pixels, original);
    }

    #[test]
    fn encode_htj2k_roundtrip_12bit() {
        let mut original = Vec::with_capacity(32);
        for _ in 0..16 {
            original.extend_from_slice(&2048u16.to_le_bytes());
        }

        let encoded = encode_htj2k(&original, 4, 4, 12, 1, true).unwrap();
        assert!(encoded.windows(2).any(|window| window == [0xFF, 0x50]));

        let decoded = crate::jp2k::decoder::decode_jp2k(&encoded).unwrap();
        assert_eq!(decoded.width, 4);
        assert_eq!(decoded.height, 4);
        assert_eq!(decoded.bits_per_sample, 12);
        assert_eq!(decoded.components, 1);
        assert_eq!(decoded.pixels, original);
    }

    #[test]
    fn encode_htj2k_lossless_roundtrip_varied_12bit() {
        let mut original = Vec::with_capacity(32);
        for i in 0u16..16 {
            original.extend_from_slice(&((i * 257) & 0x0FFF).to_le_bytes());
        }

        let encoded = encode_htj2k(&original, 4, 4, 12, 1, true).unwrap();
        let decoded = crate::jp2k::decoder::decode_jp2k(&encoded).unwrap();
        assert_eq!(decoded.width, 4);
        assert_eq!(decoded.height, 4);
        assert_eq!(decoded.bits_per_sample, 12);
        assert_eq!(decoded.components, 1);
        assert_eq!(decoded.pixels, original);
    }

    #[test]
    fn encode_htj2k_lossy_is_parseable() {
        let pixels: Vec<u8> = (0..64).collect();
        let encoded = encode_htj2k(&pixels, 8, 8, 8, 1, false).unwrap();
        assert!(encoded.windows(2).any(|window| window == [0xFF, 0x50]));

        let decoded = crate::jp2k::decoder::decode_jp2k(&encoded).unwrap();
        assert_eq!(decoded.width, 8);
        assert_eq!(decoded.height, 8);
        assert_eq!(decoded.bits_per_sample, 8);
        assert_eq!(decoded.components, 1);
    }
}
