use dicom_toolkit_jpeg2000::{DecodeSettings, Image};

const HTJ2K_FIXTURE: &[u8] = include_bytes!("fixtures/htj2k/ds0_ht_12_b11.j2k");
const PGX_REFERENCE: &[u8] = include_bytes!("fixtures/htj2k/c1p0_12-0.pgx");

fn parse_pgx_u8_reference(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let header_end = bytes
        .iter()
        .position(|&byte| byte == b'\n')
        .expect("PGX header terminator");
    let header = std::str::from_utf8(&bytes[..header_end]).expect("PGX header UTF-8");
    let mut parts = header.split_whitespace();

    assert_eq!(parts.next(), Some("PG"));
    assert_eq!(parts.next(), Some("ML"));
    assert_eq!(parts.next(), Some("+8"));

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

    let payload = bytes[header_end + 1..].to_vec();
    assert_eq!(payload.len(), (width * height) as usize);

    (width, height, payload)
}

#[test]
fn parses_real_htj2k_conformance_fixture_in_strict_mode() {
    let settings = DecodeSettings {
        resolve_palette_indices: true,
        strict: true,
        target_resolution: None,
    };

    let image = Image::new(HTJ2K_FIXTURE, &settings).expect("parse HTJ2K fixture");
    let (expected_width, expected_height, _) = parse_pgx_u8_reference(PGX_REFERENCE);

    assert_eq!(image.width(), expected_width);
    assert_eq!(image.height(), expected_height);
    assert_eq!(image.original_bit_depth(), 8);
}

#[test]
fn decodes_real_htj2k_conformance_fixture() {
    let settings = DecodeSettings {
        resolve_palette_indices: true,
        strict: true,
        target_resolution: None,
    };
    let image = Image::new(HTJ2K_FIXTURE, &settings).expect("parse HTJ2K fixture");
    let (expected_width, expected_height, expected_pixels) = parse_pgx_u8_reference(PGX_REFERENCE);

    assert_eq!(image.width(), expected_width);
    assert_eq!(image.height(), expected_height);
    assert_eq!(image.original_bit_depth(), 8);

    let decoded = image.decode_native().expect("decode HTJ2K fixture");

    assert_eq!(decoded.width, expected_width);
    assert_eq!(decoded.height, expected_height);
    assert_eq!(decoded.bit_depth, 8);
    assert_eq!(decoded.num_components, 1);
    assert_eq!(decoded.bytes_per_sample, 1);
    assert_eq!(decoded.data, expected_pixels);
}
