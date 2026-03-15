//! JPEG-LS marker parsing and writing (SOI, SOF-55, SOS, LSE, EOI, APP8).
//!
//! Port of CharLS header.cc marker handling.

use dicom_toolkit_core::error::{DcmError, DcmResult};

use super::params::{ColorTransform, InterleaveMode, JlsCustomParameters, JlsParameters};

// Marker bytes (without the leading 0xFF).
const JPEG_SOI: u8 = 0xD8;
const JPEG_EOI: u8 = 0xD9;
const JPEG_SOS: u8 = 0xDA;
const JPEG_SOF55: u8 = 0xF7;
const JPEG_LSE: u8 = 0xF8;
const JPEG_APP8: u8 = 0xE8;
const JPEG_COM: u8 = 0xFE;
const JPEG_DRI: u8 = 0xDD;

/// APP8 color-transform tag.
const COLOR_XFORM_TAG: &[u8; 4] = b"mrfx";

/// Result of parsing a JPEG-LS bitstream's markers.
#[derive(Debug, Clone)]
pub struct FrameInfo {
    pub params: JlsParameters,
    /// Byte offset where each scan's compressed data begins.
    pub scan_offsets: Vec<usize>,
}

/// Parse all JPEG-LS markers from `data`, returning frame parameters and scan data offsets.
pub fn parse_markers(data: &[u8]) -> DcmResult<FrameInfo> {
    let len = data.len();
    if len < 4 {
        return Err(DcmError::DecompressionError {
            reason: "JPEG-LS: data too short".into(),
        });
    }

    // Must start with SOI.
    if data[0] != 0xFF || data[1] != JPEG_SOI {
        return Err(DcmError::DecompressionError {
            reason: "JPEG-LS: missing SOI marker".into(),
        });
    }

    let mut pos = 2;
    let mut params = JlsParameters::default();
    let mut scan_offsets = Vec::new();
    let mut got_sof = false;

    while pos + 1 < len {
        if data[pos] != 0xFF {
            return Err(DcmError::DecompressionError {
                reason: format!("JPEG-LS: expected marker at offset {pos}"),
            });
        }
        let marker = data[pos + 1];
        pos += 2;

        match marker {
            JPEG_EOI => break,

            JPEG_SOF55 => {
                // SOF-55 (JPEG-LS frame header).
                if pos + 2 > len {
                    return Err(short_data_err());
                }
                let seg_len = read_u16_be(data, pos) as usize;
                if pos + seg_len > len || seg_len < 8 {
                    return Err(short_data_err());
                }
                params.bits_per_sample = data[pos + 2];
                params.height = read_u16_be(data, pos + 3) as u32;
                params.width = read_u16_be(data, pos + 5) as u32;
                params.components = data[pos + 7];

                // Validate basic constraints.
                if params.bits_per_sample < 2 || params.bits_per_sample > 16 {
                    return Err(DcmError::DecompressionError {
                        reason: format!(
                            "JPEG-LS: unsupported bit depth {}",
                            params.bits_per_sample
                        ),
                    });
                }
                if params.components == 0 || params.components > 4 {
                    return Err(DcmError::DecompressionError {
                        reason: format!(
                            "JPEG-LS: unsupported component count {}",
                            params.components
                        ),
                    });
                }

                got_sof = true;
                pos += seg_len;
            }

            JPEG_SOS => {
                // Start of Scan.
                if pos + 2 > len {
                    return Err(short_data_err());
                }
                let seg_len = read_u16_be(data, pos) as usize;
                if pos + seg_len > len || seg_len < 6 {
                    return Err(short_data_err());
                }

                let ns = data[pos + 2]; // Number of components in this scan.
                // Component spec entries: Ns * 2 bytes at pos+3.
                let near_offset = pos + 3 + (ns as usize * 2);
                if near_offset + 2 > pos + seg_len {
                    return Err(short_data_err());
                }
                params.near = data[near_offset] as i32;
                let ilv = data[near_offset + 1];
                params.interleave = InterleaveMode::from_u8(ilv).unwrap_or(InterleaveMode::None);

                // The scan data starts right after the SOS segment.
                let scan_start = pos + seg_len;
                scan_offsets.push(scan_start);

                // Skip over entropy-coded data to find the next marker.
                // In JPEG-LS, after 0xFF the next byte's MSB is 0 for entropy
                // data (bit-stuffed) and >= 0x80 for a real marker.
                pos = scan_start;
                while pos < len {
                    if data[pos] == 0xFF && pos + 1 < len && data[pos + 1] >= 0x80 {
                        break;
                    }
                    pos += 1;
                }
            }

            JPEG_LSE => {
                // JPEG-LS Extension (custom parameters).
                if pos + 2 > len {
                    return Err(short_data_err());
                }
                let seg_len = read_u16_be(data, pos) as usize;
                if pos + seg_len > len {
                    return Err(short_data_err());
                }
                if seg_len >= 13 {
                    let id = data[pos + 2];
                    if id == 1 {
                        // Preset coding parameters (type 1).
                        params.custom = JlsCustomParameters {
                            max_val: read_u16_be(data, pos + 3) as i32,
                            t1: read_u16_be(data, pos + 5) as i32,
                            t2: read_u16_be(data, pos + 7) as i32,
                            t3: read_u16_be(data, pos + 9) as i32,
                            reset: read_u16_be(data, pos + 11) as i32,
                        };
                    }
                }
                pos += seg_len;
            }

            JPEG_APP8 => {
                // HP color transform.
                if pos + 2 > len {
                    return Err(short_data_err());
                }
                let seg_len = read_u16_be(data, pos) as usize;
                if pos + seg_len > len {
                    return Err(short_data_err());
                }
                if seg_len >= 7 && &data[pos + 2..pos + 6] == COLOR_XFORM_TAG {
                    let xform = data[pos + 6];
                    params.color_transform =
                        ColorTransform::from_u8(xform).unwrap_or(ColorTransform::None);
                }
                pos += seg_len;
            }

            JPEG_COM | JPEG_DRI => {
                // Skip comment and restart-interval markers.
                if pos + 2 > len {
                    return Err(short_data_err());
                }
                let seg_len = read_u16_be(data, pos) as usize;
                pos += seg_len;
            }

            m if (0xE0..=0xEF).contains(&m) => {
                // Other APPn markers — skip.
                if pos + 2 > len {
                    return Err(short_data_err());
                }
                let seg_len = read_u16_be(data, pos) as usize;
                pos += seg_len;
            }

            _ => {
                // Unknown marker — try skipping if it has a length.
                if pos + 2 <= len {
                    let seg_len = read_u16_be(data, pos) as usize;
                    pos += seg_len;
                }
            }
        }
    }

    if !got_sof {
        return Err(DcmError::DecompressionError {
            reason: "JPEG-LS: no SOF-55 marker found".into(),
        });
    }

    Ok(FrameInfo {
        params,
        scan_offsets,
    })
}

/// Write a minimal JPEG-LS bitstream header (SOI + SOF55 + APP8 + SOS).
pub fn write_header(params: &JlsParameters) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);

    // SOI
    buf.push(0xFF);
    buf.push(JPEG_SOI);

    // APP8 color transform (if applicable)
    if params.color_transform != ColorTransform::None {
        buf.push(0xFF);
        buf.push(JPEG_APP8);
        let seg_len: u16 = 7; // length includes 2 length bytes + 4 tag + 1 xform
        buf.extend_from_slice(&seg_len.to_be_bytes());
        buf.extend_from_slice(COLOR_XFORM_TAG);
        buf.push(params.color_transform as u8);
    }

    // SOF-55 (JPEG-LS frame header)
    buf.push(0xFF);
    buf.push(JPEG_SOF55);
    let sof_len: u16 = 8 + 3 * params.components as u16;
    buf.extend_from_slice(&sof_len.to_be_bytes());
    buf.push(params.bits_per_sample);
    buf.extend_from_slice(&(params.height as u16).to_be_bytes());
    buf.extend_from_slice(&(params.width as u16).to_be_bytes());
    buf.push(params.components);
    for c in 0..params.components {
        buf.push(c + 1); // Component ID (1-based).
        buf.push(0x11); // Sampling factors (1×1).
        buf.push(0); // Quantization table (unused).
    }

    // LSE — custom parameters (if set)
    if params.custom.max_val > 0 {
        write_lse_params(&mut buf, &params.custom);
    }

    // SOS (Start of Scan)
    let ns = if params.interleave == InterleaveMode::None {
        1
    } else {
        params.components
    };
    buf.push(0xFF);
    buf.push(JPEG_SOS);
    let sos_len: u16 = 6 + 2 * ns as u16;
    buf.extend_from_slice(&sos_len.to_be_bytes());
    buf.push(ns);
    for c in 0..ns {
        buf.push(c + 1); // Component ID.
        buf.push(0); // Mapping table index.
    }
    buf.push(params.near as u8);
    buf.push(params.interleave as u8);
    buf.push(0); // Successive approximation (unused).

    buf
}

/// Write a JPEG-LS LSE (parameter extension) segment for custom thresholds.
fn write_lse_params(buf: &mut Vec<u8>, custom: &JlsCustomParameters) {
    buf.push(0xFF);
    buf.push(JPEG_LSE);
    let seg_len: u16 = 13;
    buf.extend_from_slice(&seg_len.to_be_bytes());
    buf.push(1); // LSE type: preset coding parameters.
    buf.extend_from_slice(&(custom.max_val as u16).to_be_bytes());
    buf.extend_from_slice(&(custom.t1 as u16).to_be_bytes());
    buf.extend_from_slice(&(custom.t2 as u16).to_be_bytes());
    buf.extend_from_slice(&(custom.t3 as u16).to_be_bytes());
    buf.extend_from_slice(&(custom.reset as u16).to_be_bytes());
}

/// Write the EOI (end of image) marker.
pub fn write_eoi(buf: &mut Vec<u8>) {
    buf.push(0xFF);
    buf.push(JPEG_EOI);
}

fn read_u16_be(data: &[u8], offset: usize) -> u16 {
    ((data[offset] as u16) << 8) | data[offset + 1] as u16
}

fn short_data_err() -> DcmError {
    DcmError::DecompressionError {
        reason: "JPEG-LS: truncated marker segment".into(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_header() {
        let params = JlsParameters {
            width: 64,
            height: 48,
            bits_per_sample: 8,
            components: 1,
            near: 0,
            interleave: InterleaveMode::None,
            ..Default::default()
        };
        let mut header = write_header(&params);
        // Add fake scan data byte + EOI.
        header.push(0x00);
        write_eoi(&mut header);

        let info = parse_markers(&header).unwrap();
        assert_eq!(info.params.width, 64);
        assert_eq!(info.params.height, 48);
        assert_eq!(info.params.bits_per_sample, 8);
        assert_eq!(info.params.components, 1);
        assert_eq!(info.params.near, 0);
        assert_eq!(info.params.interleave, InterleaveMode::None);
        assert_eq!(info.scan_offsets.len(), 1);
    }

    #[test]
    fn parse_with_color_transform() {
        let params = JlsParameters {
            width: 256,
            height: 256,
            bits_per_sample: 8,
            components: 3,
            near: 0,
            interleave: InterleaveMode::Line,
            color_transform: ColorTransform::Hp1,
            ..Default::default()
        };
        let mut header = write_header(&params);
        header.push(0x00);
        write_eoi(&mut header);

        let info = parse_markers(&header).unwrap();
        assert_eq!(info.params.color_transform, ColorTransform::Hp1);
        assert_eq!(info.params.interleave, InterleaveMode::Line);
        assert_eq!(info.params.components, 3);
    }

    #[test]
    fn parse_with_custom_params() {
        let params = JlsParameters {
            width: 32,
            height: 32,
            bits_per_sample: 12,
            components: 1,
            near: 0,
            interleave: InterleaveMode::None,
            custom: JlsCustomParameters {
                max_val: 4095,
                t1: 10,
                t2: 20,
                t3: 30,
                reset: 64,
            },
            ..Default::default()
        };
        let mut header = write_header(&params);
        header.push(0x00);
        write_eoi(&mut header);

        let info = parse_markers(&header).unwrap();
        assert_eq!(info.params.custom.max_val, 4095);
        assert_eq!(info.params.custom.t1, 10);
        assert_eq!(info.params.custom.t2, 20);
        assert_eq!(info.params.custom.t3, 30);
        assert_eq!(info.params.custom.reset, 64);
    }

    #[test]
    fn reject_missing_soi() {
        let data = [0x00, 0x00, 0xFF, 0xD9];
        assert!(parse_markers(&data).is_err());
    }

    #[test]
    fn reject_too_short() {
        let data = [0xFF, 0xD8];
        assert!(parse_markers(&data).is_err());
    }
}
