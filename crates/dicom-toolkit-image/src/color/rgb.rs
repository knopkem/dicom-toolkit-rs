//! RGB color model utilities.
//!
//! Helpers for plane-interleaved ↔ pixel-interleaved conversion and basic
//! RGB clamping.

// ── Planar conversion ─────────────────────────────────────────────────────────

/// Convert plane-interleaved (RRRR…GGGG…BBBB…) to pixel-interleaved (RGBRGB…).
///
/// Used when `PlanarConfiguration = 1`.
/// `n_pixels` is the total number of pixels (rows × columns).
pub fn planar_to_pixel(data: &[u8], n_pixels: usize) -> Vec<u8> {
    let mut out = vec![0u8; n_pixels * 3];
    let plane = n_pixels;
    for i in 0..n_pixels {
        out[i * 3] = data.get(i).copied().unwrap_or(0);
        out[i * 3 + 1] = data.get(plane + i).copied().unwrap_or(0);
        out[i * 3 + 2] = data.get(2 * plane + i).copied().unwrap_or(0);
    }
    out
}

/// Convert pixel-interleaved (RGBRGB…) to plane-interleaved (RRRR…GGGG…BBBB…).
///
/// Used when encoding with `PlanarConfiguration = 1`.
pub fn pixel_to_planar(data: &[u8], n_pixels: usize) -> Vec<u8> {
    let mut out = vec![0u8; n_pixels * 3];
    for i in 0..n_pixels {
        out[i] = data.get(i * 3).copied().unwrap_or(0);
        out[n_pixels + i] = data.get(i * 3 + 1).copied().unwrap_or(0);
        out[2 * n_pixels + i] = data.get(i * 3 + 2).copied().unwrap_or(0);
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planar_to_pixel_roundtrip() {
        // 2 pixels: R=[10,20], G=[30,40], B=[50,60]
        let planar = vec![10u8, 20, 30, 40, 50, 60];
        let pixel = planar_to_pixel(&planar, 2);
        assert_eq!(pixel, vec![10, 30, 50, 20, 40, 60]);
    }

    #[test]
    fn pixel_to_planar_roundtrip() {
        let pixel = vec![10u8, 30, 50, 20, 40, 60];
        let planar = pixel_to_planar(&pixel, 2);
        assert_eq!(planar, vec![10, 20, 30, 40, 50, 60]);
    }
}
