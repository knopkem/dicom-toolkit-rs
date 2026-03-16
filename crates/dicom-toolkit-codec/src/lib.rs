//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for diagnostic or therapeutic purposes.
//!
//! DICOM image compression codecs: JPEG, JPEG-LS, JPEG 2000, RLE, and codec registry.
//!
//! This crate ports DCMTK's `dcmjpeg` and `dcmjpls` modules.

pub mod jp2k;
pub mod jpeg;
pub mod jpeg_ls;
pub mod registry;
pub mod rle;

pub use jp2k::Jp2kCodec;
pub use jpeg::{DecodedFrame, JpegDecoder};
pub use jpeg_ls::JpegLsCodec;
pub use registry::{
    can_decode, can_encode, decode_pixel_data, supported_decode_transfer_syntaxes,
    supported_encode_transfer_syntaxes, supported_transfer_syntaxes, CodecInfo, CodecRegistry,
    ImageCodec,
};
pub use rle::{rle_decode_frame, rle_encode_frame, RleCodec};
