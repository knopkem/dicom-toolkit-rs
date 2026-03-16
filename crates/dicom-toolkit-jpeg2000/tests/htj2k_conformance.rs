use dicom_toolkit_jpeg2000::{DecodeSettings, Image};

const HTJ2K_FIXTURE_SMALL: &[u8] = include_bytes!("fixtures/htj2k/ds0_ht_12_b11.j2k");
const PGX_REFERENCE_SMALL: &[u8] = include_bytes!("fixtures/htj2k/c1p0_12-0.pgx");
const HTJ2K_FIXTURE_LINE: &[u8] = include_bytes!("fixtures/htj2k/ds0_ht_11_b10.j2k");
const PGX_REFERENCE_LINE: &[u8] = include_bytes!("fixtures/htj2k/c1p0_11-0.pgx");

#[derive(Clone, Copy)]
struct FixtureCase {
    name: &'static str,
    codestream: &'static [u8],
    reference_pgx: &'static [u8],
}

#[derive(Debug)]
struct PgxReference {
    width: u32,
    height: u32,
    bit_depth: u8,
    pixels: Vec<u8>,
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

fn parse_pgx_reference(bytes: &[u8]) -> PgxReference {
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
        .parse::<u32>()
        .expect("PGX width parse");
    let height = parts
        .next()
        .expect("PGX height")
        .parse::<u32>()
        .expect("PGX height parse");
    assert_eq!(parts.next(), None);

    let pixels = bytes[header_end + 1..].to_vec();
    assert_eq!(pixels.len(), (width * height) as usize);

    PgxReference {
        width,
        height,
        bit_depth,
        pixels,
    }
}

#[test]
fn parses_real_htj2k_conformance_fixtures_in_strict_mode() {
    let settings = DecodeSettings {
        resolve_palette_indices: true,
        strict: true,
        target_resolution: None,
    };

    for case in FIXTURE_CASES {
        let image = Image::new(case.codestream, &settings).expect("parse HTJ2K fixture");
        let reference = parse_pgx_reference(case.reference_pgx);

        assert_eq!(image.width(), reference.width, "{}", case.name);
        assert_eq!(image.height(), reference.height, "{}", case.name);
        assert_eq!(
            image.original_bit_depth(),
            reference.bit_depth,
            "{}",
            case.name
        );
    }
}

#[test]
fn decodes_real_htj2k_conformance_fixtures() {
    let settings = DecodeSettings {
        resolve_palette_indices: true,
        strict: true,
        target_resolution: None,
    };

    for case in FIXTURE_CASES {
        let image = Image::new(case.codestream, &settings).expect("parse HTJ2K fixture");
        let reference = parse_pgx_reference(case.reference_pgx);

        let decoded = image.decode_native().expect("decode HTJ2K fixture");

        assert_eq!(decoded.width, reference.width, "{}", case.name);
        assert_eq!(decoded.height, reference.height, "{}", case.name);
        assert_eq!(decoded.bit_depth, reference.bit_depth, "{}", case.name);
        assert_eq!(decoded.num_components, 1, "{}", case.name);
        assert_eq!(decoded.bytes_per_sample, 1, "{}", case.name);
        assert_eq!(decoded.data, reference.pixels, "{}", case.name);
    }
}
