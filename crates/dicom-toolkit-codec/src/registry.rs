//! Codec registry — maps DICOM transfer syntax UIDs to codec implementations.
//!
//! Ports DCMTK's `DcmCodecList` singleton. Codecs can be registered either
//! statically (via compile-time dependencies) or dynamically.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_dict::ts::transfer_syntaxes;
use dicom_toolkit_data::value::PixelData;

// ── ImageCodec trait ──────────────────────────────────────────────────────────

/// Trait for DICOM image codecs.
///
/// Each codec handles one or more transfer syntaxes.  The codec is responsible
/// for decompressing/compressing pixel data items.
pub trait ImageCodec: Send + Sync {
    /// UID(s) of the transfer syntax(es) this codec handles.
    fn transfer_syntax_uids(&self) -> &[&str];

    /// Decode all frames in `encapsulated` pixel data.
    ///
    /// Returns the raw, uncompressed pixel bytes for all frames concatenated.
    fn decode(
        &self,
        encapsulated: &PixelData,
        rows: u16,
        columns: u16,
        samples_per_pixel: u8,
        bits_allocated: u8,
    ) -> DcmResult<Vec<u8>>;

    /// Encode raw pixel bytes into an encapsulated `PixelData`.
    fn encode(
        &self,
        pixels: &[u8],
        rows: u16,
        columns: u16,
        samples_per_pixel: u8,
        bits_allocated: u8,
    ) -> DcmResult<PixelData>;
}

// ── Built-in RLE codec ────────────────────────────────────────────────────────

struct RleCodec;

impl ImageCodec for RleCodec {
    fn transfer_syntax_uids(&self) -> &[&str] {
        &[transfer_syntaxes::RLE_LOSSLESS.uid]
    }

    fn decode(
        &self,
        pixel_data: &PixelData,
        rows: u16,
        columns: u16,
        samples_per_pixel: u8,
        bits_allocated: u8,
    ) -> DcmResult<Vec<u8>> {
        let fragments = match pixel_data {
            PixelData::Encapsulated { fragments, .. } => fragments,
            PixelData::Native { bytes } => return Ok(bytes.clone()),
        };

        let mut all_frames = Vec::new();
        for fragment in fragments {
            let frame = crate::rle::rle_decode_frame(
                fragment,
                rows,
                columns,
                samples_per_pixel,
                bits_allocated,
            )?;
            all_frames.extend_from_slice(&frame);
        }
        Ok(all_frames)
    }

    fn encode(
        &self,
        pixels: &[u8],
        rows: u16,
        columns: u16,
        samples_per_pixel: u8,
        bits_allocated: u8,
    ) -> DcmResult<PixelData> {
        let encoded = crate::rle::rle_encode_frame(
            pixels,
            rows,
            columns,
            samples_per_pixel,
            bits_allocated,
        )?;
        Ok(PixelData::Encapsulated {
            offset_table: vec![0],
            fragments: vec![encoded],
        })
    }
}

// ── Built-in JPEG codec ───────────────────────────────────────────────────────

struct JpegCodec {
    uids: Vec<&'static str>,
}

impl JpegCodec {
    fn baseline() -> Self {
        Self {
            uids: vec![
                transfer_syntaxes::JPEG_BASELINE.uid,
                transfer_syntaxes::JPEG_EXTENDED.uid,
            ],
        }
    }
}

impl ImageCodec for JpegCodec {
    fn transfer_syntax_uids(&self) -> &[&str] {
        &self.uids
    }

    fn decode(
        &self,
        pixel_data: &PixelData,
        _rows: u16,
        _columns: u16,
        _samples_per_pixel: u8,
        _bits_allocated: u8,
    ) -> DcmResult<Vec<u8>> {
        let fragments = match pixel_data {
            PixelData::Encapsulated { fragments, .. } => fragments,
            PixelData::Native { bytes } => return Ok(bytes.clone()),
        };

        let mut all_frames = Vec::new();
        for fragment in fragments {
            let frame = crate::jpeg::decoder::decode_jpeg(fragment)?;
            all_frames.extend_from_slice(&frame.data);
        }
        Ok(all_frames)
    }

    fn encode(
        &self,
        pixels: &[u8],
        rows: u16,
        columns: u16,
        samples_per_pixel: u8,
        _bits_allocated: u8,
    ) -> DcmResult<PixelData> {
        use crate::jpeg::params::JpegParams;
        let encoded = crate::jpeg::encoder::encode_jpeg(
            pixels,
            columns,
            rows,
            samples_per_pixel,
            &JpegParams::default(),
        )?;
        Ok(PixelData::Encapsulated {
            offset_table: vec![0],
            fragments: vec![encoded],
        })
    }
}

// ── JPEG-LS codec ─────────────────────────────────────────────────────────────

struct JpegLsCodec;

impl ImageCodec for JpegLsCodec {
    fn transfer_syntax_uids(&self) -> &[&str] {
        &[
            transfer_syntaxes::JPEG_LS_LOSSLESS.uid,
            transfer_syntaxes::JPEG_LS_LOSSY.uid,
        ]
    }

    fn decode(
        &self,
        pixel_data: &PixelData,
        _rows: u16,
        _columns: u16,
        _samples_per_pixel: u8,
        _bits_allocated: u8,
    ) -> DcmResult<Vec<u8>> {
        let fragments = match pixel_data {
            PixelData::Encapsulated { fragments, .. } => fragments,
            PixelData::Native { bytes } => return Ok(bytes.clone()),
        };
        let empty = vec![];
        let data = fragments.first().unwrap_or(&empty);
        let decoded = crate::jpeg_ls::decoder::decode_jpeg_ls(data)?;
        Ok(decoded.pixels)
    }

    fn encode(
        &self,
        pixels: &[u8],
        rows: u16,
        columns: u16,
        samples_per_pixel: u8,
        bits_allocated: u8,
    ) -> DcmResult<PixelData> {
        let near = 0; // Lossless by default.
        let encoded = crate::jpeg_ls::encoder::encode_jpeg_ls(
            pixels,
            columns as u32,
            rows as u32,
            bits_allocated,
            samples_per_pixel,
            near,
        )?;
        Ok(PixelData::Encapsulated {
            offset_table: vec![],
            fragments: vec![encoded],
        })
    }
}

// ── CodecRegistry ─────────────────────────────────────────────────────────────

/// Registry of all available image codecs, keyed by transfer syntax UID.
pub struct CodecRegistry {
    codecs: RwLock<HashMap<String, Arc<dyn ImageCodec>>>,
}

impl CodecRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            codecs: RwLock::new(HashMap::new()),
        }
    }

    /// Register a codec (replaces any existing codec for the same UID).
    pub fn register(&self, codec: Arc<dyn ImageCodec>) {
        let mut map = self.codecs.write().unwrap();
        for uid in codec.transfer_syntax_uids() {
            map.insert(uid.to_string(), Arc::clone(&codec));
        }
    }

    /// Look up a codec by transfer syntax UID.
    pub fn find(&self, transfer_syntax_uid: &str) -> Option<Arc<dyn ImageCodec>> {
        self.codecs
            .read()
            .unwrap()
            .get(transfer_syntax_uid)
            .cloned()
    }

    /// Look up a codec or return a [`DcmError::NoCodec`] error.
    pub fn find_required(&self, transfer_syntax_uid: &str) -> DcmResult<Arc<dyn ImageCodec>> {
        self.find(transfer_syntax_uid).ok_or_else(|| DcmError::NoCodec {
            uid: transfer_syntax_uid.to_string(),
        })
    }
}

impl Default for CodecRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Global registry ───────────────────────────────────────────────────────────

/// Global codec registry, pre-populated with built-in codecs.
pub static GLOBAL_REGISTRY: LazyLock<CodecRegistry> = LazyLock::new(|| {
    let reg = CodecRegistry::new();
    reg.register(Arc::new(RleCodec));
    reg.register(Arc::new(JpegCodec::baseline()));
    reg.register(Arc::new(JpegLsCodec));
    reg
});

// ── Flat functional API ───────────────────────────────────────────────────────

/// Transfer syntax UIDs that this crate can decode.
const SUPPORTED_TS: &[&str] = &[
    transfer_syntaxes::RLE_LOSSLESS.uid,
    transfer_syntaxes::JPEG_BASELINE.uid,
    transfer_syntaxes::JPEG_EXTENDED.uid,
    transfer_syntaxes::JPEG_LOSSLESS.uid,
    transfer_syntaxes::JPEG_LOSSLESS_SV1.uid,
    transfer_syntaxes::JPEG_LS_LOSSLESS.uid,
    transfer_syntaxes::JPEG_LS_LOSSY.uid,
];

/// Registered codec information for a transfer syntax.
#[derive(Debug, Clone, Copy)]
pub struct CodecInfo {
    /// Transfer Syntax UID this codec handles.
    pub transfer_syntax_uid: &'static str,
    /// Human-readable name.
    pub name: &'static str,
}

/// Returns `true` if a decoder is available for the given transfer syntax UID.
pub fn can_decode(ts_uid: &str) -> bool {
    SUPPORTED_TS.contains(&ts_uid)
}

/// Returns all transfer syntax UIDs that this crate can decode.
pub fn supported_transfer_syntaxes() -> &'static [&'static str] {
    SUPPORTED_TS
}

/// Decode a single pixel-data fragment for the given transfer syntax.
///
/// `data` must be the raw fragment bytes (RLE header + data, or JPEG bitstream).
/// Returns the decoded pixel bytes in native little-endian order.
pub fn decode_pixel_data(
    ts_uid: &str,
    data: &[u8],
    rows: u16,
    cols: u16,
    bits_allocated: u16,
    samples: u16,
) -> DcmResult<Vec<u8>> {
    match ts_uid {
        uid if uid == transfer_syntaxes::RLE_LOSSLESS.uid => {
            crate::rle::RleCodec::decode(data, rows, cols, bits_allocated, samples)
        }
        uid if uid == transfer_syntaxes::JPEG_BASELINE.uid
            || uid == transfer_syntaxes::JPEG_EXTENDED.uid
            || uid == transfer_syntaxes::JPEG_LOSSLESS.uid
            || uid == transfer_syntaxes::JPEG_LOSSLESS_SV1.uid =>
        {
            crate::jpeg::JpegDecoder::decode_frame(data).map(|f| f.pixels)
        }
        uid if uid == transfer_syntaxes::JPEG_LS_LOSSLESS.uid
            || uid == transfer_syntaxes::JPEG_LS_LOSSY.uid =>
        {
            crate::jpeg_ls::JpegLsCodec::decode_frame(data).map(|f| f.pixels)
        }
        uid => Err(DcmError::NoCodec { uid: uid.to_string() }),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_dict::ts::transfer_syntaxes;

    #[test]
    fn global_registry_has_rle() {
        let codec = GLOBAL_REGISTRY.find(transfer_syntaxes::RLE_LOSSLESS.uid);
        assert!(codec.is_some());
    }

    #[test]
    fn global_registry_has_jpeg_baseline() {
        let codec = GLOBAL_REGISTRY.find(transfer_syntaxes::JPEG_BASELINE.uid);
        assert!(codec.is_some());
    }

    #[test]
    fn global_registry_has_jpeg_extended() {
        let codec = GLOBAL_REGISTRY.find(transfer_syntaxes::JPEG_EXTENDED.uid);
        assert!(codec.is_some());
    }

    #[test]
    fn global_registry_has_jpeg_ls() {
        let codec = GLOBAL_REGISTRY.find(transfer_syntaxes::JPEG_LS_LOSSLESS.uid);
        assert!(codec.is_some());
    }

    #[test]
    fn unknown_uid_returns_none() {
        let codec = GLOBAL_REGISTRY.find("1.2.3.4.5.999");
        assert!(codec.is_none());
    }

    #[test]
    fn find_required_returns_error_for_unknown() {
        let result = GLOBAL_REGISTRY.find_required("1.9.9.9.9");
        assert!(matches!(result, Err(DcmError::NoCodec { .. })));
    }

    // ── Flat API tests ────────────────────────────────────────────────────────

    #[test]
    fn codec_registry_can_decode_rle() {
        assert!(can_decode(transfer_syntaxes::RLE_LOSSLESS.uid));
    }

    #[test]
    fn codec_registry_can_decode_jpeg_baseline() {
        assert!(can_decode(transfer_syntaxes::JPEG_BASELINE.uid));
    }

    #[test]
    fn codec_registry_cannot_decode_unknown() {
        assert!(!can_decode("1.2.3.4.5.999"));
    }

    #[test]
    fn supported_transfer_syntaxes_is_non_empty() {
        let list = supported_transfer_syntaxes();
        assert!(!list.is_empty());
        assert!(list.contains(&transfer_syntaxes::RLE_LOSSLESS.uid));
        assert!(list.contains(&transfer_syntaxes::JPEG_BASELINE.uid));
    }

    #[test]
    fn rle_codec_roundtrip_via_registry() {
        use crate::rle::{rle_encode_frame};

        let rows = 4u16;
        let cols = 4u16;
        let samples = 1u8;
        let bits = 8u8;
        let pixels: Vec<u8> = (0u8..16).collect();

        let encoded_frame = rle_encode_frame(&pixels, rows, cols, samples, bits).unwrap();
        let pixel_data = PixelData::Encapsulated {
            offset_table: vec![0],
            fragments: vec![encoded_frame],
        };

        let codec = GLOBAL_REGISTRY.find(transfer_syntaxes::RLE_LOSSLESS.uid).unwrap();
        let decoded = codec.decode(&pixel_data, rows, cols, samples, bits).unwrap();
        assert_eq!(&decoded[..16], &pixels[..]);
    }
}
