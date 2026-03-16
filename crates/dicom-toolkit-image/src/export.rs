//! Export DICOM frames as PNG or JPEG images, either in memory or on disk.
//!
//! Uses the `png` crate for PNG encoding and `jpeg-encoder` for JPEG encoding.

use std::path::Path;

use dicom_toolkit_core::error::{DcmError, DcmResult};
use jpeg_encoder::{ColorType as JpegColorType, Encoder as JpegEncoder};

use crate::dicom_image::DicomImage;

// ── Public API ────────────────────────────────────────────────────────────────

/// Encode frame `frame` of `image` as a PNG file and write it to `path`.
///
/// Grayscale images produce an 8-bit grayscale PNG; color (RGB) images
/// produce a 24-bit RGB PNG.
pub fn export_frame_png(image: &DicomImage, frame: u32, path: impl AsRef<Path>) -> DcmResult<()> {
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

/// Encode frame `frame` of `image` as a JPEG file and write it to `path`.
///
/// JPEG export currently supports grayscale and RGB output only. `quality` must
/// be in the range `1..=100`.
pub fn export_frame_jpeg(
    image: &DicomImage,
    frame: u32,
    quality: u8,
    path: impl AsRef<Path>,
) -> DcmResult<()> {
    let bytes = frame_to_jpeg_bytes(image, frame, quality)?;
    std::fs::write(path, bytes).map_err(DcmError::Io)
}

/// Encode frame `frame` of `image` as a JPEG and return the raw bytes.
///
/// JPEG export currently supports grayscale and RGB output only. `quality` must
/// be in the range `1..=100`.
pub fn frame_to_jpeg_bytes(image: &DicomImage, frame: u32, quality: u8) -> DcmResult<Vec<u8>> {
    if !(1..=100).contains(&quality) {
        return Err(DcmError::Other(format!(
            "JPEG quality must be in the range 1..=100, got {quality}"
        )));
    }

    let pixels = image.frame_u8(frame)?;
    let color_type = match image.output_channels() {
        1 => JpegColorType::Luma,
        3 => JpegColorType::Rgb,
        n => {
            return Err(DcmError::Other(format!(
                "unsupported output channel count {n} for JPEG export"
            )))
        }
    };

    let mut buf = Vec::new();
    let encoder = JpegEncoder::new(&mut buf, quality);
    encoder
        .encode(&pixels, image.columns as u16, image.rows as u16, color_type)
        .map_err(|e| DcmError::Other(e.to_string()))?;
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

    fn tiny_rgb() -> DicomImage {
        let mut ds = DataSet::new();
        ds.set_u16(tags::ROWS, 1);
        ds.set_u16(tags::COLUMNS, 2);
        ds.set_u16(tags::SAMPLES_PER_PIXEL, 3);
        ds.set_u16(tags::BITS_ALLOCATED, 8);
        ds.set_u16(tags::BITS_STORED, 8);
        ds.set_u16(tags::HIGH_BIT, 7);
        ds.set_u16(tags::PIXEL_REPRESENTATION, 0);
        ds.set_u16(tags::PLANAR_CONFIGURATION, 0);
        ds.set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::CS, "RGB");
        ds.insert(Element::new(
            tags::PIXEL_DATA,
            Vr::OB,
            Value::PixelData(PixelData::Native {
                bytes: vec![255, 0, 0, 0, 255, 0],
            }),
        ));
        DicomImage::from_dataset(&ds).unwrap()
    }

    fn jpeg_component_count(bytes: &[u8]) -> u8 {
        assert!(bytes.starts_with(&[0xFF, 0xD8]), "expected JPEG SOI");
        let mut pos = 2usize;
        while pos + 4 <= bytes.len() {
            if bytes[pos] != 0xFF {
                pos += 1;
                continue;
            }

            let marker = bytes[pos + 1];
            pos += 2;

            if matches!(marker, 0xD8 | 0xD9) {
                continue;
            }
            if marker == 0xDA {
                break;
            }
            if pos + 2 > bytes.len() {
                break;
            }

            let segment_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
            if segment_len < 2 || pos + segment_len > bytes.len() {
                break;
            }

            if matches!(
                marker,
                0xC0 | 0xC1
                    | 0xC2
                    | 0xC3
                    | 0xC5
                    | 0xC6
                    | 0xC7
                    | 0xC9
                    | 0xCA
                    | 0xCB
                    | 0xCD
                    | 0xCE
                    | 0xCF
            ) {
                return bytes[pos + 7];
            }

            pos += segment_len;
        }

        panic!("JPEG SOF marker not found");
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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.png");
        let image = tiny_grayscale();
        export_frame_png(&image, 0, &path).unwrap();
        assert!(path.exists());
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0);
    }

    #[test]
    fn frame_to_jpeg_bytes_produces_grayscale_jpeg() {
        let image = tiny_grayscale();
        let bytes = frame_to_jpeg_bytes(&image, 0, 90).unwrap();
        assert!(bytes.starts_with(&[0xFF, 0xD8]));
        assert_eq!(jpeg_component_count(&bytes), 1);
        assert_eq!(bytes, frame_to_jpeg_bytes(&image, 0, 90).unwrap());
    }

    #[test]
    fn frame_to_jpeg_bytes_produces_rgb_jpeg() {
        let image = tiny_rgb();
        let bytes = frame_to_jpeg_bytes(&image, 0, 90).unwrap();
        assert!(bytes.starts_with(&[0xFF, 0xD8]));
        assert_eq!(jpeg_component_count(&bytes), 3);
    }

    #[test]
    fn frame_to_jpeg_bytes_rejects_invalid_quality() {
        let image = tiny_grayscale();
        assert!(frame_to_jpeg_bytes(&image, 0, 0).is_err());
    }

    #[test]
    fn frame_to_jpeg_bytes_rejects_out_of_range_frame() {
        let image = tiny_grayscale();
        assert!(frame_to_jpeg_bytes(&image, 1, 90).is_err());
    }

    #[test]
    fn export_frame_jpeg_writes_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jpg");
        let image = tiny_grayscale();
        export_frame_jpeg(&image, 0, 90, &path).unwrap();
        assert!(path.exists());
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
    }
}
