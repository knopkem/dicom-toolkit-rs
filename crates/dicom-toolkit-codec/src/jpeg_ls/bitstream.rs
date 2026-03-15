//! Bitstream reader and writer with JPEG-LS FF-bitstuffing.
//!
//! Per ISO/IEC 14495-1 §A.1: after a 0xFF byte is written/read, the next byte
//! has its high bit reserved (stuffed to 0). This means only 7 payload bits
//! follow an 0xFF byte, not 8.

use dicom_toolkit_core::error::{DcmError, DcmResult};

// ── BitReader ─────────────────────────────────────────────────────────────────

/// Reads bits from a byte slice, handling JPEG-LS FF-bitstuffing.
pub struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    /// Cached bits (MSB-aligned in a u64).
    cache: u64,
    /// Number of valid bits in `cache`.
    valid_bits: i32,
    /// Position of the next 0xFF byte (for fast-path skipping).
    next_ff: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let next_ff = find_next_ff(data, 0);
        let mut reader = Self {
            data,
            byte_pos: 0,
            cache: 0,
            valid_bits: 0,
            next_ff,
        };
        reader.fill();
        reader
    }

    /// Current byte position in the source data.
    pub fn byte_position(&self) -> usize {
        // Back-track: the cache may hold bytes that haven't been "consumed".
        let mut pos = self.byte_pos;
        let mut bits = self.valid_bits;
        while bits > 0 && pos > 0 {
            pos -= 1;
            let bits_from_byte = if pos > 0 && self.data[pos - 1] == 0xFF {
                7
            } else {
                8
            };
            bits -= bits_from_byte;
        }
        pos
    }

    /// Read `length` bits (up to 25) and return them right-aligned.
    #[inline]
    pub fn read_value(&mut self, length: i32) -> DcmResult<i32> {
        debug_assert!(length > 0 && length <= 25);
        if self.valid_bits < length {
            self.fill();
            if self.valid_bits < length {
                return Err(DcmError::DecompressionError {
                    reason: "JPEG-LS: unexpected end of bitstream".into(),
                });
            }
        }
        let result = (self.cache >> (64 - length)) as i32;
        self.skip(length);
        Ok(result)
    }

    /// Read a single bit.
    #[inline]
    pub fn read_bit(&mut self) -> DcmResult<bool> {
        if self.valid_bits <= 0 {
            self.fill();
            if self.valid_bits <= 0 {
                return Err(DcmError::DecompressionError {
                    reason: "JPEG-LS: unexpected end of bitstream".into(),
                });
            }
        }
        let set = (self.cache & (1u64 << 63)) != 0;
        self.skip(1);
        Ok(set)
    }

    /// Peek at the top 8 bits of the cache (for lookup-table decoding).
    #[inline]
    pub fn peek_byte(&mut self) -> i32 {
        if self.valid_bits < 8 {
            self.fill();
        }
        (self.cache >> 56) as i32
    }

    /// Count leading zero bits (up to 16).
    #[inline]
    pub fn read_highbits(&mut self) -> DcmResult<i32> {
        if self.valid_bits < 16 {
            self.fill();
        }
        let mut count = 0i32;
        let mut val = self.cache;
        while count < 16 {
            if (val & (1u64 << 63)) != 0 {
                self.skip(count + 1);
                return Ok(count);
            }
            val <<= 1;
            count += 1;
        }
        // More than 16 leading zeros.
        self.skip(15);
        let mut highbits = 15i32;
        loop {
            if self.read_bit()? {
                return Ok(highbits);
            }
            highbits += 1;
        }
    }

    /// Skip `n` bits in the cache.
    #[inline]
    pub fn skip(&mut self, n: i32) {
        self.valid_bits -= n;
        self.cache <<= n as u32;
    }

    /// Fill the cache from the byte stream (with FF-bitstuffing handling).
    fn fill(&mut self) {
        // Fast path: no 0xFF nearby.
        if self.byte_pos + 8 <= self.next_ff {
            let bytes_to_read = ((64 - self.valid_bits) >> 3) as usize;
            let bytes_to_read = bytes_to_read.min(self.data.len() - self.byte_pos);
            for _ in 0..bytes_to_read {
                self.cache |= (self.data[self.byte_pos] as u64) << (56 - self.valid_bits as u32);
                self.byte_pos += 1;
                self.valid_bits += 8;
            }
            return;
        }

        // Slow path: handle FF-bitstuffing.
        while self.valid_bits < 56 {
            if self.byte_pos >= self.data.len() {
                return;
            }

            let val = self.data[self.byte_pos] as u64;

            // Check if this is a marker (FF followed by >= 0x80).
            if val == 0xFF
                && (self.byte_pos + 1 >= self.data.len()
                    || (self.data[self.byte_pos + 1] & 0x80) != 0)
            {
                return; // Don't read into markers.
            }

            self.cache |= val << (56 - self.valid_bits as u32);
            self.byte_pos += 1;
            self.valid_bits += 8;

            // After reading 0xFF, the next byte has only 7 payload bits.
            if val == 0xFF {
                self.valid_bits -= 1;
            }
        }

        self.next_ff = find_next_ff(self.data, self.byte_pos);
    }
}

fn find_next_ff(data: &[u8], start: usize) -> usize {
    data[start..]
        .iter()
        .position(|&b| b == 0xFF)
        .map_or(data.len(), |i| start + i)
}

// ── BitWriter ─────────────────────────────────────────────────────────────────

/// Writes bits to a `Vec<u8>`, handling JPEG-LS FF-bitstuffing.
pub struct BitWriter {
    output: Vec<u8>,
    /// Accumulator (MSB-aligned in a u32).
    val_current: u32,
    /// Number of free bits remaining in `val_current` (starts at 32).
    bit_pos: i32,
    /// Whether the last written byte was 0xFF.
    is_ff_written: bool,
}

impl Default for BitWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl BitWriter {
    pub fn new() -> Self {
        Self {
            output: Vec::with_capacity(4096),
            val_current: 0,
            bit_pos: 32,
            is_ff_written: false,
        }
    }

    /// Append `length` bits from `value` (right-aligned) to the bitstream.
    ///
    /// Supports any non-negative length. For lengths >= 32 the value must be 0
    /// (only zero-padding uses lengths that large).
    #[inline]
    pub fn append(&mut self, value: i32, length: i32) {
        debug_assert!(length >= 0);
        debug_assert!(length < 32 || value == 0, "only 0-bits for length >= 32");

        // Handle large zero-padding (e.g. unary prefix in Golomb escape codes).
        if length >= 32 {
            let mut remaining = length;
            while remaining >= 31 {
                self.append_short(0, 31);
                remaining -= 31;
            }
            if remaining > 0 {
                self.append_short(0, remaining);
            }
            return;
        }

        self.append_short(value, length);
    }

    /// Inner append for lengths 0..31.
    #[inline]
    fn append_short(&mut self, value: i32, length: i32) {
        debug_assert!((0..32).contains(&length));
        if length == 0 {
            return;
        }
        self.bit_pos -= length;
        if self.bit_pos >= 0 {
            if self.bit_pos < 32 {
                self.val_current |= (value as u32) << (self.bit_pos as u32);
            }
            return;
        }
        // Overflow: flush and continue.
        self.val_current |= (value as u32).wrapping_shr((-self.bit_pos) as u32);
        self.flush();
        if self.bit_pos < 0 {
            self.val_current |= (value as u32).wrapping_shr((-self.bit_pos) as u32);
            self.flush();
        }
        debug_assert!(self.bit_pos >= 0);
        if self.bit_pos < 32 {
            self.val_current |= (value as u32) << (self.bit_pos as u32);
        }
    }

    /// Append `length` 1-bits.
    #[inline]
    pub fn append_ones(&mut self, length: i32) {
        self.append((1 << length) - 1, length);
    }

    /// Finalize the bitstream: flush remaining bits.
    pub fn end_scan(&mut self) {
        self.flush();
        if self.is_ff_written {
            self.append(0, (self.bit_pos - 1) % 8);
        } else {
            self.append(0, self.bit_pos % 8);
        }
        self.flush();
    }

    /// Get the written bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.output
    }

    /// Current byte length of written data.
    pub fn len(&self) -> usize {
        self.output.len()
    }

    /// Whether the writer has no data.
    pub fn is_empty(&self) -> bool {
        self.output.is_empty()
    }

    fn flush(&mut self) {
        for _ in 0..4 {
            if self.bit_pos >= 32 {
                break;
            }
            if self.is_ff_written {
                // After 0xFF: write 7 bits (inserting the 0-stuffed bit).
                self.write_byte((self.val_current >> 25) as u8);
                self.val_current <<= 7;
                self.bit_pos += 7;
                self.is_ff_written = false;
            } else {
                let byte = (self.val_current >> 24) as u8;
                self.write_byte(byte);
                self.is_ff_written = byte == 0xFF;
                self.val_current <<= 8;
                self.bit_pos += 8;
            }
        }
    }

    fn write_byte(&mut self, byte: u8) {
        self.output.push(byte);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_simple_values() {
        let mut w = BitWriter::new();
        w.append(0b101, 3);
        w.append(0b11110000, 8);
        w.append(0b1, 1);
        w.end_scan();
        let bytes = w.into_bytes();

        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_value(3).unwrap(), 0b101);
        assert_eq!(r.read_value(8).unwrap(), 0b11110000);
        assert!(r.read_bit().unwrap());
    }

    #[test]
    fn ff_bitstuffing_roundtrip() {
        // Write values that produce 0xFF bytes and verify round-trip.
        let mut w = BitWriter::new();
        w.append(0xFF, 8);
        w.append(0x01, 3);
        w.end_scan();
        let bytes = w.into_bytes();

        // After 0xFF, a 0 bit must be stuffed by the writer.
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_value(8).unwrap(), 0xFF);
        assert_eq!(r.read_value(3).unwrap(), 0x01);
    }

    #[test]
    fn read_highbits_counts_zeros() {
        let mut w = BitWriter::new();
        // 5 zeros then a 1-bit
        w.append(0b000001, 6);
        w.append(0b1, 1); // padding
        w.end_scan();
        let bytes = w.into_bytes();

        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read_highbits().unwrap(), 5);
    }

    #[test]
    fn roundtrip_many_small_values() {
        let mut w = BitWriter::new();
        for i in 0..100 {
            w.append(i & 0x1F, 5);
        }
        w.end_scan();
        let bytes = w.into_bytes();

        let mut r = BitReader::new(&bytes);
        for i in 0..100 {
            assert_eq!(r.read_value(5).unwrap(), i & 0x1F);
        }
    }

    #[test]
    fn peek_byte_works() {
        let mut w = BitWriter::new();
        w.append(0b10110100, 8);
        w.end_scan();
        let bytes = w.into_bytes();

        let mut r = BitReader::new(&bytes);
        assert_eq!(r.peek_byte(), 0b10110100);
        // peek shouldn't consume
        assert_eq!(r.read_value(8).unwrap(), 0b10110100);
    }
}
