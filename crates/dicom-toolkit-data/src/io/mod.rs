//! DICOM file and stream I/O.

pub mod codec;
pub mod part10;
pub mod reader;
pub mod transfer;
pub mod writer;

pub use part10::{
    part10_inspection_limit_from_error, read_part10_file_index, read_part10_file_layout,
    Part10FileIdentity, Part10FileIndex, Part10FileLayout, Part10InspectionLimit, Part10ReadLimits,
};
pub use reader::DicomReader;
pub use writer::{element_value_bytes, DicomWriter};
