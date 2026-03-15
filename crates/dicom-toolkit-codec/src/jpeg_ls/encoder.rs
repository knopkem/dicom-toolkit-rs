//! JPEG-LS encoder — pure Rust implementation.
//!
//! Encodes raw pixel data into JPEG-LS compressed bitstream.

use dicom_toolkit_core::error::{DcmError, DcmResult};

use super::marker;
use super::params::{
    compute_default, ColorTransform, DerivedTraits, InterleaveMode, JlsParameters, BASIC_RESET,
};
use super::sample::needs_u16;
use super::scan::ScanEncoder;

/// Encode raw pixels as a JPEG-LS compressed bitstream.
///
/// * `pixels` — raw pixel data in native byte order (LE for multi-byte)
/// * `width`, `height` — image dimensions
/// * `bits_per_sample` — 2–16
/// * `components` — 1–4
/// * `near` — 0 for lossless, >0 for near-lossless
pub fn encode_jpeg_ls(
    pixels: &[u8],
    width: u32,
    height: u32,
    bits_per_sample: u8,
    components: u8,
    near: i32,
) -> DcmResult<Vec<u8>> {
    if !(2..=16).contains(&bits_per_sample) {
        return Err(DcmError::CompressionError {
            reason: format!("JPEG-LS: unsupported bit depth {bits_per_sample}"),
        });
    }
    if components == 0 || components > 4 {
        return Err(DcmError::CompressionError {
            reason: format!("JPEG-LS: unsupported component count {components}"),
        });
    }

    let max_val = (1i32 << bits_per_sample) - 1;
    let defaults = compute_default(max_val, near);
    let traits = DerivedTraits::new(max_val, near, BASIC_RESET);
    let w = width as usize;
    let h = height as usize;
    let nc = components as usize;

    // Build JlsParameters for header.
    let interleave = if nc == 1 {
        InterleaveMode::None
    } else {
        InterleaveMode::Line
    };

    let params = JlsParameters {
        width,
        height,
        bits_per_sample,
        components,
        near,
        interleave,
        color_transform: ColorTransform::None,
        ..Default::default()
    };

    // Write header.
    let mut output = marker::write_header(&params);

    if nc == 1 || interleave == InterleaveMode::None {
        // Convert pixel bytes to i32 values.
        let pixel_values = bytes_to_i32(pixels, w * h * nc, bits_per_sample)?;

        if nc == 1 {
            let mut encoder = ScanEncoder::new(traits, defaults.t1, defaults.t2, defaults.t3, w, h);
            let scan_data = encoder.encode(&pixel_values)?;
            output.extend_from_slice(&scan_data);
        } else {
            // ILV_NONE: separate scan per component.
            for c in 0..nc {
                let component_pixels: Vec<i32> =
                    (0..(w * h)).map(|i| pixel_values[i * nc + c]).collect();

                let mut encoder =
                    ScanEncoder::new(traits, defaults.t1, defaults.t2, defaults.t3, w, h);
                let scan_data = encoder.encode(&component_pixels)?;
                output.extend_from_slice(&scan_data);

                // Write additional SOS marker for subsequent scans.
                if c + 1 < nc {
                    // For simplicity, we write single-component SOS markers.
                    // This is handled by writing the full bitstream in one go for now.
                }
            }
        }
    } else {
        // ILV_LINE: interleave component lines.
        let pixel_values = bytes_to_i32(pixels, w * h * nc, bits_per_sample)?;

        // Re-arrange: from pixel-interleaved [R,G,B,R,G,B,...] to
        // line-interleaved [R-line, G-line, B-line, R-line, G-line, B-line, ...]
        let mut line_interleaved = Vec::with_capacity(w * h * nc);
        for y in 0..h {
            for c in 0..nc {
                for x in 0..w {
                    line_interleaved.push(pixel_values[(y * w + x) * nc + c]);
                }
            }
        }

        let effective_h = h * nc;
        let mut encoder = ScanEncoder::new(
            traits,
            defaults.t1,
            defaults.t2,
            defaults.t3,
            w,
            effective_h,
        );
        let scan_data = encoder.encode(&line_interleaved)?;
        output.extend_from_slice(&scan_data);
    }

    marker::write_eoi(&mut output);
    Ok(output)
}

/// Convert raw pixel bytes to i32 values.
fn bytes_to_i32(data: &[u8], count: usize, bits_per_sample: u8) -> DcmResult<Vec<i32>> {
    if needs_u16(bits_per_sample) {
        if data.len() < count * 2 {
            return Err(DcmError::CompressionError {
                reason: format!("JPEG-LS: expected {} bytes, got {}", count * 2, data.len()),
            });
        }
        let mut values = Vec::with_capacity(count);
        for i in 0..count {
            let lo = data[i * 2] as i32;
            let hi = data[i * 2 + 1] as i32;
            values.push(lo | (hi << 8));
        }
        Ok(values)
    } else {
        if data.len() < count {
            return Err(DcmError::CompressionError {
                reason: format!("JPEG-LS: expected {} bytes, got {}", count, data.len()),
            });
        }
        Ok(data[..count].iter().map(|&b| b as i32).collect())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jpeg_ls::decoder::decode_jpeg_ls;

    #[test]
    fn encode_decode_roundtrip_8bit_grayscale() {
        let w = 16u32;
        let h = 8u32;
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                pixels.push(((x * 16 + y * 8) % 256) as u8);
            }
        }

        let encoded = encode_jpeg_ls(&pixels, w, h, 8, 1, 0).unwrap();
        let decoded = decode_jpeg_ls(&encoded).unwrap();

        assert_eq!(decoded.width, w);
        assert_eq!(decoded.height, h);
        assert_eq!(decoded.bits_per_sample, 8);
        assert_eq!(decoded.components, 1);
        assert_eq!(decoded.pixels, pixels);
    }

    #[test]
    fn encode_decode_roundtrip_constant() {
        let pixels = vec![128u8; 64];
        let encoded = encode_jpeg_ls(&pixels, 8, 8, 8, 1, 0).unwrap();
        let decoded = decode_jpeg_ls(&encoded).unwrap();
        assert_eq!(decoded.pixels, pixels);
    }

    #[test]
    fn encode_decode_roundtrip_16bit_grayscale() {
        let w = 8u32;
        let h = 4u32;
        let mut pixels = Vec::with_capacity((w * h * 2) as usize);
        for y in 0..h {
            for x in 0..w {
                let val = ((x * 1000 + y * 500) % 4096) as u16;
                pixels.push(val as u8);
                pixels.push((val >> 8) as u8);
            }
        }

        let encoded = encode_jpeg_ls(&pixels, w, h, 12, 1, 0).unwrap();
        let decoded = decode_jpeg_ls(&encoded).unwrap();

        assert_eq!(decoded.width, w);
        assert_eq!(decoded.height, h);
        assert_eq!(decoded.bits_per_sample, 12);
        assert_eq!(decoded.pixels, pixels);
    }
}
