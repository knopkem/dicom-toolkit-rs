use std::path::PathBuf;
use std::process::Command;

use dicom_toolkit_core::uid::transfer_syntax;
use dicom_toolkit_data::value::{PixelData, Value};
use dicom_toolkit_data::{Element, FileFormat};
use dicom_toolkit_dict::{tags, Vr};
use tempfile::TempDir;

const TS_EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";
const TS_JPEGLS_LOSSLESS: &str = "1.2.840.10008.1.2.4.80";
const TS_JPEG2000_LOSSLESS: &str = "1.2.840.10008.1.2.4.90";
const TS_JPEG2000: &str = "1.2.840.10008.1.2.4.91";
const TS_HTJ2K_LOSSLESS: &str = transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY;
const TS_HTJ2K: &str = transfer_syntax::HIGH_THROUGHPUT_JPEG_2000;

fn fixture_path(file_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/testfiles")
        .join(file_name)
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

fn encapsulated_fragment_count(ff: &FileFormat) -> usize {
    match ff.dataset.get(tags::PIXEL_DATA) {
        Some(elem) => match &elem.value {
            Value::PixelData(PixelData::Encapsulated { fragments, .. }) => fragments.len(),
            other => panic!("expected encapsulated pixel data, got {other:?}"),
        },
        None => panic!("missing pixel data"),
    }
}

fn parsed_number_of_frames(ff: &FileFormat) -> Option<usize> {
    ff.dataset
        .get(tags::NUMBER_OF_FRAMES)
        .and_then(|elem| match &elem.value {
            Value::Ints(values) => values.first().copied().map(|n| n.max(1) as usize),
            Value::Strings(values) => values.first().and_then(|s| s.trim().parse::<usize>().ok()),
            Value::U16(values) => values.first().copied().map(usize::from),
            Value::U32(values) => values.first().and_then(|&n| usize::try_from(n).ok()),
            _ => None,
        })
}

fn native_pixel_vr(ff: &FileFormat) -> Vr {
    if ff.dataset.get_u16(tags::BITS_ALLOCATED).unwrap_or(8) > 8 {
        Vr::OW
    } else {
        Vr::OB
    }
}

fn normalized_pixels(ff: &FileFormat, bytes: &[u8]) -> Vec<u8> {
    let bit_depth = ff.dataset.get_u16(tags::BITS_STORED).unwrap_or(8);
    let mask = (1u32 << u32::from(bit_depth)) - 1;
    let mut normalized = Vec::with_capacity(bytes.len());
    if bit_depth <= 8 {
        for &byte in bytes {
            normalized.push((u32::from(byte) & mask) as u8);
        }
    } else {
        for chunk in bytes.chunks_exact(2) {
            let value = u32::from(u16::from_le_bytes([chunk[0], chunk[1]])) & mask;
            normalized.extend_from_slice(&(value as u16).to_le_bytes());
        }
    }
    normalized
}

fn load_fixture(file_name: &str) -> FileFormat {
    FileFormat::open(fixture_path(file_name)).expect("open DICOM fixture")
}

fn build_multiframe_fixture(
    file_names: &[&str],
    sop_instance_uid: &str,
    patient_name: &str,
) -> (FileFormat, Vec<u8>) {
    let mut ff = load_fixture(file_names[0]);
    let mut combined_pixels = Vec::new();

    for file_name in file_names {
        combined_pixels.extend_from_slice(&native_pixels(&load_fixture(file_name)));
    }

    ff.dataset.set_uid(tags::SOP_INSTANCE_UID, sop_instance_uid);
    ff.dataset
        .set_string(tags::PATIENT_NAME, Vr::PN, patient_name);
    ff.dataset.set_string(
        tags::NUMBER_OF_FRAMES,
        Vr::IS,
        &file_names.len().to_string(),
    );
    ff.dataset.insert(Element {
        tag: tags::PIXEL_DATA,
        vr: native_pixel_vr(&ff),
        value: Value::PixelData(PixelData::Native {
            bytes: combined_pixels.clone(),
        }),
    });

    (ff, combined_pixels)
}

#[test]
fn dcmcjp2k_and_dcmdjp2k_lossless_real_dicom_preserves_core_metadata() {
    let temp = TempDir::new().unwrap();
    let input = fixture_path("ABDOM_1.dcm");
    let compressed = temp.path().join("compressed_j2k.dcm");
    let roundtrip = temp.path().join("roundtrip.dcm");

    let original_ff = FileFormat::open(&input).unwrap();
    let original_pixels = native_pixels(&original_ff);

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
    assert_eq!(
        roundtrip_ff.dataset.get_u16(tags::ROWS),
        original_ff.dataset.get_u16(tags::ROWS)
    );
    assert_eq!(
        roundtrip_ff.dataset.get_u16(tags::COLUMNS),
        original_ff.dataset.get_u16(tags::COLUMNS)
    );
    assert_eq!(
        roundtrip_ff.dataset.get_u16(tags::BITS_ALLOCATED),
        original_ff.dataset.get_u16(tags::BITS_ALLOCATED)
    );
    assert_eq!(
        roundtrip_ff.dataset.get_u16(tags::BITS_STORED),
        original_ff.dataset.get_u16(tags::BITS_STORED)
    );
    assert_eq!(
        roundtrip_ff.dataset.get_u16(tags::SAMPLES_PER_PIXEL),
        original_ff.dataset.get_u16(tags::SAMPLES_PER_PIXEL)
    );
    assert_eq!(native_pixels(&roundtrip_ff).len(), original_pixels.len());
}

#[test]
fn dcmcjp2k_lossy_sets_transfer_syntax_and_flag_on_real_dicom() {
    let temp = TempDir::new().unwrap();
    let input = fixture_path("ABDOM_2.dcm");
    let compressed = temp.path().join("compressed_lossy_j2k.dcm");

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
    assert_eq!(encapsulated_fragment_count(&compressed_ff), 1);
}

#[test]
fn dcmcjp2k_htj2k_lossless_roundtrip_real_dicom() {
    let temp = TempDir::new().unwrap();
    let input = fixture_path("ABDOM_3.dcm");
    let compressed = temp.path().join("compressed_htj2k.dcm");
    let roundtrip = temp.path().join("roundtrip_htj2k.dcm");

    let original_ff = FileFormat::open(&input).unwrap();
    let original_pixels = native_pixels(&original_ff);

    let status = Command::new(env!("CARGO_BIN_EXE_dcmcjp2k"))
        .arg("--htj2k")
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
        TS_HTJ2K_LOSSLESS
    );
    assert_eq!(encapsulated_fragment_count(&compressed_ff), 1);

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
    assert_eq!(
        native_pixels(&roundtrip_ff),
        normalized_pixels(&original_ff, &original_pixels)
    );
}

#[test]
fn dcmcjp2k_htj2k_lossy_sets_transfer_syntax_and_flag_on_real_dicom() {
    let temp = TempDir::new().unwrap();
    let input = fixture_path("ABDOM_4.dcm");
    let compressed = temp.path().join("compressed_htj2k_lossy.dcm");

    let status = Command::new(env!("CARGO_BIN_EXE_dcmcjp2k"))
        .arg("--htj2k")
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
        TS_HTJ2K
    );
    assert_eq!(
        compressed_ff
            .dataset
            .get_string(tags::LOSSY_IMAGE_COMPRESSION),
        Some("01")
    );
    assert_eq!(encapsulated_fragment_count(&compressed_ff), 1);
}

#[test]
fn dcmcjpls_reencodes_real_htj2k_losslessly() {
    let temp = TempDir::new().unwrap();
    let input = fixture_path("ABDOM_5.dcm");
    let compressed_htj2k = temp.path().join("compressed_htj2k.dcm");
    let compressed_jls = temp.path().join("compressed_jls.dcm");
    let roundtrip = temp.path().join("roundtrip_jls.dcm");

    let original_ff = FileFormat::open(&input).unwrap();
    let original_pixels = native_pixels(&original_ff);
    let patient_name = original_ff.dataset.get_string(tags::PATIENT_NAME);

    let status = Command::new(env!("CARGO_BIN_EXE_dcmcjp2k"))
        .arg("--htj2k")
        .arg(&input)
        .arg(&compressed_htj2k)
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(env!("CARGO_BIN_EXE_dcmcjpls"))
        .arg(&compressed_htj2k)
        .arg(&compressed_jls)
        .status()
        .unwrap();
    assert!(status.success());

    let compressed_ff = FileFormat::open(&compressed_jls).unwrap();
    assert_eq!(
        compressed_ff
            .meta
            .transfer_syntax_uid
            .trim_end_matches('\0'),
        TS_JPEGLS_LOSSLESS
    );
    assert_eq!(
        compressed_ff.dataset.get_string(tags::PATIENT_NAME),
        patient_name
    );
    assert_eq!(encapsulated_fragment_count(&compressed_ff), 1);

    let status = Command::new(env!("CARGO_BIN_EXE_dcmdjpls"))
        .arg(&compressed_jls)
        .arg(&roundtrip)
        .status()
        .unwrap();
    assert!(status.success());

    let roundtrip_ff = FileFormat::open(&roundtrip).unwrap();
    assert_eq!(
        roundtrip_ff.meta.transfer_syntax_uid.trim_end_matches('\0'),
        TS_EXPLICIT_VR_LE
    );
    assert_eq!(
        roundtrip_ff.dataset.get_string(tags::PATIENT_NAME),
        patient_name
    );
    assert_eq!(
        native_pixels(&roundtrip_ff),
        normalized_pixels(&original_ff, &original_pixels)
    );
}

#[test]
fn dcmcjpls_reencodes_multiframe_real_htj2k_losslessly() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("input_multiframe.dcm");
    let compressed_htj2k = temp.path().join("compressed_multiframe_htj2k.dcm");
    let compressed_jls = temp.path().join("compressed_multiframe_jls.dcm");
    let roundtrip = temp.path().join("roundtrip_multiframe.dcm");

    let (ff, original_pixels) = build_multiframe_fixture(
        &["ABDOM_1.dcm", "ABDOM_2.dcm"],
        "1.2.3.4.5.6.7.8.11",
        "HTJ2K^Multiframe",
    );
    ff.save(&input).unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_dcmcjp2k"))
        .arg("--htj2k")
        .arg(&input)
        .arg(&compressed_htj2k)
        .status()
        .unwrap();
    assert!(status.success());

    let compressed_htj2k_ff = FileFormat::open(&compressed_htj2k).unwrap();
    assert_eq!(
        compressed_htj2k_ff
            .meta
            .transfer_syntax_uid
            .trim_end_matches('\0'),
        TS_HTJ2K_LOSSLESS
    );
    assert_eq!(parsed_number_of_frames(&compressed_htj2k_ff), Some(2));
    assert_eq!(
        compressed_htj2k_ff.dataset.get_string(tags::PATIENT_NAME),
        Some("HTJ2K^Multiframe")
    );
    assert_eq!(encapsulated_fragment_count(&compressed_htj2k_ff), 2);

    let status = Command::new(env!("CARGO_BIN_EXE_dcmcjpls"))
        .arg(&compressed_htj2k)
        .arg(&compressed_jls)
        .status()
        .unwrap();
    assert!(status.success());

    let compressed_jls_ff = FileFormat::open(&compressed_jls).unwrap();
    assert_eq!(
        compressed_jls_ff
            .meta
            .transfer_syntax_uid
            .trim_end_matches('\0'),
        TS_JPEGLS_LOSSLESS
    );
    assert_eq!(parsed_number_of_frames(&compressed_jls_ff), Some(2));
    assert_eq!(
        compressed_jls_ff.dataset.get_string(tags::PATIENT_NAME),
        Some("HTJ2K^Multiframe")
    );
    assert_eq!(encapsulated_fragment_count(&compressed_jls_ff), 2);

    let status = Command::new(env!("CARGO_BIN_EXE_dcmdjpls"))
        .arg(&compressed_jls)
        .arg(&roundtrip)
        .status()
        .unwrap();
    assert!(status.success());

    let roundtrip_ff = FileFormat::open(&roundtrip).unwrap();
    assert_eq!(
        roundtrip_ff.meta.transfer_syntax_uid.trim_end_matches('\0'),
        TS_EXPLICIT_VR_LE
    );
    assert_eq!(parsed_number_of_frames(&roundtrip_ff), Some(2));
    assert_eq!(
        roundtrip_ff.dataset.get_string(tags::PATIENT_NAME),
        Some("HTJ2K^Multiframe")
    );
    assert_eq!(
        native_pixels(&roundtrip_ff),
        normalized_pixels(&ff, &original_pixels)
    );
}
