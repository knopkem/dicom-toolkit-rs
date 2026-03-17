//! Transfer syntax properties helpers.

use dicom_toolkit_dict::ts::{transfer_syntaxes, ByteOrder, PixelEncoding, VrEncoding};
use dicom_toolkit_dict::{transfer_syntaxes::IMPLICIT_VR_LITTLE_ENDIAN, Tag, Vr};

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
    IMPLICIT_VR_LITTLE_ENDIAN.resolve_vr(tag).unwrap_or(Vr::UN)
}

#[cfg(test)]
mod tests {
    use super::implicit_vr_for_tag;
    use dicom_toolkit_dict::{tags, Tag, Vr};

    #[test]
    fn implicit_vr_lookup_resolves_dimse_query_tags() {
        assert_eq!(implicit_vr_for_tag(tags::QUERY_RETRIEVE_LEVEL), Vr::CS);
        assert_eq!(implicit_vr_for_tag(tags::MODALITIES_IN_STUDY), Vr::CS);
        assert_eq!(implicit_vr_for_tag(tags::ISSUER_OF_PATIENT_ID), Vr::LO);
        assert_eq!(
            implicit_vr_for_tag(tags::NUMBER_OF_STUDY_RELATED_SERIES),
            Vr::IS
        );
        assert_eq!(
            implicit_vr_for_tag(tags::NUMBER_OF_STUDY_RELATED_INSTANCES),
            Vr::IS
        );
    }

    #[test]
    fn implicit_vr_lookup_falls_back_to_un_for_unknown_tags() {
        assert_eq!(implicit_vr_for_tag(Tag::new(0x9999, 0x9999)), Vr::UN);
    }
}
