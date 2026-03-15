//! JPEG 2000 codec integration for DICOM transfer syntaxes.
//!
//! Uses a forked `hayro-jpeg2000` for both encoding and decoding at native
//! bit depth (8/12/16-bit), preserving full diagnostic quality.

pub mod decoder;
pub mod encoder;

use dicom_toolkit_core::error::DcmResult;

/// JPEG 2000 codec for DICOM pixel data.
///
/// Supports encoding and decoding of JPEG 2000 compressed DICOM images at
/// native bit depth (8, 12, or 16-bit), which is critical for diagnostic-quality
/// medical imaging.
pub struct Jp2kCodec;

impl Jp2kCodec {
    /// Decode a JPEG 2000 codestream from compressed data.
    pub fn decode_frame(data: &[u8]) -> DcmResult<decoder::DecodedFrame> {
        decoder::decode_jp2k(data)
    }

    /// Encode pixel data into a JPEG 2000 codestream.
    pub fn encode_frame(
        pixels: &[u8],
        width: u32,
        height: u32,
        bits_per_sample: u8,
        samples_per_pixel: u8,
        lossless: bool,
    ) -> DcmResult<Vec<u8>> {
        encoder::encode_jp2k(
            pixels,
            width,
            height,
            bits_per_sample,
            samples_per_pixel,
            lossless,
        )
    }
}
