// Round trip encode / decode test for JPEG 2000 images

use dicom_toolkit_jpeg2000::{encode, encode_htj2k, DecodeSettings, EncodeOptions, Image};

use std::iter::zip;

struct ImageFixture {
    width: u32,
    height: u32,
    num_components: u8,
    bit_depth: u8,
    signed: bool,
    pixels: Vec<u8>,
}

fn make_fixture(width: u32, height: u32, num_components: u8) -> ImageFixture {
    let mut pixels = Vec::new();
    pixels.reserve(height as usize * width as usize * num_components as usize);
    for y in 0..height {
        for x in 0..width {
            for c in 0..num_components as u32 {
                let value = ((17 + x * 3 + y * 7 + c * 11) & 0xff) as u8;
                pixels.push(value);
            }
        }
    }
    let bit_depth = 8;
    let signed = false;
    ImageFixture {
        width,
        height,
        num_components,
        bit_depth,
        signed,
        pixels,
    }
}

type EncodeFn = fn(&[u8], u32, u32, u8, u8, bool, &EncodeOptions) -> Result<Vec<u8>, &'static str>;

fn roundtrip(encode_fn: EncodeFn, num_components: u8, width: u32, height: u32) {
    let im = make_fixture(width, height, num_components);

    let opts = &EncodeOptions {
        num_decomposition_levels: 1,
        reversible: true,
        use_ht_block_coding: false,
        guard_bits: 1,
        ..Default::default()
    }; // reversible = true (lossless)
    let cs = encode_fn(
        &im.pixels,
        im.width,
        im.height,
        im.num_components,
        im.bit_depth,
        im.signed,
        &opts,
    )
    .unwrap();
    let enc_image = Image::new(&cs, &DecodeSettings::default()).unwrap();
    let rt_pixels = enc_image.decode().unwrap();

    if im.pixels.len() < 20 {
        println!("Original pixels: {:?}", im.pixels);
        println!("Decoded pixels: {:?}", rt_pixels);
    }

    assert_eq!(im.width, enc_image.width());
    assert_eq!(im.height, enc_image.height());
    let num_comps_out =
        enc_image.color_space().num_channels() + if enc_image.has_alpha() { 1 } else { 0 };
    assert_eq!(im.num_components, num_comps_out);

    let maxdiff = zip(&im.pixels, &rt_pixels)
        .map(|(a, b)| (*a as i32 - *b as i32).abs())
        .max()
        .unwrap();

    assert_eq!(maxdiff, 0);
    return;
}

macro_rules! make_rt_testcase {
    ($encode_fn:expr, $num_components:expr, $size:expr, $testname:ident) => {
        #[test]
        fn $testname() {
            roundtrip($encode_fn, $num_components, $size, $size);
        }
    };
}

// The htj2k path doesn't support alpha channel yet.
// Single pixel roud trip tests are used to verify that the encoder and
// decoder can handle the smallest possible image.
const TEST_ONEPX_SIZE: u32 = 1;
make_rt_testcase!(encode, 1, TEST_ONEPX_SIZE, roundtrip_onepx_j2k_gray);
make_rt_testcase!(encode_htj2k, 1, TEST_ONEPX_SIZE, roundtrip_onepx_htj2k_gray);
make_rt_testcase!(encode, 2, TEST_ONEPX_SIZE, roundtrip_onepx_j2k_gray_alpha);
make_rt_testcase!(encode, 3, TEST_ONEPX_SIZE, roundtrip_onepx_j2k_rgb);
make_rt_testcase!(encode_htj2k, 3, TEST_ONEPX_SIZE, roundtrip_onepx_htj2k_rgb);
make_rt_testcase!(encode, 4, TEST_ONEPX_SIZE, roundtrip_onepx_j2k_rgba);

// Small images (2x2) are used to keep the test data small and simple.
const TEST_SMALL_SIZE: u32 = 2;
make_rt_testcase!(encode, 1, TEST_SMALL_SIZE, roundtrip_small_j2k_gray);
make_rt_testcase!(encode_htj2k, 1, TEST_SMALL_SIZE, roundtrip_small_htj2k_gray);
make_rt_testcase!(encode, 2, TEST_SMALL_SIZE, roundtrip_small_j2k_gray_alpha);
make_rt_testcase!(encode, 3, TEST_SMALL_SIZE, roundtrip_small_j2k_rgb);
make_rt_testcase!(encode_htj2k, 3, TEST_SMALL_SIZE, roundtrip_small_htj2k_rgb);
make_rt_testcase!(encode, 4, TEST_SMALL_SIZE, roundtrip_small_j2k_rgba);

const TEST_MEDIUM_SIZE: u32 = 64;
make_rt_testcase!(encode, 1, TEST_MEDIUM_SIZE, roundtrip_medium_j2k_gray);
make_rt_testcase!(
    encode_htj2k,
    1,
    TEST_MEDIUM_SIZE,
    roundtrip_medium_htj2k_gray
);
make_rt_testcase!(encode, 2, TEST_MEDIUM_SIZE, roundtrip_medium_j2k_gray_alpha);
make_rt_testcase!(encode, 3, TEST_MEDIUM_SIZE, roundtrip_medium_j2k_rgb);
make_rt_testcase!(
    encode_htj2k,
    3,
    TEST_MEDIUM_SIZE,
    roundtrip_medium_htj2k_rgb
);
make_rt_testcase!(encode, 4, TEST_MEDIUM_SIZE, roundtrip_medium_j2k_rgba);

const TEST_LARGE_SIZE: u32 = 223;
make_rt_testcase!(encode, 1, TEST_LARGE_SIZE, roundtrip_large_j2k_gray);
make_rt_testcase!(encode_htj2k, 1, TEST_LARGE_SIZE, roundtrip_large_htj2k_gray);
make_rt_testcase!(encode, 2, TEST_LARGE_SIZE, roundtrip_large_j2k_gray_alpha);
make_rt_testcase!(encode, 3, TEST_LARGE_SIZE, roundtrip_large_j2k_rgb);
make_rt_testcase!(encode_htj2k, 3, TEST_LARGE_SIZE, roundtrip_large_htj2k_rgb);
make_rt_testcase!(encode, 4, TEST_LARGE_SIZE, roundtrip_large_j2k_rgba);

// test decode from a known fixture, to verify that the decoder can handle a real JPEG 2000 file.
#[test]
fn test_decode() {
    let im = ImageFixture {
        width: 2,
        height: 2,
        num_components: 3,
        bit_depth: 8,
        signed: false,
        pixels: vec![
            0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ],
    };

    // Generated with OpenJPEG 2.5.0, using the following command:
    // opj_compress -i ./test_2x2.raw -o test_2x2.j2k -F 2,2,3,8,u -n 1
    // Note that OpenJPEG wants planar color, so the input raw file is in planar format.
    // echo -ne '\xff\x00\x00\x00\xff\x00\x00\x00\xff\x00\x00\x00' > test_2x2.raw
    let cs = include_bytes!("./fixtures/c3_2x2.j2k");
    let enc_image = Image::new(cs, &DecodeSettings::default()).unwrap();
    let rt_pixels = enc_image.decode().unwrap();

    println!("Original pixels: {:?}", im.pixels);
    println!("Decoded pixels: {:?}", rt_pixels);
    assert_eq!(im.width, enc_image.width());
    assert_eq!(im.height, enc_image.height());
    assert_eq!(im.num_components, enc_image.color_space().num_channels());

    let maxdiff = zip(&im.pixels, &rt_pixels)
        .map(|(a, b)| (*a as i32 - *b as i32).abs())
        .max()
        .unwrap();

    assert_eq!(maxdiff, 0);
    return;
}
