//! Golomb-Rice coding for JPEG-LS.
//!
//! Port of the Golomb encode/decode logic from CharLS scan.h.

use super::bitstream::{BitReader, BitWriter};
use dicom_toolkit_core::error::DcmResult;

/// Map a signed error value to an unsigned value for Golomb coding.
///
/// Mapping: 0→0, -1→1, 1→2, -2→3, 2→4, ...
/// Formula: `(err >> 31) ^ (2 * err)` (sign-magnitude interleave).
#[inline]
pub fn get_mapped_err_val(err_val: i32) -> i32 {
    let t = err_val >> 31;
    t ^ (2 * err_val)
}

/// Inverse of `get_mapped_err_val`: recover the signed error from the mapped value.
///
/// 0→0, 1→-1, 2→1, 3→-2, 4→2, ...
#[inline]
pub fn unmap_err_val(mapped: i32) -> i32 {
    if mapped & 1 == 0 {
        mapped >> 1
    } else {
        -((mapped + 1) >> 1)
    }
}

/// Golomb-Rice encode a mapped (unsigned) error value.
///
/// * `k` — Golomb parameter (from context statistics)
/// * `mapped_err` — unsigned mapped error value
/// * `limit` — maximum unary length before switching to binary escape
/// * `qbpp` — quantized bits per pixel (width of the binary portion for the escape)
pub fn encode_mapped_value(writer: &mut BitWriter, k: i32, mapped_err: i32, limit: i32, qbpp: i32) {
    let high_bits = mapped_err >> k;

    if high_bits < limit - qbpp - 1 {
        // Normal case: unary prefix (high_bits zeros + 1-bit) + k-bit binary suffix.
        if high_bits > 0 {
            writer.append(0, high_bits);
        }
        writer.append(1, 1);
        if k > 0 {
            writer.append(mapped_err & ((1 << k) - 1), k);
        }
    } else {
        // Escape: unary prefix of (limit - qbpp - 1) zeros + 1-bit, then qbpp-bit binary of (mapped_err - 1).
        writer.append(0, limit - qbpp - 1);
        writer.append(1, 1);
        writer.append(mapped_err - 1, qbpp);
    }
}

/// Golomb-Rice decode a mapped (unsigned) error value.
///
/// * `k` — Golomb parameter
/// * `limit` — max unary length
/// * `qbpp` — bits for escape code
pub fn decode_mapped_value(reader: &mut BitReader, k: i32, limit: i32, qbpp: i32) -> DcmResult<i32> {
    let high_bits = reader.read_highbits()?;

    if high_bits >= limit - qbpp - 1 {
        // Escape code.
        let val = reader.read_value(qbpp)?;
        Ok(val + 1)
    } else if k == 0 {
        Ok(high_bits)
    } else {
        let low_bits = reader.read_value(k)?;
        Ok((high_bits << k) + low_bits)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_mapping_roundtrip() {
        for val in -50..=50 {
            let mapped = get_mapped_err_val(val);
            let back = unmap_err_val(mapped);
            assert_eq!(back, val, "round-trip failed for {val}");
        }
    }

    #[test]
    fn error_mapping_known_values() {
        assert_eq!(get_mapped_err_val(0), 0);
        assert_eq!(get_mapped_err_val(-1), 1);
        assert_eq!(get_mapped_err_val(1), 2);
        assert_eq!(get_mapped_err_val(-2), 3);
        assert_eq!(get_mapped_err_val(2), 4);
    }

    #[test]
    fn unmap_known_values() {
        assert_eq!(unmap_err_val(0), 0);
        assert_eq!(unmap_err_val(1), -1);
        assert_eq!(unmap_err_val(2), 1);
        assert_eq!(unmap_err_val(3), -2);
        assert_eq!(unmap_err_val(4), 2);
    }

    #[test]
    fn golomb_encode_decode_roundtrip() {
        let limit = 32;
        let qbpp = 9;

        for k in 0..5 {
            for mapped_err in 0..20 {
                let mut writer = BitWriter::new();
                encode_mapped_value(&mut writer, k, mapped_err, limit, qbpp);
                writer.end_scan();
                let bytes = writer.into_bytes();

                let mut reader = BitReader::new(&bytes);
                let decoded = decode_mapped_value(&mut reader, k, limit, qbpp).unwrap();
                assert_eq!(decoded, mapped_err, "k={k}, mapped_err={mapped_err}");
            }
        }
    }

    #[test]
    fn golomb_escape_code() {
        // When mapped_err is large enough to trigger escape.
        let k = 2;
        let limit = 32;
        let qbpp = 9;
        let mapped_err = (limit - qbpp - 1) << k; // This forces escape.

        let mut writer = BitWriter::new();
        encode_mapped_value(&mut writer, k, mapped_err, limit, qbpp);
        writer.end_scan();
        let bytes = writer.into_bytes();

        let mut reader = BitReader::new(&bytes);
        let decoded = decode_mapped_value(&mut reader, k, limit, qbpp).unwrap();
        assert_eq!(decoded, mapped_err);
    }
}
