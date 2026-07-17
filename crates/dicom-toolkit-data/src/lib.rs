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

pub use dataset::{parse_attribute_path, resolve_attribute_path, AttributePathSegment, DataSet};
pub use element::Element;
pub use file_format::FileFormat;
pub use io::{
    element_value_bytes, part10_inspection_limit_from_error, read_part10_file_index,
    read_part10_file_layout, DicomReader, DicomWriter, Part10FileIdentity, Part10FileIndex,
    Part10FileLayout, Part10InspectionLimit, Part10ReadLimits,
};
pub use meta_info::FileMetaInformation;
pub use value::{
    build_encapsulated_pixel_data, encapsulated_frames, encapsulated_pixel_data_from_frames,
    DicomDate, DicomDateTime, DicomTime, EncapsulatedFrame, PersonName, PixelData, Value,
};
