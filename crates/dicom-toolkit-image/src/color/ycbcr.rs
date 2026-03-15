//! YCbCr / YBR color model conversions.
//!
//! Implements the DICOM YBR_FULL and YBR_FULL_422 → RGB conversions
//! per PS 3.3 §C.7.6.3.1.2.
//!
//! The conversion coefficients follow ITU-R BT.601.

use dicom_toolkit_core::error::{DcmError, DcmResult};

// ── Single-pixel conversion ───────────────────────────────────────────────────

/// Convert a single YBR_FULL pixel (Y, Cb, Cr) to (R, G, B).
///
/// Coefficients from PS 3.3 §C.7.6.3.1.2:
/// ```text
/// R = Y + 1.402 * (Cr − 128)
/// G = Y − 0.344136 * (Cb − 128) − 0.714136 * (Cr − 128)
/// B = Y + 1.772 * (Cb − 128)
/// ```
pub fn ybr_to_rgb_pixel(y: u8, cb: u8, cr: u8) -> (u8, u8, u8) {
    let y = y as f64;
    let cb = cb as f64 - 128.0;
    let cr = cr as f64 - 128.0;

    let r = (y + 1.402 * cr).round().clamp(0.0, 255.0) as u8;
    let g = (y - 0.344_136 * cb - 0.714_136 * cr).round().clamp(0.0, 255.0) as u8;
    let b = (y + 1.772 * cb).round().clamp(0.0, 255.0) as u8;

    (r, g, b)
}

// ── Frame-level conversions ───────────────────────────────────────────────────

/// Convert a pixel-interleaved YBR_FULL frame to pixel-interleaved RGB.
///
/// Input must be `[Y, Cb, Cr, Y, Cb, Cr, …]` (3 bytes per pixel).
pub fn ybr_full_to_rgb(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(3) {
        let (r, g, b) = ybr_to_rgb_pixel(chunk[0], chunk[1], chunk[2]);
        out.extend_from_slice(&[r, g, b]);
    }
    out
}

/// Convert a YBR_FULL_422 frame to pixel-interleaved RGB.
///
/// YBR_FULL_422 stores pairs of Y with shared Cb and Cr:
/// `[Cb, Y0, Cr, Y1, Cb, Y2, Cr, Y3, …]` — each 4-byte group covers 2 pixels.
///
/// `cols` must be even.
pub fn ybr_full_422_to_rgb(data: &[u8], cols: usize, rows: usize) -> DcmResult<Vec<u8>> {
    // Each row: cols/2 groups of 4 bytes → cols pixels × 3 channels
    if cols % 2 != 0 {
        return Err(DcmError::Other(format!(
            "YBR_FULL_422 requires even column count, got {cols}"
        )));
    }
    let bytes_per_row = cols * 2; // 2 bytes per pixel in 422
    let mut out = Vec::with_capacity(rows * cols * 3);

    for row in 0..rows {
        let row_start = row * bytes_per_row;
        let row_data = &data[row_start..row_start + bytes_per_row.min(data.len() - row_start)];
        for group in row_data.chunks_exact(4) {
            // DICOM 422 byte order: Cb, Y0, Cr, Y1
            let cb = group[0];
            let y0 = group[1];
            let cr = group[2];
            let y1 = group[3];
            let (r0, g0, b0) = ybr_to_rgb_pixel(y0, cb, cr);
            let (r1, g1, b1) = ybr_to_rgb_pixel(y1, cb, cr);
            out.extend_from_slice(&[r0, g0, b0, r1, g1, b1]);
        }
    }
    Ok(out)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ybr_to_rgb_white() {
        // Y=235, Cb=128, Cr=128 → no chroma offset → near-white gray
        let (r, g, b) = ybr_to_rgb_pixel(235, 128, 128);
        assert!((r as i32 - 235).abs() <= 1, "R={r}");
        assert!((g as i32 - 235).abs() <= 1, "G={g}");
        assert!((b as i32 - 235).abs() <= 1, "B={b}");
    }

    #[test]
    fn ybr_to_rgb_black() {
        // Y=16, Cb=128, Cr=128 → no chroma offset → near-black gray
        let (r, g, b) = ybr_to_rgb_pixel(16, 128, 128);
        assert!((r as i32 - 16).abs() <= 1, "R={r}");
        assert!((g as i32 - 16).abs() <= 1, "G={g}");
        assert!((b as i32 - 16).abs() <= 1, "B={b}");
    }

    #[test]
    fn ybr_full_to_rgb_frame() {
        // Achromatic: Cb=Cr=128 → R=G=B=Y
        let data = vec![128u8, 128, 128, 200, 128, 128];
        let rgb = ybr_full_to_rgb(&data);
        assert_eq!(rgb.len(), 6);
        assert!((rgb[0] as i32 - 128).abs() <= 1);
        assert!((rgb[3] as i32 - 200).abs() <= 1);
    }

    #[test]
    fn ybr_full_422_to_rgb_pair() {
        // 2 pixels sharing Cb=128, Cr=128
        let data = vec![128u8, 100, 128, 150]; // Cb, Y0, Cr, Y1
        let rgb = ybr_full_422_to_rgb(&data, 2, 1).unwrap();
        assert_eq!(rgb.len(), 6);
        // Pixel 0: Y=100, no chroma → near (100, 100, 100)
        assert!((rgb[0] as i32 - 100).abs() <= 1);
        // Pixel 1: Y=150
        assert!((rgb[3] as i32 - 150).abs() <= 1);
    }
}
