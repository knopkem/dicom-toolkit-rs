//! DICOM dataset — an ordered map of `Tag → Element`.
//!
//! Ports DCMTK's `DcmDataset` / `DcmItem`. Elements are kept in ascending
//! tag order, matching the DICOM requirement for encoded files.

use indexmap::IndexMap;
use dicom_toolkit_dict::{Tag, Vr};
use dicom_toolkit_core::error::{DcmError, DcmResult};
use crate::element::Element;
use crate::value::Value;

/// A DICOM dataset: an ordered collection of data elements.
///
/// Internally backed by an `IndexMap` that is kept sorted by tag.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataSet {
    elements: IndexMap<Tag, Element>,
}

impl DataSet {
    pub fn new() -> Self {
        Self { elements: IndexMap::new() }
    }

    // ── Core map operations ───────────────────────────────────────────────────

    /// Insert an element, maintaining ascending tag order.
    pub fn insert(&mut self, element: Element) {
        self.elements.insert(element.tag, element);
        self.elements.sort_unstable_keys();
    }

    pub fn get(&self, tag: Tag) -> Option<&Element> {
        self.elements.get(&tag)
    }

    pub fn get_mut(&mut self, tag: Tag) -> Option<&mut Element> {
        self.elements.get_mut(&tag)
    }

    pub fn remove(&mut self, tag: Tag) -> Option<Element> {
        self.elements.swap_remove(&tag)
    }

    pub fn contains(&self, tag: Tag) -> bool {
        self.elements.contains_key(&tag)
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Tag, &Element)> {
        self.elements.iter()
    }

    pub fn tags(&self) -> impl Iterator<Item = Tag> + '_ {
        self.elements.keys().copied()
    }

    /// Return the element for `tag`, or a [`DcmError::UnknownTag`] if absent.
    pub fn find_element(&self, tag: Tag) -> DcmResult<&Element> {
        self.elements.get(&tag).ok_or_else(|| DcmError::UnknownTag {
            group: tag.group,
            element: tag.element,
        })
    }

    // ── Convenience getters ───────────────────────────────────────────────────

    pub fn get_string(&self, tag: Tag) -> Option<&str> {
        self.get(tag)?.string_value()
    }

    pub fn get_strings(&self, tag: Tag) -> Option<&[String]> {
        self.get(tag)?.strings_value()
    }

    pub fn get_u16(&self, tag: Tag) -> Option<u16> {
        self.get(tag)?.u16_value()
    }

    pub fn get_u32(&self, tag: Tag) -> Option<u32> {
        self.get(tag)?.u32_value()
    }

    pub fn get_i32(&self, tag: Tag) -> Option<i32> {
        self.get(tag)?.i32_value()
    }

    pub fn get_f64(&self, tag: Tag) -> Option<f64> {
        self.get(tag)?.f64_value()
    }

    pub fn get_bytes(&self, tag: Tag) -> Option<&[u8]> {
        self.get(tag)?.bytes_value()
    }

    pub fn get_items(&self, tag: Tag) -> Option<&[DataSet]> {
        self.get(tag)?.items()
    }

    // ── Convenience setters ───────────────────────────────────────────────────

    pub fn set_string(&mut self, tag: Tag, vr: Vr, value: &str) {
        self.insert(Element::string(tag, vr, value));
    }

    pub fn set_strings(&mut self, tag: Tag, vr: Vr, values: Vec<String>) {
        self.insert(Element::new(tag, vr, Value::Strings(values)));
    }

    pub fn set_u16(&mut self, tag: Tag, value: u16) {
        self.insert(Element::u16(tag, value));
    }

    pub fn set_u32(&mut self, tag: Tag, value: u32) {
        self.insert(Element::u32(tag, value));
    }

    pub fn set_i32(&mut self, tag: Tag, value: i32) {
        self.insert(Element::i32(tag, value));
    }

    pub fn set_f64(&mut self, tag: Tag, value: f64) {
        self.insert(Element::f64(tag, value));
    }

    pub fn set_bytes(&mut self, tag: Tag, vr: Vr, data: Vec<u8>) {
        self.insert(Element::bytes(tag, vr, data));
    }

    pub fn set_sequence(&mut self, tag: Tag, items: Vec<DataSet>) {
        self.insert(Element::sequence(tag, items));
    }

    pub fn set_uid(&mut self, tag: Tag, uid: &str) {
        self.insert(Element::uid(tag, uid));
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_dict::tags;

    #[test]
    fn dataset_insert_and_get() {
        let mut ds = DataSet::new();
        ds.set_u16(tags::ROWS, 512);
        assert_eq!(ds.get_u16(tags::ROWS), Some(512));
    }

    #[test]
    fn dataset_contains_remove() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_NAME, Vr::PN, "Smith^John");
        assert!(ds.contains(tags::PATIENT_NAME));
        let removed = ds.remove(tags::PATIENT_NAME).unwrap();
        assert_eq!(removed.string_value(), Some("Smith^John"));
        assert!(!ds.contains(tags::PATIENT_NAME));
    }

    #[test]
    fn dataset_len_is_empty() {
        let mut ds = DataSet::new();
        assert!(ds.is_empty());
        assert_eq!(ds.len(), 0);
        ds.set_u16(tags::ROWS, 1);
        assert!(!ds.is_empty());
        assert_eq!(ds.len(), 1);
    }

    #[test]
    fn dataset_tag_order_ascending() {
        // Insert in reverse order; tags() should return in ascending order.
        let mut ds = DataSet::new();
        ds.set_u16(tags::COLUMNS, 256);   // (0028,0011)
        ds.set_u16(tags::ROWS, 512);       // (0028,0010)
        ds.set_string(tags::PATIENT_NAME, Vr::PN, "Doe^Jane"); // (0010,0010)

        let tags: Vec<Tag> = ds.tags().collect();
        assert!(tags.windows(2).all(|w| w[0] < w[1]), "tags not in order: {:?}", tags);
    }

    #[test]
    fn dataset_convenience_getters() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_ID, Vr::LO, "PID001");
        ds.set_strings(tags::IMAGE_TYPE, Vr::CS, vec!["ORIGINAL".into(), "PRIMARY".into()]);
        ds.set_u16(tags::ROWS, 512);
        ds.set_u32(Tag::new(0x0028, 0x0000), 42);
        ds.set_i32(Tag::new(0x0020, 0x0013), -1);
        ds.set_f64(Tag::new(0x0028, 0x1050), 1024.0);
        ds.set_uid(tags::SOP_CLASS_UID, "1.2.840.10008.1.1");

        assert_eq!(ds.get_string(tags::PATIENT_ID), Some("PID001"));
        assert_eq!(ds.get_strings(tags::IMAGE_TYPE).unwrap().len(), 2);
        assert_eq!(ds.get_u16(tags::ROWS), Some(512));
        assert_eq!(ds.get_u32(Tag::new(0x0028, 0x0000)), Some(42));
        assert_eq!(ds.get_i32(Tag::new(0x0020, 0x0013)), Some(-1));
        assert!((ds.get_f64(Tag::new(0x0028, 0x1050)).unwrap() - 1024.0).abs() < 1e-9);
        assert_eq!(ds.get_string(tags::SOP_CLASS_UID), Some("1.2.840.10008.1.1"));
    }

    #[test]
    fn dataset_set_bytes() {
        let mut ds = DataSet::new();
        let data = vec![0u8, 1, 2, 3];
        ds.set_bytes(Tag::new(0x0042, 0x0011), Vr::OB, data.clone());
        assert_eq!(ds.get_bytes(Tag::new(0x0042, 0x0011)), Some(data.as_slice()));
    }

    #[test]
    fn dataset_nested_sequence() {
        let mut item = DataSet::new();
        item.set_string(tags::PATIENT_NAME, Vr::PN, "Jones^Bob");

        let mut ds = DataSet::new();
        ds.set_sequence(Tag::new(0x0008, 0x1115), vec![item]);

        let items = ds.get_items(Tag::new(0x0008, 0x1115)).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get_string(tags::PATIENT_NAME), Some("Jones^Bob"));
    }

    #[test]
    fn dataset_find_element_ok() {
        let mut ds = DataSet::new();
        ds.set_u16(tags::ROWS, 512);
        assert!(ds.find_element(tags::ROWS).is_ok());
    }

    #[test]
    fn dataset_find_element_not_found() {
        let ds = DataSet::new();
        let err = ds.find_element(tags::ROWS).unwrap_err();
        // Should be UnknownTag
        assert!(matches!(err, DcmError::UnknownTag { .. }));
    }

    #[test]
    fn dataset_iter() {
        let mut ds = DataSet::new();
        ds.set_u16(tags::ROWS, 512);
        ds.set_u16(tags::COLUMNS, 256);
        let count = ds.iter().count();
        assert_eq!(count, 2);
    }

    #[test]
    fn dataset_overwrite() {
        let mut ds = DataSet::new();
        ds.set_u16(tags::ROWS, 512);
        ds.set_u16(tags::ROWS, 1024);
        assert_eq!(ds.len(), 1);
        assert_eq!(ds.get_u16(tags::ROWS), Some(1024));
    }
}
