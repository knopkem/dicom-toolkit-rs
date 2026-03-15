//! Sample trait for abstracting bit-depth differences in JPEG-LS.
//!
//! Provides type-level dispatch for 8-bit (`u8`) and 16-bit (`u16`) pixel values,
//! replacing the C++ template specializations in CharLS (`LosslessTraitsT`, `DefaultTraitsT`).

/// Trait abstracting a pixel sample type for JPEG-LS encoding/decoding.
pub trait Sample:
    Copy + Clone + Default + Into<i32> + std::fmt::Debug + Send + Sync + 'static
{
    /// The maximum sample value (e.g. 255 for 8-bit, 65535 for 16-bit).
    const MAX_VAL_DEFAULT: i32;

    /// Number of bytes per sample.
    const BYTES: usize;

    /// Convert from an i32, clamping to [0, max_val].
    fn from_i32_clamped(val: i32, max_val: i32) -> Self;

    /// Read a sample from a byte slice in native (little-endian) order.
    fn read_le(data: &[u8], offset: usize) -> Self;

    /// Write a sample to a byte slice in native (little-endian) order.
    fn write_le(data: &mut [u8], offset: usize, val: Self);
}

impl Sample for u8 {
    const MAX_VAL_DEFAULT: i32 = 255;
    const BYTES: usize = 1;

    #[inline]
    fn from_i32_clamped(val: i32, max_val: i32) -> Self {
        val.max(0).min(max_val) as u8
    }

    #[inline]
    fn read_le(data: &[u8], offset: usize) -> Self {
        data[offset]
    }

    #[inline]
    fn write_le(data: &mut [u8], offset: usize, val: Self) {
        data[offset] = val;
    }
}

impl Sample for u16 {
    const MAX_VAL_DEFAULT: i32 = 65535;
    const BYTES: usize = 2;

    #[inline]
    fn from_i32_clamped(val: i32, max_val: i32) -> Self {
        val.max(0).min(max_val) as u16
    }

    #[inline]
    fn read_le(data: &[u8], offset: usize) -> Self {
        let lo = data[offset] as u16;
        let hi = data[offset + 1] as u16;
        lo | (hi << 8)
    }

    #[inline]
    fn write_le(data: &mut [u8], offset: usize, val: Self) {
        data[offset] = val as u8;
        data[offset + 1] = (val >> 8) as u8;
    }
}

/// Dispatch helper: returns true if the given bits_per_sample fits in u8.
pub fn needs_u16(bits_per_sample: u8) -> bool {
    bits_per_sample > 8
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u8_sample_basics() {
        assert_eq!(u8::MAX_VAL_DEFAULT, 255);
        assert_eq!(u8::BYTES, 1);
        assert_eq!(u8::from_i32_clamped(100, 255), 100u8);
        assert_eq!(u8::from_i32_clamped(300, 255), 255u8);
        assert_eq!(u8::from_i32_clamped(-5, 255), 0u8);
    }

    #[test]
    fn u16_sample_basics() {
        assert_eq!(u16::MAX_VAL_DEFAULT, 65535);
        assert_eq!(u16::BYTES, 2);
        assert_eq!(u16::from_i32_clamped(1000, 4095), 1000u16);
        assert_eq!(u16::from_i32_clamped(5000, 4095), 4095u16);
        assert_eq!(u16::from_i32_clamped(-1, 4095), 0u16);
    }

    #[test]
    fn u8_read_write_le() {
        let mut buf = [0u8; 4];
        u8::write_le(&mut buf, 0, 42);
        u8::write_le(&mut buf, 1, 255);
        assert_eq!(u8::read_le(&buf, 0), 42);
        assert_eq!(u8::read_le(&buf, 1), 255);
    }

    #[test]
    fn u16_read_write_le() {
        let mut buf = [0u8; 8];
        u16::write_le(&mut buf, 0, 0x1234);
        u16::write_le(&mut buf, 2, 0xABCD);
        assert_eq!(u16::read_le(&buf, 0), 0x1234);
        assert_eq!(u16::read_le(&buf, 2), 0xABCD);
    }

    #[test]
    fn into_i32_works() {
        let a: u8 = 200;
        let v: i32 = a.into();
        assert_eq!(v, 200);

        let b: u16 = 4095;
        let v: i32 = b.into();
        assert_eq!(v, 4095);
    }

    #[test]
    fn needs_u16_dispatch() {
        assert!(!needs_u16(2));
        assert!(!needs_u16(8));
        assert!(needs_u16(9));
        assert!(needs_u16(12));
        assert!(needs_u16(16));
    }
}
