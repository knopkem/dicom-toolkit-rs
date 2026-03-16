//! Well-known DICOM UID registry.
//!
//! Re-exports the UID constants from `dcmtk-core` and provides a lookup table.

pub use dicom_toolkit_core::uid::sop_class;
pub use dicom_toolkit_core::uid::transfer_syntax;

use std::collections::HashMap;
use std::sync::LazyLock;

/// Mapping of UID string → human-readable name.
static UID_NAMES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut map = HashMap::new();

    // Transfer Syntaxes
    map.insert(
        transfer_syntax::IMPLICIT_VR_LITTLE_ENDIAN,
        "Implicit VR Little Endian",
    );
    map.insert(
        transfer_syntax::EXPLICIT_VR_LITTLE_ENDIAN,
        "Explicit VR Little Endian",
    );
    map.insert(
        transfer_syntax::EXPLICIT_VR_BIG_ENDIAN,
        "Explicit VR Big Endian",
    );
    map.insert(
        transfer_syntax::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN,
        "Deflated Explicit VR Little Endian",
    );
    map.insert(transfer_syntax::JPEG_BASELINE, "JPEG Baseline");
    map.insert(transfer_syntax::JPEG_EXTENDED, "JPEG Extended");
    map.insert(
        transfer_syntax::JPEG_LOSSLESS_NON_HIERARCHICAL,
        "JPEG Lossless (Process 14)",
    );
    map.insert(
        transfer_syntax::JPEG_LOSSLESS_NON_HIERARCHICAL_FIRST_ORDER,
        "JPEG Lossless SV1",
    );
    map.insert(transfer_syntax::JPEG_LS_LOSSLESS, "JPEG-LS Lossless");
    map.insert(transfer_syntax::JPEG_LS_LOSSY, "JPEG-LS Lossy");
    map.insert(transfer_syntax::JPEG_2000_LOSSLESS, "JPEG 2000 Lossless");
    map.insert(transfer_syntax::JPEG_2000, "JPEG 2000");
    map.insert(
        transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY,
        "High-Throughput JPEG 2000 Lossless Only",
    );
    map.insert(
        transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_RPCL_LOSSLESS_ONLY,
        "High-Throughput JPEG 2000 RPCL Lossless Only",
    );
    map.insert(
        transfer_syntax::HIGH_THROUGHPUT_JPEG_2000,
        "High-Throughput JPEG 2000",
    );
    map.insert(transfer_syntax::RLE_LOSSLESS, "RLE Lossless");

    // SOP Classes
    map.insert(sop_class::VERIFICATION, "Verification SOP Class");
    map.insert(sop_class::CT_IMAGE_STORAGE, "CT Image Storage");
    map.insert(sop_class::MR_IMAGE_STORAGE, "MR Image Storage");
    map.insert(
        sop_class::ULTRASOUND_IMAGE_STORAGE,
        "Ultrasound Image Storage",
    );
    map.insert(
        sop_class::SECONDARY_CAPTURE_IMAGE_STORAGE,
        "Secondary Capture Image Storage",
    );
    map.insert(sop_class::CR_IMAGE_STORAGE, "CR Image Storage");
    map.insert(
        sop_class::NM_IMAGE_STORAGE,
        "Nuclear Medicine Image Storage",
    );
    map.insert(sop_class::PET_IMAGE_STORAGE, "PET Image Storage");
    map.insert(sop_class::RT_IMAGE_STORAGE, "RT Image Storage");
    map.insert(sop_class::RT_DOSE_STORAGE, "RT Dose Storage");
    map.insert(
        sop_class::RT_STRUCTURE_SET_STORAGE,
        "RT Structure Set Storage",
    );
    map.insert(sop_class::RT_PLAN_STORAGE, "RT Plan Storage");
    map.insert(sop_class::PATIENT_ROOT_QR_FIND, "Patient Root Q/R Find");
    map.insert(sop_class::PATIENT_ROOT_QR_MOVE, "Patient Root Q/R Move");
    map.insert(sop_class::PATIENT_ROOT_QR_GET, "Patient Root Q/R Get");
    map.insert(sop_class::STUDY_ROOT_QR_FIND, "Study Root Q/R Find");
    map.insert(sop_class::STUDY_ROOT_QR_MOVE, "Study Root Q/R Move");
    map.insert(sop_class::STUDY_ROOT_QR_GET, "Study Root Q/R Get");
    map.insert(sop_class::MODALITY_WORKLIST_FIND, "Modality Worklist Find");

    map
});

/// Looks up a human-readable name for a UID.
pub fn uid_name(uid: &str) -> Option<&'static str> {
    UID_NAMES.get(uid).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_uid_lookup() {
        assert_eq!(
            uid_name(sop_class::VERIFICATION),
            Some("Verification SOP Class")
        );
        assert_eq!(
            uid_name(sop_class::CT_IMAGE_STORAGE),
            Some("CT Image Storage")
        );
    }

    #[test]
    fn unknown_uid_lookup() {
        assert_eq!(uid_name("1.2.3.4.5.6.7.8.9"), None);
    }

    #[test]
    fn transfer_syntax_lookup() {
        assert_eq!(
            uid_name(transfer_syntax::IMPLICIT_VR_LITTLE_ENDIAN),
            Some("Implicit VR Little Endian")
        );
    }
}
