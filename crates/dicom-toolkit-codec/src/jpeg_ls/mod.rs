//! Pure-Rust JPEG-LS codec (ISO/IEC 14495-1).
//!
//! Implements both lossless (NEAR=0) and near-lossless encoding/decoding
//! for 2–16 bit depths, 1–4 components, all interleave modes.
//!
//! This is a port of the CharLS algorithm — no C/C++ dependencies.

pub mod bitstream;
pub mod context;
pub mod decoder;
pub mod encoder;
pub mod golomb;
pub mod marker;
pub mod params;
pub mod prediction;
pub mod sample;
pub mod scan;

use dicom_toolkit_core::error::DcmResult;

// ── JpegLsCodec ───────────────────────────────────────────────────────────────

/// Pure-Rust JPEG-LS codec.
///
/// Supports lossless and near-lossless encoding/decoding for 2–16 bit depths.
pub struct JpegLsCodec;

impl JpegLsCodec {
    /// Decode a JPEG-LS frame from compressed data.
    pub fn decode_frame(data: &[u8]) -> DcmResult<decoder::DecodedFrame> {
        decoder::decode_jpeg_ls(data)
    }

    /// Encode raw pixels as a JPEG-LS bitstream.
    pub fn encode_frame(
        pixels: &[u8],
        width: u32,
        height: u32,
        bits_per_sample: u8,
        components: u8,
        near: i32,
    ) -> DcmResult<Vec<u8>> {
        encoder::encode_jpeg_ls(pixels, width, height, bits_per_sample, components, near)
    }
}

