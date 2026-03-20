use std::path::PathBuf;

use dicom_toolkit_codec::registry::{decode_pixel_data, GLOBAL_REGISTRY};
use dicom_toolkit_core::uid::transfer_syntax;
use dicom_toolkit_data::value::{PixelData, Value};
use dicom_toolkit_data::FileFormat;
use dicom_toolkit_dict::tags;

#[derive(Clone, Copy)]
struct FixtureCase {
    file_name: &'static str,
}

#[derive(Debug)]
struct DicomFixture {
    rows: u16,
    cols: u16,
    bits_allocated: u16,
    bits_stored: u16,
    samples_per_pixel: u8,
    pixels: Vec<u8>,
}

const FIXTURE_CASES: [FixtureCase; 2] = [
    FixtureCase {
        file_name: "ABDOM_1.dcm",
    },
    FixtureCase {
        file_name: "ABDOM_2.dcm",
    },
];

fn fixture_path(file_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/testfiles")
        .join(file_name)
}

fn load_fixture(case: FixtureCase) -> DicomFixture {
    let ff = FileFormat::open(fixture_path(case.file_name)).expect("open DICOM fixture");
    let ds = &ff.dataset;
    let pixels = match ds.get(tags::PIXEL_DATA) {
        Some(elem) => match &elem.value {
            Value::PixelData(PixelData::Native { bytes }) => bytes.clone(),
            other => panic!("expected native pixel data, got {other:?}"),
        },
        None => panic!("missing pixel data"),
    };

    DicomFixture {
        rows: ds.get_u16(tags::ROWS).expect("Rows"),
        cols: ds.get_u16(tags::COLUMNS).expect("Columns"),
        bits_allocated: ds.get_u16(tags::BITS_ALLOCATED).expect("BitsAllocated"),
        bits_stored: ds.get_u16(tags::BITS_STORED).expect("BitsStored"),
        samples_per_pixel: ds.get_u16(tags::SAMPLES_PER_PIXEL).unwrap_or(1) as u8,
        pixels,
    }
}

fn normalized_pixels(bytes: &[u8], bit_depth: u16) -> Vec<u8> {
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

fn ht_marker_is_present(codestream: &[u8]) -> bool {
    codestream.windows(2).any(|window| window == [0xFF, 0x50])
}

fn ht_cod_flag_is_set(codestream: &[u8]) -> bool {
    codestream
        .windows(2)
        .position(|window| window == [0xFF, 0x52])
        .map(|cod_offset| codestream[cod_offset + 12] == 0x40)
        .unwrap_or(false)
}

#[test]
fn encodes_and_decodes_real_dicom_pixels_via_registry_lookup() {
    for ts_uid in [
        transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY,
        transfer_syntax::HIGH_THROUGHPUT_JPEG_2000,
    ] {
        let codec = GLOBAL_REGISTRY
            .find(ts_uid)
            .expect("HTJ2K codec registration");

        for case in FIXTURE_CASES {
            let fixture = load_fixture(case);
            let encoded = codec
                .encode(
                    &fixture.pixels,
                    fixture.rows,
                    fixture.cols,
                    fixture.samples_per_pixel,
                    fixture.bits_allocated as u8,
                    fixture.bits_stored as u8,
                )
                .expect("HTJ2K registry encode");
            let fragment = match encoded {
                PixelData::Encapsulated { fragments, .. } => {
                    assert_eq!(fragments.len(), 1);
                    fragments.into_iter().next().unwrap()
                }
                PixelData::Native { .. } => panic!("expected encapsulated pixel data"),
            };

            assert!(ht_marker_is_present(&fragment), "{ts_uid}");
            assert!(ht_cod_flag_is_set(&fragment), "{ts_uid}");

            let pixel_data = PixelData::Encapsulated {
                offset_table: vec![],
                fragments: vec![fragment],
            };
            let decoded = codec
                .decode(
                    &pixel_data,
                    fixture.rows,
                    fixture.cols,
                    fixture.samples_per_pixel,
                    fixture.bits_allocated as u8,
                )
                .expect("HTJ2K registry decode");

            assert_eq!(
                decoded,
                normalized_pixels(&fixture.pixels, fixture.bits_stored),
                "{ts_uid} {}",
                case.file_name
            );
        }
    }
}

#[test]
fn decodes_real_dicom_pixels_via_flat_api_after_htj2k_encode() {
    let fixture = load_fixture(FIXTURE_CASES[0]);
    let codec = GLOBAL_REGISTRY
        .find(transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY)
        .expect("HTJ2K codec registration");
    let encoded = codec
        .encode(
            &fixture.pixels,
            fixture.rows,
            fixture.cols,
            fixture.samples_per_pixel,
            fixture.bits_allocated as u8,
            fixture.bits_stored as u8,
        )
        .expect("HTJ2K registry encode");
    let fragment = match encoded {
        PixelData::Encapsulated { fragments, .. } => fragments.into_iter().next().unwrap(),
        PixelData::Native { .. } => panic!("expected encapsulated pixel data"),
    };

    let decoded = decode_pixel_data(
        transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY,
        &fragment,
        fixture.rows,
        fixture.cols,
        fixture.bits_allocated,
        u16::from(fixture.samples_per_pixel),
    )
    .expect("flat HTJ2K decode");

    assert_eq!(
        decoded,
        normalized_pixels(&fixture.pixels, fixture.bits_stored)
    );
}
