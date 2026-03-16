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
