//! Transfer syntax properties helpers.

use dicom_toolkit_dict::ts::{ByteOrder, PixelEncoding, VrEncoding, transfer_syntaxes};
use dicom_toolkit_dict::{Tag, Vr, tags};

/// Properties of a transfer syntax relevant to encoding/decoding.
pub struct TransferSyntaxProperties {
    pub byte_order: ByteOrder,
    pub vr_encoding: VrEncoding,
    pub is_deflated: bool,
    pub is_encapsulated: bool,
}

impl TransferSyntaxProperties {
    pub fn from_uid(uid: &str) -> Self {
        if let Some(ts) = transfer_syntaxes::by_uid(uid) {
            Self {
                byte_order: ts.byte_order,
                vr_encoding: ts.vr_encoding,
                is_deflated: ts.deflated,
                is_encapsulated: ts.pixel_encoding == PixelEncoding::Encapsulated,
            }
        } else {
            // Unknown TS → Explicit VR LE (safest default)
            Self {
                byte_order: ByteOrder::LittleEndian,
                vr_encoding: VrEncoding::Explicit,
                is_deflated: false,
                is_encapsulated: false,
            }
        }
    }

    pub fn is_little_endian(&self) -> bool {
        self.byte_order == ByteOrder::LittleEndian
    }

    pub fn is_explicit_vr(&self) -> bool {
        self.vr_encoding == VrEncoding::Explicit
    }
}

/// Infer the VR for a tag in Implicit VR transfer syntaxes.
/// Returns `Vr::UN` for unknown tags.
pub fn implicit_vr_for_tag(tag: Tag) -> Vr {
    match tag {
        // File meta (group 0002)
        tags::FILE_META_INFORMATION_GROUP_LENGTH => Vr::UL,
        tags::FILE_META_INFORMATION_VERSION => Vr::OB,
        tags::MEDIA_STORAGE_SOP_CLASS_UID => Vr::UI,
        tags::MEDIA_STORAGE_SOP_INSTANCE_UID => Vr::UI,
        tags::TRANSFER_SYNTAX_UID => Vr::UI,
        tags::IMPLEMENTATION_CLASS_UID => Vr::UI,
        tags::IMPLEMENTATION_VERSION_NAME => Vr::SH,
        // General
        tags::SPECIFIC_CHARACTER_SET => Vr::CS,
        tags::IMAGE_TYPE => Vr::CS,
        tags::SOP_CLASS_UID => Vr::UI,
        tags::SOP_INSTANCE_UID => Vr::UI,
        tags::STUDY_DATE => Vr::DA,
        tags::SERIES_DATE => Vr::DA,
        tags::ACQUISITION_DATE => Vr::DA,
        tags::CONTENT_DATE => Vr::DA,
        tags::STUDY_TIME => Vr::TM,
        tags::SERIES_TIME => Vr::TM,
        tags::ACQUISITION_TIME => Vr::TM,
        tags::CONTENT_TIME => Vr::TM,
        tags::ACCESSION_NUMBER => Vr::SH,
        tags::MODALITY => Vr::CS,
        tags::MANUFACTURER => Vr::LO,
        tags::INSTITUTION_NAME => Vr::LO,
        tags::REFERRING_PHYSICIAN_NAME => Vr::PN,
        tags::STUDY_DESCRIPTION => Vr::LO,
        tags::SERIES_DESCRIPTION => Vr::LO,
        tags::PERFORMING_PHYSICIAN_NAME => Vr::PN,
        tags::OPERATORS_NAME => Vr::PN,
        tags::REFERENCED_SOP_CLASS_UID => Vr::UI,
        tags::REFERENCED_SOP_INSTANCE_UID => Vr::UI,
        // Patient
        tags::PATIENT_NAME => Vr::PN,
        tags::PATIENT_ID => Vr::LO,
        tags::PATIENT_BIRTH_DATE => Vr::DA,
        tags::PATIENT_SEX => Vr::CS,
        tags::PATIENT_AGE => Vr::AS,
        tags::PATIENT_SIZE => Vr::DS,
        tags::PATIENT_WEIGHT => Vr::DS,
        // Study / Series
        tags::STUDY_INSTANCE_UID => Vr::UI,
        tags::SERIES_INSTANCE_UID => Vr::UI,
        tags::STUDY_ID => Vr::SH,
        tags::SERIES_NUMBER => Vr::IS,
        tags::ACQUISITION_NUMBER => Vr::IS,
        tags::INSTANCE_NUMBER => Vr::IS,
        tags::IMAGE_POSITION_PATIENT => Vr::DS,
        tags::IMAGE_ORIENTATION_PATIENT => Vr::DS,
        tags::FRAME_OF_REFERENCE_UID => Vr::UI,
        tags::SLICE_LOCATION => Vr::DS,
        tags::NUMBER_OF_FRAMES => Vr::IS,
        // Image
        tags::SAMPLES_PER_PIXEL => Vr::US,
        tags::PHOTOMETRIC_INTERPRETATION => Vr::CS,
        tags::ROWS => Vr::US,
        tags::COLUMNS => Vr::US,
        tags::BITS_ALLOCATED => Vr::US,
        tags::BITS_STORED => Vr::US,
        tags::HIGH_BIT => Vr::US,
        tags::PIXEL_REPRESENTATION => Vr::US,
        tags::PLANAR_CONFIGURATION => Vr::US,
        tags::PIXEL_DATA => Vr::OW,
        // Window/Rescale
        tags::WINDOW_CENTER => Vr::DS,
        tags::WINDOW_WIDTH => Vr::DS,
        tags::RESCALE_INTERCEPT => Vr::DS,
        tags::RESCALE_SLOPE => Vr::DS,
        _ => Vr::UN,
    }
}
