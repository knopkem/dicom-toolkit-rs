//! DICOM File Meta Information (group 0002).
//!
//! Manages the header present in every DICOM Part 10 file.

use crate::dataset::DataSet;
use crate::element::Element;
use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_dict::{tags, Vr};

// ── Constants ─────────────────────────────────────────────────────────────────

/// File Meta Information Version (0002,0001): OB value [0x00, 0x01].
pub const META_VERSION: [u8; 2] = [0x00, 0x01];

/// Implementation class UID for this library.
pub const IMPL_CLASS_UID: &str = "1.2.276.0.7230010.3.1.9999.1";

/// Implementation version name for this library.
pub const IMPL_VERSION_NAME: &str = "DCMTK-RS-0.1";

// ── FileMetaInformation ───────────────────────────────────────────────────────

/// DICOM File Meta Information (group 0002,xxxx).
///
/// Every DICOM Part 10 file starts with this header, always encoded as
/// Explicit VR Little Endian.
#[derive(Debug, Clone, PartialEq)]
pub struct FileMetaInformation {
    pub media_storage_sop_class_uid: String,
    pub media_storage_sop_instance_uid: String,
    pub transfer_syntax_uid: String,
    pub implementation_class_uid: String,
    pub implementation_version_name: String,
}

impl FileMetaInformation {
    /// Create new File Meta Information with default implementation identifiers.
    pub fn new(sop_class_uid: &str, sop_instance_uid: &str, ts_uid: &str) -> Self {
        Self {
            media_storage_sop_class_uid: sop_class_uid.to_string(),
            media_storage_sop_instance_uid: sop_instance_uid.to_string(),
            transfer_syntax_uid: ts_uid.to_string(),
            implementation_class_uid: IMPL_CLASS_UID.to_string(),
            implementation_version_name: IMPL_VERSION_NAME.to_string(),
        }
    }

    /// Convert to a DataSet for encoding.
    ///
    /// Does NOT include (0002,0000) group length — the writer computes that.
    pub fn to_dataset(&self) -> DataSet {
        let mut ds = DataSet::new();

        ds.insert(Element::bytes(
            tags::FILE_META_INFORMATION_VERSION,
            Vr::OB,
            META_VERSION.to_vec(),
        ));
        ds.insert(Element::uid(
            tags::MEDIA_STORAGE_SOP_CLASS_UID,
            &self.media_storage_sop_class_uid,
        ));
        ds.insert(Element::uid(
            tags::MEDIA_STORAGE_SOP_INSTANCE_UID,
            &self.media_storage_sop_instance_uid,
        ));
        ds.insert(Element::uid(
            tags::TRANSFER_SYNTAX_UID,
            &self.transfer_syntax_uid,
        ));
        ds.insert(Element::uid(
            tags::IMPLEMENTATION_CLASS_UID,
            &self.implementation_class_uid,
        ));
        ds.insert(Element::string(
            tags::IMPLEMENTATION_VERSION_NAME,
            Vr::SH,
            &self.implementation_version_name,
        ));

        ds
    }

    /// Parse from a DataSet read from the file.
    pub fn from_dataset(ds: &DataSet) -> DcmResult<Self> {
        let transfer_syntax_uid = ds
            .get_string(tags::TRANSFER_SYNTAX_UID)
            .ok_or_else(|| DcmError::InvalidFile {
                reason: "missing Transfer Syntax UID (0002,0010)".into(),
            })?
            .trim_end_matches('\0')
            .to_string();

        let media_storage_sop_class_uid = ds
            .get_string(tags::MEDIA_STORAGE_SOP_CLASS_UID)
            .unwrap_or("")
            .trim_end_matches('\0')
            .to_string();

        let media_storage_sop_instance_uid = ds
            .get_string(tags::MEDIA_STORAGE_SOP_INSTANCE_UID)
            .unwrap_or("")
            .trim_end_matches('\0')
            .to_string();

        let implementation_class_uid = ds
            .get_string(tags::IMPLEMENTATION_CLASS_UID)
            .unwrap_or(IMPL_CLASS_UID)
            .trim_end_matches('\0')
            .to_string();

        let implementation_version_name = ds
            .get_string(tags::IMPLEMENTATION_VERSION_NAME)
            .unwrap_or(IMPL_VERSION_NAME)
            .trim_end_matches('\0')
            .to_string();

        Ok(Self {
            media_storage_sop_class_uid,
            media_storage_sop_instance_uid,
            transfer_syntax_uid,
            implementation_class_uid,
            implementation_version_name,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_roundtrip_to_from_dataset() {
        let meta = FileMetaInformation::new(
            "1.2.840.10008.5.1.4.1.1.2",
            "1.2.3.4.5.6.7",
            "1.2.840.10008.1.2.1",
        );
        let ds = meta.to_dataset();
        let back = FileMetaInformation::from_dataset(&ds).unwrap();
        assert_eq!(meta.transfer_syntax_uid, back.transfer_syntax_uid);
        assert_eq!(
            meta.media_storage_sop_class_uid,
            back.media_storage_sop_class_uid
        );
        assert_eq!(
            meta.media_storage_sop_instance_uid,
            back.media_storage_sop_instance_uid
        );
    }

    #[test]
    fn meta_from_dataset_missing_ts_uid_errors() {
        let ds = DataSet::new();
        assert!(FileMetaInformation::from_dataset(&ds).is_err());
    }

    #[test]
    fn meta_to_dataset_has_version() {
        let meta = FileMetaInformation::new("", "", "1.2.840.10008.1.2.1");
        let ds = meta.to_dataset();
        let ver = ds.get_bytes(tags::FILE_META_INFORMATION_VERSION).unwrap();
        assert_eq!(ver, &[0x00, 0x01]);
    }
}
