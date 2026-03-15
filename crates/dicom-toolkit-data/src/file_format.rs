//! High-level DICOM file API.
//!
//! `FileFormat` is the top-level object representing a DICOM Part 10 file:
//! it bundles the File Meta Information with the dataset.

use std::path::Path;
use dicom_toolkit_core::error::DcmResult;
use crate::dataset::DataSet;
use crate::meta_info::FileMetaInformation;

/// A DICOM Part 10 file: File Meta Information + dataset.
#[derive(Debug, Clone)]
pub struct FileFormat {
    pub meta: FileMetaInformation,
    pub dataset: DataSet,
}

impl FileFormat {
    pub fn new(meta: FileMetaInformation, dataset: DataSet) -> Self {
        Self { meta, dataset }
    }

    /// Create from a dataset, auto-generating File Meta Information.
    ///
    /// Uses Explicit VR Little Endian as the default transfer syntax.
    pub fn from_dataset(sop_class_uid: &str, sop_instance_uid: &str, dataset: DataSet) -> Self {
        let meta = FileMetaInformation::new(
            sop_class_uid,
            sop_instance_uid,
            "1.2.840.10008.1.2.1",
        );
        Self { meta, dataset }
    }

    /// Open a DICOM file from disk.
    pub fn open(path: impl AsRef<Path>) -> DcmResult<Self> {
        let data = std::fs::read(path.as_ref())?;
        crate::io::reader::parse_file(&data)
    }

    /// Save this file to disk using its current transfer syntax.
    pub fn save(&self, path: impl AsRef<Path>) -> DcmResult<()> {
        let bytes = crate::io::writer::encode_file(self)?;
        std::fs::write(path.as_ref(), &bytes)?;
        Ok(())
    }

    /// Save to disk using a specific transfer syntax.
    ///
    /// Only the transfer syntax UID in the meta is updated; no pixel data
    /// transcoding is performed.
    pub fn save_as(&self, path: impl AsRef<Path>, ts_uid: &str) -> DcmResult<()> {
        let mut ff = self.clone();
        ff.meta.transfer_syntax_uid = ts_uid.to_string();
        let bytes = crate::io::writer::encode_file(&ff)?;
        std::fs::write(path.as_ref(), &bytes)?;
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_dict::{Vr, tags};
    use crate::element::Element;
    use crate::io::reader::DicomReader;
    use crate::io::writer::DicomWriter;

    fn make_test_dataset() -> DataSet {
        let mut ds = DataSet::new();
        ds.insert(Element::string(tags::PATIENT_NAME, Vr::PN, "Smith^John"));
        ds.insert(Element::u16(tags::ROWS, 512));
        ds.insert(Element::u16(tags::COLUMNS, 256));
        ds
    }

    #[test]
    fn file_format_roundtrip_explicit_vr_le() {
        let ds = make_test_dataset();
        let ff = FileFormat::from_dataset(
            "1.2.840.10008.5.1.4.1.1.2",
            "1.2.3.4.5",
            ds.clone(),
        );

        let mut buf = Vec::new();
        DicomWriter::new(&mut buf).write_file(&ff).unwrap();

        let ff2 = DicomReader::new(buf.as_slice()).read_file().unwrap();
        assert_eq!(ff2.dataset.get_u16(tags::ROWS), Some(512));
        assert_eq!(ff2.dataset.get_u16(tags::COLUMNS), Some(256));
        assert_eq!(ff2.dataset.get_string(tags::PATIENT_NAME), Some("Smith^John"));
    }

    #[test]
    fn file_format_roundtrip_implicit_vr_le() {
        let mut ds = DataSet::new();
        ds.insert(Element::u16(tags::ROWS, 128));
        ds.insert(Element::u16(tags::COLUMNS, 128));

        let mut buf = Vec::new();
        DicomWriter::new(&mut buf)
            .write_dataset(&ds, "1.2.840.10008.1.2")
            .unwrap();

        let ds2 = DicomReader::new(buf.as_slice())
            .read_dataset("1.2.840.10008.1.2")
            .unwrap();
        assert_eq!(ds2.get_u16(tags::ROWS), Some(128));
        assert_eq!(ds2.get_u16(tags::COLUMNS), Some(128));
    }

    #[test]
    fn file_format_roundtrip_with_sequence() {
        let mut item = DataSet::new();
        item.insert(Element::string(tags::PATIENT_ID, Vr::LO, "PID001"));

        let mut ds = DataSet::new();
        ds.insert(Element::sequence(tags::REFERENCED_SOP_SEQUENCE, vec![item]));

        let ff = FileFormat::from_dataset("", "", ds);
        let mut buf = Vec::new();
        DicomWriter::new(&mut buf).write_file(&ff).unwrap();

        let ff2 = DicomReader::new(buf.as_slice()).read_file().unwrap();
        let items = ff2.dataset.get_items(tags::REFERENCED_SOP_SEQUENCE).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get_string(tags::PATIENT_ID), Some("PID001"));
    }

    #[test]
    fn file_format_from_disk_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.dcm");

        let ds = make_test_dataset();
        let ff = FileFormat::from_dataset("", "1.2.3", ds);
        ff.save(&path).unwrap();

        let loaded = FileFormat::open(&path).unwrap();
        assert_eq!(loaded.dataset.get_u16(tags::ROWS), Some(512));
        assert_eq!(loaded.dataset.get_string(tags::PATIENT_NAME), Some("Smith^John"));
    }

    #[test]
    fn file_format_save_as_different_ts() {
        let dir = tempfile::tempdir().unwrap();
        let path1 = dir.path().join("explicit.dcm");

        let ds = make_test_dataset();
        let ff = FileFormat::from_dataset("", "1.2.3", ds);
        ff.save_as(&path1, "1.2.840.10008.1.2").unwrap();

        let loaded = FileFormat::open(&path1).unwrap();
        assert_eq!(
            loaded.meta.transfer_syntax_uid,
            "1.2.840.10008.1.2"
        );
    }

    #[test]
    fn file_format_meta_generated() {
        let ff = FileFormat::from_dataset(
            "1.2.840.10008.5.1.4.1.1.2",
            "9.8.7.6.5",
            DataSet::new(),
        );
        assert_eq!(ff.meta.transfer_syntax_uid, "1.2.840.10008.1.2.1");
        assert_eq!(ff.meta.media_storage_sop_class_uid, "1.2.840.10008.5.1.4.1.1.2");
        assert_eq!(ff.meta.media_storage_sop_instance_uid, "9.8.7.6.5");
    }
}
