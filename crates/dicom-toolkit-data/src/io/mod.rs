//! DICOM file and stream I/O.

pub mod codec;
pub mod reader;
pub mod transfer;
pub mod writer;

pub use reader::DicomReader;
pub use writer::DicomWriter;
