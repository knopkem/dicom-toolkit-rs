//! DICOM RLE (Lossless) codec — port of DCMTK's `dcmdata/libsrc/dcrledec.h`
//! and `dcmcrle`/`dcmdrle` tools.
//!
//! # DICOM RLE frame structure (PS3.5 §G)
//!
//! Each RLE-compressed frame starts with a 64-byte header:
//! ```text
//! Uint32 num_segments        // number of RLE segments (1..15)
//! Uint32 offsets[15]         // byte offsets to each segment from start of frame
//! ```
//!
//! For an 8-bit image with N channels there are N segments.
//! For a 16-bit image with N channels there are 2×N segments
//! (MSB segment first, then LSB segment for each channel).
//!
//! Each segment is PackBits-compressed (same algorithm as TIFF PackBits).

use dicom_toolkit_core::error::{DcmError, DcmResult};

// ── PackBits (RLE) decoder ────────────────────────────────────────────────────

/// Decode a single PackBits-compressed RLE segment, stopping once `max_bytes`
/// of output have been produced (to handle DICOM even-byte padding).
fn decode_segment(data: &[u8], max_bytes: usize) -> DcmResult<Vec<u8>> {
    let mut out = Vec::with_capacity(max_bytes.min(data.len() * 2));
    let mut pos = 0;
    while pos < data.len() && out.len() < max_bytes {
        let ctrl = data[pos] as i8;
        pos += 1;
        if ctrl >= 0 {
            // Literal run: copy next (ctrl + 1) bytes
            let count = (ctrl as usize + 1).min(max_bytes - out.len());
            if pos + count > data.len() {
                // Tolerate truncated literal run at end of stream (may be due
                // to DICOM even-byte padding appended after the last segment).
                out.extend_from_slice(&data[pos..data.len().min(pos + count)]);
                break;
            }
            out.extend_from_slice(&data[pos..pos + count]);
            pos += ctrl as usize + 1; // advance by original count, not clamped
        } else if ctrl != -128_i8 {
            // Replicate run: repeat next byte (257 - ctrl_u8) times
            let repeat = (257u16 - data[pos - 1] as u16) as usize;
            let count = repeat.min(max_bytes - out.len());
            if pos >= data.len() {
                return Err(DcmError::Other(
                    "RLE replicate run: missing value byte".to_string(),
                ));
            }
            let value = data[pos];
            pos += 1;
            for _ in 0..count {
                out.push(value);
            }
        }
        // ctrl == -128 (0x80) → no-op
    }
    Ok(out)
}

/// Encode a byte slice as a PackBits-compressed RLE segment.
fn encode_segment(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 128 + 1);
    let mut i = 0;
    while i < data.len() {
        // Look ahead for a run of equal bytes.
        let mut run = 1;
        while i + run < data.len() && data[i + run] == data[i] && run < 128 {
            run += 1;
        }
        if run >= 2 {
            // Replicate run: emit 257 - run as signed byte, then the value.
            out.push((257 - run as u16) as u8);
            out.push(data[i]);
            i += run;
        } else {
            // Literal run: find how long the non-repeating section is.
            let mut lit = 1;
            while i + lit < data.len() && lit < 128 {
                // Peek ahead: if the next 2 bytes are equal, end literal run.
                if i + lit + 1 < data.len() && data[i + lit] == data[i + lit + 1] {
                    break;
                }
                lit += 1;
            }
            out.push((lit - 1) as u8); // ctrl byte: (n-1) for n literal bytes
            out.extend_from_slice(&data[i..i + lit]);
            i += lit;
        }
    }
    out
}

// ── DICOM RLE frame decoder ───────────────────────────────────────────────────

/// Decode one DICOM RLE-compressed frame into raw interleaved pixel bytes.
///
/// # Parameters
/// - `frame_data`: raw bytes of a single DICOM pixel item (including the 64-byte header)
/// - `rows`, `columns`: pixel dimensions
/// - `samples_per_pixel`: 1 (grayscale) or 3 (RGB / YCbCr)
/// - `bits_allocated`: 8 or 16
///
/// # Returns
/// Interleaved pixel bytes in row-major order.
pub fn rle_decode_frame(
    frame_data: &[u8],
    rows: u16,
    columns: u16,
    samples_per_pixel: u8,
    bits_allocated: u8,
) -> DcmResult<Vec<u8>> {
    if frame_data.len() < 64 {
        return Err(DcmError::Other(format!(
            "RLE frame too short: {} bytes (need at least 64 for header)",
            frame_data.len()
        )));
    }

    // Parse the 64-byte header (16 × u32 LE).
    let num_segments = u32::from_le_bytes(frame_data[0..4].try_into().unwrap()) as usize;
    if num_segments == 0 || num_segments > 15 {
        return Err(DcmError::Other(format!(
            "RLE header: invalid segment count {num_segments}"
        )));
    }

    let expected_segments = samples_per_pixel as usize * (bits_allocated as usize / 8);
    if num_segments != expected_segments {
        return Err(DcmError::Other(format!(
            "RLE header: expected {expected_segments} segments for {}spp/{}bpp, got {num_segments}",
            samples_per_pixel, bits_allocated
        )));
    }

    // Read segment byte offsets.
    let mut offsets = [0u32; 15];
    for (i, off) in offsets.iter_mut().enumerate() {
        let start = 4 + i * 4;
        *off = u32::from_le_bytes(frame_data[start..start + 4].try_into().unwrap());
    }

    let num_pixels = rows as usize * columns as usize;

    // Decode each segment.
    let mut segments: Vec<Vec<u8>> = Vec::with_capacity(num_segments);
    for seg_idx in 0..num_segments {
        let seg_start = offsets[seg_idx] as usize;
        let seg_end = if seg_idx + 1 < num_segments {
            offsets[seg_idx + 1] as usize
        } else {
            frame_data.len()
        };
        if seg_start > frame_data.len() || seg_end > frame_data.len() || seg_start > seg_end {
            return Err(DcmError::Other(format!(
                "RLE segment {seg_idx} offset out of bounds: {seg_start}..{seg_end} in {} bytes",
                frame_data.len()
            )));
        }
        let seg_data = decode_segment(&frame_data[seg_start..seg_end], num_pixels)?;
        if seg_data.len() < num_pixels {
            return Err(DcmError::Other(format!(
                "RLE segment {seg_idx} decoded to {} bytes, expected at least {num_pixels}",
                seg_data.len()
            )));
        }
        segments.push(seg_data);
    }

    // Interleave segments back into pixel data.
    //
    // For 8-bit, N-channel:   seg[0]=ch0, seg[1]=ch1, ... → interleave by pixel
    // For 16-bit, N-channel:  seg[0]=ch0_MSB, seg[1]=ch0_LSB, seg[2]=ch1_MSB, ...
    //                         → each pixel is 2 bytes MSB first (big-endian per DICOM RLE spec)
    let bytes_per_sample = bits_allocated as usize / 8;
    let output_len = num_pixels * samples_per_pixel as usize * bytes_per_sample;
    let mut output = vec![0u8; output_len];

    #[allow(clippy::needless_range_loop)]
    for px in 0..num_pixels {
        for ch in 0..samples_per_pixel as usize {
            for byte_plane in 0..bytes_per_sample {
                let seg_idx = ch * bytes_per_sample + byte_plane;
                let out_byte = byte_plane; // MSB first
                let out_pos = px * (samples_per_pixel as usize * bytes_per_sample)
                    + ch * bytes_per_sample
                    + out_byte;
                output[out_pos] = segments[seg_idx][px];
            }
        }
    }

    Ok(output)
}

// ── DICOM RLE frame encoder ───────────────────────────────────────────────────

/// Encode raw interleaved pixel bytes into a DICOM RLE-compressed frame.
///
/// # Parameters
/// - `pixels`: raw pixel bytes in row-major, interleaved order
/// - `rows`, `columns`: pixel dimensions
/// - `samples_per_pixel`: 1 (grayscale) or 3 (RGB / YCbCr)
/// - `bits_allocated`: 8 or 16
///
/// # Returns
/// Complete RLE frame bytes (64-byte header + compressed segments).
pub fn rle_encode_frame(
    pixels: &[u8],
    rows: u16,
    columns: u16,
    samples_per_pixel: u8,
    bits_allocated: u8,
) -> DcmResult<Vec<u8>> {
    let num_pixels = rows as usize * columns as usize;
    let bytes_per_sample = bits_allocated as usize / 8;
    let num_segments = samples_per_pixel as usize * bytes_per_sample;
    let expected_len = num_pixels * samples_per_pixel as usize * bytes_per_sample;

    if pixels.len() < expected_len {
        return Err(DcmError::Other(format!(
            "RLE encode: pixel buffer too small: {} bytes, expected {expected_len}",
            pixels.len()
        )));
    }

    // Extract one byte-plane per segment (de-interleave).
    let mut planes: Vec<Vec<u8>> = vec![Vec::with_capacity(num_pixels); num_segments];
    for px in 0..num_pixels {
        for ch in 0..samples_per_pixel as usize {
            for byte_plane in 0..bytes_per_sample {
                let in_pos = px * (samples_per_pixel as usize * bytes_per_sample)
                    + ch * bytes_per_sample
                    + byte_plane;
                let seg_idx = ch * bytes_per_sample + byte_plane;
                planes[seg_idx].push(pixels[in_pos]);
            }
        }
    }

    // Compress each plane.
    let compressed: Vec<Vec<u8>> = planes.iter().map(|p| encode_segment(p)).collect();

    // Build the 64-byte header.
    let mut header = [0u32; 16];
    header[0] = num_segments as u32;
    let mut offset = 64u32; // segments start immediately after the 64-byte header
    for (i, seg) in compressed.iter().enumerate() {
        header[i + 1] = offset;
        offset += seg.len() as u32;
    }

    // Serialize to bytes (little-endian u32 values).
    let mut out = Vec::with_capacity(64 + compressed.iter().map(|s| s.len()).sum::<usize>());
    for word in &header {
        out.extend_from_slice(&word.to_le_bytes());
    }
    for seg in &compressed {
        out.extend_from_slice(seg);
    }

    // `offset` now points just past the last segment — record the true data end.
    // The DICOM stream writer is responsible for even-byte alignment of the pixel
    // item; we do NOT add padding inside the RLE frame, as that confuses the
    // segment-boundary calculation during decoding.
    //
    // If the caller needs an even-length buffer (e.g. for direct pixel-item
    // embedding), they should pad the returned Vec themselves.
    let _data_end = offset; // == out.len() at this point

    Ok(out)
}

// ── RleCodec ──────────────────────────────────────────────────────────────────

/// DICOM RLE Lossless codec with DICOM-spec-compliant byte-plane ordering
/// (PS3.5 §G).
///
/// For multi-byte samples the most-significant byte occupies the
/// lower-numbered segment (segment 0 = MSBs, segment 1 = LSBs).
/// This matches what DCMTK's `dcmdrle`/`dcmcrle` tools produce and
/// what DICOM peers expect.
///
/// Supported configurations
/// ─────────────────────────
/// | `bits_allocated` | `samples` | segments |
/// |------------------|-----------|---------|
/// | 8                | 1         | 1       |
/// | 16               | 1         | 2       |
/// | 8                | 3         | 3       |
/// | 16               | 3         | 6       |
pub struct RleCodec;

impl RleCodec {
    /// Decode an RLE-compressed DICOM fragment into raw little-endian pixel bytes.
    ///
    /// `data` must be the complete fragment payload, starting with the 64-byte
    /// RLE segment-offset header.
    pub fn decode(
        data: &[u8],
        rows: u16,
        cols: u16,
        bits_allocated: u16,
        samples: u16,
    ) -> DcmResult<Vec<u8>> {
        rle_codec_decode(data, rows, cols, bits_allocated, samples)
    }

    /// Encode raw little-endian pixel bytes into a DICOM RLE fragment.
    ///
    /// The returned buffer includes the 64-byte segment-offset header and all
    /// compressed segments.  Its length is always even (DICOM requirement).
    pub fn encode(
        data: &[u8],
        rows: u16,
        cols: u16,
        bits_allocated: u16,
        samples: u16,
    ) -> DcmResult<Vec<u8>> {
        rle_codec_encode(data, rows, cols, bits_allocated, samples)
    }
}

// ── Implementation ────────────────────────────────────────────────────────────

/// Number of RLE segments for the given image parameters.
fn rle_num_segments(bits_allocated: u16, samples: u16) -> DcmResult<usize> {
    let bytes_per_sample: usize = match bits_allocated {
        8 => 1,
        16 => 2,
        other => {
            return Err(DcmError::DecompressionError {
                reason: format!("RLE codec supports 8- or 16-bit samples, got {}", other),
            })
        }
    };
    Ok(samples as usize * bytes_per_sample)
}

fn rle_codec_decode(
    data: &[u8],
    rows: u16,
    cols: u16,
    bits_allocated: u16,
    samples: u16,
) -> DcmResult<Vec<u8>> {
    const HDR: usize = 64;
    const MAX_SEG: usize = 15;

    if data.len() < HDR {
        return Err(DcmError::DecompressionError {
            reason: format!(
                "RLE fragment too short: {} bytes (need ≥ {})",
                data.len(),
                HDR
            ),
        });
    }

    let num_segments = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
    let expected = rle_num_segments(bits_allocated, samples)?;
    if num_segments != expected {
        return Err(DcmError::DecompressionError {
            reason: format!(
                "expected {} RLE segments for {}bpp × {} samples, got {}",
                expected, bits_allocated, samples, num_segments
            ),
        });
    }

    let mut offsets = [0u32; MAX_SEG];
    for (i, slot) in offsets.iter_mut().enumerate() {
        let b = 4 + i * 4;
        *slot = u32::from_le_bytes(data[b..b + 4].try_into().unwrap());
    }

    let num_pixels = rows as usize * cols as usize;
    let bps = (bits_allocated as usize).div_ceil(8); // bytes per sample
    let bpp = samples as usize * bps; // bytes per pixel
    let mut out = vec![0u8; num_pixels * bpp];

    for seg in 0..num_segments {
        let start = offsets[seg] as usize;
        let end = if seg + 1 < num_segments && offsets[seg + 1] != 0 {
            offsets[seg + 1] as usize
        } else {
            data.len()
        };
        if start > data.len() || end > data.len() || start > end {
            return Err(DcmError::DecompressionError {
                reason: format!(
                    "RLE segment {} has invalid byte range [{}, {})",
                    seg, start, end
                ),
            });
        }

        let seg_bytes = decode_segment(&data[start..end], num_pixels)?;
        if seg_bytes.len() < num_pixels {
            return Err(DcmError::DecompressionError {
                reason: format!(
                    "RLE segment {} decoded to {} bytes, expected {}",
                    seg,
                    seg_bytes.len(),
                    num_pixels
                ),
            });
        }

        // Segment index encodes the sample and byte-plane:
        //   sample_idx = seg / bps
        //   plane      = seg % bps   (0 = MSB plane)
        // In LE layout the MSB lives at byte offset (bps - 1), so:
        //   native byte offset within the sample = bps - 1 - plane
        let sample_idx = seg / bps;
        let plane = seg % bps;
        let byte_off = bps - 1 - plane; // LE position of this plane

        for (p, &byte) in seg_bytes.iter().enumerate().take(num_pixels) {
            out[p * bpp + sample_idx * bps + byte_off] = byte;
        }
    }

    Ok(out)
}

fn rle_codec_encode(
    data: &[u8],
    rows: u16,
    cols: u16,
    bits_allocated: u16,
    samples: u16,
) -> DcmResult<Vec<u8>> {
    const HDR: usize = 64;
    const MAX_SEG: usize = 15;

    let num_segments = rle_num_segments(bits_allocated, samples)?;
    let num_pixels = rows as usize * cols as usize;
    let bps = (bits_allocated as usize).div_ceil(8);
    let bpp = samples as usize * bps;
    let expected_len = num_pixels * bpp;

    if data.len() != expected_len {
        return Err(DcmError::CompressionError {
            reason: format!(
                "input length {} ≠ expected {} ({}×{}×{}bpp×{} sample(s))",
                data.len(),
                expected_len,
                rows,
                cols,
                bits_allocated,
                samples
            ),
        });
    }

    // Extract and compress each byte-plane.
    let mut compressed: Vec<Vec<u8>> = Vec::with_capacity(num_segments);
    for seg in 0..num_segments {
        let sample_idx = seg / bps;
        let plane = seg % bps;
        let byte_off = bps - 1 - plane; // LE position of MSB plane

        let mut plane_bytes = Vec::with_capacity(num_pixels);
        for p in 0..num_pixels {
            plane_bytes.push(data[p * bpp + sample_idx * bps + byte_off]);
        }
        compressed.push(encode_segment(&plane_bytes));
    }

    // Write 64-byte header.
    let mut out: Vec<u8> =
        Vec::with_capacity(HDR + compressed.iter().map(|s| s.len()).sum::<usize>());
    out.extend_from_slice(&(num_segments as u32).to_le_bytes());
    let mut offset = HDR as u32;
    for seg in &compressed {
        out.extend_from_slice(&offset.to_le_bytes());
        offset += seg.len() as u32;
    }
    for _ in num_segments..MAX_SEG {
        out.extend_from_slice(&0u32.to_le_bytes());
    }

    for seg_bytes in &compressed {
        out.extend_from_slice(seg_bytes);
    }

    // Even-length requirement.
    if out.len() % 2 != 0 {
        out.push(0x00);
    }

    Ok(out)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_segment_roundtrip_uniform() {
        // All-same bytes — should compress very well.
        let data = vec![42u8; 256];
        let compressed = encode_segment(&data);
        let decoded = decode_segment(&compressed, 256).unwrap();
        assert_eq!(decoded[..256], data[..]);
    }

    #[test]
    fn encode_decode_segment_roundtrip_varied() {
        // Varied bytes — will result in literal runs.
        let data: Vec<u8> = (0u8..=255).collect();
        let compressed = encode_segment(&data);
        let decoded = decode_segment(&compressed, 256).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn rle_frame_roundtrip_8bit_grayscale() {
        let rows = 4u16;
        let cols = 4u16;
        let samples = 1u8;
        let bits = 8u8;
        let pixels: Vec<u8> = (0..16).map(|i| (i * 17) as u8).collect();

        let encoded = rle_encode_frame(&pixels, rows, cols, samples, bits).unwrap();
        let decoded = rle_decode_frame(&encoded, rows, cols, samples, bits).unwrap();
        assert_eq!(&decoded[..16], &pixels[..]);
    }

    #[test]
    fn rle_frame_roundtrip_8bit_rgb() {
        let rows = 2u16;
        let cols = 2u16;
        let samples = 3u8;
        let bits = 8u8;
        let pixels: Vec<u8> = vec![
            255, 0, 0, // red
            0, 255, 0, // green
            0, 0, 255, // blue
            128, 128, 128, // grey
        ];

        let encoded = rle_encode_frame(&pixels, rows, cols, samples, bits).unwrap();
        let decoded = rle_decode_frame(&encoded, rows, cols, samples, bits).unwrap();
        assert_eq!(&decoded[..12], &pixels[..]);
    }

    #[test]
    fn rle_frame_roundtrip_16bit_grayscale() {
        let rows = 2u16;
        let cols = 2u16;
        let samples = 1u8;
        let bits = 16u8;
        // 4 pixels × 2 bytes each = 8 bytes (MSB first per segment order)
        let pixels: Vec<u8> = vec![0x01, 0x00, 0x02, 0x00, 0xFF, 0xFF, 0x80, 0x00];

        let encoded = rle_encode_frame(&pixels, rows, cols, samples, bits).unwrap();
        let decoded = rle_decode_frame(&encoded, rows, cols, samples, bits).unwrap();
        assert_eq!(&decoded[..8], &pixels[..]);
    }

    #[test]
    fn rle_header_too_short_returns_error() {
        let result = rle_decode_frame(&[0u8; 32], 4, 4, 1, 8);
        assert!(matches!(result, Err(DcmError::Other(_))));
    }

    // ── RleCodec tests ────────────────────────────────────────────────────────

    #[test]
    fn rle_encode_decode_roundtrip_8bit() {
        let data: Vec<u8> = (0u8..16).collect();
        let (rows, cols, bits, samples) = (4u16, 4u16, 8u16, 1u16);

        let enc = RleCodec::encode(&data, rows, cols, bits, samples).unwrap();
        let dec = RleCodec::decode(&enc, rows, cols, bits, samples).unwrap();
        assert_eq!(dec, data);
    }

    #[test]
    fn rle_encode_decode_roundtrip_16bit() {
        // 4×4 image, 16-bit grayscale, stored as LE u16 pairs.
        let data: Vec<u8> = (0u16..16).flat_map(|v| v.to_le_bytes()).collect();
        let (rows, cols, bits, samples) = (4u16, 4u16, 16u16, 1u16);

        let enc = RleCodec::encode(&data, rows, cols, bits, samples).unwrap();
        let dec = RleCodec::decode(&enc, rows, cols, bits, samples).unwrap();
        assert_eq!(dec, data);
    }

    #[test]
    fn rle_decode_known_pattern() {
        // Manually build an RLE fragment: 1 segment, 4 literal bytes [0,1,2,3].
        let mut fragment = vec![0u8; 64]; // 64-byte header
        fragment[0] = 1; // num_segments = 1
        fragment[4] = 64; // segment_offsets[0] = 64

        // PackBits literal run: header byte 0x03 → copy 4 bytes.
        fragment.push(3);
        fragment.push(0x00);
        fragment.push(0x01);
        fragment.push(0x02);
        fragment.push(0x03);

        let dec = RleCodec::decode(&fragment, 2, 2, 8, 1).unwrap();
        assert_eq!(dec, [0x00, 0x01, 0x02, 0x03]);
    }

    #[test]
    fn rle_encoded_output_is_even_length() {
        let data = vec![42u8; 3]; // 1×3 → odd pixel count
        let enc = RleCodec::encode(&data, 1, 3, 8, 1).unwrap();
        assert_eq!(enc.len() % 2, 0, "encoded fragment must be even-length");
    }
}
