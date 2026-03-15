//! > ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for diagnostic or therapeutic purposes.
//! Foundation types and utilities for the dcmtk-rs DICOM toolkit.

pub mod charset;
pub mod config;
pub mod error;
pub mod log;
pub mod uid;

pub use error::{DcmError, DcmResult};
pub use uid::Uid;
