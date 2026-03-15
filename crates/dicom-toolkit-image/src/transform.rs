//! Geometric image transforms: rotation, flip, and bilinear scaling.
//!
//! All functions operate on packed byte buffers with a configurable number of
//! channels per pixel (e.g. 1 for grayscale, 3 for RGB).

// ── Enums ─────────────────────────────────────────────────────────────────────

/// Clockwise rotation amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    /// No rotation.
    None,
    /// 90° clockwise.
    R90,
    /// 180°.
    R180,
    /// 270° clockwise (= 90° counter-clockwise).
    R270,
}

/// Axis-aligned flip operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flip {
    /// No flip.
    None,
    /// Mirror left-right (around the vertical axis).
    Horizontal,
    /// Mirror top-bottom (around the horizontal axis).
    Vertical,
    /// Both horizontal and vertical (equivalent to 180° rotation).
    Both,
}

// ── rotate ────────────────────────────────────────────────────────────────────

/// Rotate a packed pixel buffer by the given angle.
///
/// Returns `(rotated_pixels, new_rows, new_cols)`.  For `R90`/`R270` the row
/// and column dimensions are swapped.
///
/// # Parameters
/// - `pixels` — packed bytes, row-major, `channels` bytes per pixel.
/// - `rows` / `cols` — image dimensions.
/// - `channels` — bytes per pixel (1 = grayscale, 3 = RGB, …).
/// - `rotation` — clockwise rotation amount.
pub fn rotate(
    pixels: &[u8],
    rows: u32,
    cols: u32,
    channels: u8,
    rotation: Rotation,
) -> (Vec<u8>, u32, u32) {
    let ch = channels as usize;
    let r = rows as usize;
    let c = cols as usize;

    let (nr, nc) = match rotation {
        Rotation::None | Rotation::R180 => (r, c),
        Rotation::R90 | Rotation::R270 => (c, r),
    };

    let mut out = vec![0u8; nr * nc * ch];

    for dr in 0..nr {
        for dc in 0..nc {
            // Map destination (dr, dc) back to a source (sr, sc).
            let (sr, sc) = match rotation {
                Rotation::None => (dr, dc),
                Rotation::R90 => (r - 1 - dc, dr),
                Rotation::R180 => (r - 1 - dr, c - 1 - dc),
                Rotation::R270 => (dc, c - 1 - dr),
            };
            let src = (sr * c + sc) * ch;
            let dst = (dr * nc + dc) * ch;
            out[dst..dst + ch].copy_from_slice(&pixels[src..src + ch]);
        }
    }

    (out, nr as u32, nc as u32)
}

// ── flip ──────────────────────────────────────────────────────────────────────

/// Flip a packed pixel buffer along one or both axes.
///
/// Returns a new buffer with the same dimensions.
pub fn flip(pixels: &[u8], rows: u32, cols: u32, channels: u8, flip: Flip) -> Vec<u8> {
    let ch = channels as usize;
    let r = rows as usize;
    let c = cols as usize;
    let mut out = vec![0u8; r * c * ch];

    for dr in 0..r {
        for dc in 0..c {
            let (sr, sc) = match flip {
                Flip::None => (dr, dc),
                Flip::Horizontal => (dr, c - 1 - dc),
                Flip::Vertical => (r - 1 - dr, dc),
                Flip::Both => (r - 1 - dr, c - 1 - dc),
            };
            let src = (sr * c + sc) * ch;
            let dst = (dr * c + dc) * ch;
            out[dst..dst + ch].copy_from_slice(&pixels[src..src + ch]);
        }
    }

    out
}

// ── scale_bilinear ────────────────────────────────────────────────────────────

/// Resize a pixel buffer using bilinear interpolation.
///
/// Samples are placed at pixel centres (`pixel + 0.5`) and interpolated
/// across four nearest neighbours.
pub fn scale_bilinear(
    pixels: &[u8],
    rows: u32,
    cols: u32,
    channels: u8,
    new_rows: u32,
    new_cols: u32,
) -> Vec<u8> {
    let ch = channels as usize;
    let r = rows as usize;
    let c = cols as usize;
    let nr = new_rows as usize;
    let nc = new_cols as usize;

    let scale_r = r as f64 / nr as f64;
    let scale_c = c as f64 / nc as f64;

    let mut out = vec![0u8; nr * nc * ch];

    for dr in 0..nr {
        for dc in 0..nc {
            // Map new pixel centre to source coordinates.
            let src_r = ((dr as f64 + 0.5) * scale_r - 0.5).max(0.0);
            let src_c = ((dc as f64 + 0.5) * scale_c - 0.5).max(0.0);

            let r0 = src_r as usize;
            let c0 = src_c as usize;
            let r1 = (r0 + 1).min(r - 1);
            let c1 = (c0 + 1).min(c - 1);

            let dr_frac = (src_r - r0 as f64).clamp(0.0, 1.0);
            let dc_frac = (src_c - c0 as f64).clamp(0.0, 1.0);

            let dst = (dr * nc + dc) * ch;

            for k in 0..ch {
                let p00 = pixels[(r0 * c + c0) * ch + k] as f64;
                let p01 = pixels[(r0 * c + c1) * ch + k] as f64;
                let p10 = pixels[(r1 * c + c0) * ch + k] as f64;
                let p11 = pixels[(r1 * c + c1) * ch + k] as f64;

                let v = p00 * (1.0 - dr_frac) * (1.0 - dc_frac)
                    + p01 * (1.0 - dr_frac) * dc_frac
                    + p10 * dr_frac * (1.0 - dc_frac)
                    + p11 * dr_frac * dc_frac;

                out[dst + k] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }

    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // 2×3 grayscale image:
    //   1 2 3
    //   4 5 6
    fn sample_image() -> Vec<u8> {
        vec![1, 2, 3, 4, 5, 6]
    }

    #[test]
    fn rotate_90() {
        // 90° CW: (new_rows=3, new_cols=2)
        //   4 1
        //   5 2
        //   6 3
        let (out, nr, nc) = rotate(&sample_image(), 2, 3, 1, Rotation::R90);
        assert_eq!(nr, 3);
        assert_eq!(nc, 2);
        assert_eq!(out, vec![4, 1, 5, 2, 6, 3]);
    }

    #[test]
    fn rotate_180() {
        let (out, nr, nc) = rotate(&sample_image(), 2, 3, 1, Rotation::R180);
        assert_eq!(nr, 2);
        assert_eq!(nc, 3);
        assert_eq!(out, vec![6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn rotate_270() {
        // 270° CW (= 90° CCW): (new_rows=3, new_cols=2)
        //   3 6
        //   2 5
        //   1 4
        let (out, nr, nc) = rotate(&sample_image(), 2, 3, 1, Rotation::R270);
        assert_eq!(nr, 3);
        assert_eq!(nc, 2);
        assert_eq!(out, vec![3, 6, 2, 5, 1, 4]);
    }

    #[test]
    fn rotate_none() {
        let (out, nr, nc) = rotate(&sample_image(), 2, 3, 1, Rotation::None);
        assert_eq!((nr, nc), (2, 3));
        assert_eq!(out, sample_image());
    }

    #[test]
    fn flip_horizontal() {
        //   3 2 1
        //   6 5 4
        let out = flip(&sample_image(), 2, 3, 1, Flip::Horizontal);
        assert_eq!(out, vec![3, 2, 1, 6, 5, 4]);
    }

    #[test]
    fn flip_vertical() {
        //   4 5 6
        //   1 2 3
        let out = flip(&sample_image(), 2, 3, 1, Flip::Vertical);
        assert_eq!(out, vec![4, 5, 6, 1, 2, 3]);
    }

    #[test]
    fn flip_both() {
        let out = flip(&sample_image(), 2, 3, 1, Flip::Both);
        assert_eq!(out, vec![6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn scale_bilinear_identity() {
        // Scaling to the same size should produce the same image.
        let pixels = vec![10u8, 20, 30, 40];
        let out = scale_bilinear(&pixels, 2, 2, 1, 2, 2);
        assert_eq!(out, pixels);
    }

    #[test]
    fn scale_bilinear_upsample() {
        // 1×1 → 2×2: all four output pixels equal the single input pixel.
        let pixels = vec![200u8];
        let out = scale_bilinear(&pixels, 1, 1, 1, 2, 2);
        assert_eq!(out, vec![200, 200, 200, 200]);
    }
}
