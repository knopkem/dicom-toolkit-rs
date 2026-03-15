//! > ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for diagnostic or therapeutic purposes.
//! DICOM image processing: pixel data, windowing, LUTs, color models, export.
//!
//! This crate ports DCMTK's `dcmimgle` and `dcmimage` modules to idiomatic Rust.

pub mod color;
pub mod dicom_image;
pub mod export;
pub mod lut;
pub mod overlay;
pub mod pixel;
pub mod transform;
pub mod window;

// ── Top-level re-exports ──────────────────────────────────────────────────────

pub use color::{PaletteColorLut, PhotometricInterpretation};
pub use dicom_image::{DicomImage, PixelRepresentation};
pub use export::{export_frame_png, frame_to_png_bytes};
pub use lut::ModalityLut;
pub use overlay::Overlay;
pub use transform::{Flip, Rotation, flip, rotate, scale_bilinear};
pub use window::WindowLevel;
