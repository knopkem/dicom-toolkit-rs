//! DICOM overlay plane handling.
//!
//! Ports DCMTK's overlay extraction from `dcmimgle`.  Overlays are stored as
//! 1-bit packed planes in tags `(60xx,3000)` for groups 0x6000–0x601E (even).

use dicom_toolkit_data::{DataSet, Value};
use dicom_toolkit_dict::Tag;

// ── Overlay ───────────────────────────────────────────────────────────────────

/// A single DICOM overlay plane.
///
/// Pixel data is stored as packed 1-bit values (LSB of first byte = pixel 0).
#[derive(Debug, Clone)]
pub struct Overlay {
    /// Overlay group number (0x6000, 0x6002, …, 0x601E).
    pub group: u16,
    /// Number of rows in the overlay.
    pub rows: u16,
    /// Number of columns in the overlay.
    pub columns: u16,
    /// Row and column origin offset relative to the image (1-based per DICOM).
    pub origin: (i16, i16),
    /// Packed 1-bit pixel data (bit 0 of byte 0 = pixel at row 0, col 0).
    pub data: Vec<u8>,
}

impl Overlay {
    /// Extract all overlays present in a `DataSet`.
    ///
    /// Scans for tags whose group is in `0x6000..=0x601E` (even) and element
    /// is `0x3000` (OverlayData).  For each discovered group the corresponding
    /// rows, columns, and origin tags are read.
    pub fn from_dataset(dataset: &DataSet) -> Vec<Self> {
        // Collect unique overlay group numbers.
        let mut groups: Vec<u16> = dataset
            .iter()
            .filter_map(|(tag, _)| {
                let g = tag.group;
                if g >= 0x6000 && g <= 0x601E && g % 2 == 0 {
                    Some(g)
                } else {
                    None
                }
            })
            .collect();
        groups.sort_unstable();
        groups.dedup();

        groups
            .into_iter()
            .filter_map(|group| Self::from_group(dataset, group))
            .collect()
    }

    /// Attempt to build an `Overlay` from a specific group within a `DataSet`.
    fn from_group(dataset: &DataSet, group: u16) -> Option<Self> {
        let rows    = dataset.get_u16(Tag::new(group, 0x0010))?;
        let columns = dataset.get_u16(Tag::new(group, 0x0011))?;

        let origin = read_origin(dataset, group);

        let data_tag = Tag::new(group, 0x3000);
        let data = dataset.get_bytes(data_tag)?.to_vec();

        Some(Self { group, rows, columns, origin, data })
    }

    /// Return the value of the overlay pixel at (`row`, `col`) (0-indexed).
    ///
    /// Returns `false` for coordinates outside the overlay bounds or when the
    /// backing data is too short.
    pub fn get_pixel(&self, row: u16, col: u16) -> bool {
        if row >= self.rows || col >= self.columns {
            return false;
        }
        let bit_index = row as usize * self.columns as usize + col as usize;
        let byte_index = bit_index / 8;
        let bit_pos    = bit_index % 8;
        self.data
            .get(byte_index)
            .map(|&b| (b >> bit_pos) & 1 != 0)
            .unwrap_or(false)
    }

    /// Unpack the overlay into a 1-byte-per-pixel bitmap (0 = off, 1 = on).
    ///
    /// The returned vector has `rows × columns` elements.
    pub fn to_bitmap(&self) -> Vec<u8> {
        let n = self.rows as usize * self.columns as usize;
        (0..n)
            .map(|i| {
                self.data
                    .get(i / 8)
                    .map(|&b| (b >> (i % 8)) & 1)
                    .unwrap_or(0)
            })
            .collect()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn read_origin(dataset: &DataSet, group: u16) -> (i16, i16) {
    let tag = Tag::new(group, 0x0050);
    dataset
        .get(tag)
        .and_then(|elem| match &elem.value {
            Value::I16(vals) if vals.len() >= 2 => Some((vals[0], vals[1])),
            Value::U16(vals) if vals.len() >= 2 => Some((vals[0] as i16, vals[1] as i16)),
            _ => None,
        })
        .unwrap_or((1, 1))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_overlay(data: Vec<u8>, rows: u16, cols: u16) -> Overlay {
        Overlay {
            group: 0x6000,
            rows,
            columns: cols,
            origin: (1, 1),
            data,
        }
    }

    #[test]
    fn overlay_get_pixel() {
        // 8×8 overlay.  First byte = 0x01 → only bit 0 is set → pixel (0,0).
        let mut data = vec![0u8; 8];
        data[0] = 0b0000_0001;
        let overlay = make_overlay(data, 8, 8);

        assert!(overlay.get_pixel(0, 0), "pixel (0,0) should be set");
        assert!(!overlay.get_pixel(0, 1), "pixel (0,1) should be clear");
        assert!(!overlay.get_pixel(1, 0), "pixel (1,0) should be clear");
    }

    #[test]
    fn overlay_get_pixel_bit7() {
        // Pixel (0,7): bit index 7 → byte 0, bit 7.
        let mut data = vec![0u8; 8];
        data[0] = 0b1000_0000;
        let overlay = make_overlay(data, 8, 8);

        assert!(overlay.get_pixel(0, 7), "pixel (0,7) should be set");
        assert!(!overlay.get_pixel(0, 0), "pixel (0,0) should be clear");
    }

    #[test]
    fn overlay_to_bitmap() {
        let mut data = vec![0u8; 2]; // 4×4 = 16 pixels
        data[0] = 0b0000_0001; // pixel 0 set
        data[1] = 0b0000_0001; // pixel 8 set
        let overlay = make_overlay(data, 4, 4);
        let bm = overlay.to_bitmap();
        assert_eq!(bm.len(), 16);
        assert_eq!(bm[0], 1);  // pixel (0,0)
        assert_eq!(bm[1], 0);  // pixel (0,1)
        assert_eq!(bm[8], 1);  // pixel (2,0)
    }

    #[test]
    fn overlay_out_of_bounds() {
        let overlay = make_overlay(vec![0xFF; 8], 8, 8);
        // Accessing beyond bounds returns false.
        assert!(!overlay.get_pixel(8, 0));
        assert!(!overlay.get_pixel(0, 8));
    }
}
