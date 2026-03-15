//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for diagnostic or therapeutic purposes.
//!
//! Core DICOM data structures, file I/O, and encoding/decoding.
//!
//! This crate ports DCMTK's `dcmdata` module — the heart of DICOM data handling.

pub mod dataset;
pub mod element;
pub mod file_format;
pub mod io;
pub mod json;
pub mod meta_info;
pub mod sequence;
pub mod value;
pub mod vr;
pub mod xml;

pub use dataset::DataSet;
pub use element::Element;
pub use file_format::FileFormat;
pub use io::{DicomReader, DicomWriter};
pub use meta_info::FileMetaInformation;
pub use value::{DicomDate, DicomDateTime, DicomTime, PersonName, PixelData, Value};
