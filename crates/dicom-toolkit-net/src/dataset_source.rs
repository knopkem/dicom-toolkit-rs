//! Bounded data sources for outgoing DIMSE datasets.

use std::path::{Path, PathBuf};

use bytes::Bytes;

/// A file region containing an encoded DICOM dataset.
///
/// DIMSE C-STORE transports the dataset only, not the Part 10 preamble and File
/// Meta Information. Callers using a Part 10 file must therefore set `offset`
/// to the first byte of the dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDataset {
    path: PathBuf,
    offset: u64,
    length: Option<u64>,
}

impl FileDataset {
    /// Create a dataset source that reads from `offset` through the end of a file.
    pub fn to_end(path: impl Into<PathBuf>, offset: u64) -> Self {
        Self {
            path: path.into(),
            offset,
            length: None,
        }
    }

    /// Create a dataset source for an exact file region.
    pub fn region(path: impl Into<PathBuf>, offset: u64, length: u64) -> Self {
        Self {
            path: path.into(),
            offset,
            length: Some(length),
        }
    }

    /// Return the file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the byte offset of the encoded dataset.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Return the exact dataset length, or `None` when the region extends to EOF.
    pub fn length(&self) -> Option<u64> {
        self.length
    }
}

/// An encoded dataset supplied either from memory or from a bounded file region.
///
/// The file-backed variant is intended for large instances and keeps memory use
/// independent of the encoded dataset size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatasetSource {
    /// An immutable in-memory dataset, suitable for small payloads and tests.
    Bytes(Bytes),
    /// A dataset read lazily from a file region.
    File(FileDataset),
}

impl DatasetSource {
    /// Create an in-memory dataset source without copying an existing `Bytes` value.
    pub fn bytes(value: impl Into<Bytes>) -> Self {
        Self::Bytes(value.into())
    }

    /// Create a file-backed dataset source that reads through EOF.
    pub fn file_to_end(path: impl Into<PathBuf>, offset: u64) -> Self {
        Self::File(FileDataset::to_end(path, offset))
    }

    /// Create a file-backed dataset source with an exact byte length.
    pub fn file_region(path: impl Into<PathBuf>, offset: u64, length: u64) -> Self {
        Self::File(FileDataset::region(path, offset, length))
    }
}

impl From<Vec<u8>> for DatasetSource {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(Bytes::from(value))
    }
}

impl From<Bytes> for DatasetSource {
    fn from(value: Bytes) -> Self {
        Self::Bytes(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_region_preserves_bounds() {
        let source = FileDataset::region("instance.dcm", 256, 1_024);

        assert_eq!(source.path(), Path::new("instance.dcm"));
        assert_eq!(source.offset(), 256);
        assert_eq!(source.length(), Some(1_024));
    }

    #[test]
    fn bytes_conversion_is_lossless() {
        let source = DatasetSource::from(vec![1, 2, 3]);

        assert_eq!(source, DatasetSource::Bytes(Bytes::from_static(&[1, 2, 3])));
    }
}
