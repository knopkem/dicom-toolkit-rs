//! DICOM tag definitions and tag constants.
//!
//! Ports `DcmTag`/`DcmTagKey` from DCMTK's `dctag.h` and tag constants from
//! `dcdeftag.h`.

use crate::vr::Vr;
use std::fmt;

/// A DICOM tag, identified by a (group, element) pair.
///
/// Tags are the fundamental addressing mechanism in DICOM. Each data element
/// in a dataset is identified by its tag.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag {
    pub group: u16,
    pub element: u16,
}

impl Tag {
    /// Creates a new tag from group and element numbers.
    pub const fn new(group: u16, element: u16) -> Self {
        Self { group, element }
    }

    /// Creates a tag from a 32-bit value (group in upper 16, element in lower 16).
    pub const fn from_u32(value: u32) -> Self {
        Self {
            group: (value >> 16) as u16,
            element: value as u16,
        }
    }

    /// Returns the tag as a 32-bit value.
    pub const fn to_u32(self) -> u32 {
        (self.group as u32) << 16 | self.element as u32
    }

    /// Returns `true` if this is a group length tag (element == 0x0000).
    pub const fn is_group_length(self) -> bool {
        self.element == 0x0000
    }

    /// Returns `true` if this is a private tag (odd group number).
    pub const fn is_private(self) -> bool {
        self.group % 2 != 0
    }

    /// Returns `true` if this tag is in File Meta Information (group 0x0002).
    pub const fn is_file_meta(self) -> bool {
        self.group == 0x0002
    }

    /// Returns `true` if this is the Item tag (FFFE,E000).
    pub const fn is_item(self) -> bool {
        self.group == 0xFFFE && self.element == 0xE000
    }

    /// Returns `true` if this is the Item Delimitation tag (FFFE,E00D).
    pub const fn is_item_delimitation(self) -> bool {
        self.group == 0xFFFE && self.element == 0xE00D
    }

    /// Returns `true` if this is the Sequence Delimitation tag (FFFE,E0DD).
    pub const fn is_sequence_delimitation(self) -> bool {
        self.group == 0xFFFE && self.element == 0xE0DD
    }

    /// Returns `true` if this is a delimiter tag (Item, Item Delim, or Seq Delim).
    pub const fn is_delimiter(self) -> bool {
        self.group == 0xFFFE
    }
}

impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tag({:04X},{:04X})", self.group, self.element)
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:04X},{:04X})", self.group, self.element)
    }
}

/// Sentinel tags used during DICOM parsing.
pub const ITEM: Tag = Tag::new(0xFFFE, 0xE000);
pub const ITEM_DELIMITATION: Tag = Tag::new(0xFFFE, 0xE00D);
pub const SEQUENCE_DELIMITATION: Tag = Tag::new(0xFFFE, 0xE0DD);

/// An entry in the DICOM data dictionary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeRestriction {
    Unspecified,
    Odd,
    Even,
}

/// Sentinel used for dictionary entries with variable VM (`1-n`, `2-n`, ...).
pub const VARIABLE_VM: u32 = u32::MAX;

/// An entry in the DICOM data dictionary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DictEntry {
    pub tag: Tag,
    pub upper_tag: Tag,
    pub vr: Vr,
    pub raw_vr: &'static str,
    pub name: &'static str,
    pub keyword: &'static str,
    pub vm_min: u32,
    pub vm_max: u32,
    pub standard_version: &'static str,
    pub group_restriction: RangeRestriction,
    pub element_restriction: RangeRestriction,
    pub private_creator: Option<&'static str>,
}

impl DictEntry {
    pub const fn is_repeating(&self) -> bool {
        self.tag.group != self.upper_tag.group || self.tag.element != self.upper_tag.element
    }

    pub fn private_creator_matches(&self, private_creator: Option<&str>) -> bool {
        match (self.private_creator, private_creator) {
            (None, None) => true,
            (Some(expected), Some(actual)) => expected == actual,
            _ => false,
        }
    }

    pub fn contains(&self, key: Tag, private_creator: Option<&str>) -> bool {
        if self.group_restriction == RangeRestriction::Even && key.group % 2 != 0 {
            return false;
        }
        if self.group_restriction == RangeRestriction::Odd && key.group % 2 == 0 {
            return false;
        }
        if self.element_restriction == RangeRestriction::Even && key.element % 2 != 0 {
            return false;
        }
        if self.element_restriction == RangeRestriction::Odd && key.element % 2 == 0 {
            return false;
        }
        if !self.private_creator_matches(private_creator) {
            return false;
        }

        let group_matches = (self.tag.group..=self.upper_tag.group).contains(&key.group);
        let mut found =
            group_matches && (self.tag.element..=self.upper_tag.element).contains(&key.element);

        if !found && group_matches && private_creator.is_some() {
            let low_element = key.element & 0x00FF;
            found = (self.tag.element..=self.upper_tag.element).contains(&low_element);
        }

        found
    }
}

#[path = "generated_dictionary.rs"]
mod generated_dictionary;

use generated_dictionary::{EXACT_ENTRIES, REPEATING_ENTRIES};

fn exact_lower_bound(tag: Tag) -> usize {
    let mut left = 0usize;
    let mut right = EXACT_ENTRIES.len();
    while left < right {
        let mid = left + (right - left) / 2;
        if EXACT_ENTRIES[mid].tag < tag {
            left = mid + 1;
        } else {
            right = mid;
        }
    }
    left
}

fn exact_upper_bound(tag: Tag) -> usize {
    let mut left = 0usize;
    let mut right = EXACT_ENTRIES.len();
    while left < right {
        let mid = left + (right - left) / 2;
        if EXACT_ENTRIES[mid].tag <= tag {
            left = mid + 1;
        } else {
            right = mid;
        }
    }
    left
}

/// Look up a dictionary entry by tag.
pub fn lookup_entry(tag: Tag) -> Option<&'static DictEntry> {
    lookup_entry_with_private_creator(tag, None)
}

/// Look up a dictionary entry by tag and optional private creator identifier.
pub fn lookup_entry_with_private_creator(
    tag: Tag,
    private_creator: Option<&str>,
) -> Option<&'static DictEntry> {
    let start = exact_lower_bound(tag);
    let end = exact_upper_bound(tag);
    if start < end {
        if let Some(entry) = EXACT_ENTRIES[start..end]
            .iter()
            .find(|entry| entry.private_creator_matches(private_creator))
        {
            return Some(entry);
        }
    }

    REPEATING_ENTRIES
        .iter()
        .find(|entry| entry.contains(tag, private_creator))
}

/// Look up the raw DCMTK VR code for a tag (`px`, `ox`, `xs`, ...).
pub fn raw_vr_for_tag(tag: Tag) -> Option<&'static str> {
    lookup_entry(tag).map(|entry| entry.raw_vr)
}

/// Look up the best-effort standard VR for a tag.
///
/// This follows DCMTK's `DcmVR::getValidEVR()` policy for non-standard
/// dictionary VRs, e.g. `up -> UL`, `xs -> US`, `lt -> OW`, `ox/px -> OB`.
pub fn vr_for_tag(tag: Tag) -> Option<Vr> {
    lookup_entry(tag).map(|entry| entry.vr)
}

/// Well-known DICOM tag constants.
///
/// Ported from DCMTK's `dcdeftag.h`. Constants use SCREAMING_SNAKE_CASE
/// per Rust conventions. The tag name corresponds to the DICOM keyword.
#[allow(missing_docs)]
pub mod tags {
    use super::Tag;

    // ── File Meta Information (0002,xxxx) ─────────────────────────────
    pub const FILE_META_INFORMATION_GROUP_LENGTH: Tag = Tag::new(0x0002, 0x0000);
    pub const FILE_META_INFORMATION_VERSION: Tag = Tag::new(0x0002, 0x0001);
    pub const MEDIA_STORAGE_SOP_CLASS_UID: Tag = Tag::new(0x0002, 0x0002);
    pub const MEDIA_STORAGE_SOP_INSTANCE_UID: Tag = Tag::new(0x0002, 0x0003);
    pub const TRANSFER_SYNTAX_UID: Tag = Tag::new(0x0002, 0x0010);
    pub const IMPLEMENTATION_CLASS_UID: Tag = Tag::new(0x0002, 0x0012);
    pub const IMPLEMENTATION_VERSION_NAME: Tag = Tag::new(0x0002, 0x0013);
    pub const SOURCE_APPLICATION_ENTITY_TITLE: Tag = Tag::new(0x0002, 0x0016);
    pub const SENDING_APPLICATION_ENTITY_TITLE: Tag = Tag::new(0x0002, 0x0017);
    pub const RECEIVING_APPLICATION_ENTITY_TITLE: Tag = Tag::new(0x0002, 0x0018);
    pub const PRIVATE_INFORMATION_CREATOR_UID: Tag = Tag::new(0x0002, 0x0100);
    pub const PRIVATE_INFORMATION: Tag = Tag::new(0x0002, 0x0102);

    // ── Patient (0010,xxxx) ──────────────────────────────────────────
    pub const PATIENT_NAME: Tag = Tag::new(0x0010, 0x0010);
    pub const PATIENT_ID: Tag = Tag::new(0x0010, 0x0020);
    pub const PATIENT_BIRTH_DATE: Tag = Tag::new(0x0010, 0x0030);
    /// Issuer of Patient ID (0010,0021) — LO.
    pub const ISSUER_OF_PATIENT_ID: Tag = Tag::new(0x0010, 0x0021);
    pub const PATIENT_SEX: Tag = Tag::new(0x0010, 0x0040);
    pub const PATIENT_AGE: Tag = Tag::new(0x0010, 0x1010);
    pub const PATIENT_SIZE: Tag = Tag::new(0x0010, 0x1020);
    pub const PATIENT_WEIGHT: Tag = Tag::new(0x0010, 0x1030);

    // ── General Study (0008,xxxx) ────────────────────────────────────
    pub const SPECIFIC_CHARACTER_SET: Tag = Tag::new(0x0008, 0x0005);
    pub const IMAGE_TYPE: Tag = Tag::new(0x0008, 0x0008);
    pub const SOP_CLASS_UID: Tag = Tag::new(0x0008, 0x0016);
    pub const SOP_INSTANCE_UID: Tag = Tag::new(0x0008, 0x0018);
    pub const STUDY_DATE: Tag = Tag::new(0x0008, 0x0020);
    pub const SERIES_DATE: Tag = Tag::new(0x0008, 0x0021);
    pub const ACQUISITION_DATE: Tag = Tag::new(0x0008, 0x0022);
    pub const CONTENT_DATE: Tag = Tag::new(0x0008, 0x0023);
    pub const STUDY_TIME: Tag = Tag::new(0x0008, 0x0030);
    pub const SERIES_TIME: Tag = Tag::new(0x0008, 0x0031);
    pub const ACQUISITION_TIME: Tag = Tag::new(0x0008, 0x0032);
    pub const CONTENT_TIME: Tag = Tag::new(0x0008, 0x0033);
    pub const ACCESSION_NUMBER: Tag = Tag::new(0x0008, 0x0050);
    pub const QUERY_RETRIEVE_LEVEL: Tag = Tag::new(0x0008, 0x0052);
    pub const MODALITY: Tag = Tag::new(0x0008, 0x0060);
    /// Modalities in Study (0008,0061) — CS.
    pub const MODALITIES_IN_STUDY: Tag = Tag::new(0x0008, 0x0061);
    pub const MANUFACTURER: Tag = Tag::new(0x0008, 0x0070);
    pub const INSTITUTION_NAME: Tag = Tag::new(0x0008, 0x0080);
    pub const REFERRING_PHYSICIAN_NAME: Tag = Tag::new(0x0008, 0x0090);
    pub const STUDY_DESCRIPTION: Tag = Tag::new(0x0008, 0x1030);
    pub const SERIES_DESCRIPTION: Tag = Tag::new(0x0008, 0x103E);
    pub const PERFORMING_PHYSICIAN_NAME: Tag = Tag::new(0x0008, 0x1050);
    pub const OPERATORS_NAME: Tag = Tag::new(0x0008, 0x1070);
    pub const REFERENCED_SOP_SEQUENCE: Tag = Tag::new(0x0008, 0x1115);
    pub const REFERENCED_SOP_CLASS_UID: Tag = Tag::new(0x0008, 0x1150);
    pub const REFERENCED_SOP_INSTANCE_UID: Tag = Tag::new(0x0008, 0x1155);

    // ── Study (0020,xxxx) ────────────────────────────────────────────
    pub const STUDY_INSTANCE_UID: Tag = Tag::new(0x0020, 0x000D);
    pub const SERIES_INSTANCE_UID: Tag = Tag::new(0x0020, 0x000E);
    pub const STUDY_ID: Tag = Tag::new(0x0020, 0x0010);
    pub const SERIES_NUMBER: Tag = Tag::new(0x0020, 0x0011);
    pub const ACQUISITION_NUMBER: Tag = Tag::new(0x0020, 0x0012);
    pub const INSTANCE_NUMBER: Tag = Tag::new(0x0020, 0x0013);
    pub const IMAGE_POSITION_PATIENT: Tag = Tag::new(0x0020, 0x0032);
    pub const IMAGE_ORIENTATION_PATIENT: Tag = Tag::new(0x0020, 0x0037);
    pub const FRAME_OF_REFERENCE_UID: Tag = Tag::new(0x0020, 0x0052);
    pub const SLICE_LOCATION: Tag = Tag::new(0x0020, 0x1041);
    /// Number of Patient Related Studies (0020,1200) — IS.
    pub const NUMBER_OF_PATIENT_RELATED_STUDIES: Tag = Tag::new(0x0020, 0x1200);
    /// Number of Patient Related Series (0020,1202) — IS.
    pub const NUMBER_OF_PATIENT_RELATED_SERIES: Tag = Tag::new(0x0020, 0x1202);
    /// Number of Patient Related Instances (0020,1204) — IS.
    pub const NUMBER_OF_PATIENT_RELATED_INSTANCES: Tag = Tag::new(0x0020, 0x1204);
    /// Number of Study Related Series (0020,1206) — IS.
    pub const NUMBER_OF_STUDY_RELATED_SERIES: Tag = Tag::new(0x0020, 0x1206);
    /// Number of Study Related Instances (0020,1208) — IS.
    pub const NUMBER_OF_STUDY_RELATED_INSTANCES: Tag = Tag::new(0x0020, 0x1208);
    /// Number of Series Related Instances (0020,1209) — IS.
    pub const NUMBER_OF_SERIES_RELATED_INSTANCES: Tag = Tag::new(0x0020, 0x1209);
    pub const NUMBER_OF_FRAMES: Tag = Tag::new(0x0028, 0x0008);

    // ── Image Pixel Module (0028,xxxx) ───────────────────────────────
    pub const SAMPLES_PER_PIXEL: Tag = Tag::new(0x0028, 0x0002);
    pub const PHOTOMETRIC_INTERPRETATION: Tag = Tag::new(0x0028, 0x0004);
    pub const ROWS: Tag = Tag::new(0x0028, 0x0010);
    pub const COLUMNS: Tag = Tag::new(0x0028, 0x0011);
    pub const BITS_ALLOCATED: Tag = Tag::new(0x0028, 0x0100);
    pub const BITS_STORED: Tag = Tag::new(0x0028, 0x0101);
    pub const HIGH_BIT: Tag = Tag::new(0x0028, 0x0102);
    pub const PIXEL_REPRESENTATION: Tag = Tag::new(0x0028, 0x0103);
    pub const PLANAR_CONFIGURATION: Tag = Tag::new(0x0028, 0x0006);
    pub const LOSSY_IMAGE_COMPRESSION: Tag = Tag::new(0x0028, 0x2110);
    pub const PIXEL_DATA: Tag = Tag::new(0x7FE0, 0x0010);

    // ── Window / LUT ─────────────────────────────────────────────────
    pub const WINDOW_CENTER: Tag = Tag::new(0x0028, 0x1050);
    pub const WINDOW_WIDTH: Tag = Tag::new(0x0028, 0x1051);
    pub const RESCALE_INTERCEPT: Tag = Tag::new(0x0028, 0x1052);
    pub const RESCALE_SLOPE: Tag = Tag::new(0x0028, 0x1053);

    // ── Palette Color LUT (0028,1xxx / 0028,12xx) ───────────────────
    pub const RED_PALETTE_COLOR_LUT_DESCRIPTOR: Tag = Tag::new(0x0028, 0x1101);
    pub const GREEN_PALETTE_COLOR_LUT_DESCRIPTOR: Tag = Tag::new(0x0028, 0x1102);
    pub const BLUE_PALETTE_COLOR_LUT_DESCRIPTOR: Tag = Tag::new(0x0028, 0x1103);
    pub const RED_PALETTE_COLOR_LUT_DATA: Tag = Tag::new(0x0028, 0x1201);
    pub const GREEN_PALETTE_COLOR_LUT_DATA: Tag = Tag::new(0x0028, 0x1202);
    pub const BLUE_PALETTE_COLOR_LUT_DATA: Tag = Tag::new(0x0028, 0x1203);

    // ── Overlay (60xx,xxxx) ──────────────────────────────────────────
    pub const OVERLAY_ROWS: Tag = Tag::new(0x6000, 0x0010);
    pub const OVERLAY_COLUMNS: Tag = Tag::new(0x6000, 0x0011);
    pub const OVERLAY_ORIGIN: Tag = Tag::new(0x6000, 0x0050);
    pub const OVERLAY_DATA: Tag = Tag::new(0x6000, 0x3000);

    // ── DIMSE Command Elements (0000,xxxx) ───────────────────────────
    pub const COMMAND_GROUP_LENGTH: Tag = Tag::new(0x0000, 0x0000);
    pub const AFFECTED_SOP_CLASS_UID: Tag = Tag::new(0x0000, 0x0002);
    pub const REQUESTED_SOP_CLASS_UID: Tag = Tag::new(0x0000, 0x0003);
    pub const COMMAND_FIELD: Tag = Tag::new(0x0000, 0x0100);
    pub const MESSAGE_ID: Tag = Tag::new(0x0000, 0x0110);
    pub const MESSAGE_ID_BEING_RESPONDED_TO: Tag = Tag::new(0x0000, 0x0120);
    pub const MOVE_DESTINATION: Tag = Tag::new(0x0000, 0x0600);
    pub const PRIORITY: Tag = Tag::new(0x0000, 0x0700);
    pub const COMMAND_DATA_SET_TYPE: Tag = Tag::new(0x0000, 0x0800);
    pub const STATUS: Tag = Tag::new(0x0000, 0x0900);
    pub const AFFECTED_SOP_INSTANCE_UID: Tag = Tag::new(0x0000, 0x1000);
    pub const REQUESTED_SOP_INSTANCE_UID: Tag = Tag::new(0x0000, 0x1001);
    pub const NUMBER_OF_REMAINING_SUB_OPERATIONS: Tag = Tag::new(0x0000, 0x1020);
    pub const NUMBER_OF_COMPLETED_SUB_OPERATIONS: Tag = Tag::new(0x0000, 0x1021);
    pub const NUMBER_OF_FAILED_SUB_OPERATIONS: Tag = Tag::new(0x0000, 0x1022);
    pub const NUMBER_OF_WARNING_SUB_OPERATIONS: Tag = Tag::new(0x0000, 0x1023);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_display() {
        let tag = Tag::new(0x0010, 0x0010);
        assert_eq!(tag.to_string(), "(0010,0010)");
    }

    #[test]
    fn tag_ordering() {
        assert!(tags::PATIENT_NAME < tags::PATIENT_ID);
        assert!(tags::SOP_CLASS_UID < tags::PATIENT_NAME);
    }

    #[test]
    fn tag_u32_roundtrip() {
        let tag = Tag::new(0x0008, 0x0016);
        assert_eq!(Tag::from_u32(tag.to_u32()), tag);
    }

    #[test]
    fn private_tag_detection() {
        assert!(!Tag::new(0x0010, 0x0010).is_private());
        assert!(Tag::new(0x0011, 0x0010).is_private());
    }

    #[test]
    fn file_meta_detection() {
        assert!(tags::TRANSFER_SYNTAX_UID.is_file_meta());
        assert!(!tags::PATIENT_NAME.is_file_meta());
    }

    #[test]
    fn vr_lookup_resolves_query_tags() {
        assert_eq!(vr_for_tag(tags::QUERY_RETRIEVE_LEVEL), Some(Vr::CS));
        assert_eq!(vr_for_tag(tags::MODALITIES_IN_STUDY), Some(Vr::CS));
        assert_eq!(vr_for_tag(tags::ISSUER_OF_PATIENT_ID), Some(Vr::LO));
        assert_eq!(
            vr_for_tag(tags::NUMBER_OF_STUDY_RELATED_SERIES),
            Some(Vr::IS)
        );
        assert_eq!(
            vr_for_tag(tags::NUMBER_OF_STUDY_RELATED_INSTANCES),
            Some(Vr::IS)
        );
    }

    #[test]
    fn vr_lookup_returns_none_for_unknown_tags() {
        assert_eq!(vr_for_tag(Tag::new(0x9999, 0x9999)), None);
    }

    #[test]
    fn lookup_entry_matches_repeating_even_group_entries() {
        let overlay_rows = lookup_entry(Tag::new(0x6002, 0x0010)).expect("overlay rows entry");
        assert_eq!(overlay_rows.keyword, "OverlayRows");
        assert!(overlay_rows.is_repeating());
        assert_eq!(overlay_rows.group_restriction, RangeRestriction::Even);
        assert!(overlay_rows.contains(Tag::new(0x6002, 0x0010), None));
        assert!(!overlay_rows.contains(Tag::new(0x6001, 0x0010), None));

        let private_creator =
            lookup_entry(Tag::new(0x6001, 0x0010)).expect("generic private creator entry");
        assert_eq!(private_creator.keyword, "PrivateCreator");
    }

    #[test]
    fn lookup_entry_exposes_dcmtk_raw_vr_information() {
        let pixel_data = lookup_entry(tags::PIXEL_DATA).expect("pixel data entry");
        assert_eq!(pixel_data.raw_vr, "px");
        assert_eq!(pixel_data.vr, Vr::OB);

        let directory_offset =
            lookup_entry(Tag::new(0x0004, 0x1200)).expect("directory offset entry");
        assert_eq!(directory_offset.raw_vr, "up");
        assert_eq!(directory_offset.vr, Vr::UL);
        assert_eq!(raw_vr_for_tag(tags::PIXEL_DATA), Some("px"));
    }

    #[test]
    fn delimiter_tags() {
        assert!(ITEM.is_item());
        assert!(ITEM.is_delimiter());
        assert!(ITEM_DELIMITATION.is_item_delimitation());
        assert!(SEQUENCE_DELIMITATION.is_sequence_delimitation());
    }
}
