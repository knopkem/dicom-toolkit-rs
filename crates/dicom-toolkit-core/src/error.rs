//! DICOM error types, replacing DCMTK's `OFCondition` system.
//!
//! Every fallible operation returns `DcmResult<T>` which is an alias for
//! `Result<T, DcmError>`.

use std::fmt;

/// Central error type for all dcmtk-rs operations.
///
/// Mirrors the module+code+text structure of DCMTK's `OFCondition` but uses
/// Rust's enum-based error handling for exhaustive matching.
#[derive(Debug, thiserror::Error)]
pub enum DcmError {
    // ── I/O ──────────────────────────────────────────────────────────────
    /// Wraps a `std::io::Error`.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Unexpected end of data while parsing.
    #[error("unexpected end of data at offset {offset}")]
    UnexpectedEof { offset: u64 },

    // ── Parsing / Encoding ───────────────────────────────────────────────
    /// The DICOM preamble or prefix ("DICM") is invalid.
    #[error("invalid DICOM file: {reason}")]
    InvalidFile { reason: String },

    /// A tag could not be interpreted.
    #[error("invalid tag ({group:04X},{element:04X}): {reason}")]
    InvalidTag {
        group: u16,
        element: u16,
        reason: String,
    },

    /// Value representation mismatch.
    #[error("VR mismatch for tag ({group:04X},{element:04X}): expected {expected}, found {found}")]
    VrMismatch {
        group: u16,
        element: u16,
        expected: String,
        found: String,
    },

    /// A value could not be decoded from the underlying bytes.
    #[error("invalid value for tag ({group:04X},{element:04X}): {reason}")]
    InvalidValue {
        group: u16,
        element: u16,
        reason: String,
    },

    /// An element has an invalid or unsupported length.
    #[error("invalid element length {length} for tag ({group:04X},{element:04X})")]
    InvalidLength {
        group: u16,
        element: u16,
        length: u64,
    },

    // ── Transfer Syntax ──────────────────────────────────────────────────
    /// The transfer syntax UID is not recognized or not supported.
    #[error("unsupported transfer syntax: {uid}")]
    UnsupportedTransferSyntax { uid: String },

    /// No codec is registered for a compressed transfer syntax.
    #[error("no codec available for transfer syntax: {uid}")]
    NoCodec { uid: String },

    // ── Data Dictionary ──────────────────────────────────────────────────
    /// A tag was not found in the data dictionary.
    #[error("unknown tag ({group:04X},{element:04X})")]
    UnknownTag { group: u16, element: u16 },

    // ── UID ──────────────────────────────────────────────────────────────
    /// A UID string is syntactically invalid.
    #[error("invalid UID: {reason}")]
    InvalidUid { reason: String },

    // ── Character Encoding ───────────────────────────────────────────────
    /// Character set conversion failed.
    #[error("character encoding error: {reason}")]
    CharsetError { reason: String },

    // ── Network ──────────────────────────────────────────────────────────
    /// DICOM association was rejected by the remote peer.
    #[error("association rejected: {reason}")]
    AssociationRejected { reason: String },

    /// DICOM association was aborted.
    #[error("association aborted: abort_source={abort_source}, reason={reason}")]
    AssociationAborted {
        abort_source: String,
        reason: String,
    },

    /// A DIMSE operation failed with a status code.
    #[error("DIMSE error: status 0x{status:04X} ({description})")]
    DimseError { status: u16, description: String },

    /// Network timeout.
    #[error("network timeout after {seconds}s")]
    Timeout { seconds: u64 },

    /// Presentation context negotiation failed.
    #[error("no accepted presentation context for SOP class {sop_class_uid}")]
    NoPresentationContext { sop_class_uid: String },

    // ── TLS ──────────────────────────────────────────────────────────────
    /// TLS handshake or transport error.
    #[error("TLS error: {reason}")]
    TlsError { reason: String },

    // ── Codec ────────────────────────────────────────────────────────────
    /// Image decompression failed.
    #[error("decompression error: {reason}")]
    DecompressionError { reason: String },

    /// Image compression failed.
    #[error("compression error: {reason}")]
    CompressionError { reason: String },

    // ── Generic ──────────────────────────────────────────────────────────
    /// Catch-all for errors that don't fit other variants.
    #[error("{0}")]
    Other(String),
}

/// Convenience alias used throughout the crate.
pub type DcmResult<T> = Result<T, DcmError>;

/// DICOM status codes returned in DIMSE responses.
///
/// Mirrors the status code definitions from DCMTK's `dimse.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DimseStatus(pub u16);

impl DimseStatus {
    pub const SUCCESS: Self = Self(0x0000);
    pub const CANCEL: Self = Self(0xFE00);
    pub const PENDING: Self = Self(0xFF00);
    pub const PENDING_WITH_WARNINGS: Self = Self(0xFF01);

    // Failure codes
    pub const REFUSED_OUT_OF_RESOURCES: Self = Self(0xA700);
    pub const REFUSED_MOVE_DESTINATION_UNKNOWN: Self = Self(0xA801);
    pub const ERROR_DATA_SET_DOES_NOT_MATCH: Self = Self(0xA900);
    pub const ERROR_CANNOT_UNDERSTAND: Self = Self(0xC000);

    /// Returns `true` if this status indicates success.
    pub fn is_success(self) -> bool {
        self.0 == 0x0000
    }

    /// Returns `true` if this status indicates a pending response (more results follow).
    pub fn is_pending(self) -> bool {
        self.0 == 0xFF00 || self.0 == 0xFF01
    }

    /// Returns `true` if this status indicates a failure.
    pub fn is_failure(self) -> bool {
        // Failures occupy ranges 0xAxxx, 0xBxxx, 0xCxxx
        matches!(self.0 >> 12, 0xA | 0xB | 0xC)
    }

    /// Returns `true` if this status indicates a warning.
    pub fn is_warning(self) -> bool {
        // Warnings: 0x0001 and 0xB000-range
        self.0 == 0x0001 || (self.0 >> 12 == 0xB && !self.is_failure())
    }
}

impl fmt::Display for DimseStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::SUCCESS => write!(f, "Success (0x0000)"),
            Self::CANCEL => write!(f, "Cancel (0xFE00)"),
            Self::PENDING => write!(f, "Pending (0xFF00)"),
            Self::PENDING_WITH_WARNINGS => write!(f, "Pending with warnings (0xFF01)"),
            other => write!(f, "Status 0x{:04X}", other.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimse_status_classification() {
        assert!(DimseStatus::SUCCESS.is_success());
        assert!(!DimseStatus::SUCCESS.is_failure());
        assert!(!DimseStatus::SUCCESS.is_pending());

        assert!(DimseStatus::PENDING.is_pending());
        assert!(DimseStatus::PENDING_WITH_WARNINGS.is_pending());

        assert!(DimseStatus::REFUSED_OUT_OF_RESOURCES.is_failure());
        assert!(DimseStatus::ERROR_CANNOT_UNDERSTAND.is_failure());
    }

    #[test]
    fn error_display() {
        let err = DcmError::InvalidTag {
            group: 0x0008,
            element: 0x0010,
            reason: "missing".into(),
        };
        assert_eq!(err.to_string(), "invalid tag (0008,0010): missing");
    }

    #[test]
    fn io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let dcm_err: DcmError = io_err.into();
        assert!(matches!(dcm_err, DcmError::Io(_)));
    }
}
