use std::process::Command;

use tempfile::TempDir;

use dicom_toolkit_data::value::{PixelData, Value};
use dicom_toolkit_data::{DataSet, Element, FileFormat};
use dicom_toolkit_dict::{tags, Vr};

const TS_EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";
const TS_JPEG2000_LOSSLESS: &str = "1.2.840.10008.1.2.4.90";
const TS_JPEG2000: &str = "1.2.840.10008.1.2.4.91";

fn make_test_dicom_8bit(pixels: Vec<u8>) -> FileFormat {
    let mut ds = DataSet::new();
    ds.set_uid(tags::SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.1.2");
    ds.set_uid(tags::SOP_INSTANCE_UID, "1.2.3.4.5.6.7.8");
    ds.set_u16(tags::ROWS, 8);
    ds.set_u16(tags::COLUMNS, 8);
    ds.set_u16(tags::BITS_ALLOCATED, 8);
    ds.set_u16(tags::BITS_STORED, 8);
    ds.set_u16(tags::HIGH_BIT, 7);
    ds.set_u16(tags::PIXEL_REPRESENTATION, 0);
    ds.set_u16(tags::SAMPLES_PER_PIXEL, 1);
    ds.set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::CS, "MONOCHROME2");
    ds.insert(Element {
        tag: tags::PIXEL_DATA,
        vr: Vr::OB,
        value: Value::PixelData(PixelData::Native { bytes: pixels }),
    });
    FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", "1.2.3.4.5.6.7.8", ds)
}

fn native_pixels(ff: &FileFormat) -> Vec<u8> {
    match ff.dataset.get(tags::PIXEL_DATA) {
        Some(elem) => match &elem.value {
            Value::PixelData(PixelData::Native { bytes }) => bytes.clone(),
            other => panic!("expected native pixel data, got {other:?}"),
        },
        None => panic!("missing pixel data"),
    }
}

#[test]
fn dcmcjp2k_and_dcmdjp2k_lossless_roundtrip() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("input.dcm");
    let compressed = temp.path().join("compressed_j2k.dcm");
    let roundtrip = temp.path().join("roundtrip.dcm");

    let original: Vec<u8> = (0u8..64).collect();
    let ff = make_test_dicom_8bit(original.clone());
    ff.save(&input).unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_dcmcjp2k"))
        .arg(&input)
        .arg(&compressed)
        .status()
        .unwrap();
    assert!(status.success());

    let compressed_ff = FileFormat::open(&compressed).unwrap();
    assert_eq!(
        compressed_ff
            .meta
            .transfer_syntax_uid
            .trim_end_matches('\0'),
        TS_JPEG2000_LOSSLESS
    );

    let status = Command::new(env!("CARGO_BIN_EXE_dcmdjp2k"))
        .arg(&compressed)
        .arg(&roundtrip)
        .status()
        .unwrap();
    assert!(status.success());

    let roundtrip_ff = FileFormat::open(&roundtrip).unwrap();
    assert_eq!(
        roundtrip_ff.meta.transfer_syntax_uid.trim_end_matches('\0'),
        TS_EXPLICIT_VR_LE
    );
    assert_eq!(native_pixels(&roundtrip_ff), original);
}

#[test]
fn dcmcjp2k_lossy_sets_transfer_syntax_and_flag() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("input.dcm");
    let compressed = temp.path().join("compressed_lossy_j2k.dcm");

    let original: Vec<u8> = (0u8..64).rev().collect();
    let ff = make_test_dicom_8bit(original);
    ff.save(&input).unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_dcmcjp2k"))
        .arg("--encode-lossy")
        .arg(&input)
        .arg(&compressed)
        .status()
        .unwrap();
    assert!(status.success());

    let compressed_ff = FileFormat::open(&compressed).unwrap();
    assert_eq!(
        compressed_ff
            .meta
            .transfer_syntax_uid
            .trim_end_matches('\0'),
        TS_JPEG2000
    );
    assert_eq!(
        compressed_ff
            .dataset
            .get_string(tags::LOSSY_IMAGE_COMPRESSION),
        Some("01")
    );
    match compressed_ff.dataset.get(tags::PIXEL_DATA) {
        Some(elem) => match &elem.value {
            Value::PixelData(PixelData::Encapsulated { fragments, .. }) => {
                assert_eq!(fragments.len(), 1);
                assert!(!fragments[0].is_empty());
            }
            other => panic!("expected encapsulated pixel data, got {other:?}"),
        },
        None => panic!("missing pixel data"),
    }
}
