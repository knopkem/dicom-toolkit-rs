//! JPEG Lossless (Process 14) encoder.
//!
//! This encoder emits a simple lossless sequential Huffman JPEG stream using a
//! fixed DC Huffman table and predictor selection value 1 by default.

use dicom_toolkit_core::error::{DcmError, DcmResult};

const MARKER_SOI: u8 = 0xD8;
const MARKER_EOI: u8 = 0xD9;
const MARKER_SOF3: u8 = 0xC3;
const MARKER_DHT: u8 = 0xC4;
const MARKER_SOS: u8 = 0xDA;

const LOSSLESS_DC_BITS: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 17, 0, 0, 0, 0, 0, 0, 0, 0];
const LOSSLESS_DC_VALUES: [u8; 17] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

/// Encode raw pixels as classic JPEG Lossless (Process 14 Huffman).
pub fn encode_jpeg_lossless(
    pixels: &[u8],
    width: u16,
    height: u16,
    samples_per_pixel: u8,
    bits_allocated: u8,
    bits_stored: u8,
    predictor: u8,
) -> DcmResult<Vec<u8>> {
    if width == 0 || height == 0 {
        return Err(DcmError::CompressionError {
            reason: "JPEG Lossless encoder: image dimensions must be non-zero".into(),
        });
    }
    if !(1..=4).contains(&samples_per_pixel) {
        return Err(DcmError::CompressionError {
            reason: format!(
                "JPEG Lossless encoder: unsupported samples per pixel {samples_per_pixel}"
            ),
        });
    }
    if !matches!(bits_allocated, 8 | 16) {
        return Err(DcmError::CompressionError {
            reason: format!("JPEG Lossless encoder: unsupported BitsAllocated {bits_allocated}"),
        });
    }
    if bits_stored == 0 || bits_stored > 16 || bits_stored > bits_allocated {
        return Err(DcmError::CompressionError {
            reason: format!(
                "JPEG Lossless encoder: invalid BitsStored {bits_stored} for BitsAllocated {bits_allocated}"
            ),
        });
    }
    if predictor > 7 {
        return Err(DcmError::CompressionError {
            reason: format!("JPEG Lossless encoder: invalid predictor selection {predictor}"),
        });
    }

    let component_samples = split_component_samples(
        pixels,
        width,
        height,
        samples_per_pixel,
        bits_allocated,
        bits_stored,
    )?;

    let mut out = Vec::new();
    write_marker(&mut out, MARKER_SOI);
    write_sof3(&mut out, width, height, bits_stored, samples_per_pixel);
    write_dht(&mut out);
    write_sos(&mut out, samples_per_pixel, predictor);

    let mut entropy = EntropyWriter::default();
    let width = width as usize;
    let height = height as usize;
    let components = samples_per_pixel as usize;
    for y in 0..height {
        for x in 0..width {
            for samples in component_samples.iter().take(components) {
                let sample = samples[y * width + x];
                let prediction = predict(samples, width, x, y, predictor, bits_stored);
                let diff = wrap_difference(sample, prediction);
                let (category, magnitude_bits, magnitude_len) = diff_to_code(diff)?;
                entropy.write_bits(u32::from(category), 8);
                if magnitude_len > 0 {
                    entropy.write_bits(magnitude_bits, magnitude_len);
                }
            }
        }
    }

    entropy.flush();
    out.extend_from_slice(&entropy.bytes);
    write_marker(&mut out, MARKER_EOI);
    Ok(out)
}

fn split_component_samples(
    pixels: &[u8],
    width: u16,
    height: u16,
    samples_per_pixel: u8,
    bits_allocated: u8,
    bits_stored: u8,
) -> DcmResult<Vec<Vec<u16>>> {
    let components = samples_per_pixel as usize;
    let npixels = width as usize * height as usize;
    let bytes_per_sample = usize::from(bits_allocated > 8) + 1;
    let expected_len = npixels
        .checked_mul(components)
        .and_then(|n| n.checked_mul(bytes_per_sample))
        .ok_or_else(|| DcmError::CompressionError {
            reason: "JPEG Lossless encoder: input length overflow".into(),
        })?;

    if pixels.len() != expected_len {
        return Err(DcmError::CompressionError {
            reason: format!(
                "JPEG Lossless encoder: expected {expected_len} input bytes, got {}",
                pixels.len()
            ),
        });
    }

    let mask = if bits_stored == 16 {
        u16::MAX
    } else {
        ((1u32 << bits_stored) - 1) as u16
    };

    let mut planes = vec![vec![0u16; npixels]; components];
    if bits_allocated == 8 {
        for (pixel_index, chunk) in pixels.chunks_exact(components).enumerate() {
            for (plane, &sample) in planes.iter_mut().zip(chunk.iter()) {
                plane[pixel_index] = u16::from(sample) & mask;
            }
        }
    } else {
        let bytes_per_pixel = components * 2;
        for (pixel_index, chunk) in pixels.chunks_exact(bytes_per_pixel).enumerate() {
            for (plane, sample_bytes) in planes.iter_mut().zip(chunk.chunks_exact(2)) {
                plane[pixel_index] = u16::from_le_bytes([sample_bytes[0], sample_bytes[1]]) & mask;
            }
        }
    }

    Ok(planes)
}

fn predict(
    samples: &[u16],
    width: usize,
    x: usize,
    y: usize,
    predictor: u8,
    bits_stored: u8,
) -> i32 {
    if x == 0 && y == 0 {
        if bits_stored > 1 {
            1 << (bits_stored - 1)
        } else {
            0
        }
    } else if y == 0 {
        i32::from(samples[y * width + x - 1])
    } else if x == 0 {
        i32::from(samples[(y - 1) * width + x])
    } else {
        let ra = i32::from(samples[y * width + x - 1]);
        let rb = i32::from(samples[(y - 1) * width + x]);
        let rc = i32::from(samples[(y - 1) * width + x - 1]);
        match predictor {
            0 => 0,
            1 => ra,
            2 => rb,
            3 => rc,
            4 => ra + rb - rc,
            5 => ra + ((rb - rc) >> 1),
            6 => rb + ((ra - rc) >> 1),
            7 => (ra + rb) / 2,
            _ => unreachable!("validated predictor"),
        }
    }
}

fn wrap_difference(sample: u16, prediction: i32) -> i32 {
    let mut diff = i32::from(sample) - prediction;
    if diff <= -32768 {
        diff += 65536;
    } else if diff > 32768 {
        diff -= 65536;
    }
    diff
}

fn diff_to_code(diff: i32) -> DcmResult<(u8, u32, u8)> {
    if diff == 0 {
        return Ok((0, 0, 0));
    }
    if diff == 32768 {
        return Ok((16, 0, 0));
    }

    let magnitude = diff.unsigned_abs();
    let size = (32 - magnitude.leading_zeros()) as u8;
    if size == 0 || size > 15 {
        return Err(DcmError::CompressionError {
            reason: format!("JPEG Lossless encoder: unsupported difference magnitude {diff}"),
        });
    }

    let bits = if diff > 0 {
        diff as u32
    } else {
        (((1i32 << size) - 1) + diff) as u32
    };
    Ok((size, bits, size))
}

fn write_sof3(out: &mut Vec<u8>, width: u16, height: u16, precision: u8, components: u8) {
    write_marker(out, MARKER_SOF3);
    write_be_u16(out, 8 + 3 * u16::from(components));
    out.push(precision);
    write_be_u16(out, height);
    write_be_u16(out, width);
    out.push(components);
    for id in 1..=components {
        out.push(id);
        out.push(0x11);
        out.push(0);
    }
}

fn write_dht(out: &mut Vec<u8>) {
    write_marker(out, MARKER_DHT);
    write_be_u16(
        out,
        2 + 1 + LOSSLESS_DC_BITS.len() as u16 + LOSSLESS_DC_VALUES.len() as u16,
    );
    out.push(0x00);
    out.extend_from_slice(&LOSSLESS_DC_BITS);
    out.extend_from_slice(&LOSSLESS_DC_VALUES);
}

fn write_sos(out: &mut Vec<u8>, components: u8, predictor: u8) {
    write_marker(out, MARKER_SOS);
    write_be_u16(out, 6 + 2 * u16::from(components));
    out.push(components);
    for id in 1..=components {
        out.push(id);
        out.push(0x00);
    }
    out.push(predictor);
    out.push(0);
    out.push(0);
}

fn write_marker(out: &mut Vec<u8>, marker: u8) {
    out.extend_from_slice(&[0xFF, marker]);
}

fn write_be_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

#[derive(Default)]
struct EntropyWriter {
    bytes: Vec<u8>,
    bit_buffer: u32,
    bit_count: u8,
}

impl EntropyWriter {
    fn write_bits(&mut self, bits: u32, count: u8) {
        if count == 0 {
            return;
        }

        let mask = if count == 32 {
            u32::MAX
        } else {
            (1u32 << count) - 1
        };
        self.bit_buffer = (self.bit_buffer << count) | (bits & mask);
        self.bit_count += count;

        while self.bit_count >= 8 {
            let shift = self.bit_count - 8;
            let byte = ((self.bit_buffer >> shift) & 0xFF) as u8;
            self.emit(byte);
            self.bit_count -= 8;
            if self.bit_count == 0 {
                self.bit_buffer = 0;
            } else {
                self.bit_buffer &= (1u32 << self.bit_count) - 1;
            }
        }
    }

    fn flush(&mut self) {
        if self.bit_count == 0 {
            return;
        }

        let pad_bits = 8 - self.bit_count;
        let byte = ((self.bit_buffer << pad_bits) | ((1u32 << pad_bits) - 1)) as u8;
        self.emit(byte);
        self.bit_buffer = 0;
        self.bit_count = 0;
    }

    fn emit(&mut self, byte: u8) {
        self.bytes.push(byte);
        if byte == 0xFF {
            self.bytes.push(0x00);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jpeg::decoder::decode_jpeg;

    fn sof3_marker(bytes: &[u8]) -> bool {
        bytes.windows(2).any(|w| w == [0xFF, MARKER_SOF3])
    }

    #[test]
    fn encode_lossless_grayscale_8bit_roundtrip() {
        let pixels: Vec<u8> = (0u8..64).collect();
        let encoded = encode_jpeg_lossless(&pixels, 8, 8, 1, 8, 8, 1).unwrap();
        assert!(sof3_marker(&encoded));

        let decoded = decode_jpeg(&encoded).unwrap();
        assert_eq!(decoded.width, 8);
        assert_eq!(decoded.height, 8);
        assert_eq!(decoded.samples_per_pixel, 1);
        assert_eq!(decoded.data, pixels);
    }

    #[test]
    fn encode_lossless_grayscale_16bit_roundtrip() {
        let mut pixels = Vec::new();
        for i in 0u16..16 {
            pixels.extend_from_slice(&(i * 257).to_le_bytes());
        }

        let encoded = encode_jpeg_lossless(&pixels, 4, 4, 1, 16, 16, 1).unwrap();
        let decoded = decode_jpeg(&encoded).unwrap();
        assert_eq!(decoded.samples_per_pixel, 1);
        assert_eq!(decoded.data, pixels);
    }

    #[test]
    fn encode_lossless_rgb_8bit_roundtrip() {
        let pixels = vec![
            255, 0, 0, 0, 255, 0, 0, 0, 255, 32, 64, 96, 10, 20, 30, 200, 210, 220,
        ];

        let encoded = encode_jpeg_lossless(&pixels, 2, 3, 3, 8, 8, 1).unwrap();
        let decoded = decode_jpeg(&encoded).unwrap();
        assert_eq!(decoded.samples_per_pixel, 3);
        assert_eq!(decoded.data, pixels);
    }

    #[test]
    fn encode_lossless_rejects_invalid_predictor() {
        let err = encode_jpeg_lossless(&[0u8; 4], 2, 2, 1, 8, 8, 8).unwrap_err();
        assert!(err.to_string().contains("predictor"));
    }
}
