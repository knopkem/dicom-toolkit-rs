//! Export DICOM frames as PNG images or raw byte vectors.
//!
//! Uses the `png` crate for PNG encoding.

use std::path::Path;

use dicom_toolkit_core::error::{DcmError, DcmResult};

use crate::dicom_image::DicomImage;

// ── Public API ────────────────────────────────────────────────────────────────

/// Encode frame `frame` of `image` as a PNG file and write it to `path`.
///
/// Grayscale images produce an 8-bit grayscale PNG; color (RGB) images
/// produce a 24-bit RGB PNG.
pub fn export_frame_png(
    image: &DicomImage,
    frame: u32,
    path: impl AsRef<Path>,
) -> DcmResult<()> {
    let bytes = frame_to_png_bytes(image, frame)?;
    std::fs::write(path, bytes).map_err(DcmError::Io)
}

/// Encode frame `frame` of `image` as a PNG and return the raw bytes.
///
/// Equivalent to [`export_frame_png`] but returns the PNG in memory rather
/// than writing it to disk.
pub fn frame_to_png_bytes(image: &DicomImage, frame: u32) -> DcmResult<Vec<u8>> {
    let pixels = image.frame_u8(frame)?;

    let color_type = match image.output_channels() {
        1 => png::ColorType::Grayscale,
        3 => png::ColorType::Rgb,
        4 => png::ColorType::Rgba,
        n => {
            return Err(DcmError::Other(format!(
                "unsupported output channel count {n} for PNG export"
            )))
        }
    };

    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, image.columns, image.rows);
        encoder.set_color(color_type);
        encoder.set_depth(png::BitDepth::Eight);

        let mut writer = encoder
            .write_header()
            .map_err(|e| DcmError::Other(e.to_string()))?;

        writer
            .write_image_data(&pixels)
            .map_err(|e| DcmError::Other(e.to_string()))?;
    }

    Ok(buf)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_data::{DataSet, Element, PixelData, Value};
    use dicom_toolkit_dict::{tags, Vr};

    fn tiny_grayscale() -> DicomImage {
        let mut ds = DataSet::new();
        ds.set_u16(tags::ROWS, 2);
        ds.set_u16(tags::COLUMNS, 2);
        ds.set_u16(tags::SAMPLES_PER_PIXEL, 1);
        ds.set_u16(tags::BITS_ALLOCATED, 8);
        ds.set_u16(tags::BITS_STORED, 8);
        ds.set_u16(tags::HIGH_BIT, 7);
        ds.set_u16(tags::PIXEL_REPRESENTATION, 0);
        ds.set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::CS, "MONOCHROME2");
        ds.insert(Element::new(
            tags::PIXEL_DATA,
            Vr::OB,
            Value::PixelData(PixelData::Native {
                bytes: vec![0, 64, 128, 255],
            }),
        ));
        DicomImage::from_dataset(&ds).unwrap()
    }

    #[test]
    fn frame_to_png_bytes_produces_valid_png() {
        let image = tiny_grayscale();
        let bytes = frame_to_png_bytes(&image, 0).unwrap();
        // PNG magic bytes: 0x89 P N G \r \n 0x1A \n
        assert!(bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]));
    }

    #[test]
    fn export_frame_png_writes_file() {
        let dir  = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.png");
        let image = tiny_grayscale();
        export_frame_png(&image, 0, &path).unwrap();
        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
    }
}
