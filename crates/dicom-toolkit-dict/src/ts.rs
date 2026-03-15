//! Transfer Syntax definitions and registry.
//!
//! Ports the transfer syntax handling from DCMTK's `dcxfer.h`.

use crate::vr::Vr;
use std::fmt;

/// Byte ordering for multi-byte values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ByteOrder {
    LittleEndian,
    BigEndian,
}

/// How the VR is encoded in the data stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VrEncoding {
    /// VR is implicit (looked up from data dictionary).
    Implicit,
    /// VR is explicitly encoded in the data stream.
    Explicit,
}

/// Whether pixel data is encapsulated (compressed) or native.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelEncoding {
    /// Pixel data stored natively (uncompressed).
    Native,
    /// Pixel data encapsulated in fragments (compressed).
    Encapsulated,
}

/// A DICOM Transfer Syntax definition.
#[derive(Debug, Clone)]
pub struct TransferSyntax {
    /// Transfer Syntax UID.
    pub uid: &'static str,
    /// Human-readable name.
    pub name: &'static str,
    /// Byte order for multi-byte values.
    pub byte_order: ByteOrder,
    /// Whether VR is implicit or explicit.
    pub vr_encoding: VrEncoding,
    /// Whether pixel data is native or encapsulated.
    pub pixel_encoding: PixelEncoding,
    /// Whether the data stream is deflate-compressed.
    pub deflated: bool,
}

impl TransferSyntax {
    /// Returns `true` if this transfer syntax uses implicit VR.
    pub fn is_implicit_vr(&self) -> bool {
        self.vr_encoding == VrEncoding::Implicit
    }

    /// Returns `true` if this transfer syntax uses explicit VR.
    pub fn is_explicit_vr(&self) -> bool {
        self.vr_encoding == VrEncoding::Explicit
    }

    /// Returns `true` if pixel data is encapsulated (compressed).
    pub fn is_encapsulated(&self) -> bool {
        self.pixel_encoding == PixelEncoding::Encapsulated
    }

    /// Returns `true` if this is little-endian byte order.
    pub fn is_little_endian(&self) -> bool {
        self.byte_order == ByteOrder::LittleEndian
    }

    /// Looks up the VR for a tag when using implicit VR transfer syntax.
    /// Returns `None` if the VR cannot be determined.
    pub fn resolve_vr(&self, _tag: crate::tag::Tag) -> Option<Vr> {
        // TODO: Look up from data dictionary when impl-dict-tag is done.
        None
    }
}

impl fmt::Display for TransferSyntax {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.uid)
    }
}

/// Well-known transfer syntax definitions.
pub mod transfer_syntaxes {
    use super::*;

    pub const IMPLICIT_VR_LITTLE_ENDIAN: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2",
        name: "Implicit VR Little Endian",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Implicit,
        pixel_encoding: PixelEncoding::Native,
        deflated: false,
    };

    pub const EXPLICIT_VR_LITTLE_ENDIAN: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.1",
        name: "Explicit VR Little Endian",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Native,
        deflated: false,
    };

    pub const EXPLICIT_VR_BIG_ENDIAN: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.2",
        name: "Explicit VR Big Endian",
        byte_order: ByteOrder::BigEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Native,
        deflated: false,
    };

    pub const DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.1.99",
        name: "Deflated Explicit VR Little Endian",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Native,
        deflated: true,
    };

    pub const JPEG_BASELINE: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.4.50",
        name: "JPEG Baseline (Process 1)",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Encapsulated,
        deflated: false,
    };

    pub const JPEG_EXTENDED: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.4.51",
        name: "JPEG Extended (Process 2 & 4)",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Encapsulated,
        deflated: false,
    };

    pub const JPEG_LOSSLESS: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.4.57",
        name: "JPEG Lossless, Non-Hierarchical (Process 14)",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Encapsulated,
        deflated: false,
    };

    pub const JPEG_LOSSLESS_SV1: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.4.70",
        name: "JPEG Lossless, Non-Hierarchical, First-Order Prediction (Process 14 [Selection Value 1])",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Encapsulated,
        deflated: false,
    };

    pub const JPEG_LS_LOSSLESS: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.4.80",
        name: "JPEG-LS Lossless Image Compression",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Encapsulated,
        deflated: false,
    };

    pub const JPEG_LS_LOSSY: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.4.81",
        name: "JPEG-LS Lossy (Near-Lossless) Image Compression",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Encapsulated,
        deflated: false,
    };

    pub const JPEG_2000_LOSSLESS: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.4.90",
        name: "JPEG 2000 Image Compression (Lossless Only)",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Encapsulated,
        deflated: false,
    };

    pub const JPEG_2000: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.4.91",
        name: "JPEG 2000 Image Compression",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Encapsulated,
        deflated: false,
    };

    pub const RLE_LOSSLESS: TransferSyntax = TransferSyntax {
        uid: "1.2.840.10008.1.2.5",
        name: "RLE Lossless",
        byte_order: ByteOrder::LittleEndian,
        vr_encoding: VrEncoding::Explicit,
        pixel_encoding: PixelEncoding::Encapsulated,
        deflated: false,
    };

    /// All built-in transfer syntaxes.
    pub const ALL: &[&TransferSyntax] = &[
        &IMPLICIT_VR_LITTLE_ENDIAN,
        &EXPLICIT_VR_LITTLE_ENDIAN,
        &EXPLICIT_VR_BIG_ENDIAN,
        &DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN,
        &JPEG_BASELINE,
        &JPEG_EXTENDED,
        &JPEG_LOSSLESS,
        &JPEG_LOSSLESS_SV1,
        &JPEG_LS_LOSSLESS,
        &JPEG_LS_LOSSY,
        &JPEG_2000_LOSSLESS,
        &JPEG_2000,
        &RLE_LOSSLESS,
    ];

    /// Looks up a transfer syntax by UID string.
    pub fn by_uid(uid: &str) -> Option<&'static TransferSyntax> {
        ALL.iter().find(|ts| ts.uid == uid).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::transfer_syntaxes::*;

    #[test]
    fn lookup_by_uid() {
        let ts = by_uid("1.2.840.10008.1.2").unwrap();
        assert_eq!(ts.uid, IMPLICIT_VR_LITTLE_ENDIAN.uid);
        assert!(ts.is_implicit_vr());
        assert!(ts.is_little_endian());
        assert!(!ts.is_encapsulated());
    }

    #[test]
    fn explicit_vr_le() {
        let ts = by_uid("1.2.840.10008.1.2.1").unwrap();
        assert!(ts.is_explicit_vr());
        assert!(ts.is_little_endian());
    }

    #[test]
    fn jpeg_is_encapsulated() {
        let ts = by_uid("1.2.840.10008.1.2.4.50").unwrap();
        assert!(ts.is_encapsulated());
        assert!(ts.is_explicit_vr());
    }

    #[test]
    fn unknown_uid() {
        assert!(by_uid("1.2.3.4.5.6.7.8.9").is_none());
    }

    #[test]
    fn all_ts_have_unique_uids() {
        let mut uids: Vec<&str> = ALL.iter().map(|ts| ts.uid).collect();
        let count = uids.len();
        uids.sort();
        uids.dedup();
        assert_eq!(uids.len(), count, "duplicate UIDs in transfer syntax list");
    }
}
