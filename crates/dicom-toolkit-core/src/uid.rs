//! DICOM Unique Identifier (UID) handling.
//!
//! Ports the UID generation and well-known UID constant functionality from
//! DCMTK's `dcuid.h` / `ofuuid.h`.

use crate::error::{DcmError, DcmResult};
use std::fmt;

/// Maximum length of a DICOM UID (64 characters per PS3.5 §9.1).
pub const MAX_UID_LENGTH: usize = 64;

/// A validated DICOM UID.
///
/// UIDs consist of dot-separated numeric components (digits and dots only),
/// with a maximum length of 64 characters.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Uid(String);

impl Uid {
    /// Creates a new `Uid` from a string, validating the format.
    pub fn new(s: impl Into<String>) -> DcmResult<Self> {
        let s = s.into();
        Self::validate(&s)?;
        Ok(Self(s))
    }

    /// Returns `true` if `s` is a syntactically valid DICOM UID.
    pub fn is_valid(s: &str) -> bool {
        Self::validate(s).is_ok()
    }

    /// Creates a `Uid` without validation. Use only for known-valid UIDs
    /// (e.g., compile-time constants).
    ///
    /// # Safety (logical)
    /// The caller must ensure the string is a valid DICOM UID.
    pub const fn from_static(_s: &'static str) -> Self {
        // Can't validate at const time, but used only for known constants.
        Self(String::new()) // placeholder — see below
    }

    /// Internal helper that creates a Uid from a known-valid string at runtime.
    #[allow(dead_code)]
    pub(crate) fn from_valid(s: String) -> Self {
        Self(s)
    }

    /// Returns the UID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Generates a new unique UID under the given `root` prefix.
    ///
    /// The generated UID has the form `{root}.{uuid_based_suffix}` and is
    /// guaranteed to be ≤ 64 characters.
    pub fn generate(root: &str) -> DcmResult<Self> {
        let uuid_val = uuid::Uuid::new_v4();
        // Convert UUID to numeric form (remove hyphens, treat as decimal-ish)
        let uuid_bytes = uuid_val.as_bytes();
        let num = u128::from_be_bytes(*uuid_bytes);
        let suffix = num.to_string();

        let uid_str = format!("{root}.{suffix}");
        if uid_str.len() > MAX_UID_LENGTH {
            // Truncate suffix to fit
            let max_suffix = MAX_UID_LENGTH - root.len() - 1;
            let uid_str = format!("{root}.{}", &suffix[..max_suffix]);
            return Self::new(uid_str);
        }
        Self::new(uid_str)
    }

    /// Validates that a string is a legal DICOM UID.
    fn validate(s: &str) -> DcmResult<()> {
        if s.is_empty() {
            return Err(DcmError::InvalidUid {
                reason: "UID must not be empty".into(),
            });
        }
        if s.len() > MAX_UID_LENGTH {
            return Err(DcmError::InvalidUid {
                reason: format!(
                    "UID exceeds maximum length of {MAX_UID_LENGTH}: got {}",
                    s.len()
                ),
            });
        }
        if s.starts_with('.') || s.ends_with('.') {
            return Err(DcmError::InvalidUid {
                reason: "UID must not start or end with a dot".into(),
            });
        }
        if s.contains("..") {
            return Err(DcmError::InvalidUid {
                reason: "UID must not contain consecutive dots".into(),
            });
        }
        for ch in s.chars() {
            if !ch.is_ascii_digit() && ch != '.' {
                return Err(DcmError::InvalidUid {
                    reason: format!("UID contains invalid character: '{ch}'"),
                });
            }
        }
        // Each component must not have a leading zero (except "0" itself)
        for component in s.split('.') {
            if component.is_empty() {
                return Err(DcmError::InvalidUid {
                    reason: "UID contains an empty component".into(),
                });
            }
            if component.len() > 1 && component.starts_with('0') {
                return Err(DcmError::InvalidUid {
                    reason: format!("UID component has leading zero: '{component}'"),
                });
            }
        }
        Ok(())
    }
}

impl fmt::Debug for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uid(\"{}\")", self.0)
    }
}

impl fmt::Display for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Uid {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Uid {
    type Error = DcmError;
    fn try_from(s: String) -> DcmResult<Self> {
        Self::new(s)
    }
}

impl TryFrom<&str> for Uid {
    type Error = DcmError;
    fn try_from(s: &str) -> DcmResult<Self> {
        Self::new(s)
    }
}

// ── Well-Known DICOM UIDs ────────────────────────────────────────────────

/// Well-known DICOM SOP Class UIDs.
///
/// Ported from DCMTK's `dcuid.h`.
pub mod sop_class {
    /// Verification SOP Class (C-ECHO).
    pub const VERIFICATION: &str = "1.2.840.10008.1.1";

    // ── Storage SOP Classes ──────────────────────────────────────────
    pub const CT_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.2";
    pub const ENHANCED_CT_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.2.1";
    pub const MR_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.4";
    pub const ENHANCED_MR_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.4.1";
    pub const ULTRASOUND_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.6.1";
    pub const SECONDARY_CAPTURE_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.7";
    pub const DIGITAL_XRAY_IMAGE_STORAGE_FOR_PRESENTATION: &str = "1.2.840.10008.5.1.4.1.1.1.1";
    pub const DIGITAL_XRAY_IMAGE_STORAGE_FOR_PROCESSING: &str = "1.2.840.10008.5.1.4.1.1.1.1.1";
    pub const DIGITAL_MAMMOGRAPHY_IMAGE_STORAGE_FOR_PRESENTATION: &str =
        "1.2.840.10008.5.1.4.1.1.1.2";
    pub const CR_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.1";
    pub const NM_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.20";
    pub const PET_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.128";
    pub const RT_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.481.1";
    pub const RT_DOSE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.481.2";
    pub const RT_STRUCTURE_SET_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.481.3";
    pub const RT_PLAN_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.481.5";
    pub const XA_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.12.1";
    pub const VL_PHOTOGRAPHIC_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.77.1.4";
    pub const VIDEO_ENDOSCOPIC_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.77.1.1.1";
    pub const ENCAPSULATED_PDF_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.104.1";
    pub const ENCAPSULATED_CDA_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.104.2";
    pub const BASIC_TEXT_SR_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.88.11";
    pub const ENHANCED_SR_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.88.22";
    pub const COMPREHENSIVE_SR_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.88.33";
    pub const SEGMENTATION_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.66.4";

    // ── Query/Retrieve SOP Classes ───────────────────────────────────
    pub const PATIENT_ROOT_QR_FIND: &str = "1.2.840.10008.5.1.4.1.2.1.1";
    pub const PATIENT_ROOT_QR_MOVE: &str = "1.2.840.10008.5.1.4.1.2.1.2";
    pub const PATIENT_ROOT_QR_GET: &str = "1.2.840.10008.5.1.4.1.2.1.3";
    pub const STUDY_ROOT_QR_FIND: &str = "1.2.840.10008.5.1.4.1.2.2.1";
    pub const STUDY_ROOT_QR_MOVE: &str = "1.2.840.10008.5.1.4.1.2.2.2";
    pub const STUDY_ROOT_QR_GET: &str = "1.2.840.10008.5.1.4.1.2.2.3";

    // ── Worklist SOP Classes ─────────────────────────────────────────
    pub const MODALITY_WORKLIST_FIND: &str = "1.2.840.10008.5.1.4.31";

    // ── Print Management ─────────────────────────────────────────────
    pub const BASIC_FILM_SESSION: &str = "1.2.840.10008.5.1.1.1";
    pub const BASIC_FILM_BOX: &str = "1.2.840.10008.5.1.1.2";
    pub const BASIC_GRAYSCALE_IMAGE_BOX: &str = "1.2.840.10008.5.1.1.4";
    pub const BASIC_COLOR_IMAGE_BOX: &str = "1.2.840.10008.5.1.1.4.1";
    pub const PRINTER: &str = "1.2.840.10008.5.1.1.16";

    // ── Storage Commitment ───────────────────────────────────────────
    pub const STORAGE_COMMITMENT_PUSH_MODEL: &str = "1.2.840.10008.1.20.1";
}

/// Well-known DICOM Transfer Syntax UIDs.
///
/// Ported from DCMTK's `dcxfer.h`.
pub mod transfer_syntax {
    pub const IMPLICIT_VR_LITTLE_ENDIAN: &str = "1.2.840.10008.1.2";
    pub const EXPLICIT_VR_LITTLE_ENDIAN: &str = "1.2.840.10008.1.2.1";
    pub const EXPLICIT_VR_BIG_ENDIAN: &str = "1.2.840.10008.1.2.2";
    pub const DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN: &str = "1.2.840.10008.1.2.1.99";

    // JPEG
    pub const JPEG_BASELINE: &str = "1.2.840.10008.1.2.4.50";
    pub const JPEG_EXTENDED: &str = "1.2.840.10008.1.2.4.51";
    pub const JPEG_LOSSLESS_NON_HIERARCHICAL: &str = "1.2.840.10008.1.2.4.57";
    pub const JPEG_LOSSLESS_NON_HIERARCHICAL_FIRST_ORDER: &str = "1.2.840.10008.1.2.4.70";

    // JPEG-LS
    pub const JPEG_LS_LOSSLESS: &str = "1.2.840.10008.1.2.4.80";
    pub const JPEG_LS_LOSSY: &str = "1.2.840.10008.1.2.4.81";

    // JPEG 2000
    pub const JPEG_2000_LOSSLESS: &str = "1.2.840.10008.1.2.4.90";
    pub const JPEG_2000: &str = "1.2.840.10008.1.2.4.91";

    // RLE
    pub const RLE_LOSSLESS: &str = "1.2.840.10008.1.2.5";

    // Encapsulated Uncompressed
    pub const ENCAPSULATED_UNCOMPRESSED: &str = "1.2.840.10008.1.2.1.98";
}

/// DCMTK-RS implementation class UID root.
pub const DCMTK_RS_UID_ROOT: &str = "1.2.826.0.1.3680043.8.498";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_uid() {
        assert!(Uid::new("1.2.840.10008.1.1").is_ok());
        assert!(Uid::new("1.2.3").is_ok());
        assert!(Uid::new("0").is_ok());
    }

    #[test]
    fn invalid_uid_empty() {
        assert!(Uid::new("").is_err());
    }

    #[test]
    fn invalid_uid_too_long() {
        let long = "1.".repeat(33);
        assert!(Uid::new(&long[..long.len() - 1]).is_err());
    }

    #[test]
    fn invalid_uid_leading_dot() {
        assert!(Uid::new(".1.2.3").is_err());
    }

    #[test]
    fn invalid_uid_trailing_dot() {
        assert!(Uid::new("1.2.3.").is_err());
    }

    #[test]
    fn invalid_uid_consecutive_dots() {
        assert!(Uid::new("1.2..3").is_err());
    }

    #[test]
    fn invalid_uid_non_numeric() {
        assert!(Uid::new("1.2.abc").is_err());
    }

    #[test]
    fn invalid_uid_leading_zero() {
        assert!(Uid::new("1.02.3").is_err());
    }

    #[test]
    fn generate_uid() {
        let uid = Uid::generate("1.2.3").unwrap();
        assert!(uid.as_str().starts_with("1.2.3."));
        assert!(uid.as_str().len() <= MAX_UID_LENGTH);
    }

    #[test]
    fn uid_display() {
        let uid = Uid::new("1.2.840.10008.1.1").unwrap();
        assert_eq!(uid.to_string(), "1.2.840.10008.1.1");
    }
}
