use dicom_toolkit_codec::registry::{decode_pixel_data, GLOBAL_REGISTRY};
use dicom_toolkit_core::uid::transfer_syntax;
use dicom_toolkit_data::value::PixelData;

const HTJ2K_FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../dicom-toolkit-jpeg2000/tests/fixtures/htj2k/ds0_ht_12_b11.j2k"
));
const PGX_REFERENCE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../dicom-toolkit-jpeg2000/tests/fixtures/htj2k/c1p0_12-0.pgx"
));

fn parse_pgx_u8_reference(bytes: &[u8]) -> (u16, u16, Vec<u8>) {
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

#[test]
fn decodes_real_htj2k_via_registry_decoder_lookup() {
    let (width, height, expected_pixels) = parse_pgx_u8_reference(PGX_REFERENCE);
    let pixel_data = PixelData::Encapsulated {
        offset_table: vec![],
        fragments: vec![HTJ2K_FIXTURE.to_vec()],
    };

    let codec = GLOBAL_REGISTRY
        .find_decoder(transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY)
        .expect("HTJ2K decoder registration");
    let decoded = codec
        .decode(&pixel_data, height, width, 1, 8)
        .expect("HTJ2K registry decode");

    assert_eq!(decoded, expected_pixels);
}

#[test]
fn decodes_real_htj2k_via_flat_decode_api() {
    let (width, height, expected_pixels) = parse_pgx_u8_reference(PGX_REFERENCE);

    let decoded = decode_pixel_data(
        transfer_syntax::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY,
        HTJ2K_FIXTURE,
        height,
        width,
        8,
        1,
    )
    .expect("flat HTJ2K decode");

    assert_eq!(decoded, expected_pixels);
}
