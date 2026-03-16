//! High-level rendered frame helpers for DICOMweb-style output options.

use dicom_toolkit_core::error::{DcmError, DcmResult};

use crate::dicom_image::DicomImage;
use crate::transform::scale_bilinear;

/// A normalized crop region within a rendered frame.
///
/// Coordinates are expressed relative to the full rendered frame, where `0.0`
/// is the first row/column edge and `1.0` is the far edge. The region is
/// defined by its top-left corner plus width/height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderedRegion {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

/// Rendering options for producing an 8-bit display-ready frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RenderedFrameOptions {
    pub frame: u32,
    pub window_center: Option<f64>,
    pub window_width: Option<f64>,
    pub rows: Option<u32>,
    pub columns: Option<u32>,
    pub region: Option<RenderedRegion>,
    pub burn_in_overlays: bool,
}

/// Render a frame with optional window override, crop, and resize operations.
pub fn render_frame_u8(image: &DicomImage, options: &RenderedFrameOptions) -> DcmResult<Vec<u8>> {
    if options.burn_in_overlays {
        return Err(DcmError::Other(
            "burn_in_overlays is not supported by render_frame_u8 because DicomImage does not retain overlay planes"
                .into(),
        ));
    }

    let mut image = image.clone();
    match (options.window_center, options.window_width) {
        (Some(center), Some(width)) => image.set_window(center, width),
        (None, None) => {}
        _ => {
            return Err(DcmError::Other(
                "window_center and window_width must be provided together".into(),
            ))
        }
    }

    let mut pixels = image.frame_u8(options.frame)?;
    let channels = image.output_channels();
    let mut rows = image.rows;
    let mut columns = image.columns;

    if let Some(region) = options.region {
        let (cropped, cropped_rows, cropped_columns) =
            crop_region(&pixels, rows, columns, channels, region)?;
        pixels = cropped;
        rows = cropped_rows;
        columns = cropped_columns;
    }

    let (target_rows, target_columns) =
        target_dimensions(rows, columns, options.rows, options.columns)?;
    if target_rows != rows || target_columns != columns {
        pixels = scale_bilinear(
            &pixels,
            rows,
            columns,
            channels,
            target_rows,
            target_columns,
        );
    }

    Ok(pixels)
}

fn crop_region(
    pixels: &[u8],
    rows: u32,
    columns: u32,
    channels: u8,
    region: RenderedRegion,
) -> DcmResult<(Vec<u8>, u32, u32)> {
    validate_region(region)?;

    let start_row = (region.top * rows as f64).floor() as u32;
    let end_row = ((region.top + region.height) * rows as f64).ceil() as u32;
    let start_col = (region.left * columns as f64).floor() as u32;
    let end_col = ((region.left + region.width) * columns as f64).ceil() as u32;

    let cropped_rows = end_row.saturating_sub(start_row);
    let cropped_columns = end_col.saturating_sub(start_col);
    if cropped_rows == 0 || cropped_columns == 0 {
        return Err(DcmError::Other(
            "rendered crop region resolved to an empty image".into(),
        ));
    }

    let ch = channels as usize;
    let mut out = vec![0u8; cropped_rows as usize * cropped_columns as usize * ch];
    for row in 0..cropped_rows as usize {
        let src_start = (((start_row as usize + row) * columns as usize) + start_col as usize) * ch;
        let src_end = src_start + cropped_columns as usize * ch;
        let dst_start = row * cropped_columns as usize * ch;
        let dst_end = dst_start + cropped_columns as usize * ch;
        out[dst_start..dst_end].copy_from_slice(&pixels[src_start..src_end]);
    }

    Ok((out, cropped_rows, cropped_columns))
}

fn validate_region(region: RenderedRegion) -> DcmResult<()> {
    let values = [region.left, region.top, region.width, region.height];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(DcmError::Other(
            "rendered region values must be finite".into(),
        ));
    }
    if region.left < 0.0
        || region.top < 0.0
        || region.width <= 0.0
        || region.height <= 0.0
        || region.left + region.width > 1.0
        || region.top + region.height > 1.0
    {
        return Err(DcmError::Other(
            "rendered region must stay within [0.0, 1.0] and have positive width/height".into(),
        ));
    }
    Ok(())
}

fn target_dimensions(
    rows: u32,
    columns: u32,
    target_rows: Option<u32>,
    target_columns: Option<u32>,
) -> DcmResult<(u32, u32)> {
    let original_rows = rows;
    let original_columns = columns;
    let (rows, columns) = match (target_rows, target_columns) {
        (Some(rows), Some(columns)) => (rows, columns),
        (Some(rows), None) => (
            rows,
            scale_preserving_aspect(original_columns, rows, original_rows)?,
        ),
        (None, Some(columns)) => (
            scale_preserving_aspect(original_rows, columns, original_columns)?,
            columns,
        ),
        (None, None) => (rows, columns),
    };

    if rows == 0 || columns == 0 {
        return Err(DcmError::Other(
            "rendered output dimensions must be greater than zero".into(),
        ));
    }

    Ok((rows, columns))
}

fn scale_preserving_aspect(numerator: u32, scaled_by: u32, divisor: u32) -> DcmResult<u32> {
    let scaled =
        (u64::from(numerator) * u64::from(scaled_by) + u64::from(divisor) / 2) / u64::from(divisor);
    u32::try_from(scaled)
        .map_err(|_| DcmError::Other("scaled rendered dimension exceeds u32 range".into()))
}

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
    fn render_frame_u8_applies_window_override() {
        let image = tiny_grayscale();
        let options = RenderedFrameOptions {
            frame: 0,
            window_center: Some(128.0),
            window_width: Some(64.0),
            ..Default::default()
        };

        let rendered = render_frame_u8(&image, &options).unwrap();

        let mut expected_image = image.clone();
        expected_image.set_window(128.0, 64.0);
        let expected = expected_image.frame_u8(0).unwrap();
        assert_eq!(rendered, expected);
    }

    #[test]
    fn render_frame_u8_crops_normalized_region() {
        let image = tiny_grayscale();
        let options = RenderedFrameOptions {
            frame: 0,
            region: Some(RenderedRegion {
                left: 0.5,
                top: 0.5,
                width: 0.5,
                height: 0.5,
            }),
            ..Default::default()
        };

        let rendered = render_frame_u8(&image, &options).unwrap();
        assert_eq!(rendered, vec![255]);
    }

    #[test]
    fn render_frame_u8_resizes_deterministically() {
        let image = tiny_grayscale();
        let options = RenderedFrameOptions {
            frame: 0,
            rows: Some(4),
            columns: Some(4),
            ..Default::default()
        };

        let first = render_frame_u8(&image, &options).unwrap();
        let second = render_frame_u8(&image, &options).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 16);
    }

    #[test]
    fn render_frame_u8_rejects_overlay_burn_in() {
        let image = tiny_grayscale();
        let options = RenderedFrameOptions {
            burn_in_overlays: true,
            ..Default::default()
        };

        let err = render_frame_u8(&image, &options).unwrap_err();
        assert!(err.to_string().contains("burn_in_overlays"));
    }
}
