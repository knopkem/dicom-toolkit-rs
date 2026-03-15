//! JPEG-LS parameters and default threshold computation (ISO/IEC 14495-1).

/// Interleave mode for multi-component images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterleaveMode {
    /// Each component encoded in a separate scan.
    None = 0,
    /// Components interleaved line by line.
    Line = 1,
    /// Components interleaved sample by sample (pixel interleaved).
    Sample = 2,
}

impl InterleaveMode {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::Line),
            2 => Some(Self::Sample),
            _ => None,
        }
    }
}

/// HP color transform identifiers (extension, not part of base standard).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTransform {
    None = 0,
    /// R' = R - G, B' = B - G
    Hp1 = 1,
    /// Iterative green centering
    Hp2 = 2,
    /// Complex bit-shifted
    Hp3 = 3,
}

impl ColorTransform {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::None),
            1 => Some(Self::Hp1),
            2 => Some(Self::Hp2),
            3 => Some(Self::Hp3),
            _ => None,
        }
    }
}

/// Custom JPEG-LS coding parameters (T1, T2, T3, RESET, MAXVAL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JlsCustomParameters {
    pub max_val: i32,
    pub t1: i32,
    pub t2: i32,
    pub t3: i32,
    pub reset: i32,
}

/// Frame-level JPEG-LS parameters.
#[derive(Debug, Clone)]
pub struct JlsParameters {
    pub width: u32,
    pub height: u32,
    pub bits_per_sample: u8,
    pub components: u8,
    /// NEAR parameter: 0 = lossless, >0 = near-lossless with bounded error.
    pub near: i32,
    pub interleave: InterleaveMode,
    pub color_transform: ColorTransform,
    pub custom: JlsCustomParameters,
}

impl Default for JlsParameters {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            bits_per_sample: 8,
            components: 1,
            near: 0,
            interleave: InterleaveMode::None,
            color_transform: ColorTransform::None,
            custom: JlsCustomParameters::default(),
        }
    }
}

// Default threshold constants from the standard.
const BASIC_T1: i32 = 3;
const BASIC_T2: i32 = 7;
const BASIC_T3: i32 = 21;

/// Default reset interval.
pub const BASIC_RESET: i32 = 64;

fn clamp(val: i32, lo: i32, hi: i32) -> i32 {
    if val > hi || val < lo {
        lo
    } else {
        val
    }
}

/// Compute the default T1, T2, T3, RESET values for a given MAXVAL and NEAR.
///
/// Per ISO/IEC 14495-1 §C.2.4.1.1 and CharLS `ComputeDefault`.
pub fn compute_default(max_val: i32, near: i32) -> JlsCustomParameters {
    let factor = (max_val.min(4095) + 128) / 256;

    let t1 = clamp(factor * (BASIC_T1 - 2) + 2 + 3 * near, near + 1, max_val);
    let t2 = clamp(factor * (BASIC_T2 - 3) + 3 + 5 * near, t1, max_val);
    let t3 = clamp(factor * (BASIC_T3 - 4) + 4 + 7 * near, t2, max_val);

    JlsCustomParameters {
        max_val,
        t1,
        t2,
        t3,
        reset: BASIC_RESET,
    }
}

/// Compute derived traits for the codec from MAXVAL and NEAR.
#[derive(Debug, Clone, Copy)]
pub struct DerivedTraits {
    pub max_val: i32,
    pub near: i32,
    pub range: i32,
    pub bpp: i32,
    pub qbpp: i32,
    pub limit: i32,
    pub reset: i32,
}

impl DerivedTraits {
    pub fn new(max_val: i32, near: i32, reset: i32) -> Self {
        let range = (max_val + 2 * near) / (2 * near + 1) + 1;
        let bpp = log2_ceil(max_val);
        let qbpp = log2_ceil(range);
        let limit = 2 * (bpp + bpp.max(8));
        Self {
            max_val,
            near,
            range,
            bpp,
            qbpp,
            limit,
            reset,
        }
    }

    /// Quantize an error value for near-lossless mode.
    #[inline]
    pub fn quantize_error(&self, e: i32) -> i32 {
        if self.near == 0 {
            return e;
        }
        if e > 0 {
            (e + self.near) / (2 * self.near + 1)
        } else {
            -(self.near - e) / (2 * self.near + 1)
        }
    }

    /// Map an error value into the modular range.
    #[inline]
    pub fn mod_range(&self, mut err: i32) -> i32 {
        if err < 0 {
            err += self.range;
        }
        if err >= (self.range + 1) / 2 {
            err -= self.range;
        }
        err
    }

    /// Compute the error value: quantize then mod-range.
    #[inline]
    pub fn compute_error_val(&self, e: i32) -> i32 {
        self.mod_range(self.quantize_error(e))
    }

    /// De-quantize an error value.
    #[inline]
    pub fn dequantize(&self, err: i32) -> i32 {
        err * (2 * self.near + 1)
    }

    /// Reconstruct the sample from prediction and error.
    #[inline]
    pub fn compute_reconstructed(&self, px: i32, err: i32) -> i32 {
        let val = px + self.dequantize(err);
        self.fix_reconstructed(val)
    }

    fn fix_reconstructed(&self, mut val: i32) -> i32 {
        if val < -self.near {
            val += self.range * (2 * self.near + 1);
        } else if val > self.max_val + self.near {
            val -= self.range * (2 * self.near + 1);
        }
        self.correct_prediction(val)
    }

    /// Clamp a predicted value to [0, MAXVAL].
    #[inline]
    pub fn correct_prediction(&self, pxc: i32) -> i32 {
        if (pxc & self.max_val) == pxc {
            pxc
        } else {
            (!(pxc >> 31)) & self.max_val
        }
    }
}

/// Compute ⌈log₂(n)⌉ (minimum bits needed to represent values up to n).
fn log2_ceil(n: i32) -> i32 {
    let mut x = 0;
    while n > (1i32 << x) {
        x += 1;
    }
    x
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_thresholds_8bit() {
        let p = compute_default(255, 0);
        assert_eq!(p.t1, 3);
        assert_eq!(p.t2, 7);
        assert_eq!(p.t3, 21);
        assert_eq!(p.reset, 64);
        assert_eq!(p.max_val, 255);
    }

    #[test]
    fn default_thresholds_12bit() {
        let p = compute_default(4095, 0);
        assert_eq!(p.t1, 18);
        assert_eq!(p.t2, 67);
        assert_eq!(p.t3, 276);
        assert_eq!(p.reset, 64);
    }

    #[test]
    fn default_thresholds_16bit() {
        let p = compute_default(65535, 0);
        // FACTOR = min(65535,4095)+128)/256 = (4095+128)/256 = 16
        assert_eq!(p.t1, 18);
        assert_eq!(p.t2, 67);
        assert_eq!(p.t3, 276);
    }

    #[test]
    fn default_thresholds_near_lossless() {
        let p = compute_default(255, 2);
        // FACTOR=1, T1 = clamp(1*(3-2)+2+6, 3, 255) = clamp(9,3,255) = 9
        assert_eq!(p.t1, 9);
        // T2 = clamp(1*(7-3)+3+10, 9, 255) = clamp(17,9,255) = 17
        assert_eq!(p.t2, 17);
        // T3 = clamp(1*(21-4)+4+14, 17, 255) = clamp(35,17,255) = 35
        assert_eq!(p.t3, 35);
    }

    #[test]
    fn derived_traits_lossless_8bit() {
        let t = DerivedTraits::new(255, 0, BASIC_RESET);
        assert_eq!(t.range, 256);
        assert_eq!(t.bpp, 8);
        assert_eq!(t.qbpp, 8); // log2_ceil(256) = 8 since 2^8 = 256
        assert_eq!(t.limit, 32);
    }

    #[test]
    fn derived_traits_near_lossless() {
        let t = DerivedTraits::new(255, 2, BASIC_RESET);
        // RANGE = (255 + 4) / 5 + 1 = 51 + 1 = 52
        assert_eq!(t.range, 52);
        assert_eq!(t.bpp, 8);
        assert_eq!(t.qbpp, 6); // log2_ceil(52) = 6
    }

    #[test]
    fn log2_ceil_values() {
        assert_eq!(log2_ceil(1), 0);
        assert_eq!(log2_ceil(2), 1);
        assert_eq!(log2_ceil(255), 8);
        assert_eq!(log2_ceil(256), 8);
        assert_eq!(log2_ceil(257), 9);
        assert_eq!(log2_ceil(4095), 12);
        assert_eq!(log2_ceil(65535), 16);
    }

    #[test]
    fn mod_range_lossless_8bit() {
        let t = DerivedTraits::new(255, 0, BASIC_RESET);
        assert_eq!(t.mod_range(0), 0);
        assert_eq!(t.mod_range(1), 1);
        assert_eq!(t.mod_range(127), 127);
        assert_eq!(t.mod_range(128), -128);
        assert_eq!(t.mod_range(-1), -1);
        assert_eq!(t.mod_range(-128), -128);
    }

    #[test]
    fn correct_prediction_clamps() {
        let t = DerivedTraits::new(255, 0, BASIC_RESET);
        assert_eq!(t.correct_prediction(100), 100);
        assert_eq!(t.correct_prediction(0), 0);
        assert_eq!(t.correct_prediction(255), 255);
        assert_eq!(t.correct_prediction(300), 255); // clamped high
        assert_eq!(t.correct_prediction(-1), 0); // clamped low (sign bit flip)
    }
}
