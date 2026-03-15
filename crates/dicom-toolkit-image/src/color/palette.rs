//! Palette color look-up table.
//!
//! Implements DICOM palette color images (PhotometricInterpretation =
//! "PALETTE COLOR") by reading the three channel LUT tables from a `DataSet`
//! and mapping pixel indices to RGB triples.

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_data::{DataSet, Value};
use dicom_toolkit_dict::Tag;

// ── PaletteColorLut ───────────────────────────────────────────────────────────

/// A three-channel (R, G, B) palette color look-up table.
///
/// Pixel indices are looked up in `red`, `green`, and `blue` tables to obtain
/// RGB output.  Table values are stored as 16-bit words; `bits_per_entry`
/// controls the number of significant bits.
#[derive(Debug, Clone)]
pub struct PaletteColorLut {
    /// Red channel LUT entries (16-bit words).
    pub red: Vec<u16>,
    /// Green channel LUT entries (16-bit words).
    pub green: Vec<u16>,
    /// Blue channel LUT entries (16-bit words).
    pub blue: Vec<u16>,
    /// First pixel value mapped by this LUT (descriptor field 2).
    pub first_value: u16,
    /// Number of significant bits per entry (8 or 16).
    pub bits_per_entry: u8,
}

impl PaletteColorLut {
    /// Build a `PaletteColorLut` from a DICOM `DataSet`.
    ///
    /// Reads the six palette color tags:
    /// - `(0028,1101)` / `(0028,1102)` / `(0028,1103)` — descriptors
    /// - `(0028,1201)` / `(0028,1202)` / `(0028,1203)` — LUT data
    pub fn from_dataset(dataset: &DataSet) -> DcmResult<Self> {
        let red_desc_tag = Tag::new(0x0028, 0x1101);
        let green_desc_tag = Tag::new(0x0028, 0x1102);
        let blue_desc_tag = Tag::new(0x0028, 0x1103);
        let red_data_tag = Tag::new(0x0028, 0x1201);
        let green_data_tag = Tag::new(0x0028, 0x1202);
        let blue_data_tag = Tag::new(0x0028, 0x1203);

        let (n_entries, first_value, bits_per_entry) = read_lut_descriptor(dataset, red_desc_tag)?;

        // Verify the other two descriptors match (best-effort).
        let _ = read_lut_descriptor(dataset, green_desc_tag);
        let _ = read_lut_descriptor(dataset, blue_desc_tag);

        let n = if n_entries == 0 { 256 } else { n_entries };

        let red = read_lut_data(dataset, red_data_tag, n)?;
        let green = read_lut_data(dataset, green_data_tag, n)?;
        let blue = read_lut_data(dataset, blue_data_tag, n)?;

        Ok(Self {
            red,
            green,
            blue,
            first_value,
            bits_per_entry,
        })
    }

    /// Look up an 8-bit RGB triple for the given pixel `index`.
    ///
    /// Values outside `[first_value, first_value + len)` are clamped to the
    /// nearest table boundary.
    pub fn lookup(&self, index: u16) -> (u8, u8, u8) {
        let i = (index.saturating_sub(self.first_value)) as usize;
        let i = i.min(self.red.len().saturating_sub(1));

        // Scale 16-bit → 8-bit: if entries are 16-bit significant, take high byte.
        let shift = if self.bits_per_entry > 8 { 8 } else { 0 };

        let r = (self.red.get(i).copied().unwrap_or(0) >> shift) as u8;
        let g = (self.green.get(i).copied().unwrap_or(0) >> shift) as u8;
        let b = (self.blue.get(i).copied().unwrap_or(0) >> shift) as u8;

        (r, g, b)
    }

    /// Map a slice of 16-bit pixel indices to RGB-interleaved `u8` bytes.
    ///
    /// Returns `[R, G, B, R, G, B, …]` with length `3 × indices.len()`.
    pub fn apply_to_frame(&self, indices: &[u16]) -> Vec<u8> {
        let mut out = Vec::with_capacity(indices.len() * 3);
        for &idx in indices {
            let (r, g, b) = self.lookup(idx);
            out.extend_from_slice(&[r, g, b]);
        }
        out
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn read_lut_descriptor(dataset: &DataSet, tag: Tag) -> DcmResult<(usize, u16, u8)> {
    let elem = dataset.find_element(tag)?;
    match &elem.value {
        Value::U16(vals) if vals.len() >= 3 => Ok((vals[0] as usize, vals[1], vals[2] as u8)),
        Value::I16(vals) if vals.len() >= 3 => {
            Ok((vals[0] as usize, vals[1] as u16, vals[2] as u8))
        }
        _ => Err(DcmError::Other(format!(
            "invalid palette LUT descriptor at ({:04X},{:04X})",
            tag.group, tag.element
        ))),
    }
}

fn read_lut_data(dataset: &DataSet, tag: Tag, _expected_n: usize) -> DcmResult<Vec<u16>> {
    let elem = dataset.find_element(tag)?;
    match &elem.value {
        Value::U16(data) => Ok(data.clone()),
        Value::U8(data) => {
            // 8-bit entries packed as low bytes of 16-bit words — keep as-is
            Ok(data.iter().map(|&v| v as u16).collect())
        }
        _ => Err(DcmError::Other(format!(
            "invalid palette LUT data at ({:04X},{:04X})",
            tag.group, tag.element
        ))),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lut() -> PaletteColorLut {
        // 256-entry 8-bit identity palette (index → same shade of gray)
        let table: Vec<u16> = (0u16..256).collect();
        PaletteColorLut {
            red: table.clone(),
            green: table.clone(),
            blue: table,
            first_value: 0,
            bits_per_entry: 8,
        }
    }

    #[test]
    fn palette_lookup_identity() {
        let lut = make_lut();
        assert_eq!(lut.lookup(0), (0, 0, 0));
        assert_eq!(lut.lookup(128), (128, 128, 128));
        assert_eq!(lut.lookup(255), (255, 255, 255));
    }

    #[test]
    fn palette_apply_to_frame() {
        let lut = make_lut();
        let indices = vec![0u16, 128, 255];
        let rgb = lut.apply_to_frame(&indices);
        assert_eq!(rgb, vec![0, 0, 0, 128, 128, 128, 255, 255, 255]);
    }

    #[test]
    fn palette_clamps_out_of_range() {
        let lut = make_lut();
        // index 300 → clamped to 255
        let (r, g, b) = lut.lookup(300);
        assert_eq!((r, g, b), (255, 255, 255));
    }
}
