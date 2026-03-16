use std::process::Command;

use dicom_toolkit_core::uid::transfer_syntax;
use tempfile::TempDir;

use dicom_toolkit_data::value::{PixelData, Value};
use dicom_toolkit_data::{DataSet, Element, FileFormat};
use dicom_toolkit_dict::{tags, Vr};

const TS_EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";
const TS_JPEG2000_LOSSLESS: &str = "1.2.840.10008.1.2.4.90";
const TS_JPEG2000: &str = "1.2.840.10008.1.2.4.91";
const TS_HTJ2K_LOSSLESS: &str = transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY;
const HTJ2K_FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../dicom-toolkit-jpeg2000/tests/fixtures/htj2k/ds0_ht_12_b11.j2k"
));
const HTJ2K_PGX_REFERENCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../dicom-toolkit-jpeg2000/tests/fixtures/htj2k/c1p0_12-0.pgx"
));

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

fn make_test_dicom_htj2k_8bit(codestream: Vec<u8>, width: u16, height: u16) -> FileFormat {
    let mut ds = DataSet::new();
    ds.set_uid(tags::SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.1.2");
    ds.set_uid(tags::SOP_INSTANCE_UID, "1.2.3.4.5.6.7.8.9");
    ds.set_u16(tags::ROWS, height);
    ds.set_u16(tags::COLUMNS, width);
    ds.set_u16(tags::BITS_ALLOCATED, 8);
    ds.set_u16(tags::BITS_STORED, 8);
    ds.set_u16(tags::HIGH_BIT, 7);
    ds.set_u16(tags::PIXEL_REPRESENTATION, 0);
    ds.set_u16(tags::SAMPLES_PER_PIXEL, 1);
    ds.set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::CS, "MONOCHROME2");
    ds.insert(Element {
        tag: tags::PIXEL_DATA,
        vr: Vr::OB,
        value: Value::PixelData(PixelData::Encapsulated {
            offset_table: vec![],
            fragments: vec![codestream],
        }),
    });

    let mut ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", "1.2.3.4.5.6.7.8.9", ds);
    ff.meta.transfer_syntax_uid = TS_HTJ2K_LOSSLESS.to_string();
    ff
}

fn parse_pgx_u8_reference(bytes: &[u8]) -> (u16, u16, Vec<u8>) {
    let header_end = bytes
        .iter()
        .position(|&byte| byte == b'\n')
        .expect("PGX header terminator");
    let header = std::str::from_utf8(&bytes[..header_end]).expect("PGX header UTF-8");
    let mut parts = header.split_whitespace();

    assert_eq!(parts.next(), Some("PG"));
    let endianness = parts.next().expect("PGX byte order");
    assert!(matches!(endianness, "ML" | "LM"));
    let precision = parts.next().expect("PGX precision");
    assert!(
        !precision.starts_with('-'),
        "signed PGX references are not yet supported in this test helper"
    );
    let bit_depth = precision
        .trim_start_matches('+')
        .parse::<u8>()
        .expect("PGX precision parse");
    assert_eq!(bit_depth, 8);

    let width = parts
        .next()
        .expect("PGX width")
        .parse::<u16>()
        .expect("PGX width parse");
    let height = parts
        .next()
        .expect("PGX height")
        .parse::<u16>()
        .expect("PGX height parse");
    assert_eq!(parts.next(), None);

    let payload = bytes[header_end + 1..].to_vec();
    assert_eq!(payload.len(), width as usize * height as usize);

    (width, height, payload)
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

#[test]
fn dcmdjp2k_decompresses_htj2k_lossless_dicom() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("input_htj2k.dcm");
    let output = temp.path().join("output_htj2k.dcm");

    let (width, height, expected_pixels) = parse_pgx_u8_reference(HTJ2K_PGX_REFERENCE);
    let ff = make_test_dicom_htj2k_8bit(HTJ2K_FIXTURE.to_vec(), width, height);
    ff.save(&input).unwrap();

    let input_ff = FileFormat::open(&input).unwrap();
    assert_eq!(
        input_ff.meta.transfer_syntax_uid.trim_end_matches('\0'),
        TS_HTJ2K_LOSSLESS
    );

    let status = Command::new(env!("CARGO_BIN_EXE_dcmdjp2k"))
        .arg(&input)
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());

    let output_ff = FileFormat::open(&output).unwrap();
    assert_eq!(
        output_ff.meta.transfer_syntax_uid.trim_end_matches('\0'),
        TS_EXPLICIT_VR_LE
    );
    let output_pixels = native_pixels(&output_ff);
    assert_eq!(
        &output_pixels[..expected_pixels.len()],
        expected_pixels.as_slice()
    );
    assert!(output_pixels[expected_pixels.len()..]
        .iter()
        .all(|&byte| byte == 0));
}
