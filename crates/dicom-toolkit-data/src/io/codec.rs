//! Pixel codec trait and built-in codecs (RLE Lossless).
//!
//! DICOM RLE Lossless per PS3.5 Annex G (PackBits variant).

use dicom_toolkit_core::error::{DcmError, DcmResult};

// ── PixelCodec trait ──────────────────────────────────────────────────────────

pub trait PixelCodec: Send + Sync {
    fn uid(&self) -> &'static str;
    fn decode_frame(&self, data: &[u8], rows: u16, cols: u16, bits: u16) -> DcmResult<Vec<u8>>;
    fn encode_frame(&self, data: &[u8], rows: u16, cols: u16, bits: u16) -> DcmResult<Vec<u8>>;
}

// ── RLE Lossless ──────────────────────────────────────────────────────────────

pub struct RleCodec;

impl PixelCodec for RleCodec {
    fn uid(&self) -> &'static str {
        "1.2.840.10008.1.2.5"
    }

    fn decode_frame(&self, data: &[u8], rows: u16, cols: u16, bits: u16) -> DcmResult<Vec<u8>> {
        rle_decode(data, rows, cols, bits)
    }

    fn encode_frame(&self, data: &[u8], rows: u16, cols: u16, bits: u16) -> DcmResult<Vec<u8>> {
        rle_encode(data, rows, cols, bits)
    }
}

// ── RLE decode ────────────────────────────────────────────────────────────────

fn rle_decode(data: &[u8], rows: u16, cols: u16, bits: u16) -> DcmResult<Vec<u8>> {
    if data.len() < 64 {
        return Err(DcmError::DecompressionError {
            reason: "RLE header too small".into(),
        });
    }

    let pixel_count = rows as usize * cols as usize;
    let bytes_per_sample = if bits <= 8 { 1usize } else { 2usize };

    let num_segments = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if num_segments == 0 || num_segments > 15 {
        return Err(DcmError::DecompressionError {
            reason: format!("invalid RLE segment count: {}", num_segments),
        });
    }

    // Read segment offsets (15 × u32 at bytes 4..64)
    let mut offsets = [0u32; 15];
    for (i, offset) in offsets.iter_mut().enumerate() {
        let base = 4 + i * 4;
        *offset = u32::from_le_bytes([data[base], data[base + 1], data[base + 2], data[base + 3]]);
    }

    // Decode each segment
    let mut segments: Vec<Vec<u8>> = Vec::with_capacity(num_segments);
    for i in 0..num_segments {
        let start = offsets[i] as usize;
        let end = if i + 1 < num_segments {
            offsets[i + 1] as usize
        } else {
            data.len()
        };
        if start > data.len() || end > data.len() || start > end {
            return Err(DcmError::DecompressionError {
                reason: format!("invalid RLE segment offset: {} > {}", start, end),
            });
        }
        let seg = rle_decode_segment(&data[start..end], pixel_count)?;
        segments.push(seg);
    }

    // Reassemble: for 8-bit mono each segment is one byte plane;
    // for 16-bit each pair of segments is high/low byte of each pixel.
    let samples = num_segments / bytes_per_sample;
    let total_bytes = pixel_count * samples * bytes_per_sample;
    let mut out = vec![0u8; total_bytes];

    for sample in 0..samples {
        for pixel in 0..pixel_count {
            if bytes_per_sample == 1 {
                out[pixel * samples + sample] = segments[sample][pixel];
            } else {
                // 16-bit: high byte is segment[sample*2], low byte is segment[sample*2+1]
                let hi = segments[sample * 2][pixel];
                let lo = segments[sample * 2 + 1][pixel];
                let out_idx = (pixel * samples + sample) * 2;
                out[out_idx] = lo; // little-endian word
                out[out_idx + 1] = hi;
            }
        }
    }

    Ok(out)
}

fn rle_decode_segment(data: &[u8], pixel_count: usize) -> DcmResult<Vec<u8>> {
    let mut out = Vec::with_capacity(pixel_count);
    let mut pos = 0;

    while pos < data.len() && out.len() < pixel_count {
        let n = data[pos] as i8;
        pos += 1;
        if n >= 0 {
            // Literal run: copy next n+1 bytes
            let count = n as usize + 1;
            let end = pos + count;
            if end > data.len() {
                return Err(DcmError::DecompressionError {
                    reason: "RLE literal run overflows data".into(),
                });
            }
            out.extend_from_slice(&data[pos..end]);
            pos = end;
        } else if n != -128 {
            // Replicated run: repeat next byte (1 - n) times
            if pos >= data.len() {
                return Err(DcmError::DecompressionError {
                    reason: "RLE replicated run missing byte".into(),
                });
            }
            let byte = data[pos];
            pos += 1;
            let count = (1i32 - n as i32) as usize;
            for _ in 0..count {
                out.push(byte);
            }
        }
        // n == -128 (0x80) is a no-op
    }

    Ok(out)
}

// ── RLE encode ────────────────────────────────────────────────────────────────

fn rle_encode(data: &[u8], rows: u16, cols: u16, bits: u16) -> DcmResult<Vec<u8>> {
    let pixel_count = rows as usize * cols as usize;
    let bytes_per_sample = if bits <= 8 { 1usize } else { 2usize };

    let total_pixels = pixel_count * bytes_per_sample;
    if data.len() < total_pixels {
        return Err(DcmError::CompressionError {
            reason: "insufficient pixel data for RLE encoding".into(),
        });
    }
    let samples = data.len() / total_pixels;
    let num_segments = samples * bytes_per_sample;

    // Split data into byte planes (segments)
    let mut byte_planes: Vec<Vec<u8>> = vec![Vec::with_capacity(pixel_count); num_segments];
    for pixel in 0..pixel_count {
        for sample in 0..samples {
            if bytes_per_sample == 1 {
                byte_planes[sample].push(data[pixel * samples + sample]);
            } else {
                let idx = (pixel * samples + sample) * 2;
                let lo = data[idx];
                let hi = data[idx + 1];
                // High byte plane first, then low byte plane
                byte_planes[sample * 2].push(hi);
                byte_planes[sample * 2 + 1].push(lo);
            }
        }
    }

    // Encode each plane
    let mut encoded_segments: Vec<Vec<u8>> = Vec::with_capacity(num_segments);
    for plane in &byte_planes {
        encoded_segments.push(rle_encode_segment(plane));
    }

    // Build output: 64-byte header + segment data
    let mut header = vec![0u8; 64];
    let n = num_segments as u32;
    header[0..4].copy_from_slice(&n.to_le_bytes());

    // Segment offsets (relative to start of byte stream, i.e., after header offset 0)
    let mut offset = 64u32;
    for (i, seg) in encoded_segments.iter().enumerate() {
        let base = 4 + i * 4;
        header[base..base + 4].copy_from_slice(&offset.to_le_bytes());
        offset += seg.len() as u32;
    }

    let mut out = header;
    for seg in encoded_segments {
        out.extend_from_slice(&seg);
    }

    // Pad to even length
    if out.len() % 2 != 0 {
        out.push(0);
    }

    Ok(out)
}

fn rle_encode_segment(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;

    while i < data.len() {
        // Check for run of identical bytes
        let mut run_len = 1;
        while i + run_len < data.len() && data[i + run_len] == data[i] && run_len < 128 {
            run_len += 1;
        }

        if run_len > 1 {
            // Encode as replicated run
            out.push((257 - run_len as i32) as u8);
            out.push(data[i]);
            i += run_len;
        } else {
            // Find length of literal run (until a run of 2+ identical bytes or end)
            let lit_start = i;
            while i < data.len() {
                let mut next_run = 1;
                while i + next_run < data.len() && data[i + next_run] == data[i] && next_run < 128 {
                    next_run += 1;
                }
                if next_run >= 2 {
                    break;
                }
                i += 1;
                if i - lit_start >= 128 {
                    break;
                }
            }
            let lit_len = i - lit_start;
            out.push((lit_len - 1) as u8);
            out.extend_from_slice(&data[lit_start..i]);
        }
    }

    out
}

/// Return a codec for the given transfer syntax UID, if one is built-in.
pub fn codec_for_uid(uid: &str) -> Option<Box<dyn PixelCodec>> {
    match uid {
        "1.2.840.10008.1.2.5" => Some(Box::new(RleCodec)),
        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_encode_decode_roundtrip_8bit() {
        let rows = 4u16;
        let cols = 4u16;
        let bits = 8u16;
        let data: Vec<u8> = (0..16u8).collect();

        let codec = RleCodec;
        let encoded = codec.encode_frame(&data, rows, cols, bits).unwrap();
        let decoded = codec.decode_frame(&encoded, rows, cols, bits).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn rle_encode_decode_roundtrip_16bit() {
        let rows = 2u16;
        let cols = 2u16;
        let bits = 16u16;
        let data: Vec<u8> = vec![0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04];

        let codec = RleCodec;
        let encoded = codec.encode_frame(&data, rows, cols, bits).unwrap();
        let decoded = codec.decode_frame(&encoded, rows, cols, bits).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn rle_decode_segment_runs() {
        // 0x01 (2 literals: 0xAA, 0xBB), then 0xFE (replicate 0xCC 3 times)
        let data = [0x01u8, 0xAA, 0xBB, 0xFE, 0xCC];
        let result = rle_decode_segment(&data, 5).unwrap();
        assert_eq!(result, vec![0xAA, 0xBB, 0xCC, 0xCC, 0xCC]);
    }
}
