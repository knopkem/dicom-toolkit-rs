use dicom_toolkit_codec::registry::{decode_pixel_data, GLOBAL_REGISTRY};
use dicom_toolkit_core::uid::transfer_syntax;
use dicom_toolkit_data::value::PixelData;

const HTJ2K_FIXTURE_SMALL: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../dicom-toolkit-jpeg2000/tests/fixtures/htj2k/ds0_ht_12_b11.j2k"
));
const PGX_REFERENCE_SMALL: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../dicom-toolkit-jpeg2000/tests/fixtures/htj2k/c1p0_12-0.pgx"
));
const HTJ2K_FIXTURE_LINE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../dicom-toolkit-jpeg2000/tests/fixtures/htj2k/ds0_ht_11_b10.j2k"
));
const PGX_REFERENCE_LINE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../dicom-toolkit-jpeg2000/tests/fixtures/htj2k/c1p0_11-0.pgx"
));

#[derive(Clone, Copy)]
struct FixtureCase {
    name: &'static str,
    codestream: &'static [u8],
    reference_pgx: &'static [u8],
}

const FIXTURE_CASES: [FixtureCase; 2] = [
    FixtureCase {
        name: "ds0_ht_12_b11",
        codestream: HTJ2K_FIXTURE_SMALL,
        reference_pgx: PGX_REFERENCE_SMALL,
    },
    FixtureCase {
        name: "ds0_ht_11_b10",
        codestream: HTJ2K_FIXTURE_LINE,
        reference_pgx: PGX_REFERENCE_LINE,
    },
];

fn parse_pgx_u8_reference(bytes: &[u8]) -> (u16, u16, u8, Vec<u8>) {
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
    assert!(
        bit_depth <= 8,
        "only <=8-bit PGX references are supported in this test helper"
    );

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

    (width, height, bit_depth, payload)
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
fn decodes_real_htj2k_fixtures_via_registry_decoder_lookup() {
    let codec = GLOBAL_REGISTRY
        .find_decoder(transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY)
        .expect("HTJ2K decoder registration");

    for case in FIXTURE_CASES {
        let (width, height, bit_depth, expected_pixels) =
            parse_pgx_u8_reference(case.reference_pgx);
        let pixel_data = PixelData::Encapsulated {
            offset_table: vec![],
            fragments: vec![case.codestream.to_vec()],
        };

        let decoded = codec
            .decode(&pixel_data, height, width, 1, bit_depth)
            .expect("HTJ2K registry decode");

        assert_eq!(decoded, expected_pixels, "{}", case.name);
    }
}

#[test]
fn decodes_real_htj2k_fixtures_via_flat_decode_api() {
    for case in FIXTURE_CASES {
        let (width, height, bit_depth, expected_pixels) =
            parse_pgx_u8_reference(case.reference_pgx);

        let decoded = decode_pixel_data(
            transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY,
            case.codestream,
            height,
            width,
            u16::from(bit_depth),
            1,
        )
        .expect("flat HTJ2K decode");

        assert_eq!(decoded, expected_pixels, "{}", case.name);
    }
}

#[test]
fn encodes_htj2k_lossless_via_registry_lookup() {
    let rows = 4u16;
    let cols = 4u16;
    let samples = 1u8;
    let bits_allocated = 16u8;
    let bits_stored = 12u8;
    let mut pixels = Vec::with_capacity(32);
    for _ in 0..16 {
        pixels.extend_from_slice(&2048u16.to_le_bytes());
    }

    for ts_uid in [
        transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY,
        transfer_syntax::HIGH_THROUGHPUT_JPEG_2000,
    ] {
        let codec = GLOBAL_REGISTRY
            .find(ts_uid)
            .expect("HTJ2K encoder registration");
        let encoded = codec
            .encode(&pixels, rows, cols, samples, bits_allocated, bits_stored)
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

        let decoded = decode_pixel_data(
            ts_uid,
            &fragment,
            rows,
            cols,
            u16::from(bits_allocated),
            u16::from(samples),
        )
        .expect("flat HTJ2K decode");
        assert_eq!(decoded, pixels, "{ts_uid}");
    }
}
