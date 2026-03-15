//! DICOM data element — a `(Tag, VR, Value)` triple.
//!
//! Ports DCMTK's `DcmElement` hierarchy into a single flat struct.

use std::fmt;
use dicom_toolkit_dict::{Tag, Vr};
use crate::dataset::DataSet;
use crate::value::Value;

/// A single DICOM data element.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub tag: Tag,
    pub vr: Vr,
    pub value: Value,
}

impl Element {
    // ── Constructors ──────────────────────────────────────────────────────────

    pub fn new(tag: Tag, vr: Vr, value: Value) -> Self {
        Self { tag, vr, value }
    }

    /// Single or multi-valued string element.
    pub fn string(tag: Tag, vr: Vr, s: &str) -> Self {
        Self::new(tag, vr, Value::Strings(vec![s.to_string()]))
    }

    /// Multi-valued string element from a slice.
    pub fn strings(tag: Tag, vr: Vr, values: &[&str]) -> Self {
        Self::new(tag, vr, Value::Strings(values.iter().map(|s| s.to_string()).collect()))
    }

    pub fn u16(tag: Tag, value: u16) -> Self {
        Self::new(tag, Vr::US, Value::U16(vec![value]))
    }

    pub fn u32(tag: Tag, value: u32) -> Self {
        Self::new(tag, Vr::UL, Value::U32(vec![value]))
    }

    pub fn i32(tag: Tag, value: i32) -> Self {
        Self::new(tag, Vr::SL, Value::I32(vec![value]))
    }

    pub fn f64(tag: Tag, value: f64) -> Self {
        Self::new(tag, Vr::FD, Value::F64(vec![value]))
    }

    pub fn bytes(tag: Tag, vr: Vr, data: Vec<u8>) -> Self {
        Self::new(tag, vr, Value::U8(data))
    }

    pub fn sequence(tag: Tag, items: Vec<DataSet>) -> Self {
        Self::new(tag, Vr::SQ, Value::Sequence(items))
    }

    pub fn uid(tag: Tag, uid: &str) -> Self {
        Self::new(tag, Vr::UI, Value::Uid(uid.to_string()))
    }

    // ── Getters ───────────────────────────────────────────────────────────────

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn string_value(&self) -> Option<&str> {
        self.value.as_string()
    }

    pub fn strings_value(&self) -> Option<&[String]> {
        self.value.as_strings()
    }

    pub fn u16_value(&self) -> Option<u16> {
        self.value.as_u16()
    }

    pub fn u32_value(&self) -> Option<u32> {
        self.value.as_u32()
    }

    pub fn i32_value(&self) -> Option<i32> {
        self.value.as_i32()
    }

    pub fn f64_value(&self) -> Option<f64> {
        self.value.as_f64()
    }

    pub fn bytes_value(&self) -> Option<&[u8]> {
        self.value.as_bytes()
    }

    pub fn items(&self) -> Option<&[DataSet]> {
        match &self.value {
            Value::Sequence(items) => Some(items.as_slice()),
            _ => None,
        }
    }
}

impl fmt::Display for Element {
    /// Format like dcmdump: `(GGGG,EEEE) VR [value] # length, multiplicity`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag_str = format!("({:04X},{:04X})", self.tag.group, self.tag.element);
        let vr_str = self.vr.code();
        let display_val = self.value.to_display_string();
        let length = self.value.encoded_len();
        let mult = self.value.multiplicity();

        // String VRs use [brackets]; numeric and binary VRs display bare.
        let value_part = if self.vr.is_string() || matches!(self.vr, Vr::UI | Vr::PN | Vr::DA | Vr::TM | Vr::DT | Vr::AT) {
            if display_val.is_empty() {
                "(no value available)".to_string()
            } else {
                format!("[{}]", display_val)
            }
        } else if matches!(self.value, Value::Sequence(_)) {
            display_val
        } else {
            display_val
        };

        write!(f, "{} {} {} # {}, {}", tag_str, vr_str, value_part, length, mult)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_dict::tags;

    #[test]
    fn element_string_roundtrip() {
        let e = Element::string(tags::PATIENT_NAME, Vr::PN, "Smith^John");
        assert_eq!(e.string_value(), Some("Smith^John"));
        assert!(!e.is_empty());
    }

    #[test]
    fn element_strings_roundtrip() {
        let e = Element::strings(tags::IMAGE_TYPE, Vr::CS, &["ORIGINAL", "PRIMARY"]);
        let s = e.strings_value().unwrap();
        assert_eq!(s, &["ORIGINAL", "PRIMARY"]);
        assert_eq!(e.value.multiplicity(), 2);
    }

    #[test]
    fn element_u16_roundtrip() {
        let e = Element::u16(tags::ROWS, 512);
        assert_eq!(e.u16_value(), Some(512));
        assert_eq!(e.vr, Vr::US);
    }

    #[test]
    fn element_u32_roundtrip() {
        let e = Element::u32(tags::STUDY_INSTANCE_UID, 999);
        // tag mismatch in real DICOM, but tests constructors
        assert_eq!(e.u32_value(), Some(999));
        assert_eq!(e.vr, Vr::UL);
    }

    #[test]
    fn element_i32_roundtrip() {
        let tag = Tag::new(0x0020, 0x0013);
        let e = Element::i32(tag, -42);
        assert_eq!(e.i32_value(), Some(-42));
        assert_eq!(e.vr, Vr::SL);
    }

    #[test]
    fn element_f64_roundtrip() {
        let tag = Tag::new(0x0018, 0x0050);
        let e = Element::f64(tag, 2.78);
        assert!((e.f64_value().unwrap() - 2.78).abs() < 1e-9);
        assert_eq!(e.vr, Vr::FD);
    }

    #[test]
    fn element_bytes_roundtrip() {
        let tag = Tag::new(0x0042, 0x0011);
        let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let e = Element::bytes(tag, Vr::OB, data.clone());
        assert_eq!(e.bytes_value(), Some(data.as_slice()));
    }

    #[test]
    fn element_uid_roundtrip() {
        let e = Element::uid(tags::SOP_CLASS_UID, "1.2.840.10008.1.1");
        assert_eq!(e.string_value(), Some("1.2.840.10008.1.1"));
        assert_eq!(e.vr, Vr::UI);
    }

    #[test]
    fn element_sequence_roundtrip() {
        let item = DataSet::new();
        let e = Element::sequence(Tag::new(0x0008, 0x1115), vec![item]);
        assert_eq!(e.items().unwrap().len(), 1);
        assert_eq!(e.vr, Vr::SQ);
    }

    #[test]
    fn element_display_string() {
        let e = Element::string(tags::PATIENT_NAME, Vr::PN, "Smith^John");
        let s = e.to_string();
        assert!(s.contains("(0010,0010)"), "tag not in display: {}", s);
        assert!(s.contains("PN"), "VR not in display: {}", s);
        assert!(s.contains("[Smith^John]"), "value not in display: {}", s);
        // # length, multiplicity
        assert!(s.contains("# "), "length marker missing: {}", s);
        assert!(s.contains(", 1"), "multiplicity not in display: {}", s);
    }

    #[test]
    fn element_display_u16() {
        let e = Element::u16(tags::ROWS, 512);
        let s = e.to_string();
        assert!(s.contains("(0028,0010)"), "tag not in display: {}", s);
        assert!(s.contains("US"), "VR not in display: {}", s);
        assert!(s.contains("512"), "value not in display: {}", s);
    }

    #[test]
    fn element_is_empty() {
        let e = Element::new(tags::PATIENT_NAME, Vr::PN, Value::Empty);
        assert!(e.is_empty());
    }
}
