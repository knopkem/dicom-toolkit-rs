//! Codec registry — maps DICOM transfer syntax UIDs to codec implementations.
//!
//! Ports DCMTK's `DcmCodecList` singleton. Codecs can be registered either
//! statically (via compile-time dependencies) or dynamically.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_data::value::PixelData;
use dicom_toolkit_dict::ts::transfer_syntaxes;

// ── ImageCodec trait ──────────────────────────────────────────────────────────

/// Trait for DICOM image codecs.
///
/// Each codec handles one or more transfer syntaxes.  The codec is responsible
/// for decompressing/compressing pixel data items.
pub trait ImageCodec: Send + Sync {
    /// UID(s) of the transfer syntax(es) this codec handles.
    fn transfer_syntax_uids(&self) -> &[&str];

    /// UID(s) of the transfer syntax(es) this codec can decode.
    fn decode_transfer_syntax_uids(&self) -> &[&str] {
        self.transfer_syntax_uids()
    }

    /// UID(s) of the transfer syntax(es) this codec can encode.
    fn encode_transfer_syntax_uids(&self) -> &[&str] {
        self.transfer_syntax_uids()
    }

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
    ///
    /// `bits_allocated` describes the native storage width of `pixels`.
    /// `bits_stored` is the actual sample precision to encode into the
    /// compressed stream and must be `<= bits_allocated`.
    fn encode(
        &self,
        pixels: &[u8],
        rows: u16,
        columns: u16,
        samples_per_pixel: u8,
        bits_allocated: u8,
        bits_stored: u8,
    ) -> DcmResult<PixelData>;
}

fn validate_stored_bits(codec_name: &str, bits_allocated: u8, bits_stored: u8) -> DcmResult<()> {
    if bits_stored == 0 {
        return Err(DcmError::CompressionError {
            reason: format!("{codec_name}: BitsStored must be at least 1"),
        });
    }
    if bits_stored > bits_allocated {
        return Err(DcmError::CompressionError {
            reason: format!(
                "{codec_name}: BitsStored ({bits_stored}) exceeds BitsAllocated ({bits_allocated})"
            ),
        });
    }
    Ok(())
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
        _bits_stored: u8,
    ) -> DcmResult<PixelData> {
        let encoded =
            crate::rle::rle_encode_frame(pixels, rows, columns, samples_per_pixel, bits_allocated)?;
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
        _bits_stored: u8,
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
        bits_stored: u8,
    ) -> DcmResult<PixelData> {
        validate_stored_bits("JPEG-LS", bits_allocated, bits_stored)?;
        let near = 0; // Lossless by default.
        let encoded = crate::jpeg_ls::encoder::encode_jpeg_ls(
            pixels,
            columns as u32,
            rows as u32,
            bits_stored,
            samples_per_pixel,
            near,
        )?;
        Ok(PixelData::Encapsulated {
            offset_table: vec![],
            fragments: vec![encoded],
        })
    }
}

// ── JPEG 2000 codec ───────────────────────────────────────────────────────────

struct Jp2kDecodeCodec;

struct Jp2kEncodeCodec {
    transfer_syntax_uid: &'static str,
    high_throughput: bool,
    lossless: bool,
}

const JP2K_DECODE_TRANSFER_SYNTAXES: &[&str] = &[
    transfer_syntaxes::JPEG_2000_LOSSLESS.uid,
    transfer_syntaxes::JPEG_2000.uid,
    transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY.uid,
    transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_RPCL_LOSSLESS_ONLY.uid,
    transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid,
];

const NO_TRANSFER_SYNTAXES: &[&str] = &[];

impl Jp2kEncodeCodec {
    const fn new(transfer_syntax_uid: &'static str, high_throughput: bool, lossless: bool) -> Self {
        Self {
            transfer_syntax_uid,
            high_throughput,
            lossless,
        }
    }

    fn codec_name(&self) -> &'static str {
        if self.high_throughput {
            "HTJ2K"
        } else {
            "JPEG 2000"
        }
    }
}

fn decode_jp2k_pixel_data(pixel_data: &PixelData) -> DcmResult<Vec<u8>> {
    let fragments = match pixel_data {
        PixelData::Encapsulated { fragments, .. } => fragments,
        PixelData::Native { bytes } => return Ok(bytes.clone()),
    };

    let mut all_pixels = Vec::new();
    for fragment in fragments {
        if fragment.is_empty() {
            continue;
        }
        let decoded = crate::jp2k::decoder::decode_jp2k(fragment)?;
        all_pixels.extend_from_slice(&decoded.pixels);
    }

    Ok(all_pixels)
}

fn encode_jp2k_pixel_data(
    codec: &Jp2kEncodeCodec,
    pixels: &[u8],
    rows: u16,
    columns: u16,
    samples_per_pixel: u8,
    bits_allocated: u8,
    bits_stored: u8,
) -> DcmResult<PixelData> {
    validate_stored_bits(codec.codec_name(), bits_allocated, bits_stored)?;
    let encoded = if codec.high_throughput {
        crate::jp2k::encoder::encode_htj2k(
            pixels,
            columns as u32,
            rows as u32,
            bits_stored,
            samples_per_pixel,
            codec.lossless,
        )?
    } else {
        crate::jp2k::encoder::encode_jp2k(
            pixels,
            columns as u32,
            rows as u32,
            bits_stored,
            samples_per_pixel,
            codec.lossless,
        )?
    };

    Ok(PixelData::Encapsulated {
        offset_table: vec![],
        fragments: vec![encoded],
    })
}

impl ImageCodec for Jp2kDecodeCodec {
    fn transfer_syntax_uids(&self) -> &[&str] {
        JP2K_DECODE_TRANSFER_SYNTAXES
    }

    fn encode_transfer_syntax_uids(&self) -> &[&str] {
        NO_TRANSFER_SYNTAXES
    }

    fn decode(
        &self,
        pixel_data: &PixelData,
        _rows: u16,
        _columns: u16,
        _samples_per_pixel: u8,
        _bits_allocated: u8,
    ) -> DcmResult<Vec<u8>> {
        decode_jp2k_pixel_data(pixel_data)
    }

    fn encode(
        &self,
        _pixels: &[u8],
        _rows: u16,
        _columns: u16,
        _samples_per_pixel: u8,
        _bits_allocated: u8,
        _bits_stored: u8,
    ) -> DcmResult<PixelData> {
        Err(DcmError::CompressionError {
            reason: "JPEG 2000 encoding is not available for this transfer syntax".to_string(),
        })
    }
}

impl ImageCodec for Jp2kEncodeCodec {
    fn transfer_syntax_uids(&self) -> &[&str] {
        std::slice::from_ref(&self.transfer_syntax_uid)
    }

    fn decode(
        &self,
        pixel_data: &PixelData,
        _rows: u16,
        _columns: u16,
        _samples_per_pixel: u8,
        _bits_allocated: u8,
    ) -> DcmResult<Vec<u8>> {
        decode_jp2k_pixel_data(pixel_data)
    }

    fn encode(
        &self,
        pixels: &[u8],
        rows: u16,
        columns: u16,
        samples_per_pixel: u8,
        bits_allocated: u8,
        bits_stored: u8,
    ) -> DcmResult<PixelData> {
        encode_jp2k_pixel_data(
            self,
            pixels,
            rows,
            columns,
            samples_per_pixel,
            bits_allocated,
            bits_stored,
        )
    }
}

// ── CodecRegistry ─────────────────────────────────────────────────────────────

/// Registry of all available image codecs, keyed by transfer syntax UID.
pub struct CodecRegistry {
    codecs: RwLock<HashMap<String, Arc<dyn ImageCodec>>>,
    decoder_codecs: RwLock<HashMap<String, Arc<dyn ImageCodec>>>,
    encoder_codecs: RwLock<HashMap<String, Arc<dyn ImageCodec>>>,
}

impl CodecRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            codecs: RwLock::new(HashMap::new()),
            decoder_codecs: RwLock::new(HashMap::new()),
            encoder_codecs: RwLock::new(HashMap::new()),
        }
    }

    /// Register a codec (replaces any existing codec for the same UID).
    pub fn register(&self, codec: Arc<dyn ImageCodec>) {
        let decode_uids = codec.decode_transfer_syntax_uids();
        let encode_uids = codec.encode_transfer_syntax_uids();

        let mut codecs = self.codecs.write().unwrap();
        let mut decoder_codecs = self.decoder_codecs.write().unwrap();
        let mut encoder_codecs = self.encoder_codecs.write().unwrap();

        for uid in decode_uids {
            decoder_codecs.insert(uid.to_string(), Arc::clone(&codec));
            if encode_uids.contains(uid) {
                codecs.insert(uid.to_string(), Arc::clone(&codec));
            }
        }

        for uid in encode_uids {
            encoder_codecs.insert(uid.to_string(), Arc::clone(&codec));
        }
    }

    /// Look up a codec that supports both decoding and encoding for a transfer
    /// syntax UID.
    pub fn find(&self, transfer_syntax_uid: &str) -> Option<Arc<dyn ImageCodec>> {
        self.codecs
            .read()
            .unwrap()
            .get(transfer_syntax_uid)
            .cloned()
    }

    /// Look up a decoder by transfer syntax UID.
    pub fn find_decoder(&self, transfer_syntax_uid: &str) -> Option<Arc<dyn ImageCodec>> {
        self.decoder_codecs
            .read()
            .unwrap()
            .get(transfer_syntax_uid)
            .cloned()
    }

    /// Look up an encoder by transfer syntax UID.
    pub fn find_encoder(&self, transfer_syntax_uid: &str) -> Option<Arc<dyn ImageCodec>> {
        self.encoder_codecs
            .read()
            .unwrap()
            .get(transfer_syntax_uid)
            .cloned()
    }

    /// Look up a codec that supports both decoding and encoding or return a
    /// [`DcmError::NoCodec`] error.
    pub fn find_required(&self, transfer_syntax_uid: &str) -> DcmResult<Arc<dyn ImageCodec>> {
        self.find(transfer_syntax_uid)
            .ok_or_else(|| DcmError::NoCodec {
                uid: transfer_syntax_uid.to_string(),
            })
    }

    /// Look up a decoder or return a [`DcmError::NoCodec`] error.
    pub fn find_decoder_required(
        &self,
        transfer_syntax_uid: &str,
    ) -> DcmResult<Arc<dyn ImageCodec>> {
        self.find_decoder(transfer_syntax_uid)
            .ok_or_else(|| DcmError::NoCodec {
                uid: transfer_syntax_uid.to_string(),
            })
    }

    /// Look up an encoder or return a [`DcmError::NoCodec`] error.
    pub fn find_encoder_required(
        &self,
        transfer_syntax_uid: &str,
    ) -> DcmResult<Arc<dyn ImageCodec>> {
        self.find_encoder(transfer_syntax_uid)
            .ok_or_else(|| DcmError::NoCodec {
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
    reg.register(Arc::new(Jp2kDecodeCodec));
    reg.register(Arc::new(Jp2kEncodeCodec::new(
        transfer_syntaxes::JPEG_2000_LOSSLESS.uid,
        false,
        true,
    )));
    reg.register(Arc::new(Jp2kEncodeCodec::new(
        transfer_syntaxes::JPEG_2000.uid,
        false,
        true,
    )));
    reg.register(Arc::new(Jp2kEncodeCodec::new(
        transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY.uid,
        true,
        true,
    )));
    // `.203` supports both lossless and lossy streams, but the registry encode
    // API has no quality selection, so default to a lossless HTJ2K stream here.
    reg.register(Arc::new(Jp2kEncodeCodec::new(
        transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid,
        true,
        true,
    )));
    reg
});

// ── Flat functional API ───────────────────────────────────────────────────────

/// Transfer syntax UIDs that this crate can decode.
const SUPPORTED_DECODE_TS: &[&str] = &[
    transfer_syntaxes::RLE_LOSSLESS.uid,
    transfer_syntaxes::JPEG_BASELINE.uid,
    transfer_syntaxes::JPEG_EXTENDED.uid,
    transfer_syntaxes::JPEG_LOSSLESS.uid,
    transfer_syntaxes::JPEG_LOSSLESS_SV1.uid,
    transfer_syntaxes::JPEG_LS_LOSSLESS.uid,
    transfer_syntaxes::JPEG_LS_LOSSY.uid,
    transfer_syntaxes::JPEG_2000_LOSSLESS.uid,
    transfer_syntaxes::JPEG_2000.uid,
    transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY.uid,
    transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_RPCL_LOSSLESS_ONLY.uid,
    transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid,
];

/// Transfer syntax UIDs that this crate can encode.
const SUPPORTED_ENCODE_TS: &[&str] = &[
    transfer_syntaxes::RLE_LOSSLESS.uid,
    transfer_syntaxes::JPEG_BASELINE.uid,
    transfer_syntaxes::JPEG_EXTENDED.uid,
    transfer_syntaxes::JPEG_LS_LOSSLESS.uid,
    transfer_syntaxes::JPEG_LS_LOSSY.uid,
    transfer_syntaxes::JPEG_2000_LOSSLESS.uid,
    transfer_syntaxes::JPEG_2000.uid,
    transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY.uid,
    transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid,
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
    SUPPORTED_DECODE_TS.contains(&ts_uid)
}

/// Returns all transfer syntax UIDs that this crate can decode.
pub fn supported_decode_transfer_syntaxes() -> &'static [&'static str] {
    SUPPORTED_DECODE_TS
}

/// Returns `true` if an encoder is available for the given transfer syntax UID.
pub fn can_encode(ts_uid: &str) -> bool {
    SUPPORTED_ENCODE_TS.contains(&ts_uid)
}

/// Returns all transfer syntax UIDs that this crate can encode.
pub fn supported_encode_transfer_syntaxes() -> &'static [&'static str] {
    SUPPORTED_ENCODE_TS
}

/// Returns all transfer syntax UIDs that this crate can decode.
pub fn supported_transfer_syntaxes() -> &'static [&'static str] {
    supported_decode_transfer_syntaxes()
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
        uid if uid == transfer_syntaxes::JPEG_2000_LOSSLESS.uid
            || uid == transfer_syntaxes::JPEG_2000.uid
            || uid == transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY.uid
            || uid == transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_RPCL_LOSSLESS_ONLY.uid
            || uid == transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid =>
        {
            crate::jp2k::Jp2kCodec::decode_frame(data).map(|f| f.pixels)
        }
        uid => Err(DcmError::NoCodec {
            uid: uid.to_string(),
        }),
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
    fn global_registry_has_htj2k_decoder() {
        let codec = GLOBAL_REGISTRY.find_decoder(transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid);
        assert!(codec.is_some());
    }

    #[test]
    fn global_registry_exposes_htj2k_encoder_variants() {
        let lossless =
            GLOBAL_REGISTRY.find(transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY.uid);
        let generic = GLOBAL_REGISTRY.find(transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid);
        assert!(lossless.is_some());
        assert!(generic.is_some());
    }

    #[test]
    fn global_registry_keeps_htj2k_rpcl_decode_only() {
        let codec = GLOBAL_REGISTRY
            .find(transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_RPCL_LOSSLESS_ONLY.uid);
        assert!(codec.is_none());
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
    fn codec_registry_can_decode_htj2k() {
        assert!(can_decode(transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid));
    }

    #[test]
    fn codec_registry_cannot_decode_unknown() {
        assert!(!can_decode("1.2.3.4.5.999"));
    }

    #[test]
    fn supported_transfer_syntaxes_is_non_empty() {
        let list = supported_decode_transfer_syntaxes();
        assert!(!list.is_empty());
        assert!(list.contains(&transfer_syntaxes::RLE_LOSSLESS.uid));
        assert!(list.contains(&transfer_syntaxes::JPEG_BASELINE.uid));
        assert!(list.contains(&transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid));
    }

    #[test]
    fn supported_encode_transfer_syntaxes_excludes_decode_only_codecs() {
        let list = supported_encode_transfer_syntaxes();
        assert!(list.contains(&transfer_syntaxes::RLE_LOSSLESS.uid));
        assert!(list.contains(&transfer_syntaxes::JPEG_2000_LOSSLESS.uid));
        assert!(list.contains(&transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000.uid));
        assert!(!list.contains(&transfer_syntaxes::JPEG_LOSSLESS.uid));
        assert!(!list.contains(&transfer_syntaxes::JPEG_LOSSLESS_SV1.uid));
        assert!(
            !list.contains(&transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_RPCL_LOSSLESS_ONLY.uid)
        );
    }

    #[test]
    fn can_encode_distinguishes_decode_only_transfer_syntaxes() {
        assert!(can_encode(transfer_syntaxes::JPEG_BASELINE.uid));
        assert!(can_encode(transfer_syntaxes::JPEG_LS_LOSSLESS.uid));
        assert!(!can_encode(transfer_syntaxes::JPEG_LOSSLESS.uid));
        assert!(!can_encode(
            transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_RPCL_LOSSLESS_ONLY.uid
        ));
    }

    #[test]
    fn rle_codec_roundtrip_via_registry() {
        use crate::rle::rle_encode_frame;

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

        let codec = GLOBAL_REGISTRY
            .find(transfer_syntaxes::RLE_LOSSLESS.uid)
            .unwrap();
        let decoded = codec
            .decode(&pixel_data, rows, cols, samples, bits)
            .unwrap();
        assert_eq!(&decoded[..16], &pixels[..]);
    }

    #[test]
    fn jp2k_codec_multiframe_decode_via_registry() {
        let rows = 4u16;
        let cols = 4u16;
        let samples = 1u8;
        let bits = 8u8;
        let frame_a: Vec<u8> = (0u8..16).collect();
        let frame_b: Vec<u8> = (16u8..32).collect();

        let encoded_a = crate::jp2k::encoder::encode_jp2k(
            &frame_a,
            cols as u32,
            rows as u32,
            bits,
            samples,
            true,
        )
        .unwrap();
        let encoded_b = crate::jp2k::encoder::encode_jp2k(
            &frame_b,
            cols as u32,
            rows as u32,
            bits,
            samples,
            true,
        )
        .unwrap();

        let pixel_data = PixelData::Encapsulated {
            offset_table: vec![],
            fragments: vec![encoded_a, encoded_b],
        };

        let codec = GLOBAL_REGISTRY
            .find(transfer_syntaxes::JPEG_2000_LOSSLESS.uid)
            .unwrap();
        let decoded = codec
            .decode(&pixel_data, rows, cols, samples, bits)
            .unwrap();

        let mut expected = frame_a;
        expected.extend_from_slice(&frame_b);
        assert_eq!(decoded, expected);
    }

    #[test]
    fn jpeg_ls_codec_encode_uses_bits_stored_precision() {
        let rows = 4u16;
        let cols = 4u16;
        let samples = 1u8;
        let bits_allocated = 16u8;
        let bits_stored = 12u8;
        let mut pixels = Vec::with_capacity(32);
        for i in 0u16..16 {
            pixels.extend_from_slice(&((i * 257) & 0x0FFF).to_le_bytes());
        }

        let codec = GLOBAL_REGISTRY
            .find(transfer_syntaxes::JPEG_LS_LOSSLESS.uid)
            .unwrap();
        let encoded = codec
            .encode(&pixels, rows, cols, samples, bits_allocated, bits_stored)
            .unwrap();
        let fragment = match encoded {
            PixelData::Encapsulated { fragments, .. } => fragments.into_iter().next().unwrap(),
            PixelData::Native { .. } => panic!("expected encapsulated pixel data"),
        };

        let decoded = crate::jpeg_ls::decoder::decode_jpeg_ls(&fragment).unwrap();
        assert_eq!(decoded.bits_per_sample, bits_stored);
        assert_eq!(decoded.pixels, pixels);
    }

    #[test]
    fn jp2k_codec_encode_uses_bits_stored_precision() {
        let rows = 4u16;
        let cols = 4u16;
        let samples = 1u8;
        let bits_allocated = 16u8;
        let bits_stored = 12u8;
        let mut pixels = Vec::with_capacity(32);
        for i in 0u16..16 {
            pixels.extend_from_slice(&((i * 257) & 0x0FFF).to_le_bytes());
        }

        let codec = GLOBAL_REGISTRY
            .find(transfer_syntaxes::JPEG_2000_LOSSLESS.uid)
            .unwrap();
        let encoded = codec
            .encode(&pixels, rows, cols, samples, bits_allocated, bits_stored)
            .unwrap();
        let fragment = match encoded {
            PixelData::Encapsulated { fragments, .. } => fragments.into_iter().next().unwrap(),
            PixelData::Native { .. } => panic!("expected encapsulated pixel data"),
        };

        let decoded = crate::jp2k::decoder::decode_jp2k(&fragment).unwrap();
        assert_eq!(decoded.bits_per_sample, bits_stored);
        assert_eq!(decoded.pixels, pixels);
    }

    #[test]
    fn htj2k_codec_encode_uses_bits_stored_precision() {
        let rows = 4u16;
        let cols = 4u16;
        let samples = 1u8;
        let bits_allocated = 16u8;
        let bits_stored = 12u8;
        let mut pixels = Vec::with_capacity(32);
        for _ in 0..16 {
            pixels.extend_from_slice(&2048u16.to_le_bytes());
        }

        let codec = GLOBAL_REGISTRY
            .find(transfer_syntaxes::HIGH_THROUGHPUT_JPEG_2000_LOSSLESS_ONLY.uid)
            .unwrap();
        let encoded = codec
            .encode(&pixels, rows, cols, samples, bits_allocated, bits_stored)
            .unwrap();
        let fragment = match encoded {
            PixelData::Encapsulated { fragments, .. } => fragments.into_iter().next().unwrap(),
            PixelData::Native { .. } => panic!("expected encapsulated pixel data"),
        };

        assert!(fragment.windows(2).any(|window| window == [0xFF, 0x50]));
        let decoded = crate::jp2k::decoder::decode_jp2k(&fragment).unwrap();
        assert_eq!(decoded.bits_per_sample, bits_stored);
        assert_eq!(decoded.pixels, pixels);
    }
}
