//! Core DICOM image abstraction.
//!
//! Ports DCMTK's `DicomImage` class from `dcmimgle`.  Reads image geometry
//! and pixel attributes from a `DataSet`, owns the raw pixel bytes, and
//! exposes frame-level access with windowing and color-model handling.

use std::sync::Arc;

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_data::{DataSet, PixelData, Value};
use dicom_toolkit_dict::tags;

use crate::color::{PaletteColorLut, PhotometricInterpretation};
use crate::lut::ModalityLut;
use crate::window::WindowLevel;
use crate::{color, pixel};

// ── Supporting types ──────────────────────────────────────────────────────────

/// Pixel sign convention (tag `(0028,0103)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelRepresentation {
    /// Unsigned integer pixels (value 0).
    Unsigned,
    /// 2's-complement signed integer pixels (value 1).
    Signed,
}

// ── DicomImage ────────────────────────────────────────────────────────────────

/// A decoded DICOM image with access to frame pixels, windowing, and metadata.
///
/// Create from a `DataSet` that already contains uncompressed pixel data via
/// [`DicomImage::from_dataset`].  Compressed (encapsulated) data must first be
/// decompressed with `dcmtk-codec`.
#[derive(Debug, Clone)]
pub struct DicomImage {
    /// Number of pixel rows per frame.
    pub rows: u32,
    /// Number of pixel columns per frame.
    pub columns: u32,
    /// Number of frames (1 for a single image).
    pub frames: u32,
    /// Samples per pixel: 1 for grayscale, 3 for color.
    pub samples_per_pixel: u16,
    /// Bits of storage allocated per sample (8 or 16).
    pub bits_allocated: u16,
    /// Bits actually used per sample (≤ `bits_allocated`).
    pub bits_stored: u16,
    /// Bit index (0-based from LSB) of the most significant stored bit.
    pub high_bit: u16,
    /// Sign convention for pixel values.
    pub pixel_representation: PixelRepresentation,
    /// How pixel values map to display intensities.
    pub photometric: PhotometricInterpretation,
    /// `0` = pixel-interleaved, `1` = plane-interleaved (color only).
    pub planar_config: u16,
    /// Optional display window center (`(0028,1050)` WindowCenter).
    pub window_center: Option<f64>,
    /// Optional display window width (`(0028,1051)` WindowWidth).
    pub window_width: Option<f64>,
    /// Modality LUT intercept (`(0028,1052)` RescaleIntercept).
    pub rescale_intercept: f64,
    /// Modality LUT slope (`(0028,1053)` RescaleSlope).
    pub rescale_slope: f64,

    pixel_data: Arc<Vec<u8>>,
    palette_lut: Option<PaletteColorLut>,
}

impl DicomImage {
    // ── Constructor ───────────────────────────────────────────────────────────

    /// Build a `DicomImage` from a `DataSet`.
    ///
    /// Requires:
    /// - Standard image pixel module attributes (`(0028,xxxx)`).
    /// - Uncompressed pixel data at `(7FE0,0010)`.
    ///
    /// Returns `Err` if any mandatory attribute is missing or if the pixel data
    /// is encapsulated (compressed).
    pub fn from_dataset(dataset: &DataSet) -> DcmResult<Self> {
        // ── Mandatory geometry attributes ─────────────────────────────────────
        let rows = dataset
            .get_u16(tags::ROWS)
            .ok_or_else(|| missing("Rows (0028,0010)"))? as u32;
        let columns = dataset
            .get_u16(tags::COLUMNS)
            .ok_or_else(|| missing("Columns (0028,0011)"))? as u32;

        // ── Pixel attributes ──────────────────────────────────────────────────
        let samples_per_pixel = dataset
            .get_u16(tags::SAMPLES_PER_PIXEL)
            .unwrap_or(1);
        let bits_allocated = dataset
            .get_u16(tags::BITS_ALLOCATED)
            .ok_or_else(|| missing("BitsAllocated (0028,0100)"))?;
        let bits_stored = dataset
            .get_u16(tags::BITS_STORED)
            .unwrap_or(bits_allocated);
        let high_bit = dataset
            .get_u16(tags::HIGH_BIT)
            .unwrap_or(bits_stored - 1);
        let pixel_representation = match dataset.get_u16(tags::PIXEL_REPRESENTATION).unwrap_or(0) {
            1 => PixelRepresentation::Signed,
            _ => PixelRepresentation::Unsigned,
        };
        let planar_config = dataset.get_u16(tags::PLANAR_CONFIGURATION).unwrap_or(0);
        let frames = get_number_of_frames(dataset);

        // ── Photometric interpretation ────────────────────────────────────────
        let photometric = dataset
            .get_string(tags::PHOTOMETRIC_INTERPRETATION)
            .map(PhotometricInterpretation::from_str)
            .unwrap_or(PhotometricInterpretation::Monochrome2);

        // ── Window / LUT ──────────────────────────────────────────────────────
        let window_center = get_decimal(dataset, tags::WINDOW_CENTER);
        let window_width  = get_decimal(dataset, tags::WINDOW_WIDTH);
        let rescale_intercept = get_decimal(dataset, tags::RESCALE_INTERCEPT).unwrap_or(0.0);
        let rescale_slope     = get_decimal(dataset, tags::RESCALE_SLOPE).unwrap_or(1.0);

        // ── Pixel data ────────────────────────────────────────────────────────
        let pixel_bytes = extract_pixel_bytes(dataset)?;
        let pixel_data  = Arc::new(pixel_bytes);

        // ── Optional palette LUT ──────────────────────────────────────────────
        let palette_lut = if photometric == PhotometricInterpretation::PaletteColor {
            PaletteColorLut::from_dataset(dataset).ok()
        } else {
            None
        };

        Ok(Self {
            rows,
            columns,
            frames,
            samples_per_pixel,
            bits_allocated,
            bits_stored,
            high_bit,
            pixel_representation,
            photometric,
            planar_config,
            window_center,
            window_width,
            rescale_intercept,
            rescale_slope,
            pixel_data,
            palette_lut,
        })
    }

    // ── Geometry helpers ──────────────────────────────────────────────────────

    /// Number of bytes per sample (1 or 2).
    pub fn bytes_per_sample(&self) -> usize {
        ((self.bits_allocated as usize) + 7) / 8
    }

    /// Total number of pixels per frame (rows × columns × samples_per_pixel).
    pub fn pixels_per_frame(&self) -> usize {
        self.rows as usize * self.columns as usize * self.samples_per_pixel as usize
    }

    /// Number of raw bytes per frame.
    pub fn bytes_per_frame(&self) -> usize {
        self.pixels_per_frame() * self.bytes_per_sample()
    }

    /// Number of output channels produced by [`frame_u8`] / [`frame_normalized`].
    ///
    /// Equal to `samples_per_pixel` for most images, but **3** for
    /// `PALETTE COLOR` (which expands single-channel indices to RGB).
    pub fn output_channels(&self) -> u8 {
        if self.photometric == PhotometricInterpretation::PaletteColor {
            3
        } else {
            self.samples_per_pixel as u8
        }
    }

    // ── Raw frame access ──────────────────────────────────────────────────────

    /// Return the raw encoded pixel bytes for `frame` (0-based).
    ///
    /// The bytes are in the native storage format (e.g. little-endian 16-bit
    /// words for 16-bit images).
    pub fn frame_bytes(&self, frame: u32) -> DcmResult<&[u8]> {
        if frame >= self.frames {
            return Err(DcmError::Other(format!(
                "frame {frame} out of range [0, {})",
                self.frames
            )));
        }
        let bpf   = self.bytes_per_frame();
        let start = frame as usize * bpf;
        let end   = start + bpf;
        if end > self.pixel_data.len() {
            return Err(DcmError::Other(format!(
                "pixel data too short: need {end} bytes, have {}",
                self.pixel_data.len()
            )));
        }
        Ok(&self.pixel_data[start..end])
    }

    // ── Processed frame access ────────────────────────────────────────────────

    /// Return frame `frame` as 8-bit pixels (0–255) ready for display.
    ///
    /// - **Grayscale**: applies the modality LUT then the VOI LUT (window/level),
    ///   inverting for MONOCHROME1.
    /// - **RGB / YBR**: converts to pixel-interleaved RGB.
    /// - **Palette color**: expands indices to RGB via the palette LUT.
    pub fn frame_u8(&self, frame: u32) -> DcmResult<Vec<u8>> {
        match &self.photometric {
            PhotometricInterpretation::Monochrome1
            | PhotometricInterpretation::Monochrome2
            | PhotometricInterpretation::Unknown(_) => self.frame_u8_grayscale(frame),

            PhotometricInterpretation::Rgb
            | PhotometricInterpretation::YbrFull
            | PhotometricInterpretation::YbrFull422
            | PhotometricInterpretation::PaletteColor => self.frame_u8_color(frame),
        }
    }

    /// Return frame `frame` as normalized `f32` values in `[0.0, 1.0]`.
    ///
    /// Applies the same pipeline as [`frame_u8`] and then divides by 255.
    pub fn frame_normalized(&self, frame: u32) -> DcmResult<Vec<f32>> {
        let u8_pixels = self.frame_u8(frame)?;
        Ok(u8_pixels.iter().map(|&v| v as f32 / 255.0).collect())
    }

    // ── Window / level control ────────────────────────────────────────────────

    /// Set the display window center and width.
    pub fn set_window(&mut self, center: f64, width: f64) {
        self.window_center = Some(center);
        self.window_width  = Some(width);
    }

    /// Automatically compute a window from the pixel value range across **all**
    /// frames.
    ///
    /// Sets `window_center = (min + max) / 2` and `window_width = max − min`.
    pub fn auto_window(&mut self) {
        let Some((min_val, max_val)) = self.pixel_minmax() else { return };
        if (max_val - min_val).abs() < f64::EPSILON {
            return;
        }
        self.window_center = Some((min_val + max_val) / 2.0);
        self.window_width  = Some(max_val - min_val);
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn frame_u8_grayscale(&self, frame: u32) -> DcmResult<Vec<u8>> {
        let raw = self.frame_bytes(frame)?;
        let modality = ModalityLut::new(self.rescale_intercept, self.rescale_slope);

        let values: Vec<f64> = match (self.bits_allocated, self.pixel_representation) {
            (8, _) => modality.apply_to_frame_u8(raw),

            (16, PixelRepresentation::Unsigned) => {
                let px = pixel::decode_u16_le(raw);
                let px = pixel::mask_u16(&px, self.bits_stored, self.high_bit);
                modality.apply_to_frame_u16(&px)
            }

            (16, PixelRepresentation::Signed) => {
                let px = pixel::decode_i16_le(raw);
                let px = pixel::mask_i16(&px, self.bits_stored, self.high_bit);
                modality.apply_to_frame_i16(&px)
            }

            _ => {
                return Err(DcmError::Other(format!(
                    "unsupported BitsAllocated={} for grayscale rendering",
                    self.bits_allocated
                )))
            }
        };

        // Determine window center / width, falling back to auto-window.
        let (center, width) = match (self.window_center, self.window_width) {
            (Some(c), Some(w)) => (c, w),
            _ => {
                let (lo, hi) = values.iter().fold(
                    (f64::INFINITY, f64::NEG_INFINITY),
                    |(mn, mx), &v| (mn.min(v), mx.max(v)),
                );
                if lo >= hi {
                    // Constant image: place window so all pixels land at output_min.
                    (lo + 0.5, 1.0)
                } else {
                    ((lo + hi) / 2.0, (hi - lo).max(1.0))
                }
            }
        };

        let wl = WindowLevel::new(center, width);
        let mut result = wl.apply_to_frame(&values);

        if self.photometric == PhotometricInterpretation::Monochrome1 {
            result.iter_mut().for_each(|v| *v = 255 - *v);
        }

        Ok(result)
    }

    fn frame_u8_color(&self, frame: u32) -> DcmResult<Vec<u8>> {
        let raw = self.frame_bytes(frame)?;
        let n   = self.rows as usize * self.columns as usize;

        match &self.photometric {
            PhotometricInterpretation::Rgb => {
                if self.planar_config == 1 {
                    Ok(color::rgb::planar_to_pixel(raw, n))
                } else {
                    Ok(raw.to_vec())
                }
            }

            PhotometricInterpretation::YbrFull => {
                let pixel_data = if self.planar_config == 1 {
                    color::rgb::planar_to_pixel(raw, n)
                } else {
                    raw.to_vec()
                };
                Ok(color::ycbcr::ybr_full_to_rgb(&pixel_data))
            }

            PhotometricInterpretation::YbrFull422 => {
                color::ycbcr::ybr_full_422_to_rgb(
                    raw,
                    self.columns as usize,
                    self.rows as usize,
                )
            }

            PhotometricInterpretation::PaletteColor => {
                let lut = self.palette_lut.as_ref().ok_or_else(|| {
                    DcmError::Other(
                        "PALETTE COLOR image has no palette LUT (not loaded from dataset)".into(),
                    )
                })?;
                let indices: Vec<u16> = match self.bits_allocated {
                    8  => raw.iter().map(|&v| v as u16).collect(),
                    16 => pixel::decode_u16_le(raw),
                    b  => {
                        return Err(DcmError::Other(format!(
                            "unsupported BitsAllocated={b} for PALETTE COLOR"
                        )))
                    }
                };
                Ok(lut.apply_to_frame(&indices))
            }

            // Grayscale cases handled by frame_u8_grayscale; won't reach here.
            _ => Err(DcmError::Other(format!(
                "unexpected photometric in color path: {:?}",
                self.photometric
            ))),
        }
    }

    /// Compute min/max over **all** frames after the modality LUT.
    fn pixel_minmax(&self) -> Option<(f64, f64)> {
        let modality = ModalityLut::new(self.rescale_intercept, self.rescale_slope);
        let mut global_min = f64::MAX;
        let mut global_max = f64::MIN;

        for f in 0..self.frames {
            let Ok(raw) = self.frame_bytes(f) else { continue };
            let values: Vec<f64> = match (self.bits_allocated, self.pixel_representation) {
                (8, _) => modality.apply_to_frame_u8(raw),
                (16, PixelRepresentation::Unsigned) => {
                    let px = pixel::decode_u16_le(raw);
                    let px = pixel::mask_u16(&px, self.bits_stored, self.high_bit);
                    modality.apply_to_frame_u16(&px)
                }
                (16, PixelRepresentation::Signed) => {
                    let px = pixel::decode_i16_le(raw);
                    let px = pixel::mask_i16(&px, self.bits_stored, self.high_bit);
                    modality.apply_to_frame_i16(&px)
                }
                _ => continue,
            };
            for &v in &values {
                global_min = global_min.min(v);
                global_max = global_max.max(v);
            }
        }

        if global_min <= global_max {
            Some((global_min, global_max))
        } else {
            None
        }
    }
}

// ── Free helpers ──────────────────────────────────────────────────────────────

fn missing(name: &str) -> DcmError {
    DcmError::Other(format!("missing mandatory attribute: {name}"))
}

/// Extract `NumberOfFrames` (IS VR) from the dataset, defaulting to 1.
fn get_number_of_frames(dataset: &DataSet) -> u32 {
    dataset
        .get(tags::NUMBER_OF_FRAMES)
        .and_then(|elem| match &elem.value {
            Value::Ints(v)    => v.first().copied().map(|n| n.max(1) as u32),
            Value::Strings(v) => v.first().and_then(|s| s.trim().parse::<u32>().ok()),
            Value::U16(v)     => v.first().copied().map(|n| n as u32),
            Value::U32(v)     => v.first().copied(),
            _                 => None,
        })
        .unwrap_or(1)
}

/// Extract a DS / FD / F32 decimal value from the dataset.
fn get_decimal(dataset: &DataSet, tag: dicom_toolkit_dict::Tag) -> Option<f64> {
    let elem = dataset.get(tag)?;
    match &elem.value {
        Value::Decimals(v) => v.first().copied(),
        Value::F64(v)      => v.first().copied(),
        Value::F32(v)      => v.first().map(|&n| n as f64),
        Value::Strings(v)  => v.first().and_then(|s| s.trim().parse::<f64>().ok()),
        _                  => None,
    }
}

/// Extract native (uncompressed) pixel bytes from a dataset.
fn extract_pixel_bytes(dataset: &DataSet) -> DcmResult<Vec<u8>> {
    let elem = dataset.find_element(tags::PIXEL_DATA)?;
    match &elem.value {
        Value::PixelData(PixelData::Native { bytes }) => Ok(bytes.clone()),
        Value::U8(bytes) => Ok(bytes.clone()),
        Value::PixelData(PixelData::Encapsulated { .. }) => Err(DcmError::Other(
            "encapsulated (compressed) pixel data is not supported; \
             decompress with dcmtk-codec first"
                .into(),
        )),
        _ => Err(DcmError::Other(
            "PixelData (7FE0,0010) has an unexpected value type".into(),
        )),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_data::{DataSet, Value};
    use dicom_toolkit_dict::{tags, Vr};

    fn make_grayscale_8bit(rows: u16, cols: u16, pixels: Vec<u8>) -> DataSet {
        let mut ds = DataSet::new();
        ds.set_u16(tags::ROWS, rows);
        ds.set_u16(tags::COLUMNS, cols);
        ds.set_u16(tags::SAMPLES_PER_PIXEL, 1);
        ds.set_u16(tags::BITS_ALLOCATED, 8);
        ds.set_u16(tags::BITS_STORED, 8);
        ds.set_u16(tags::HIGH_BIT, 7);
        ds.set_u16(tags::PIXEL_REPRESENTATION, 0);
        ds.set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::CS, "MONOCHROME2");
        ds.insert(dicom_toolkit_data::Element::new(
            tags::PIXEL_DATA,
            Vr::OB,
            Value::PixelData(PixelData::Native { bytes: pixels }),
        ));
        ds
    }

    #[test]
    fn from_dataset_basic() {
        let pixels = vec![0u8; 4]; // 2×2
        let ds = make_grayscale_8bit(2, 2, pixels);
        let img = DicomImage::from_dataset(&ds).unwrap();
        assert_eq!(img.rows, 2);
        assert_eq!(img.columns, 2);
        assert_eq!(img.frames, 1);
        assert_eq!(img.bits_allocated, 8);
        assert_eq!(img.bytes_per_frame(), 4);
    }

    #[test]
    fn frame_bytes_range_check() {
        let ds = make_grayscale_8bit(2, 2, vec![0u8; 4]);
        let img = DicomImage::from_dataset(&ds).unwrap();
        assert!(img.frame_bytes(0).is_ok());
        assert!(img.frame_bytes(1).is_err()); // only 1 frame
    }

    #[test]
    fn frame_u8_grayscale_all_zeros() {
        let ds = make_grayscale_8bit(2, 2, vec![0u8; 4]);
        let img = DicomImage::from_dataset(&ds).unwrap();
        let out = img.frame_u8(0).unwrap();
        assert_eq!(out.len(), 4);
        // Constant image (all 0): auto-window places all pixels at output_min → all 0.
        assert!(out.iter().all(|&v| v == 0), "expected all zeros, got {out:?}");
    }

    #[test]
    fn frame_u8_monochrome1_inversion() {
        // MONOCHROME1: max value (255) should appear as black (0) after window.
        let mut ds = make_grayscale_8bit(1, 2, vec![0u8, 255]);
        ds.set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::CS, "MONOCHROME1");
        let mut img = DicomImage::from_dataset(&ds).unwrap();
        img.set_window(127.5, 256.0);
        let out = img.frame_u8(0).unwrap();
        // pixel 0 → 0 → after window: near 0 → after inversion: near 255
        // pixel 255 → 255 → after window: near 255 → after inversion: near 0
        assert!(out[0] > 200, "expected inverted bright pixel, got {}", out[0]);
        assert!(out[1] < 50,  "expected inverted dark pixel, got {}",  out[1]);
    }

    #[test]
    fn frame_normalized_range() {
        let pixels: Vec<u8> = (0u8..=255).collect();
        let ds = make_grayscale_8bit(16, 16, pixels);
        let mut img = DicomImage::from_dataset(&ds).unwrap();
        img.set_window(127.5, 256.0);
        let norm = img.frame_normalized(0).unwrap();
        assert!(norm.iter().all(|&v| v >= 0.0 && v <= 1.0));
    }

    #[test]
    fn auto_window_sets_center_width() {
        let pixels: Vec<u8> = (0u8..=255).collect();
        let ds = make_grayscale_8bit(16, 16, pixels);
        let mut img = DicomImage::from_dataset(&ds).unwrap();
        img.auto_window();
        assert!(img.window_center.is_some());
        assert!(img.window_width.is_some());
    }

    #[test]
    fn bytes_per_frame_16bit() {
        let mut ds = DataSet::new();
        ds.set_u16(tags::ROWS, 2);
        ds.set_u16(tags::COLUMNS, 2);
        ds.set_u16(tags::SAMPLES_PER_PIXEL, 1);
        ds.set_u16(tags::BITS_ALLOCATED, 16);
        ds.set_u16(tags::BITS_STORED, 16);
        ds.set_u16(tags::HIGH_BIT, 15);
        ds.set_u16(tags::PIXEL_REPRESENTATION, 0);
        ds.set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::CS, "MONOCHROME2");
        let raw = vec![0u8; 8]; // 2×2×2 bytes
        ds.insert(dicom_toolkit_data::Element::new(
            tags::PIXEL_DATA,
            Vr::OW,
            Value::PixelData(PixelData::Native { bytes: raw }),
        ));
        let img = DicomImage::from_dataset(&ds).unwrap();
        assert_eq!(img.bytes_per_frame(), 8);
        assert_eq!(img.bytes_per_sample(), 2);
    }
}
