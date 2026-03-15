//! Low-level pixel data extraction and bit-field decoding.
//!
//! Handles the conversion of raw DICOM byte streams into typed pixel values,
//! respecting `BitsAllocated`, `BitsStored`, `HighBit`, and
//! `PixelRepresentation`.

// ── Decoding raw bytes ────────────────────────────────────────────────────────

/// Decode a raw byte slice as little-endian 16-bit unsigned integers.
///
/// Used for `BitsAllocated = 16`, `PixelRepresentation = 0`.
pub fn decode_u16_le(data: &[u8]) -> Vec<u16> {
    data.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// Decode a raw byte slice as little-endian 16-bit signed integers.
///
/// Used for `BitsAllocated = 16`, `PixelRepresentation = 1`.
pub fn decode_i16_le(data: &[u8]) -> Vec<i16> {
    data.chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

// ── Bit masking ───────────────────────────────────────────────────────────────

/// Apply the DICOM bit mask to 16-bit unsigned pixels.
///
/// Extracts `bits_stored` bits occupying positions
/// `[high_bit .. high_bit − bits_stored + 1]` (0-indexed from LSB).
/// For the common case `high_bit == bits_stored − 1` this reduces to a
/// simple mask with no shift.
pub fn mask_u16(pixels: &[u16], bits_stored: u16, high_bit: u16) -> Vec<u16> {
    if bits_stored >= 16 {
        return pixels.to_vec();
    }
    let shift = high_bit + 1 - bits_stored;
    let mask = (1u16 << bits_stored).wrapping_sub(1);
    pixels.iter().map(|&p| (p >> shift) & mask).collect()
}

/// Apply the DICOM bit mask to 16-bit signed pixels with sign extension.
///
/// Extracts `bits_stored` bits and sign-extends from the stored MSB position.
pub fn mask_i16(pixels: &[i16], bits_stored: u16, high_bit: u16) -> Vec<i16> {
    if bits_stored >= 16 {
        return pixels.to_vec();
    }
    let shift = high_bit + 1 - bits_stored;
    let mask = (1i16 << bits_stored).wrapping_sub(1);
    let sign_bit = 1i16 << (bits_stored - 1);
    pixels
        .iter()
        .map(|&p| {
            let raw = (p >> shift) & mask;
            // Sign-extend: if the stored MSB is set, fill upper bits with 1s.
            if raw & sign_bit != 0 {
                raw | !mask
            } else {
                raw
            }
        })
        .collect()
}

// ── Conversion to f64 ─────────────────────────────────────────────────────────

/// Convert 8-bit unsigned pixels to `f64` for LUT/window processing.
pub fn u8_to_f64(pixels: &[u8]) -> Vec<f64> {
    pixels.iter().map(|&v| v as f64).collect()
}

/// Convert 16-bit unsigned pixels to `f64` for LUT/window processing.
pub fn u16_to_f64(pixels: &[u16]) -> Vec<f64> {
    pixels.iter().map(|&v| v as f64).collect()
}

/// Convert 16-bit signed pixels to `f64` for LUT/window processing.
pub fn i16_to_f64(pixels: &[i16]) -> Vec<f64> {
    pixels.iter().map(|&v| v as f64).collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_u16_le_roundtrip() {
        let data = vec![0x01u8, 0x00, 0xFF, 0xFF, 0x34, 0x12];
        let result = decode_u16_le(&data);
        assert_eq!(result, vec![0x0001, 0xFFFF, 0x1234]);
    }

    #[test]
    fn decode_i16_le_signed() {
        let data = vec![0x00u8, 0x80]; // -32768 in little-endian
        let result = decode_i16_le(&data);
        assert_eq!(result, vec![i16::MIN]);
    }

    #[test]
    fn mask_u16_12bit() {
        // BitsStored=12, HighBit=11: bits [11:0], shift=0, mask=0xFFF
        let pixels = vec![0x0FFFu16, 0x0800, 0x1234];
        let masked = mask_u16(&pixels, 12, 11);
        assert_eq!(masked, vec![0x0FFF, 0x0800, 0x0234]);
    }

    #[test]
    fn mask_u16_identity_16bit() {
        let pixels: Vec<u16> = (0u16..8).collect();
        let masked = mask_u16(&pixels, 16, 15);
        assert_eq!(masked, pixels);
    }

    #[test]
    fn mask_i16_sign_extend() {
        // BitsStored=12, HighBit=11: value 0x800 = -2048 when sign-extended to 12 bits
        let pixels = vec![0x0800i16];
        let masked = mask_i16(&pixels, 12, 11);
        // 0x800 has bit 11 set → negative. Sign extension: 0x800 | 0xF000 = -2048
        assert_eq!(masked[0], -2048i16);
    }
}
