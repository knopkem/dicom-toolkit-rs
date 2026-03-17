//! Character set handling for DICOM's Specific Character Set (0008,0005).
//!
//! Wraps the `encoding_rs` crate to provide DICOM-aware character encoding
//! and decoding, replacing DCMTK's `oficonv` module.
//!
//! Supports:
//! - Single-byte character sets (Latin-1 through Latin-9, Cyrillic, Arabic,
//!   Greek, Hebrew, Thai)
//! - Multi-byte character sets (Japanese, Korean, Simplified Chinese)
//! - ISO 2022 code extensions with escape sequence switching
//! - UTF-8 (ISO_IR 192)

use crate::error::{DcmError, DcmResult};
use encoding_rs::Encoding;

/// Maps a DICOM Specific Character Set defined term to an `encoding_rs` encoding.
///
/// Reference: DICOM PS3.3 C.12.1.1.2, Table C.12-2 and C.12-3.
pub fn encoding_for_term(term: &str) -> DcmResult<&'static Encoding> {
    let encoding = match term.trim() {
        // Default repertoire — ASCII (we use WINDOWS_1252 as the nearest superset;
        // encoding_rs does not have a pure ISO 646/ASCII codec)
        "" | "ISO_IR 6" | "ISO 2022 IR 6" => encoding_rs::WINDOWS_1252,

        // Latin alphabet No. 1 — ISO 8859-1
        "ISO_IR 100" | "ISO 2022 IR 100" => encoding_rs::WINDOWS_1252,
        // Latin alphabet No. 2 — ISO 8859-2
        "ISO_IR 101" | "ISO 2022 IR 101" => encoding_rs::ISO_8859_2,
        // Latin alphabet No. 3 — ISO 8859-3
        "ISO_IR 109" | "ISO 2022 IR 109" => encoding_rs::ISO_8859_3,
        // Latin alphabet No. 4 — ISO 8859-4
        "ISO_IR 110" | "ISO 2022 IR 110" => encoding_rs::ISO_8859_4,
        // Cyrillic — ISO 8859-5
        "ISO_IR 144" | "ISO 2022 IR 144" => encoding_rs::ISO_8859_5,
        // Arabic — ISO 8859-6
        "ISO_IR 127" | "ISO 2022 IR 127" => encoding_rs::ISO_8859_6,
        // Greek — ISO 8859-7
        "ISO_IR 126" | "ISO 2022 IR 126" => encoding_rs::ISO_8859_7,
        // Hebrew — ISO 8859-8
        "ISO_IR 138" | "ISO 2022 IR 138" => encoding_rs::ISO_8859_8,
        // Latin alphabet No. 5 — ISO 8859-9 (encoding_rs maps WINDOWS_1254 ≈ 8859-9)
        "ISO_IR 148" | "ISO 2022 IR 148" => encoding_rs::WINDOWS_1254,
        // Latin alphabet No. 9 — ISO 8859-15 (encoding_rs maps ISO_8859_15)
        "ISO_IR 203" | "ISO 2022 IR 203" => encoding_rs::ISO_8859_15,

        // Thai — TIS 620-2533 (WINDOWS_874 is the superset)
        "ISO_IR 166" | "ISO 2022 IR 166" => encoding_rs::WINDOWS_874,

        // Japanese — JIS X 0201 (Shift_JIS covers both Romaji and Katakana halves)
        "ISO_IR 13" | "ISO 2022 IR 13" => encoding_rs::SHIFT_JIS,
        // Japanese — JIS X 0208 (Kanji) via ISO-2022-JP
        "ISO 2022 IR 87" => encoding_rs::ISO_2022_JP,
        // Japanese — JIS X 0212 (Supplementary Kanji)
        "ISO 2022 IR 159" => encoding_rs::ISO_2022_JP,

        // Korean — KS X 1001
        "ISO 2022 IR 149" => encoding_rs::EUC_KR,
        // Simplified Chinese — GB 2312
        "ISO 2022 IR 58" => encoding_rs::GB18030,

        // Unicode
        "ISO_IR 192" => encoding_rs::UTF_8,

        // GBK / GB18030 (extensions used by some Chinese implementations)
        "GBK" => encoding_rs::GBK,
        "GB18030" => encoding_rs::GB18030,

        _ => {
            return Err(DcmError::CharsetError {
                reason: format!("unknown DICOM character set term: '{term}'"),
            });
        }
    };
    Ok(encoding)
}

/// Decodes a byte slice using the specified DICOM character set term.
pub fn decode_string(bytes: &[u8], term: &str) -> DcmResult<String> {
    let encoding = encoding_for_term(term)?;
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        return Err(DcmError::CharsetError {
            reason: format!("decoding error using charset '{term}'"),
        });
    }
    Ok(decoded.into_owned())
}

/// Encodes a string using the specified DICOM character set term.
pub fn encode_string(s: &str, term: &str) -> DcmResult<Vec<u8>> {
    let encoding = encoding_for_term(term)?;
    let (encoded, _, had_errors) = encoding.encode(s);
    if had_errors {
        return Err(DcmError::CharsetError {
            reason: format!("encoding error using charset '{term}'"),
        });
    }
    Ok(encoded.into_owned())
}

/// Handles DICOM's multi-valued Specific Character Set with ISO 2022
/// code extension support.
///
/// When (0008,0005) contains multiple values (e.g., `ISO_IR 100\ISO 2022 IR 87`),
/// different segments of a string may use different encodings, separated by
/// ISO 2022 escape sequences. This decoder handles the segment splitting and
/// per-segment decoding, matching DCMTK's `DcmSpecificCharacterSet`.
pub struct DicomCharsetDecoder {
    /// Default encoding (first term, or ASCII if empty).
    default_encoding: &'static Encoding,
    /// Default defined term (used to restore ISO 2022 state).
    default_term: String,
    /// Scan mode for the default term.
    default_scan_mode: ScanMode,
    /// Map from ISO 2022 defined term → encoding, for code extension switching.
    extensions: Vec<(String, &'static Encoding)>,
    /// True if we have multiple charsets (ISO 2022 code extensions).
    has_extensions: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanMode {
    SingleByte,
    FixedWidth(usize),
    HighBitLead(usize),
}

/// ISO 2022 escape sequence to DICOM defined term mapping.
///
/// Ported from DCMTK dcspchrs.cc `convertStringWithCodeExtensions()`.
fn escape_to_term(esc: &[u8]) -> Option<&'static str> {
    if esc.len() < 2 {
        return None;
    }
    match (esc[0], esc[1]) {
        (0x28, 0x42) => Some("ISO 2022 IR 6"),   // ASCII, G0
        (0x2D, 0x41) => Some("ISO 2022 IR 100"), // Latin-1, G1
        (0x2D, 0x42) => Some("ISO 2022 IR 101"), // Latin-2, G1
        (0x2D, 0x43) => Some("ISO 2022 IR 109"), // Latin-3, G1
        (0x2D, 0x44) => Some("ISO 2022 IR 110"), // Latin-4, G1
        (0x2D, 0x4C) => Some("ISO 2022 IR 144"), // Cyrillic, G1
        (0x2D, 0x47) => Some("ISO 2022 IR 127"), // Arabic, G1
        (0x2D, 0x46) => Some("ISO 2022 IR 126"), // Greek, G1
        (0x2D, 0x48) => Some("ISO 2022 IR 138"), // Hebrew, G1
        (0x2D, 0x4D) => Some("ISO 2022 IR 148"), // Latin-5, G1
        (0x2D, 0x62) => Some("ISO 2022 IR 203"), // Latin-9, G1
        (0x29, 0x49) => Some("ISO 2022 IR 13"),  // Japanese Katakana, G1
        (0x28, 0x4A) => Some("ISO 2022 IR 13"),  // Japanese Romaji, G0
        (0x2D, 0x54) => Some("ISO 2022 IR 166"), // Thai, G1
        (0x24, 0x42) => Some("ISO 2022 IR 87"),  // Japanese Kanji (JIS X0208)
        (0x24, 0x28) if esc.len() >= 3 && esc[2] == 0x44 => {
            Some("ISO 2022 IR 159") // Japanese Supplementary Kanji (JIS X0212)
        }
        (0x24, 0x29) if esc.len() >= 3 => match esc[2] {
            0x43 => Some("ISO 2022 IR 149"), // Korean
            0x41 => Some("ISO 2022 IR 58"),  // Simplified Chinese
            _ => None,
        },
        _ => None,
    }
}

/// Returns the length of an ISO 2022 escape sequence (after the ESC byte).
fn escape_seq_len(data: &[u8]) -> usize {
    if data.len() < 2 {
        return 0;
    }
    match (data[0], data[1]) {
        (0x24, 0x28) | (0x24, 0x29) => 3, // 4-byte sequences (ESC + 3)
        _ => 2,                           // 3-byte sequences (ESC + 2)
    }
}

impl DicomCharsetDecoder {
    /// Creates a new decoder from a DICOM Specific Character Set value.
    ///
    /// The value may contain multiple backslash-separated terms.
    pub fn new(specific_charset: &str) -> DcmResult<Self> {
        let terms: Vec<&str> = specific_charset.split('\\').collect();
        let default_term = terms.first().copied().unwrap_or("").trim().to_string();
        let default_encoding = encoding_for_term(&default_term)?;
        let default_scan_mode = scan_mode_for_term(&default_term);

        let mut extensions = Vec::new();
        let mut has_extensions = false;
        for term in terms.iter().skip(1) {
            let trimmed = term.trim();
            if !trimmed.is_empty() {
                let enc = encoding_for_term(trimmed)?;
                extensions.push((trimmed.to_string(), enc));
                has_extensions = true;
            }
        }

        // Also register the default encoding under its "ISO 2022 IR" name,
        // and always include ASCII as a fallback.
        if has_extensions {
            let first_term = terms.first().copied().unwrap_or("").trim();
            if !first_term.is_empty() {
                extensions.push((first_term.to_string(), default_encoding));
            }
            // ASCII is always available
            extensions.push(("ISO 2022 IR 6".to_string(), encoding_rs::WINDOWS_1252));
        }

        Ok(Self {
            default_encoding,
            default_term,
            default_scan_mode,
            extensions,
            has_extensions,
        })
    }

    /// Create a decoder for a single (non-ISO 2022) charset.
    pub fn single(encoding: &'static Encoding) -> Self {
        Self {
            default_encoding: encoding,
            default_term: String::new(),
            default_scan_mode: ScanMode::SingleByte,
            extensions: Vec::new(),
            has_extensions: false,
        }
    }

    /// Default decoder (ASCII / WINDOWS-1252).
    pub fn default_ascii() -> Self {
        Self {
            default_encoding: encoding_rs::WINDOWS_1252,
            default_term: String::new(),
            default_scan_mode: ScanMode::SingleByte,
            extensions: Vec::new(),
            has_extensions: false,
        }
    }

    /// Return the default encoding.
    pub fn default_encoding(&self) -> &'static Encoding {
        self.default_encoding
    }

    /// Decodes a byte string using the configured character sets.
    ///
    /// For single-charset configs, decodes the whole buffer with the default encoding.
    /// For multi-charset configs (ISO 2022 code extensions), splits on ESC sequences
    /// and decodes each segment with the appropriate encoding.
    pub fn decode(&self, bytes: &[u8]) -> DcmResult<String> {
        if bytes.is_empty() {
            return Ok(String::new());
        }

        // Fast path: UTF-8 input — avoid re-encoding
        if self.default_encoding == encoding_rs::UTF_8 && !self.has_extensions {
            return match std::str::from_utf8(bytes) {
                Ok(s) => Ok(s.to_string()),
                Err(_) => Ok(String::from_utf8_lossy(bytes).into_owned()),
            };
        }

        // No code extensions: simple single-encoding decode
        if !self.has_extensions {
            return self.decode_with(bytes, self.default_encoding);
        }

        // ISO 2022 code-extension mode: scan for ESC (0x1B) and delimiter
        // characters, decode each segment with the active charset.
        self.decode_with_extensions(bytes)
    }

    /// Encode a string back to bytes using the default encoding.
    pub fn encode(&self, s: &str) -> DcmResult<Vec<u8>> {
        if self.default_encoding == encoding_rs::UTF_8 {
            return Ok(s.as_bytes().to_vec());
        }
        let (encoded, _, had_errors) = self.default_encoding.encode(s);
        if had_errors {
            return Err(DcmError::CharsetError {
                reason: "character encoding error".into(),
            });
        }
        Ok(encoded.into_owned())
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    fn decode_with(&self, bytes: &[u8], encoding: &'static Encoding) -> DcmResult<String> {
        let (decoded, _, had_errors) = encoding.decode(bytes);
        if had_errors {
            // Fall back to lossy decode rather than hard error — many real-world
            // DICOM files have minor charset issues.
            let (lossy, _, _) = encoding.decode(bytes);
            return Ok(lossy.into_owned());
        }
        Ok(decoded.into_owned())
    }

    fn decode_with_extensions(&self, bytes: &[u8]) -> DcmResult<String> {
        let mut result = String::new();
        let mut current_term = self.default_term.as_str();
        let mut current_encoding = self.default_encoding;
        let mut current_scan_mode = self.default_scan_mode;
        let mut segment_start = 0;
        let mut pos = 0;

        while pos < bytes.len() {
            let b = bytes[pos];

            // Check for ESC (0x1B) — charset switch
            if b == 0x1B {
                // Decode segment before the ESC
                if pos > segment_start {
                    let segment = &bytes[segment_start..pos];
                    result.push_str(&self.decode_segment(
                        segment,
                        current_term,
                        current_encoding,
                    )?);
                }

                // Parse escape sequence
                let remaining = &bytes[pos + 1..];
                let esc_len = escape_seq_len(remaining);

                if esc_len > 0 && remaining.len() >= esc_len {
                    if let Some(term) = escape_to_term(&remaining[..esc_len]) {
                        // Look up the encoding for this term
                        current_term = term;
                        current_encoding = self.find_encoding(term);
                        current_scan_mode = scan_mode_for_term(term);
                    }
                    pos += 1 + esc_len; // skip ESC + sequence bytes
                } else {
                    // Unknown escape sequence — skip ESC and emit it
                    pos += 1;
                }
                segment_start = pos;
                continue;
            }

            // Delimiters (CR, LF, FF, HT) reset to default encoding
            // per ISO 2022 / DICOM PS3.5 6.1.2.5.3
            if b == 0x0D || b == 0x0A || b == 0x0C || b == 0x09 {
                // Decode segment before delimiter
                if pos > segment_start {
                    let segment = &bytes[segment_start..pos];
                    result.push_str(&self.decode_segment(
                        segment,
                        current_term,
                        current_encoding,
                    )?);
                }
                result.push(b as char);
                current_term = self.default_term.as_str();
                current_encoding = self.default_encoding;
                current_scan_mode = self.default_scan_mode;
                pos += 1;
                segment_start = pos;
                continue;
            }

            if let Some(skip) = current_scan_mode.skip_bytes(b, pos, bytes.len()) {
                pos += skip;
            }
            pos += 1;
        }

        // Decode final segment
        if segment_start < bytes.len() {
            let segment = &bytes[segment_start..];
            result.push_str(&self.decode_segment(segment, current_term, current_encoding)?);
        }

        Ok(result)
    }

    fn decode_segment(
        &self,
        bytes: &[u8],
        term: &str,
        encoding: &'static Encoding,
    ) -> DcmResult<String> {
        if bytes.is_empty() {
            return Ok(String::new());
        }
        let wrapped;
        let bytes = if let Some(segment) = wrap_iso2022_segment(term, bytes) {
            wrapped = segment;
            wrapped.as_slice()
        } else {
            bytes
        };
        let (decoded, _, had_errors) = encoding.decode(bytes);
        if had_errors && matches!(term, "ISO 2022 IR 87" | "ISO 2022 IR 159") {
            return Err(DcmError::CharsetError {
                reason: format!("decoding error using charset '{term}'"),
            });
        }
        Ok(decoded.into_owned())
    }

    fn find_encoding(&self, term: &str) -> &'static Encoding {
        for (t, enc) in &self.extensions {
            if t == term {
                return enc;
            }
        }
        // Fallback: try the global mapping
        encoding_for_term(term).unwrap_or(self.default_encoding)
    }
}

impl ScanMode {
    fn skip_bytes(self, first_byte: u8, pos: usize, len: usize) -> Option<usize> {
        match self {
            ScanMode::SingleByte => None,
            ScanMode::FixedWidth(width) if width > 1 && pos + width - 1 < len => Some(width - 1),
            ScanMode::HighBitLead(width)
                if width > 1 && (first_byte & 0x80) != 0 && pos + width - 1 < len =>
            {
                Some(width - 1)
            }
            _ => None,
        }
    }
}

fn scan_mode_for_term(term: &str) -> ScanMode {
    match term {
        "ISO 2022 IR 87" | "ISO 2022 IR 159" | "ISO 2022 IR 58" => ScanMode::FixedWidth(2),
        "ISO 2022 IR 149" => ScanMode::HighBitLead(2),
        _ => ScanMode::SingleByte,
    }
}

fn wrap_iso2022_segment(term: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    let prefix = match term {
        "ISO 2022 IR 87" => &[0x1B, 0x24, 0x42][..],
        "ISO 2022 IR 159" => &[0x1B, 0x24, 0x28, 0x44][..],
        _ => return None,
    };

    let mut wrapped = Vec::with_capacity(prefix.len() + bytes.len() + 3);
    wrapped.extend_from_slice(prefix);
    wrapped.extend_from_slice(bytes);
    wrapped.extend_from_slice(&[0x1B, 0x28, 0x42]);
    Some(wrapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_charset() {
        assert!(encoding_for_term("").is_ok());
        assert!(encoding_for_term("ISO_IR 6").is_ok());
        assert!(encoding_for_term("ISO 2022 IR 6").is_ok());
    }

    #[test]
    fn utf8_charset() {
        let encoding = encoding_for_term("ISO_IR 192").unwrap();
        assert_eq!(encoding, encoding_rs::UTF_8);
    }

    #[test]
    fn latin1_maps_to_windows1252() {
        // ISO_IR 100 must map to WINDOWS_1252 (superset of ISO-8859-1)
        let enc = encoding_for_term("ISO_IR 100").unwrap();
        assert_eq!(enc, encoding_rs::WINDOWS_1252);
    }

    #[test]
    fn latin9_supported() {
        let enc = encoding_for_term("ISO_IR 203").unwrap();
        assert_eq!(enc, encoding_rs::ISO_8859_15);
    }

    #[test]
    fn unknown_charset() {
        assert!(encoding_for_term("UNKNOWN_CHARSET").is_err());
    }

    #[test]
    fn decode_ascii() {
        let result = decode_string(b"Hello", "").unwrap();
        assert_eq!(result, "Hello");
    }

    #[test]
    fn decode_utf8() {
        let result = decode_string("日本語".as_bytes(), "ISO_IR 192").unwrap();
        assert_eq!(result, "日本語");
    }

    #[test]
    fn decode_latin1_umlaut() {
        // "Müller" in ISO-8859-1: ü = 0xFC
        let bytes = vec![b'M', 0xFC, b'l', b'l', b'e', b'r'];
        let result = decode_string(&bytes, "ISO_IR 100").unwrap();
        assert_eq!(result, "Müller");
    }

    #[test]
    fn decode_latin2() {
        // Polish "Łódź" in ISO-8859-2: Ł=0xA3, ó=0xF3, ź=0xBC
        let bytes = vec![0xA3, 0xF3, b'd', 0xBC];
        let result = decode_string(&bytes, "ISO_IR 101").unwrap();
        assert_eq!(result, "Łódź");
    }

    #[test]
    fn decode_cyrillic() {
        // "Иванов" in ISO-8859-5
        let bytes = vec![0xB8, 0xD2, 0xD0, 0xDD, 0xDE, 0xD2];
        let result = decode_string(&bytes, "ISO_IR 144").unwrap();
        assert_eq!(result, "Иванов");
    }

    #[test]
    fn encode_roundtrip_latin1() {
        let original = "Müller^Hans";
        let encoded = encode_string(original, "ISO_IR 100").unwrap();
        let decoded = decode_string(&encoded, "ISO_IR 100").unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn multi_charset_decoder_single() {
        let decoder = DicomCharsetDecoder::new("ISO_IR 100").unwrap();
        let bytes = vec![b'M', 0xFC, b'l', b'l', b'e', b'r'];
        let result = decoder.decode(&bytes).unwrap();
        assert_eq!(result, "Müller");
    }

    #[test]
    fn multi_charset_decoder_utf8() {
        let decoder = DicomCharsetDecoder::new("ISO_IR 192").unwrap();
        let result = decoder.decode("田中太郎".as_bytes()).unwrap();
        assert_eq!(result, "田中太郎");
    }

    #[test]
    fn escape_to_term_known_sequences() {
        assert_eq!(escape_to_term(&[0x28, 0x42]), Some("ISO 2022 IR 6"));
        assert_eq!(escape_to_term(&[0x2D, 0x41]), Some("ISO 2022 IR 100"));
        assert_eq!(escape_to_term(&[0x24, 0x42]), Some("ISO 2022 IR 87"));
        assert_eq!(escape_to_term(&[0x24, 0x28, 0x44]), Some("ISO 2022 IR 159"));
        assert_eq!(escape_to_term(&[0x24, 0x29, 0x43]), Some("ISO 2022 IR 149"));
        assert_eq!(escape_to_term(&[0x24, 0x29, 0x41]), Some("ISO 2022 IR 58"));
    }

    #[test]
    fn escape_to_term_unknown() {
        assert_eq!(escape_to_term(&[0x99, 0x99]), None);
    }

    #[test]
    fn decoder_with_iso2022_japanese() {
        // Simulate: Latin text, then ESC to JIS X0208, then ESC back to ASCII.
        // "Yamada^日本"
        let decoder = DicomCharsetDecoder::new("\\ISO 2022 IR 87").unwrap();
        let bytes = [
            b'Y', b'a', b'm', b'a', b'd', b'a', b'^', 0x1B, 0x24, 0x42, 0x46, 0x7C, 0x4B, 0x5C,
            0x1B, 0x28, 0x42,
        ];
        let result = decoder.decode(&bytes).unwrap();
        assert_eq!(result, "Yamada^日本");
    }

    #[test]
    fn decoder_encode_roundtrip() {
        let decoder = DicomCharsetDecoder::new("ISO_IR 100").unwrap();
        let original = "Schöne Grüße";
        let encoded = decoder.encode(original).unwrap();
        let decoded = decoder.decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }
}
