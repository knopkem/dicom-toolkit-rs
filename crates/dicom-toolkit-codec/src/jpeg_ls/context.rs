//! JPEG-LS context modeling (statistical accumulators for adaptive Golomb-Rice coding).
//!
//! Port of CharLS `context.h` and `ctxtrmod.h`.

/// Context for regular (non-run) mode.
///
/// Each context stores running statistics used to compute the Golomb parameter
/// and prediction correction (bias C).
#[derive(Debug, Clone)]
pub struct JlsContext {
    /// Accumulated |error| (magnitude).
    pub a: i32,
    /// Accumulated error (signed, used for bias).
    pub b: i32,
    /// Prediction correction (bias).
    pub c: i8,
    /// Count (number of samples coded in this context).
    pub n: i16,
}

/// Lookup table for bias adjustment. Index = `B/N + 128` clamped to [0, 255].
/// The table maps the bias ratio to a correction increment of -1, 0, or +1.
static TABLE_C: [i8; 256] = {
    let mut table = [0i8; 256];
    // B/N < -1  → C += 1 (indices 0..127)
    let mut i = 0;
    while i < 127 {
        table[i] = 1;
        i += 1;
    }
    // B/N == -1 → C += 0 (index 127)
    table[127] = 0;
    // B/N == 0  → C += 0 (index 128)
    table[128] = 0;
    // B/N > 0   → C -= 1 (indices 129..255)
    let mut i = 129;
    while i < 256 {
        table[i] = -1;
        i += 1;
    }
    table
};

impl JlsContext {
    /// Create a new context with initial values per the JPEG-LS standard.
    pub fn new(a_init: i32) -> Self {
        Self {
            a: a_init,
            b: 0,
            c: 0,
            n: 1,
        }
    }

    /// Compute the Golomb coding parameter k for this context.
    #[inline]
    pub fn get_golomb(&self) -> i32 {
        let mut k = 0;
        let mut ntest = self.n as i32;
        while ntest < self.a {
            k += 1;
            ntest <<= 1;
        }
        k
    }

    /// Get the error correction value for Golomb parameter k.
    #[inline]
    pub fn get_error_correction(&self, k: i32) -> i32 {
        if k != 0 {
            0
        } else if 2 * self.b <= -(self.n as i32) {
            1
        } else {
            0
        }
    }

    /// Update context statistics after coding an error value.
    /// `err_val` is the mapped (quantized, mod-ranged) error value.
    #[inline]
    pub fn update_variables(&mut self, err_val: i32, near: i32, reset_value: i32) {
        self.a += (if err_val < 0 { -err_val } else { err_val }) - (self.near_correction(near));
        self.b += err_val * (2 * near + 1);
        self.adjust_bias(reset_value);
    }

    /// Compute near-lossless correction: A must be adjusted to reflect quantized errors.
    #[inline]
    fn near_correction(&self, _near: i32) -> i32 {
        // In the original CharLS, this is handled differently;
        // the error is already quantized before being passed here.
        0
    }

    /// Adjust bias (C) and halve counters when N reaches RESET.
    fn adjust_bias(&mut self, reset_value: i32) {
        let n = self.n as i32;

        if self.b + n <= 0 {
            self.b += n;
            if self.b <= -n {
                self.b = -n + 1;
            }
            // Look up bias correction from table.
            let idx = ((self.b.wrapping_div(n.max(1)) + 128) as usize).min(255);
            self.c = self.c.wrapping_add(TABLE_C[idx]);
            self.c = self.c.max(-128);
        } else if self.b > 0 {
            self.b -= n;
            if self.b > 0 {
                self.b = 0;
            }
            let idx = ((self.b.wrapping_div(n.max(1)) + 128) as usize).min(255);
            self.c = self.c.wrapping_add(TABLE_C[idx]);
        }

        self.n += 1;
        if self.n as i32 == reset_value {
            self.a >>= 1;
            self.b >>= 1;
            self.n >>= 1;
            // Ensure N doesn't drop to 0.
            if self.n == 0 {
                self.n = 1;
            }
        }
    }
}

// ── Run mode context ──────────────────────────────────────────────────────────

/// Context for run-interruption mode.
///
/// When the codec detects a run of identical pixels and the run ends,
/// it uses a separate set of statistics for coding the interruption sample.
#[derive(Debug, Clone)]
pub struct RunModeContext {
    /// Accumulated |error|.
    pub a: i32,
    /// Sample count.
    pub n: i32,
    /// Count of negative errors (used for map).
    pub nn: i32,
    /// Run interruption type (0 or 1).
    pub ri_type: i32,
    /// Reset threshold.
    pub reset: i32,
}

impl RunModeContext {
    pub fn new(a_init: i32, reset: i32) -> Self {
        Self {
            a: a_init,
            n: 1,
            nn: 0,
            ri_type: 0,
            reset,
        }
    }

    /// Compute Golomb parameter for run interruption.
    #[inline]
    pub fn get_golomb(&self) -> i32 {
        let mut k = 0;
        let mut ntest = self.n;
        while ntest < self.a {
            k += 1;
            ntest <<= 1;
        }
        k
    }

    /// Compute the map value (whether to add 1 to the mapped error).
    #[inline]
    pub fn compute_map(&self, err_val: i32, k: i32) -> i32 {
        if (k == 0 && err_val > 0 && 2 * self.nn < self.n)
            || (err_val < 0 && (2 * self.nn >= self.n || k != 0))
        {
            1
        } else {
            0
        }
    }

    /// Compute the mapped error value for Golomb-Rice coding.
    pub fn compute_map_negative_e(&self, k: i32) -> bool {
        k != 0 || 2 * self.nn >= self.n
    }

    /// Update run-mode statistics after coding an error.
    pub fn update_variables(&mut self, err_val: i32, e_mapped: i32) {
        if err_val < 0 {
            self.nn += 1;
        }
        self.a += (e_mapped + 1 - self.ri_type) >> 1;
        self.n += 1;

        if self.n == self.reset {
            self.a >>= 1;
            self.n >>= 1;
            self.nn >>= 1;
            if self.n == 0 {
                self.n = 1;
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_golomb_parameter() {
        // Initial A = 4, N = 1 → k should satisfy N << k >= A → 1 << 2 = 4 >= 4, so k=2
        let ctx = JlsContext::new(4);
        assert_eq!(ctx.get_golomb(), 2);
    }

    #[test]
    fn golomb_k_zero() {
        // A <= N → k = 0
        let ctx = JlsContext {
            a: 1,
            b: 0,
            c: 0,
            n: 2,
        };
        assert_eq!(ctx.get_golomb(), 0);
    }

    #[test]
    fn error_correction_at_k_zero() {
        // k=0, 2*B <= -N → correction = 1
        let ctx = JlsContext {
            a: 1,
            b: -5,
            c: 0,
            n: 2,
        };
        assert_eq!(ctx.get_error_correction(0), 1);

        // k=0, 2*B > -N → correction = 0
        let ctx2 = JlsContext {
            a: 1,
            b: 0,
            c: 0,
            n: 2,
        };
        assert_eq!(ctx2.get_error_correction(0), 0);

        // k != 0 → correction = 0
        assert_eq!(ctx.get_error_correction(1), 0);
    }

    #[test]
    fn update_variables_accumulates() {
        let mut ctx = JlsContext::new(4);
        ctx.update_variables(3, 0, 64);
        assert_eq!(ctx.a, 7); // 4 + |3|
        ctx.update_variables(-2, 0, 64);
        assert_eq!(ctx.a, 9); // 7 + |-2|
    }

    #[test]
    fn counter_halving_at_reset() {
        let mut ctx = JlsContext::new(100);
        ctx.n = 63; // just before reset
        ctx.a = 200;
        ctx.b = 10;
        ctx.update_variables(5, 0, 64);
        // N was 63, incremented to 64, hits reset → halve
        assert!(ctx.n <= 32);
        assert!(ctx.a <= 103); // (200+5)/2 ≈ 102
    }

    #[test]
    fn run_mode_context_golomb() {
        let ctx = RunModeContext::new(4, 64);
        assert_eq!(ctx.get_golomb(), 2);
    }

    #[test]
    fn run_mode_update() {
        let mut ctx = RunModeContext::new(1, 64);
        ctx.update_variables(-1, 2);
        assert_eq!(ctx.nn, 1);
        assert!(ctx.n > 1);
    }
}
