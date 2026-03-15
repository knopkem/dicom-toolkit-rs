//! Workspace-level integration tests: DICOM file I/O roundtrip.

use tempfile::NamedTempFile;

use dicom_toolkit_data::{DataSet, Element, FileFormat};
use dicom_toolkit_dict::{Vr, tags};

// ── File roundtrip ────────────────────────────────────────────────────────────

#[test]
fn roundtrip_dicom_file() {
    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_NAME, Vr::PN, "Test^Patient");
    ds.set_string(tags::PATIENT_ID, Vr::LO, "TEST-001");
    ds.set_uid(tags::SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.1.2");
    ds.set_uid(tags::SOP_INSTANCE_UID, "1.2.3.4.5.6.7");
    ds.set_u16(tags::ROWS, 128);
    ds.set_u16(tags::COLUMNS, 128);

    let ff = FileFormat::from_dataset(
        "1.2.840.10008.5.1.4.1.1.2",
        "1.2.3.4.5.6.7",
        ds,
    );

    let tmp = NamedTempFile::new().unwrap();
    ff.save(tmp.path()).unwrap();

    let ff2 = FileFormat::open(tmp.path()).unwrap();

    assert_eq!(ff2.dataset.get_string(tags::PATIENT_NAME), Some("Test^Patient"));
    assert_eq!(ff2.dataset.get_string(tags::PATIENT_ID), Some("TEST-001"));
    assert_eq!(ff2.dataset.get_u16(tags::ROWS), Some(128));
    assert_eq!(ff2.dataset.get_u16(tags::COLUMNS), Some(128));
    assert_eq!(
        ff2.dataset.get_string(tags::SOP_INSTANCE_UID),
        Some("1.2.3.4.5.6.7")
    );

    // Meta information should be preserved
    assert_eq!(
        ff2.meta.media_storage_sop_class_uid.trim_end_matches('\0'),
        "1.2.840.10008.5.1.4.1.1.2"
    );
    assert_eq!(
        ff2.meta.transfer_syntax_uid.trim_end_matches('\0'),
        "1.2.840.10008.1.2.1"
    );
}

#[test]
fn roundtrip_dicom_file_with_sequence() {
    let mut item = DataSet::new();
    item.insert(Element::uid(
        tags::REFERENCED_SOP_CLASS_UID,
        "1.2.840.10008.5.1.4.1.1.2",
    ));
    item.insert(Element::uid(tags::REFERENCED_SOP_INSTANCE_UID, "1.2.3"));

    let mut ds = DataSet::new();
    ds.insert(Element::sequence(tags::REFERENCED_SOP_SEQUENCE, vec![item]));

    let ff = FileFormat::from_dataset("", "1.2.3", ds);
    let tmp = NamedTempFile::new().unwrap();
    ff.save(tmp.path()).unwrap();

    let ff2 = FileFormat::open(tmp.path()).unwrap();
    let items = ff2
        .dataset
        .get_items(tags::REFERENCED_SOP_SEQUENCE)
        .expect("sequence should be present");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].get_string(tags::REFERENCED_SOP_CLASS_UID),
        Some("1.2.840.10008.5.1.4.1.1.2")
    );
}

#[test]
fn roundtrip_multiple_numeric_vrs() {
    let mut ds = DataSet::new();
    ds.set_u16(tags::ROWS, 512);
    ds.set_u16(tags::COLUMNS, 256);
    ds.set_u16(tags::BITS_ALLOCATED, 16);
    ds.set_u16(tags::BITS_STORED, 12);
    ds.set_u16(tags::HIGH_BIT, 11);
    ds.set_u16(tags::PIXEL_REPRESENTATION, 0);

    let ff = FileFormat::from_dataset(
        "1.2.840.10008.5.1.4.1.1.2",
        "9.8.7.6.5.4.3.2.1",
        ds,
    );
    let tmp = NamedTempFile::new().unwrap();
    ff.save(tmp.path()).unwrap();

    let ff2 = FileFormat::open(tmp.path()).unwrap();
    assert_eq!(ff2.dataset.get_u16(tags::ROWS), Some(512));
    assert_eq!(ff2.dataset.get_u16(tags::COLUMNS), Some(256));
    assert_eq!(ff2.dataset.get_u16(tags::BITS_ALLOCATED), Some(16));
    assert_eq!(ff2.dataset.get_u16(tags::BITS_STORED), Some(12));
    assert_eq!(ff2.dataset.get_u16(tags::HIGH_BIT), Some(11));
    assert_eq!(ff2.dataset.get_u16(tags::PIXEL_REPRESENTATION), Some(0));
}

// ── JSON roundtrip ────────────────────────────────────────────────────────────

#[test]
fn roundtrip_json_via_dataset() {
    use dicom_toolkit_data::json;

    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_NAME, Vr::PN, "JSON^Test");
    ds.set_u16(tags::ROWS, 64);
    ds.set_string(tags::PATIENT_ID, Vr::LO, "JSON-002");
    ds.set_uid(tags::SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.1.4");

    let json_str = json::to_json(&ds).unwrap();
    assert!(!json_str.is_empty());

    let parsed = json::from_json(&json_str).unwrap();

    assert_eq!(parsed.get_string(tags::PATIENT_NAME), Some("JSON^Test"));
    assert_eq!(parsed.get_u16(tags::ROWS), Some(64));
    assert_eq!(parsed.get_string(tags::PATIENT_ID), Some("JSON-002"));
    assert_eq!(
        parsed.get_string(tags::SOP_CLASS_UID),
        Some("1.2.840.10008.5.1.4.1.1.4")
    );
}

#[test]
fn roundtrip_json_pretty() {
    use dicom_toolkit_data::json;

    let mut ds = DataSet::new();
    ds.set_string(tags::MODALITY, Vr::CS, "CT");
    ds.set_u16(tags::SAMPLES_PER_PIXEL, 1);

    let pretty = json::to_json_pretty(&ds).unwrap();
    assert!(pretty.contains('\n'), "pretty JSON should be multi-line");

    let parsed = json::from_json(&pretty).unwrap();
    assert_eq!(parsed.get_string(tags::MODALITY), Some("CT"));
    assert_eq!(parsed.get_u16(tags::SAMPLES_PER_PIXEL), Some(1));
}

// ── Writer/Reader roundtrip via memory ────────────────────────────────────────

#[test]
fn roundtrip_dataset_in_memory_explicit_vr_le() {
    use dicom_toolkit_data::{DicomReader, DicomWriter};

    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_NAME, Vr::PN, "Memory^Test");
    ds.set_u16(tags::ROWS, 32);
    ds.set_uid(tags::SOP_INSTANCE_UID, "5.6.7.8.9");

    let mut buf = Vec::new();
    DicomWriter::new(&mut buf)
        .write_dataset(&ds, "1.2.840.10008.1.2.1")
        .unwrap();

    let ds2 = DicomReader::new(buf.as_slice())
        .read_dataset("1.2.840.10008.1.2.1")
        .unwrap();

    assert_eq!(ds2.get_string(tags::PATIENT_NAME), Some("Memory^Test"));
    assert_eq!(ds2.get_u16(tags::ROWS), Some(32));
    assert_eq!(ds2.get_string(tags::SOP_INSTANCE_UID), Some("5.6.7.8.9"));
}

#[test]
fn roundtrip_file_format_in_memory() {
    use dicom_toolkit_data::{DicomReader, DicomWriter};

    let mut ds = DataSet::new();
    ds.set_string(tags::STUDY_DESCRIPTION, Vr::LO, "Test Study");
    ds.set_u16(tags::SERIES_NUMBER, 1);

    let ff = FileFormat::from_dataset(
        "1.2.840.10008.5.1.4.1.1.2",
        "1.1.1.1.1",
        ds,
    );

    let mut buf = Vec::new();
    DicomWriter::new(&mut buf).write_file(&ff).unwrap();

    let ff2 = DicomReader::new(buf.as_slice()).read_file().unwrap();
    assert_eq!(ff2.dataset.get_string(tags::STUDY_DESCRIPTION), Some("Test Study"));
    assert_eq!(ff2.dataset.get_u16(tags::SERIES_NUMBER), Some(1));
    assert_eq!(
        ff2.meta.transfer_syntax_uid.trim_end_matches('\0'),
        "1.2.840.10008.1.2.1"
    );
}
