//! Integration tests for dcmtk-dict, porting DCMTK's tag and VR test suite.
//!
//! Ported from:
//! - dcmdata/tests/ttag.cc (tag tests)
//! - dcmdata/tests/txfer.cc (transfer syntax tests)
//! - dcmdata/tests/tvrcomp.cc (VR comparison tests)

use dicom_toolkit_dict::tag::{Tag, tags, ITEM, ITEM_DELIMITATION, SEQUENCE_DELIMITATION};
use dicom_toolkit_dict::vr::Vr;
use dicom_toolkit_dict::ts::transfer_syntaxes;

// ── Ported from dcmdata/tests/ttag.cc ────────────────────────────────────────

/// Tags are ordered by (group, element).
/// Ports: ttag ordering tests
#[test]
fn tag_ordering_by_group_element() {
    assert!(Tag::new(0x0008, 0x0000) < Tag::new(0x0008, 0x0001));
    assert!(Tag::new(0x0008, 0xFFFF) < Tag::new(0x0010, 0x0000));
    assert!(Tag::new(0x0000, 0x0000) < Tag::new(0xFFFF, 0xFFFF));
    assert_eq!(Tag::new(0x0010, 0x0010), Tag::new(0x0010, 0x0010));
}

/// Tag group/element accessors are correct.
#[test]
fn tag_group_element_accessors() {
    let tag = Tag::new(0x0010, 0x0020);
    assert_eq!(tag.group, 0x0010);
    assert_eq!(tag.element, 0x0020);
}

/// Tag display format is (GGGG,EEEE).
/// Ports: ttag display tests
#[test]
fn tag_display_format() {
    assert_eq!(tags::PATIENT_NAME.to_string(), "(0010,0010)");
    assert_eq!(tags::SOP_CLASS_UID.to_string(), "(0008,0016)");
    assert_eq!(Tag::new(0x0000, 0x0000).to_string(), "(0000,0000)");
    assert_eq!(Tag::new(0xFFFF, 0xFFFF).to_string(), "(FFFF,FFFF)");
}

/// Private tag detection (odd group numbers).
#[test]
fn tag_private_detection() {
    assert!(!Tag::new(0x0010, 0x0010).is_private(), "even group = standard");
    assert!(Tag::new(0x0011, 0x0010).is_private(), "odd group = private");
    assert!(Tag::new(0x0009, 0x0010).is_private(), "odd group = private");
    assert!(!Tag::new(0x0008, 0x0010).is_private(), "even group = standard");
}

/// Group length tag detection (element == 0x0000).
#[test]
fn tag_group_length_detection() {
    assert!(Tag::new(0x0008, 0x0000).is_group_length());
    assert!(!Tag::new(0x0008, 0x0001).is_group_length());
    assert!(tags::FILE_META_INFORMATION_GROUP_LENGTH.is_group_length());
}

/// File meta tags are in group 0002.
#[test]
fn tag_file_meta_detection() {
    assert!(tags::TRANSFER_SYNTAX_UID.is_file_meta());
    assert!(tags::MEDIA_STORAGE_SOP_CLASS_UID.is_file_meta());
    assert!(!tags::PATIENT_NAME.is_file_meta());
    assert!(!tags::SOP_CLASS_UID.is_file_meta());
}

/// Delimiter tag detection.
#[test]
fn tag_delimiter_detection() {
    assert!(ITEM.is_item());
    assert!(ITEM.is_delimiter());
    assert!(!ITEM.is_item_delimitation());
    assert!(!ITEM.is_sequence_delimitation());

    assert!(ITEM_DELIMITATION.is_item_delimitation());
    assert!(ITEM_DELIMITATION.is_delimiter());
    assert!(!ITEM_DELIMITATION.is_item());

    assert!(SEQUENCE_DELIMITATION.is_sequence_delimitation());
    assert!(SEQUENCE_DELIMITATION.is_delimiter());
    assert!(!SEQUENCE_DELIMITATION.is_item());
}

/// u32 roundtrip for tags.
#[test]
fn tag_u32_roundtrip() {
    let tags_to_test = [
        tags::PATIENT_NAME,
        tags::SOP_CLASS_UID,
        tags::PIXEL_DATA,
        Tag::new(0x0000, 0x0000),
        Tag::new(0xFFFF, 0xFFFF),
    ];
    for tag in &tags_to_test {
        assert_eq!(Tag::from_u32(tag.to_u32()), *tag);
    }
}

// ── Ported from dcmdata/tests/txfer.cc ───────────────────────────────────────

/// Standard transfer syntaxes are registered and retrievable.
/// Ports: txfer transfer syntax lookup tests
#[test]
fn transfer_syntax_lookup_standard() {
    let implicit = transfer_syntaxes::by_uid("1.2.840.10008.1.2").unwrap();
    assert!(implicit.is_implicit_vr());
    assert!(implicit.is_little_endian());

    let explicit_le = transfer_syntaxes::by_uid("1.2.840.10008.1.2.1").unwrap();
    assert!(explicit_le.is_explicit_vr());
    assert!(explicit_le.is_little_endian());
    assert!(!explicit_le.is_encapsulated());

    let explicit_be = transfer_syntaxes::by_uid("1.2.840.10008.1.2.2").unwrap();
    assert!(explicit_be.is_explicit_vr());
    assert!(!explicit_be.is_little_endian());
}

/// Deflated transfer syntax is marked correctly.
#[test]
fn transfer_syntax_deflated() {
    let deflated = transfer_syntaxes::by_uid("1.2.840.10008.1.2.1.99").unwrap();
    assert!(deflated.deflated);
    assert!(deflated.is_explicit_vr());
    assert!(deflated.is_little_endian());
}

/// Compressed transfer syntaxes use encapsulated pixel encoding.
#[test]
fn transfer_syntax_encapsulated() {
    let jpeg = transfer_syntaxes::by_uid("1.2.840.10008.1.2.4.50").unwrap();
    assert!(jpeg.is_encapsulated());

    let rle = transfer_syntaxes::by_uid("1.2.840.10008.1.2.5").unwrap();
    assert!(rle.is_encapsulated());
}

/// All registered transfer syntaxes have unique UIDs.
#[test]
fn transfer_syntax_all_unique_uids() {
    use std::collections::HashSet;
    let uids: HashSet<&str> = transfer_syntaxes::ALL.iter().map(|ts| ts.uid).collect();
    assert_eq!(uids.len(), transfer_syntaxes::ALL.len(), "duplicate UIDs in transfer syntax registry");
}

/// Unknown UID returns None.
#[test]
fn transfer_syntax_unknown_uid() {
    assert!(transfer_syntaxes::by_uid("9.9.9.9.9.9.9.9").is_none());
}

// ── Ported from dcmdata/tests/tvrcomp.cc ─────────────────────────────────────

/// All VR codes roundtrip through bytes.
/// Ports: tvrcomp VR parsing tests
#[test]
fn vr_byte_roundtrip_all() {
    let all = [
        Vr::AE, Vr::AS, Vr::AT, Vr::CS, Vr::DA, Vr::DS, Vr::DT, Vr::FD, Vr::FL,
        Vr::IS, Vr::LO, Vr::LT, Vr::OB, Vr::OD, Vr::OF, Vr::OL, Vr::OV, Vr::OW,
        Vr::PN, Vr::SH, Vr::SL, Vr::SQ, Vr::SS, Vr::ST, Vr::SV, Vr::TM, Vr::UC,
        Vr::UI, Vr::UL, Vr::UN, Vr::UR, Vr::US, Vr::UT, Vr::UV,
    ];
    for vr in &all {
        let bytes = vr.to_bytes();
        let parsed = Vr::from_bytes(bytes).unwrap_or_else(|| panic!("VR {vr} failed to roundtrip"));
        assert_eq!(*vr, parsed);
    }
}

/// VR string properties.
#[test]
fn vr_string_classification() {
    // String VRs
    for vr in [Vr::AE, Vr::AS, Vr::CS, Vr::DA, Vr::DS, Vr::DT, Vr::IS, Vr::LO,
                Vr::LT, Vr::PN, Vr::SH, Vr::ST, Vr::TM, Vr::UC, Vr::UI, Vr::UR, Vr::UT] {
        assert!(vr.is_string(), "{vr} should be a string VR");
    }
    // Non-string VRs
    for vr in [Vr::OB, Vr::OD, Vr::OF, Vr::OL, Vr::OV, Vr::OW, Vr::SQ,
                Vr::FL, Vr::FD, Vr::SL, Vr::SS, Vr::UL, Vr::US, Vr::UN, Vr::AT] {
        assert!(!vr.is_string(), "{vr} should NOT be a string VR");
    }
}

/// VRs with long explicit length encoding.
#[test]
fn vr_long_explicit_length() {
    // These need the extended (4-byte) length field in explicit VR
    for vr in [Vr::OB, Vr::OD, Vr::OF, Vr::OL, Vr::OV, Vr::OW,
                Vr::SQ, Vr::UC, Vr::UN, Vr::UR, Vr::UT, Vr::SV, Vr::UV] {
        assert!(vr.has_long_explicit_length(), "{vr} should have long explicit length");
    }
    // These use the short (2-byte) length field
    for vr in [Vr::CS, Vr::DA, Vr::DS, Vr::LO, Vr::PN, Vr::SH, Vr::UI,
                Vr::US, Vr::UL, Vr::SS, Vr::SL, Vr::FL, Vr::FD, Vr::AT] {
        assert!(!vr.has_long_explicit_length(), "{vr} should NOT have long explicit length");
    }
}

/// Fixed-size VR sizes match DICOM standard.
#[test]
fn vr_fixed_sizes() {
    assert_eq!(Vr::US.fixed_value_size(), Some(2));
    assert_eq!(Vr::SS.fixed_value_size(), Some(2));
    assert_eq!(Vr::UL.fixed_value_size(), Some(4));
    assert_eq!(Vr::SL.fixed_value_size(), Some(4));
    assert_eq!(Vr::FL.fixed_value_size(), Some(4));
    assert_eq!(Vr::FD.fixed_value_size(), Some(8));
    assert_eq!(Vr::UV.fixed_value_size(), Some(8));
    assert_eq!(Vr::SV.fixed_value_size(), Some(8));
    assert_eq!(Vr::AT.fixed_value_size(), Some(4)); // two u16 = 4 bytes

    // Variable-length VRs
    assert_eq!(Vr::LO.fixed_value_size(), None);
    assert_eq!(Vr::PN.fixed_value_size(), None);
    assert_eq!(Vr::OB.fixed_value_size(), None);
    assert_eq!(Vr::SQ.fixed_value_size(), None);
}

/// VR padding bytes match DICOM specification.
#[test]
fn vr_padding_bytes() {
    // String VRs (except UI) pad with space (0x20)
    assert_eq!(Vr::LO.padding_byte(), 0x20);
    assert_eq!(Vr::PN.padding_byte(), 0x20);
    assert_eq!(Vr::CS.padding_byte(), 0x20);

    // UI pads with null (0x00)
    assert_eq!(Vr::UI.padding_byte(), 0x00);

    // Binary VRs pad with null
    assert_eq!(Vr::OB.padding_byte(), 0x00);
    assert_eq!(Vr::US.padding_byte(), 0x00);
}
