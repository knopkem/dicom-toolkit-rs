//! Integration tests for character set encoding/decoding round-trips.
//!
//! Tests that DICOM files with non-ASCII character sets can be:
//! 1. Written with proper charset encoding
//! 2. Read back with correct charset decoding
//! 3. Round-tripped without data loss

use dicom_toolkit_data::meta_info::FileMetaInformation;
use dicom_toolkit_data::value::Value;
use dicom_toolkit_data::{DataSet, Element, FileFormat};
use dicom_toolkit_dict::{tags, Vr};
use std::path::PathBuf;

/// Build a minimal DICOM file with a given charset and patient name.
fn build_dicom_with_charset(charset: &str, patient_name: &str) -> FileFormat {
    let mut ds = DataSet::new();
    ds.set_string(tags::SPECIFIC_CHARACTER_SET, Vr::CS, charset);
    ds.set_string(tags::PATIENT_NAME, Vr::PN, patient_name);
    ds.set_string(tags::PATIENT_ID, Vr::LO, "TEST-001");

    let meta = FileMetaInformation::new(
        "1.2.3.4.5.6.7.8.9",
        "1.2.840.10008.5.1.4.1.1.2", // CT Image Storage
        "1.2.840.10008.1.2.1",       // Explicit VR Little Endian
    );
    FileFormat { meta, dataset: ds }
}

/// Temp file path for a test.
fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("dcmtk_charset_test_{name}.dcm"))
}

/// Write a FileFormat to a file and read it back.
fn roundtrip(ff: &FileFormat, name: &str) -> FileFormat {
    let path = temp_path(name);
    ff.save(&path).expect("write failed");
    let rt = FileFormat::open(&path).expect("read failed");
    let _ = std::fs::remove_file(&path);
    rt
}

#[test]
fn roundtrip_ascii_default() {
    let ff = build_dicom_with_charset("", "Doe^John");
    let rt = roundtrip(&ff, "ascii");
    assert_eq!(rt.dataset.get_string(tags::PATIENT_NAME), Some("Doe^John"));
}

#[test]
fn roundtrip_latin1_german() {
    let ff = build_dicom_with_charset("ISO_IR 100", "Müller^Jürgen");
    let rt = roundtrip(&ff, "latin1_german");
    assert_eq!(
        rt.dataset.get_string(tags::PATIENT_NAME),
        Some("Müller^Jürgen")
    );
}

#[test]
fn roundtrip_latin1_special_chars() {
    let ff = build_dicom_with_charset("ISO_IR 100", "Großmann^André");
    let rt = roundtrip(&ff, "latin1_special");
    assert_eq!(
        rt.dataset.get_string(tags::PATIENT_NAME),
        Some("Großmann^André")
    );
}

#[test]
fn roundtrip_utf8() {
    let ff = build_dicom_with_charset("ISO_IR 192", "田中^太郎");
    let rt = roundtrip(&ff, "utf8_japanese");
    assert_eq!(rt.dataset.get_string(tags::PATIENT_NAME), Some("田中^太郎"));
}

#[test]
fn roundtrip_utf8_cyrillic() {
    let ff = build_dicom_with_charset("ISO_IR 192", "Иванов^Иван");
    let rt = roundtrip(&ff, "utf8_cyrillic");
    assert_eq!(
        rt.dataset.get_string(tags::PATIENT_NAME),
        Some("Иванов^Иван")
    );
}

#[test]
fn roundtrip_utf8_mixed() {
    let ff = build_dicom_with_charset("ISO_IR 192", "Ñoño^José María");
    let rt = roundtrip(&ff, "utf8_mixed");
    assert_eq!(
        rt.dataset.get_string(tags::PATIENT_NAME),
        Some("Ñoño^José María")
    );
}

#[test]
fn roundtrip_latin2_polish() {
    let ff = build_dicom_with_charset("ISO_IR 101", "Łukasz^Żółć");
    let rt = roundtrip(&ff, "latin2_polish");
    assert_eq!(
        rt.dataset.get_string(tags::PATIENT_NAME),
        Some("Łukasz^Żółć")
    );
}

#[test]
fn roundtrip_latin9() {
    // ISO_IR 203 = Latin-9 (ISO 8859-15) — includes €, Œ, Ÿ
    let ff = build_dicom_with_charset("ISO_IR 203", "Œuvre^Ÿvan");
    let rt = roundtrip(&ff, "latin9");
    assert_eq!(
        rt.dataset.get_string(tags::PATIENT_NAME),
        Some("Œuvre^Ÿvan")
    );
}

#[test]
fn roundtrip_preserves_uid_ascii() {
    let ff = build_dicom_with_charset("ISO_IR 192", "田中^太郎");
    let rt = roundtrip(&ff, "uid_ascii");
    assert_eq!(
        rt.meta.media_storage_sop_instance_uid,
        ff.meta.media_storage_sop_instance_uid
    );
}

#[test]
fn roundtrip_multivalue_string_with_charset() {
    let mut ds = DataSet::new();
    ds.set_string(tags::SPECIFIC_CHARACTER_SET, Vr::CS, "ISO_IR 100");
    ds.insert(Element::new(
        tags::IMAGE_TYPE,
        Vr::CS,
        Value::Strings(vec![
            "ORIGINAL".to_string(),
            "PRIMARY".to_string(),
            "LOCALIZER".to_string(),
        ]),
    ));

    let meta = FileMetaInformation::new(
        "1.2.3.4.5.6.7.8.9",
        "1.2.840.10008.5.1.4.1.1.2",
        "1.2.840.10008.1.2.1",
    );
    let ff = FileFormat { meta, dataset: ds };
    let rt = roundtrip(&ff, "multivalue");
    match rt.dataset.get(tags::IMAGE_TYPE).map(|e| &e.value) {
        Some(Value::Strings(ss)) => {
            assert_eq!(ss, &["ORIGINAL", "PRIMARY", "LOCALIZER"]);
        }
        other => panic!("unexpected: {:?}", other),
    }
}

#[test]
fn latin1_bytes_are_not_utf8() {
    // Verify that the encoded bytes actually use ISO-8859-1 (not UTF-8)
    let ff = build_dicom_with_charset("ISO_IR 100", "Müller");
    let path = temp_path("latin1_raw");
    ff.save(&path).expect("write failed");
    let bytes = std::fs::read(&path).expect("read raw failed");
    let _ = std::fs::remove_file(&path);

    // In ISO-8859-1, 'ü' is a single byte 0xFC
    // In UTF-8, 'ü' is two bytes 0xC3 0xBC
    let has_latin1_u_umlaut = bytes.windows(1).any(|w| w[0] == 0xFC);
    let has_utf8_u_umlaut = bytes.windows(2).any(|w| w[0] == 0xC3 && w[1] == 0xBC);
    assert!(
        has_latin1_u_umlaut,
        "should contain Latin-1 encoded ü (0xFC)"
    );
    assert!(
        !has_utf8_u_umlaut,
        "should not contain UTF-8 encoded ü (0xC3 0xBC)"
    );
}
