//! JPEG-LS prediction and gradient quantization.
//!
//! Port of CharLS scan.h prediction logic.

/// J[] table — run-length coding order (ISO 14495-1 Table A.1).
pub const J: [i32; 32] = [
    0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13,
    14, 15,
];

/// Median-edge predictor (MED).
///
/// Neighbors: Ra = left, Rb = above, Rc = above-left.
#[inline]
pub fn get_predicted_value(ra: i32, rb: i32, rc: i32) -> i32 {
    // Sign trick from CharLS — reduces branches.
    let sgn = bitwise_sign(rb - ra);
    if (sgn ^ (rc - ra)) < 0 {
        rb
    } else if (sgn ^ (rb - rc)) < 0 {
        ra
    } else {
        ra + rb - rc
    }
}

/// Apply sign: `(sign ^ i) - sign`.
#[inline]
pub fn apply_sign(i: i32, sign: i32) -> i32 {
    (sign ^ i) - sign
}

/// Arithmetic sign: returns -1 if n < 0, else 0.
#[inline]
pub fn bitwise_sign(n: i32) -> i32 {
    n >> 31
}

/// Compute sign: returns 1 if n >= 0, -1 if n < 0.
#[inline]
pub fn sign(n: i32) -> i32 {
    (n >> 31) | 1
}

/// Quantize a gradient into one of 9 buckets (-4..4).
#[inline]
pub fn quantize_gradient(di: i32, t1: i32, t2: i32, t3: i32, near: i32) -> i8 {
    if di <= -t3 {
        -4
    } else if di <= -t2 {
        -3
    } else if di <= -t1 {
        -2
    } else if di < -near {
        -1
    } else if di <= near {
        0
    } else if di < t1 {
        1
    } else if di < t2 {
        2
    } else if di < t3 {
        3
    } else {
        4
    }
}

/// Compute context ID from three quantized gradients (Q1, Q2, Q3).
///
/// Returns a value in 0..728 (= 9*9*9 - 1).
#[inline]
pub fn compute_context_id(q1: i32, q2: i32, q3: i32) -> i32 {
    (q1 * 9 + q2) * 9 + q3
}

/// Build a quantization lookup table for a given bit depth and thresholds.
///
/// The table maps gradient values `di` (in `[-range, range)`) to quantized buckets.
/// The returned vector is indexed by `di + range`.
pub fn build_quantization_lut(bpp: i32, t1: i32, t2: i32, t3: i32, near: i32) -> Vec<i8> {
    let range = 1i32 << bpp;
    let size = (range * 2) as usize;
    let mut lut = vec![0i8; size];
    for di in -range..range {
        lut[(di + range) as usize] = quantize_gradient(di, t1, t2, t3, near);
    }
    lut
}

/// Helper to look up a gradient in the quantization LUT.
#[inline]
pub fn quantize_from_lut(lut: &[i8], di: i32, range: i32) -> i32 {
    lut[(di + range) as usize] as i32
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn med_predictor_basic() {
        // Case: Rc >= max(Ra, Rb) → min(Ra, Rb)
        assert_eq!(get_predicted_value(10, 20, 30), 10);
        // Case: Rc <= min(Ra, Rb) → max(Ra, Rb)
        assert_eq!(get_predicted_value(20, 30, 10), 30);
        // Default: Ra + Rb - Rc
        assert_eq!(get_predicted_value(10, 20, 15), 15);
    }

    #[test]
    fn med_predictor_equal() {
        assert_eq!(get_predicted_value(100, 100, 100), 100);
    }

    #[test]
    fn quantize_gradient_buckets() {
        let (t1, t2, t3) = (3, 7, 21);
        let near = 0;

        assert_eq!(quantize_gradient(0, t1, t2, t3, near), 0);
        assert_eq!(quantize_gradient(1, t1, t2, t3, near), 1);
        assert_eq!(quantize_gradient(3, t1, t2, t3, near), 2);
        assert_eq!(quantize_gradient(7, t1, t2, t3, near), 3);
        assert_eq!(quantize_gradient(21, t1, t2, t3, near), 4);
        assert_eq!(quantize_gradient(-1, t1, t2, t3, near), -1);
        assert_eq!(quantize_gradient(-3, t1, t2, t3, near), -2);
        assert_eq!(quantize_gradient(-7, t1, t2, t3, near), -3);
        assert_eq!(quantize_gradient(-21, t1, t2, t3, near), -4);
    }

    #[test]
    fn context_id_range() {
        // Minimum: (-4, -4, -4) → (-4*9 + -4)*9 + -4 = -364
        let min_id = compute_context_id(-4, -4, -4);
        assert_eq!(min_id, -364);
        // Maximum: (4, 4, 4) → (4*9+4)*9+4 = 364
        let max_id = compute_context_id(4, 4, 4);
        assert_eq!(max_id, 364);
        // Zero: (0, 0, 0) → 0
        assert_eq!(compute_context_id(0, 0, 0), 0);
    }

    #[test]
    fn sign_functions() {
        assert_eq!(sign(5), 1);
        assert_eq!(sign(0), 1);
        assert_eq!(sign(-1), -1);
        assert_eq!(bitwise_sign(5), 0);
        assert_eq!(bitwise_sign(-1), -1);
    }

    #[test]
    fn apply_sign_function() {
        // sign=0 (positive context): apply_sign(i, 0) = (0 ^ i) - 0 = i
        assert_eq!(apply_sign(5, 0), 5);
        // sign=-1 (negative context): apply_sign(i, -1) = (-1 ^ i) - (-1) = ~i + 1 = -i
        assert_eq!(apply_sign(5, -1), -5);
        assert_eq!(apply_sign(0, -1), 0);
    }

    #[test]
    fn quantization_lut_consistent() {
        let lut = build_quantization_lut(8, 3, 7, 21, 0);
        let range = 256i32;
        for di in -range..range {
            let from_lut = quantize_from_lut(&lut, di, range);
            let direct = quantize_gradient(di, 3, 7, 21, 0) as i32;
            assert_eq!(from_lut, direct, "mismatch at di={di}");
        }
    }
}
