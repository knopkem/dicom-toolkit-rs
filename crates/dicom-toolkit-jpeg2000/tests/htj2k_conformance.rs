use std::path::PathBuf;

use dicom_toolkit_data::value::{PixelData, Value};
use dicom_toolkit_data::FileFormat;
use dicom_toolkit_dict::tags;
use dicom_toolkit_jpeg2000::{encode_htj2k, DecodeSettings, EncodeOptions, Image};

#[derive(Clone, Copy)]
struct FixtureCase {
    name: &'static str,
    file_name: &'static str,
}

#[derive(Debug)]
struct DicomFixture {
    width: u32,
    height: u32,
    bit_depth: u8,
    signed: bool,
    num_components: u8,
    pixels: Vec<u8>,
}

const FIXTURE_CASES: [FixtureCase; 2] = [
    FixtureCase {
        name: "ABDOM_1",
        file_name: "ABDOM_1.dcm",
    },
    FixtureCase {
        name: "ABDOM_3",
        file_name: "ABDOM_3.dcm",
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
        width: u32::from(ds.get_u16(tags::COLUMNS).expect("Columns")),
        height: u32::from(ds.get_u16(tags::ROWS).expect("Rows")),
        bit_depth: ds.get_u16(tags::BITS_STORED).expect("BitsStored") as u8,
        signed: ds.get_u16(tags::PIXEL_REPRESENTATION).unwrap_or(0) != 0,
        num_components: ds.get_u16(tags::SAMPLES_PER_PIXEL).unwrap_or(1) as u8,
        pixels,
    }
}

fn encode_fixture(case: FixtureCase, reversible: bool) -> (DicomFixture, Vec<u8>) {
    let fixture = load_fixture(case);
    let options = EncodeOptions {
        reversible,
        ..EncodeOptions::default()
    };
    let encoded = encode_htj2k(
        &fixture.pixels,
        fixture.width,
        fixture.height,
        fixture.num_components,
        fixture.bit_depth,
        fixture.signed,
        &options,
    )
    .expect("encode HTJ2K");
    (fixture, encoded)
}

fn normalized_pixels(bytes: &[u8], bit_depth: u8, signed: bool) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(bytes.len());
    let mask = ((1u32 << u32::from(bit_depth)) - 1) as u16;
    if bit_depth <= 8 {
        for &byte in bytes {
            let value = u16::from(byte) & mask;
            if signed {
                normalized.push(((value as i16) << (8 - bit_depth)) as i8 as u8);
            } else {
                normalized.push(value as u8);
            }
        }
    } else {
        for chunk in bytes.chunks_exact(2) {
            let value = u16::from_le_bytes([chunk[0], chunk[1]]) & mask;
            if signed {
                let shift = 16 - u32::from(bit_depth);
                let signed_value = ((u32::from(value) << shift) as i32 >> shift) as i16;
                normalized.extend_from_slice(&signed_value.to_le_bytes());
            } else {
                normalized.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    normalized
}

#[test]
fn parses_real_dicom_htj2k_codestreams_in_strict_mode() {
    let settings = DecodeSettings {
        resolve_palette_indices: true,
        strict: true,
        target_resolution: None,
    };

    for case in FIXTURE_CASES {
        let (fixture, encoded) = encode_fixture(case, true);
        let image = Image::new(&encoded, &settings).expect("parse HTJ2K codestream");

        assert_eq!(image.width(), fixture.width, "{}", case.name);
        assert_eq!(image.height(), fixture.height, "{}", case.name);
        assert_eq!(
            image.original_bit_depth(),
            fixture.bit_depth,
            "{}",
            case.name
        );
    }
}

#[test]
fn roundtrips_real_dicom_pixels_through_lossless_htj2k() {
    let settings = DecodeSettings::default();

    for case in FIXTURE_CASES {
        let (fixture, encoded) = encode_fixture(case, true);
        assert!(encoded.windows(2).any(|window| window == [0xFF, 0x50]));

        let image = Image::new(&encoded, &settings).expect("parse HTJ2K codestream");
        let decoded = image.decode_native().expect("decode HTJ2K codestream");

        assert_eq!(decoded.width, fixture.width, "{}", case.name);
        assert_eq!(decoded.height, fixture.height, "{}", case.name);
        assert_eq!(decoded.bit_depth, fixture.bit_depth, "{}", case.name);
        assert_eq!(
            decoded.num_components, fixture.num_components,
            "{}",
            case.name
        );
        assert_eq!(
            decoded.data,
            normalized_pixels(&fixture.pixels, fixture.bit_depth, fixture.signed),
            "{}",
            case.name
        );
    }
}

#[test]
fn decodes_real_dicom_pixels_from_lossy_htj2k_with_stable_metadata() {
    let settings = DecodeSettings::default();
    let (fixture, encoded) = encode_fixture(FIXTURE_CASES[0], false);
    assert!(encoded.windows(2).any(|window| window == [0xFF, 0x50]));

    let image = Image::new(&encoded, &settings).expect("parse lossy HTJ2K codestream");
    let decoded = image
        .decode_native()
        .expect("decode lossy HTJ2K codestream");

    assert_eq!(decoded.width, fixture.width);
    assert_eq!(decoded.height, fixture.height);
    assert_eq!(decoded.bit_depth, fixture.bit_depth);
    assert_eq!(decoded.num_components, fixture.num_components);
}
