//! Top-level JPEG 2000 encode orchestration.
//!
//! Coordinates the full encoding pipeline:
//!   pixels → MCT → DWT → quantize → EBCOT T1 → T2 → codestream
//!
//! Supports both lossless (5-3 reversible) and lossy (9-7 irreversible) encoding.

use alloc::vec;
use alloc::vec::Vec;

use super::bitplane_encode;
use super::build::SubBandType;
use super::codestream_write::{self, BlockCodingMode, EncodeParams};
use super::fdwt::{self, DwtDecomposition};
use super::forward_mct;
use super::packet_encode::{self, CodeBlockPacketData, ResolutionPacket, SubbandPrecinct};
use super::quantize::{self, QuantStepSize};

/// Encoding options for JPEG 2000.
#[derive(Debug, Clone)]
pub struct EncodeOptions {
    /// Number of decomposition levels (default: 5).
    pub num_decomposition_levels: u8,
    /// Use reversible (lossless) transform (default: true).
    pub reversible: bool,
    /// Code-block width exponent minus 2 (default: 4, meaning 2^6=64).
    pub code_block_width_exp: u8,
    /// Code-block height exponent minus 2 (default: 4, meaning 2^6=64).
    pub code_block_height_exp: u8,
    /// Number of guard bits (default: 1 for reversible, 2 for irreversible).
    pub guard_bits: u8,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            num_decomposition_levels: 5,
            reversible: true,
            code_block_width_exp: 4,
            code_block_height_exp: 4,
            guard_bits: 1,
        }
    }
}

/// Encode pixel data into a JPEG 2000 codestream.
///
/// # Arguments
/// * `pixels` — Raw pixel data. For 8-bit: one byte per sample. For >8-bit: two bytes per sample (little-endian u16).
/// * `width` — Image width in pixels.
/// * `height` — Image height in pixels.
/// * `num_components` — Number of components (1 for grayscale, 3 for RGB).
/// * `bit_depth` — Bits per sample (e.g., 8, 12, 16).
/// * `signed` — Whether samples are signed.
/// * `options` — Encoding parameters.
///
/// # Returns
/// The encoded JPEG 2000 codestream bytes (`.j2c` format).
pub fn encode(
    pixels: &[u8],
    width: u32,
    height: u32,
    num_components: u8,
    bit_depth: u8,
    signed: bool,
    options: &EncodeOptions,
) -> Result<Vec<u8>, &'static str> {
    encode_impl(
        pixels,
        width,
        height,
        num_components,
        bit_depth,
        signed,
        options,
        BlockCodingMode::Classic,
    )
}

fn encode_impl(
    pixels: &[u8],
    width: u32,
    height: u32,
    num_components: u8,
    bit_depth: u8,
    signed: bool,
    options: &EncodeOptions,
    block_coding_mode: BlockCodingMode,
) -> Result<Vec<u8>, &'static str> {
    if width == 0 || height == 0 {
        return Err("invalid dimensions");
    }
    if num_components == 0 || num_components > 4 {
        return Err("unsupported component count");
    }
    if bit_depth == 0 || bit_depth > 16 {
        return Err("unsupported bit depth");
    }

    let num_pixels = (width * height) as usize;
    let bytes_per_sample = if bit_depth <= 8 { 1 } else { 2 };
    let expected_len = num_pixels * num_components as usize * bytes_per_sample;
    if pixels.len() < expected_len {
        return Err("pixel data too short");
    }

    // Step 1: Convert pixel bytes to f32 component arrays
    let mut components = deinterleave_to_f32(pixels, num_pixels, num_components, bit_depth, signed);

    // Step 2: Apply forward MCT if RGB with 3+ components
    let use_mct = num_components >= 3;
    if use_mct {
        if options.reversible {
            forward_mct::forward_rct(&mut components);
        } else {
            forward_mct::forward_ict(&mut components);
        }
    }

    // Step 3: Apply forward DWT to each component
    let num_levels = options.num_decomposition_levels.min(
        // Don't decompose more than the image supports
        max_decomposition_levels(width, height),
    );

    let decompositions: Vec<DwtDecomposition> = components
        .iter()
        .map(|comp| fdwt::forward_dwt(comp, width, height, num_levels, options.reversible))
        .collect();

    // Step 4: Compute quantization step sizes
    let guard_bits = if options.reversible {
        options.guard_bits
    } else {
        options.guard_bits.max(2)
    };

    let step_sizes =
        quantize::compute_step_sizes(bit_depth, num_levels, options.reversible, guard_bits);

    // Step 5: Quantize and encode code-blocks for each component
    let cb_width = 1u32 << (options.code_block_width_exp + 2);
    let cb_height = 1u32 << (options.code_block_height_exp + 2);

    let mut resolution_packets: Vec<ResolutionPacket> = Vec::new();

    for decomp in decompositions.iter().take(num_components as usize) {
        // LL subband (resolution 0)
        let ll_subband = encode_subband(
            &decomp.ll,
            decomp.ll_width,
            decomp.ll_height,
            &step_sizes[0],
            guard_bits,
            options.reversible,
            block_coding_mode,
            cb_width,
            cb_height,
            SubBandType::LowLow,
        )?;
        resolution_packets.push(ResolutionPacket {
            subbands: vec![ll_subband],
        });

        // Higher resolution levels
        for (level_idx, level) in decomp.levels.iter().enumerate() {
            let step_base = 1 + level_idx * 3;

            // HL subband
            let hl_subband = encode_subband(
                &level.hl,
                level.high_width,
                level.low_height,
                &step_sizes[step_base],
                guard_bits,
                options.reversible,
                block_coding_mode,
                cb_width,
                cb_height,
                SubBandType::HighLow,
            )?;

            // LH subband
            let lh_subband = encode_subband(
                &level.lh,
                level.low_width,
                level.high_height,
                &step_sizes[step_base + 1],
                guard_bits,
                options.reversible,
                block_coding_mode,
                cb_width,
                cb_height,
                SubBandType::LowHigh,
            )?;

            // HH subband
            let hh_subband = encode_subband(
                &level.hh,
                level.high_width,
                level.high_height,
                &step_sizes[step_base + 2],
                guard_bits,
                options.reversible,
                block_coding_mode,
                cb_width,
                cb_height,
                SubBandType::HighHigh,
            )?;

            resolution_packets.push(ResolutionPacket {
                subbands: vec![hl_subband, lh_subband, hh_subband],
            });
        }
    }

    // Step 6: Form tile bitstream (T2)
    let tile_data = packet_encode::form_tile_bitstream(&mut resolution_packets, 1, num_components);

    // Step 7: Write codestream
    let quant_params: Vec<(u16, u16)> = step_sizes
        .iter()
        .map(|s| (s.exponent, s.mantissa))
        .collect();

    let params = EncodeParams {
        width,
        height,
        num_components,
        bit_depth,
        signed,
        num_decomposition_levels: num_levels,
        reversible: options.reversible,
        code_block_width_exp: options.code_block_width_exp,
        code_block_height_exp: options.code_block_height_exp,
        num_layers: 1,
        use_mct,
        guard_bits,
        block_coding_mode,
    };

    Ok(codestream_write::write_codestream(
        &params,
        &tile_data,
        &quant_params,
    ))
}

/// Encode a single subband into a SubbandPrecinct.
fn encode_subband(
    coefficients: &[f32],
    width: u32,
    height: u32,
    step_size: &QuantStepSize,
    guard_bits: u8,
    reversible: bool,
    block_coding_mode: BlockCodingMode,
    cb_width: u32,
    cb_height: u32,
    sub_band_type: SubBandType,
) -> Result<SubbandPrecinct, &'static str> {
    if width == 0 || height == 0 {
        return Ok(SubbandPrecinct {
            code_blocks: Vec::new(),
            num_cbs_x: 0,
            num_cbs_y: 0,
        });
    }

    // Quantize
    let quantized = quantize::quantize_subband(coefficients, step_size, guard_bits, reversible);
    debug_assert!(step_size.exponent <= u16::from(u8::MAX));
    let total_bitplanes = guard_bits
        .saturating_add(step_size.exponent as u8)
        .saturating_sub(1);

    // Split into code-blocks
    let num_cbs_x = width.div_ceil(cb_width);
    let num_cbs_y = height.div_ceil(cb_height);
    let mut code_blocks = Vec::with_capacity((num_cbs_x * num_cbs_y) as usize);

    for cby in 0..num_cbs_y {
        for cbx in 0..num_cbs_x {
            let x0 = cbx * cb_width;
            let y0 = cby * cb_height;
            let x1 = (x0 + cb_width).min(width);
            let y1 = (y0 + cb_height).min(height);
            let cbw = x1 - x0;
            let cbh = y1 - y0;

            // Extract code-block coefficients
            let mut cb_coeffs = vec![0i32; (cbw * cbh) as usize];
            for y in 0..cbh {
                for x in 0..cbw {
                    cb_coeffs[(y * cbw + x) as usize] =
                        quantized[((y0 + y) * width + (x0 + x)) as usize];
                }
            }

            // Encode
            if block_coding_mode == BlockCodingMode::HighThroughput
                && cb_coeffs.iter().any(|&coefficient| coefficient != 0)
            {
                return Err("HTJ2K block encoding not implemented");
            }

            let encoded = bitplane_encode::encode_code_block(
                &cb_coeffs,
                cbw,
                cbh,
                sub_band_type,
                total_bitplanes,
            );

            code_blocks.push(CodeBlockPacketData {
                data: encoded.data,
                num_coding_passes: encoded.num_coding_passes,
                num_zero_bitplanes: encoded.num_zero_bitplanes,
                previously_included: false,
                l_block: 3,
            });
        }
    }

    Ok(SubbandPrecinct {
        code_blocks,
        num_cbs_x,
        num_cbs_y,
    })
}

/// Convert interleaved pixel bytes to per-component f32 arrays.
fn deinterleave_to_f32(
    pixels: &[u8],
    num_pixels: usize,
    num_components: u8,
    bit_depth: u8,
    signed: bool,
) -> Vec<Vec<f32>> {
    let nc = num_components as usize;
    let mut components = vec![vec![0.0f32; num_pixels]; nc];
    let unsigned_offset = if signed {
        0.0
    } else {
        (1u32 << (bit_depth as u32 - 1)) as f32
    };

    if bit_depth <= 8 {
        for (i, pixel) in pixels.chunks_exact(nc).take(num_pixels).enumerate() {
            for (c, component) in components.iter_mut().enumerate().take(nc) {
                let val = pixel[c];
                component[i] = if signed {
                    (val as i8) as f32
                } else {
                    val as f32 - unsigned_offset
                };
            }
        }
    } else {
        // 16-bit samples (little-endian)
        for (i, pixel) in pixels.chunks_exact(nc * 2).take(num_pixels).enumerate() {
            for (c, component) in components.iter_mut().enumerate().take(nc) {
                let offset = c * 2;
                let val = u16::from_le_bytes([pixel[offset], pixel[offset + 1]]);
                component[i] = if signed {
                    (val as i16) as f32
                } else {
                    val as f32 - unsigned_offset
                };
            }
        }
    }

    components
}

/// Calculate the maximum number of decomposition levels for given dimensions.
fn max_decomposition_levels(width: u32, height: u32) -> u8 {
    let min_dim = width.min(height);
    if min_dim <= 1 {
        return 0;
    }
    (min_dim as f32).log2().floor() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DecodeSettings, Image};

    fn encode_high_throughput_for_test(
        pixels: &[u8],
        width: u32,
        height: u32,
        num_components: u8,
        bit_depth: u8,
        signed: bool,
        options: &EncodeOptions,
    ) -> Result<Vec<u8>, &'static str> {
        encode_impl(
            pixels,
            width,
            height,
            num_components,
            bit_depth,
            signed,
            options,
            BlockCodingMode::HighThroughput,
        )
    }

    #[test]
    fn test_encode_8bit_gray() {
        let width = 8u32;
        let height = 8u32;
        let pixels: Vec<u8> = (0..64).collect();

        let result = encode(
            &pixels,
            width,
            height,
            1,
            8,
            false,
            &EncodeOptions {
                num_decomposition_levels: 2,
                ..Default::default()
            },
        );

        assert!(result.is_ok());
        let codestream = result.unwrap();
        // Verify SOC marker
        assert_eq!(codestream[0], 0xFF);
        assert_eq!(codestream[1], 0x4F);
        // Verify EOC marker
        let len = codestream.len();
        assert_eq!(codestream[len - 2], 0xFF);
        assert_eq!(codestream[len - 1], 0xD9);
    }

    #[test]
    fn test_encode_16bit_gray() {
        let width = 8u32;
        let height = 8u32;
        let mut pixels = Vec::with_capacity(128);
        for i in 0..64u16 {
            let val = i * 100;
            pixels.extend_from_slice(&val.to_le_bytes());
        }

        let result = encode(
            &pixels,
            width,
            height,
            1,
            16,
            false,
            &EncodeOptions {
                num_decomposition_levels: 2,
                ..Default::default()
            },
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_encode_rgb() {
        let width = 16u32;
        let height = 16u32;
        let pixels: Vec<u8> = (0..width * height * 3).map(|i| (i & 0xFF) as u8).collect();

        let result = encode(
            &pixels,
            width,
            height,
            3,
            8,
            false,
            &EncodeOptions {
                num_decomposition_levels: 3,
                ..Default::default()
            },
        );

        assert!(result.is_ok(), "RGB encode failed: {:?}", result.err());
    }

    #[test]
    fn test_encode_lossy() {
        let pixels: Vec<u8> = (0..64).collect();

        let result = encode(
            &pixels,
            8,
            8,
            1,
            8,
            false,
            &EncodeOptions {
                num_decomposition_levels: 2,
                reversible: false,
                guard_bits: 2,
                ..Default::default()
            },
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_encode_high_throughput_zero_image_roundtrip() {
        let width = 4u32;
        let height = 4u32;
        let sample = 2048u16.to_le_bytes();
        let mut pixels = Vec::with_capacity((width * height * 2) as usize);
        for _ in 0..(width * height) {
            pixels.extend_from_slice(&sample);
        }

        let codestream = encode_high_throughput_for_test(
            &pixels,
            width,
            height,
            1,
            12,
            false,
            &EncodeOptions {
                num_decomposition_levels: 2,
                ..Default::default()
            },
        )
        .expect("HT all-zero encode");

        assert!(codestream.windows(2).any(|window| window == [0xFF, 0x50]));
        let cod_offset = codestream
            .windows(2)
            .position(|window| window == [0xFF, 0x52])
            .expect("COD marker");
        assert_eq!(codestream[cod_offset + 12], 0x40);

        let image =
            Image::new(&codestream, &DecodeSettings::default()).expect("parse HT codestream");
        let decoded = image.decode_native().expect("decode HT codestream");

        assert_eq!(decoded.width, width);
        assert_eq!(decoded.height, height);
        assert_eq!(decoded.bit_depth, 12);
        assert_eq!(decoded.data, pixels);
    }

    #[test]
    fn test_encode_high_throughput_rejects_nonzero_blocks() {
        let width = 4u32;
        let height = 4u32;
        let pixels: Vec<u8> = (0..(width * height)).map(|i| i as u8).collect();

        let error = encode_high_throughput_for_test(
            &pixels,
            width,
            height,
            1,
            8,
            false,
            &EncodeOptions {
                num_decomposition_levels: 2,
                ..Default::default()
            },
        )
        .expect_err("HT non-zero encode should fail");

        assert_eq!(error, "HTJ2K block encoding not implemented");
    }

    #[test]
    fn test_encode_invalid_dimensions() {
        let result = encode(&[], 0, 0, 1, 8, false, &EncodeOptions::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_encode_too_short() {
        let pixels = vec![0u8; 10]; // Way too short for 8x8
        let result = encode(&pixels, 8, 8, 1, 8, false, &EncodeOptions::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_deinterleave_rgb() {
        let pixels = vec![
            10u8, 20, 30, // pixel 0: R=10, G=20, B=30
            40, 50, 60, // pixel 1: R=40, G=50, B=60
        ];
        let comps = deinterleave_to_f32(&pixels, 2, 3, 8, false);
        assert_eq!(comps[0], vec![-118.0, -88.0]); // R
        assert_eq!(comps[1], vec![-108.0, -78.0]); // G
        assert_eq!(comps[2], vec![-98.0, -68.0]); // B
    }

    #[test]
    fn test_encode_decode_roundtrip_gray_8bit() {
        use crate::{DecodeSettings, Image};

        // Constant image: all pixels = 42 — simplest possible test
        let original: Vec<u8> = vec![42u8; 64]; // 8x8, all same value
        let encoded = encode(
            &original,
            8,
            8,
            1,
            8,
            false,
            &EncodeOptions {
                num_decomposition_levels: 0,
                reversible: true,
                ..Default::default()
            },
        )
        .expect("encode failed");

        let settings = DecodeSettings {
            resolve_palette_indices: false,
            strict: false,
            target_resolution: None,
        };
        let image = Image::new(&encoded, &settings).expect("parse failed");
        let decoded = image.decode_native().expect("decode failed");

        assert_eq!(decoded.width, 8);
        assert_eq!(decoded.height, 8);
        assert_eq!(decoded.data, original, "round-trip mismatch");
    }
}
