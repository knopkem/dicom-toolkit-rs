//! DICOM Part 10 file reader.
//!
//! Reads binary DICOM files into a `FileFormat` or raw `DataSet`.

use crate::dataset::DataSet;
use crate::element::Element;
use crate::file_format::FileFormat;
use crate::io::transfer::{implicit_vr_for_tag, TransferSyntaxProperties};
use crate::meta_info::FileMetaInformation;
use crate::value::{DicomDate, DicomDateTime, DicomTime, PersonName, PixelData, Value};
use dicom_toolkit_core::charset::DicomCharsetDecoder;
use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_dict::{tags, Tag, Vr};
use std::io::Read;

/// Streaming DICOM reader.
pub struct DicomReader<R: Read> {
    reader: R,
}

impl<R: Read> DicomReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    /// Read a complete DICOM Part 10 file.
    pub fn read_file(&mut self) -> DcmResult<FileFormat> {
        let mut data = Vec::new();
        self.reader.read_to_end(&mut data)?;
        parse_file(&data)
    }

    /// Read a raw dataset (no preamble/meta) using the given transfer syntax UID.
    pub fn read_dataset(&mut self, ts_uid: &str) -> DcmResult<DataSet> {
        let mut data = Vec::new();
        self.reader.read_to_end(&mut data)?;
        let props = TransferSyntaxProperties::from_uid(ts_uid);
        let actual: std::borrow::Cow<[u8]> = if props.is_deflated {
            std::borrow::Cow::Owned(decompress_deflated(&data)?)
        } else {
            std::borrow::Cow::Borrowed(&data)
        };
        let mut cursor = DicomCursor::new(&actual);
        cursor.read_dataset_impl(
            props.is_explicit_vr(),
            props.is_little_endian(),
            actual.len(),
        )
    }
}

// ── Internal parse entry point ────────────────────────────────────────────────

pub(crate) fn parse_file(data: &[u8]) -> DcmResult<FileFormat> {
    // Short files without preamble — try raw implicit VR LE
    if data.len() < 132 {
        let mut cursor = DicomCursor::new(data);
        let ds = cursor.read_dataset_impl(false, true, data.len())?;
        let meta = FileMetaInformation::new("", "", "1.2.840.10008.1.2");
        return Ok(FileFormat::new(meta, ds));
    }

    let has_dicm = &data[128..132] == b"DICM";

    if !has_dicm {
        // No magic: try implicit VR LE from the start
        let mut cursor = DicomCursor::new(data);
        let ds = cursor.read_dataset_impl(false, true, data.len())?;
        let meta = FileMetaInformation::new("", "", "1.2.840.10008.1.2");
        return Ok(FileFormat::new(meta, ds));
    }

    let mut cursor = DicomCursor::new(data);
    cursor.pos = 132; // skip 128-byte preamble + "DICM"

    // Read File Meta Information (always explicit VR LE)
    let meta_ds = cursor.read_meta()?;
    let meta = FileMetaInformation::from_dataset(&meta_ds)?;
    let ts_uid = meta.transfer_syntax_uid.clone();
    let props = TransferSyntaxProperties::from_uid(&ts_uid);

    let dataset = if props.is_deflated {
        let remaining = &data[cursor.pos..];
        let decompressed = decompress_deflated(remaining)?;
        let mut dc = DicomCursor::new(&decompressed);
        dc.read_dataset_impl(true, true, decompressed.len())?
    } else {
        cursor.read_dataset_impl(props.is_explicit_vr(), props.is_little_endian(), data.len())?
    };

    Ok(FileFormat::new(meta, dataset))
}

fn decompress_deflated(data: &[u8]) -> DcmResult<Vec<u8>> {
    use flate2::read::DeflateDecoder;
    let mut decoder = DeflateDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).map_err(DcmError::Io)?;
    Ok(out)
}

// ── Cursor ────────────────────────────────────────────────────────────────────

struct DicomCursor<'a> {
    data: &'a [u8],
    pos: usize,
    /// Character set decoder, updated when (0008,0005) is encountered.
    charset: DicomCharsetDecoder,
}

impl<'a> DicomCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            charset: DicomCharsetDecoder::default_ascii(),
        }
    }

    // ── Primitives ────────────────────────────────────────────────────────────

    fn read_u8(&mut self) -> DcmResult<u8> {
        if self.pos >= self.data.len() {
            return Err(DcmError::UnexpectedEof {
                offset: self.pos as u64,
            });
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_u16(&mut self, le: bool) -> DcmResult<u16> {
        let a = self.read_u8()?;
        let b = self.read_u8()?;
        Ok(if le {
            u16::from_le_bytes([a, b])
        } else {
            u16::from_be_bytes([a, b])
        })
    }

    fn read_u32(&mut self, le: bool) -> DcmResult<u32> {
        let a = self.read_u8()?;
        let b = self.read_u8()?;
        let c = self.read_u8()?;
        let d = self.read_u8()?;
        Ok(if le {
            u32::from_le_bytes([a, b, c, d])
        } else {
            u32::from_be_bytes([a, b, c, d])
        })
    }

    fn read_bytes(&mut self, n: usize) -> DcmResult<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(DcmError::UnexpectedEof {
                offset: self.pos as u64,
            });
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn peek_tag(&self, le: bool) -> DcmResult<Tag> {
        if self.pos + 4 > self.data.len() {
            return Err(DcmError::UnexpectedEof {
                offset: self.pos as u64,
            });
        }
        let g0 = self.data[self.pos];
        let g1 = self.data[self.pos + 1];
        let e0 = self.data[self.pos + 2];
        let e1 = self.data[self.pos + 3];
        let group = if le {
            u16::from_le_bytes([g0, g1])
        } else {
            u16::from_be_bytes([g0, g1])
        };
        let element = if le {
            u16::from_le_bytes([e0, e1])
        } else {
            u16::from_be_bytes([e0, e1])
        };
        Ok(Tag::new(group, element))
    }

    fn read_tag(&mut self, le: bool) -> DcmResult<Tag> {
        let tag = self.peek_tag(le)?;
        self.pos += 4;
        Ok(tag)
    }

    // ── Meta ──────────────────────────────────────────────────────────────────

    /// Read group 0002 elements (explicit VR LE). Stops when group != 0002.
    fn read_meta(&mut self) -> DcmResult<DataSet> {
        let mut meta = DataSet::new();
        while self.pos + 4 <= self.data.len() {
            let group = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
            if group != 0x0002 {
                break;
            }
            let elem = self.read_element(true, true)?;
            // Skip group-length element — computed at write time
            if !elem.tag.is_group_length() {
                meta.insert(elem);
            }
        }
        Ok(meta)
    }

    // ── Dataset ───────────────────────────────────────────────────────────────

    fn read_dataset_impl(&mut self, explicit: bool, le: bool, end: usize) -> DcmResult<DataSet> {
        let mut ds = DataSet::new();
        while self.pos < end && self.pos + 4 <= self.data.len() {
            let tag = self.peek_tag(le)?;

            // Stop on sequence or item delimitation
            if tag.is_sequence_delimitation() || tag.is_item_delimitation() {
                self.pos += 4;
                let _ = self.read_u32(true); // consume length
                break;
            }

            // ITEM tags shouldn't appear at dataset level; stop gracefully
            if tag.is_item() {
                break;
            }

            let elem = self.read_element(explicit, le)?;

            // Update charset decoder when Specific Character Set is encountered
            if elem.tag == tags::SPECIFIC_CHARACTER_SET {
                if let Value::Strings(ref terms) = elem.value {
                    let charset_value = terms.join("\\");
                    if let Ok(decoder) = DicomCharsetDecoder::new(&charset_value) {
                        self.charset = decoder;
                    }
                }
            }

            ds.insert(elem);
        }
        Ok(ds)
    }

    // ── Element ───────────────────────────────────────────────────────────────

    fn read_element(&mut self, explicit: bool, le: bool) -> DcmResult<Element> {
        let tag = self.read_tag(le)?;

        let (vr, len, undef_len) = if tag.is_delimiter() {
            let len = self.read_u32(true)?;
            (Vr::UN, len, false)
        } else if explicit {
            let vr_b0 = self.read_u8()?;
            let vr_b1 = self.read_u8()?;
            let vr = Vr::from_bytes([vr_b0, vr_b1]).unwrap_or(Vr::UN);
            if vr.has_long_explicit_length() {
                let _reserved = self.read_u16(le)?;
                let len = self.read_u32(le)?;
                (vr, len, len == 0xFFFF_FFFF)
            } else {
                let len = self.read_u16(le)? as u32;
                (vr, len, false)
            }
        } else {
            let len = self.read_u32(le)?;
            let vr = implicit_vr_for_tag(tag);
            (vr, len, len == 0xFFFF_FFFF)
        };

        let value = self.read_value(tag, vr, len, undef_len, explicit, le)?;
        let effective_vr = if vr == Vr::UN && undef_len && matches!(value, Value::Sequence(_)) {
            // Mirror DCMTK CP-246 handling: undefined-length UN is normalized to SQ.
            Vr::SQ
        } else {
            vr
        };
        Ok(Element::new(tag, effective_vr, value))
    }

    // ── Value ─────────────────────────────────────────────────────────────────

    fn read_value(
        &mut self,
        tag: Tag,
        vr: Vr,
        len: u32,
        undef_len: bool,
        explicit: bool,
        le: bool,
    ) -> DcmResult<Value> {
        match vr {
            Vr::SQ => {
                let items = self.read_sequence(len, undef_len, explicit, le)?;
                Ok(Value::Sequence(items))
            }
            _ if tag == tags::PIXEL_DATA => self.read_pixel_data(len, undef_len, le),
            Vr::UN if undef_len => {
                // CP-246: undefined-length UN is interpreted as a sequence encoded
                // using Implicit VR Little Endian semantics.
                let items = self.read_sequence(len, true, false, true)?;
                Ok(Value::Sequence(items))
            }
            _ => {
                if undef_len {
                    return Err(DcmError::InvalidLength {
                        group: tag.group,
                        element: tag.element,
                        length: 0xFFFF_FFFF,
                    });
                }
                let bytes = self.read_bytes(len as usize)?;
                parse_value_bytes(vr, bytes, le, &self.charset)
            }
        }
    }

    // ── Sequence ──────────────────────────────────────────────────────────────

    fn read_sequence(
        &mut self,
        len: u32,
        undef_len: bool,
        explicit: bool,
        le: bool,
    ) -> DcmResult<Vec<DataSet>> {
        let end = if undef_len {
            usize::MAX
        } else {
            self.pos.saturating_add(len as usize)
        };

        let mut items = Vec::new();

        while self.pos < end && self.pos + 4 <= self.data.len() {
            let tag = self.peek_tag(le)?;

            if tag.is_sequence_delimitation() {
                self.pos += 4;
                let _ = self.read_u32(true);
                break;
            }

            if tag.is_item() {
                self.pos += 4; // consume ITEM tag
                let item_len = self.read_u32(le)?;
                let item_undef = item_len == 0xFFFF_FFFF;
                let item_end = if item_undef {
                    usize::MAX
                } else {
                    self.pos.saturating_add(item_len as usize)
                };
                let item_ds = self.read_dataset_impl(explicit, le, item_end)?;
                items.push(item_ds);
            } else {
                break;
            }
        }

        Ok(items)
    }

    // ── Pixel data ────────────────────────────────────────────────────────────

    fn read_pixel_data(&mut self, len: u32, undef_len: bool, le: bool) -> DcmResult<Value> {
        if !undef_len {
            let bytes = self.read_bytes(len as usize)?.to_vec();
            return Ok(Value::PixelData(PixelData::Native { bytes }));
        }

        // Encapsulated pixel data — undefined length
        let mut offset_table: Vec<u32> = Vec::new();
        let mut fragments: Vec<Vec<u8>> = Vec::new();
        let mut first_item = true;

        loop {
            if self.pos + 4 > self.data.len() {
                break;
            }
            let tag = self.peek_tag(le)?;

            if tag.is_sequence_delimitation() {
                self.pos += 4;
                let _ = self.read_u32(true);
                break;
            }

            if tag.is_item() {
                self.pos += 4;
                let item_len = self.read_u32(le)?;
                let item_bytes = self.read_bytes(item_len as usize)?.to_vec();

                if first_item {
                    // Basic offset table
                    let n = item_bytes.len() / 4;
                    for i in 0..n {
                        let b = &item_bytes[i * 4..i * 4 + 4];
                        offset_table.push(u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                    }
                    first_item = false;
                } else {
                    fragments.push(item_bytes);
                }
            } else {
                break;
            }
        }

        Ok(Value::PixelData(PixelData::Encapsulated {
            offset_table,
            fragments,
        }))
    }
}

// ── Value byte parser ─────────────────────────────────────────────────────────

fn parse_value_bytes(
    vr: Vr,
    bytes: &[u8],
    le: bool,
    charset: &DicomCharsetDecoder,
) -> DcmResult<Value> {
    if bytes.is_empty() {
        return Ok(Value::Empty);
    }

    match vr {
        Vr::UI => {
            // UIDs are always ASCII — no charset decoding needed
            let s = std::str::from_utf8(bytes)
                .unwrap_or("")
                .trim_end_matches('\0');
            Ok(Value::Uid(s.to_string()))
        }

        // VRs that use the Specific Character Set:
        // AE, CS are restricted to ASCII in DICOM, but we decode them through
        // the charset decoder for robustness (some non-conformant files exist).
        Vr::AE | Vr::AS | Vr::CS | Vr::LO | Vr::SH => {
            let s = decode_string_with_charset(bytes, charset);
            let s = s.trim_end_matches('\0').trim_end_matches(' ');
            let parts: Vec<String> = s.split('\\').map(str::to_string).collect();
            Ok(Value::Strings(parts))
        }
        Vr::LT | Vr::ST | Vr::UT | Vr::UC | Vr::UR => {
            let s = decode_string_with_charset(bytes, charset);
            let s = s.trim_end_matches('\0').trim_end_matches(' ').to_string();
            Ok(Value::Strings(vec![s]))
        }
        Vr::PN => {
            let s = decode_string_with_charset(bytes, charset);
            let s = s.trim_end_matches('\0').trim_end_matches(' ');
            if s.is_empty() {
                return Ok(Value::Empty);
            }
            let names: Vec<PersonName> = s.split('\\').map(PersonName::parse).collect();
            Ok(Value::PersonNames(names))
        }

        // Numeric-string VRs — always ASCII content, but decoded through charset
        // for consistent handling of padding.
        Vr::DA => {
            let s = decode_ascii_string(bytes);
            if s.is_empty() {
                return Ok(Value::Empty);
            }
            let res: Result<Vec<_>, _> = s
                .split('\\')
                .map(|p| DicomDate::from_da_str(p.trim()))
                .collect();
            res.map(Value::Date)
                .map_err(|_| DcmError::Other("invalid DA value".into()))
        }
        Vr::TM => {
            let s = decode_ascii_string(bytes);
            if s.is_empty() {
                return Ok(Value::Empty);
            }
            let res: Result<Vec<_>, _> =
                s.split('\\').map(|p| DicomTime::parse(p.trim())).collect();
            res.map(Value::Time)
                .map_err(|_| DcmError::Other("invalid TM value".into()))
        }
        Vr::DT => {
            let s = decode_ascii_string(bytes);
            if s.is_empty() {
                return Ok(Value::Empty);
            }
            let res: Result<Vec<_>, _> = s
                .split('\\')
                .map(|p| DicomDateTime::parse(p.trim()))
                .collect();
            res.map(Value::DateTime)
                .map_err(|_| DcmError::Other("invalid DT value".into()))
        }
        Vr::IS => {
            let s = decode_ascii_string(bytes);
            if s.is_empty() {
                return Ok(Value::Empty);
            }
            let res: Result<Vec<i64>, _> = s
                .split('\\')
                .map(|p| {
                    p.trim()
                        .parse::<i64>()
                        .map_err(|_| DcmError::Other(format!("invalid IS: {p}")))
                })
                .collect();
            res.map(Value::Ints)
        }
        Vr::DS => {
            let s = decode_ascii_string(bytes);
            if s.is_empty() {
                return Ok(Value::Empty);
            }
            let res: Result<Vec<f64>, _> = s
                .split('\\')
                .map(|p| {
                    p.trim()
                        .parse::<f64>()
                        .map_err(|_| DcmError::Other(format!("invalid DS: {p}")))
                })
                .collect();
            res.map(Value::Decimals)
        }

        Vr::US | Vr::OW => {
            if bytes.len() % 2 != 0 {
                return Err(DcmError::Other(format!(
                    "{} value has odd byte length",
                    vr.code()
                )));
            }
            let vals: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| {
                    if le {
                        u16::from_le_bytes([c[0], c[1]])
                    } else {
                        u16::from_be_bytes([c[0], c[1]])
                    }
                })
                .collect();
            Ok(Value::U16(vals))
        }
        Vr::SS => {
            if bytes.len() % 2 != 0 {
                return Err(DcmError::Other("SS value has odd byte length".into()));
            }
            let vals: Vec<i16> = bytes
                .chunks_exact(2)
                .map(|c| {
                    if le {
                        i16::from_le_bytes([c[0], c[1]])
                    } else {
                        i16::from_be_bytes([c[0], c[1]])
                    }
                })
                .collect();
            Ok(Value::I16(vals))
        }
        Vr::UL | Vr::OL => {
            if bytes.len() % 4 != 0 {
                return Err(DcmError::Other(format!(
                    "{} value length not multiple of 4",
                    vr.code()
                )));
            }
            let vals: Vec<u32> = bytes
                .chunks_exact(4)
                .map(|c| {
                    if le {
                        u32::from_le_bytes([c[0], c[1], c[2], c[3]])
                    } else {
                        u32::from_be_bytes([c[0], c[1], c[2], c[3]])
                    }
                })
                .collect();
            Ok(Value::U32(vals))
        }
        Vr::SL => {
            if bytes.len() % 4 != 0 {
                return Err(DcmError::Other("SL value length not multiple of 4".into()));
            }
            let vals: Vec<i32> = bytes
                .chunks_exact(4)
                .map(|c| {
                    if le {
                        i32::from_le_bytes([c[0], c[1], c[2], c[3]])
                    } else {
                        i32::from_be_bytes([c[0], c[1], c[2], c[3]])
                    }
                })
                .collect();
            Ok(Value::I32(vals))
        }
        Vr::FL | Vr::OF => {
            if bytes.len() % 4 != 0 {
                return Err(DcmError::Other(format!(
                    "{} value length not multiple of 4",
                    vr.code()
                )));
            }
            let vals: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|c| {
                    if le {
                        f32::from_le_bytes([c[0], c[1], c[2], c[3]])
                    } else {
                        f32::from_be_bytes([c[0], c[1], c[2], c[3]])
                    }
                })
                .collect();
            Ok(Value::F32(vals))
        }
        Vr::FD | Vr::OD => {
            if bytes.len() % 8 != 0 {
                return Err(DcmError::Other(format!(
                    "{} value length not multiple of 8",
                    vr.code()
                )));
            }
            let vals: Vec<f64> = bytes
                .chunks_exact(8)
                .map(|c| {
                    if le {
                        f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
                    } else {
                        f64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
                    }
                })
                .collect();
            Ok(Value::F64(vals))
        }
        Vr::SV => {
            if bytes.len() % 8 != 0 {
                return Err(DcmError::Other("SV value length not multiple of 8".into()));
            }
            let vals: Vec<i64> = bytes
                .chunks_exact(8)
                .map(|c| {
                    if le {
                        i64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
                    } else {
                        i64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
                    }
                })
                .collect();
            Ok(Value::I64(vals))
        }
        Vr::UV | Vr::OV => {
            if bytes.len() % 8 != 0 {
                return Err(DcmError::Other(format!(
                    "{} value length not multiple of 8",
                    vr.code()
                )));
            }
            let vals: Vec<u64> = bytes
                .chunks_exact(8)
                .map(|c| {
                    if le {
                        u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
                    } else {
                        u64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]])
                    }
                })
                .collect();
            Ok(Value::U64(vals))
        }
        Vr::AT => {
            if bytes.len() % 4 != 0 {
                return Err(DcmError::Other("AT value length not multiple of 4".into()));
            }
            let tags: Vec<Tag> = bytes
                .chunks_exact(4)
                .map(|c| {
                    let g = if le {
                        u16::from_le_bytes([c[0], c[1]])
                    } else {
                        u16::from_be_bytes([c[0], c[1]])
                    };
                    let e = if le {
                        u16::from_le_bytes([c[2], c[3]])
                    } else {
                        u16::from_be_bytes([c[2], c[3]])
                    };
                    Tag::new(g, e)
                })
                .collect();
            Ok(Value::Tags(tags))
        }
        Vr::OB | Vr::UN => Ok(Value::U8(bytes.to_vec())),
        Vr::SQ => Err(DcmError::Other("parse_value_bytes called for SQ".into())),
    }
}

/// Decode a byte slice using the active charset decoder.
fn decode_string_with_charset(bytes: &[u8], charset: &DicomCharsetDecoder) -> String {
    charset
        .decode(bytes)
        .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned())
}

/// Decode an ASCII-only string (for numeric VRs like DA, TM, IS, DS).
fn decode_ascii_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\0')
        .trim_end_matches(' ')
        .to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use dicom_toolkit_dict::{tags, Tag, Vr};

    fn ascii() -> DicomCharsetDecoder {
        DicomCharsetDecoder::default_ascii()
    }

    #[test]
    fn parse_us_bytes() {
        let bytes = 512u16.to_le_bytes();
        let v = parse_value_bytes(Vr::US, &bytes, true, &ascii()).unwrap();
        assert_eq!(v.as_u16(), Some(512));
    }

    #[test]
    fn parse_ui_bytes() {
        let uid = b"1.2.840.10008.1.2.1";
        let v = parse_value_bytes(Vr::UI, uid, true, &ascii()).unwrap();
        assert_eq!(v.as_string(), Some("1.2.840.10008.1.2.1"));
    }

    #[test]
    fn parse_lo_bytes_backslash() {
        let s = b"foo\\bar";
        let v = parse_value_bytes(Vr::LO, s, true, &ascii()).unwrap();
        match v {
            Value::Strings(ss) => assert_eq!(ss, &["foo", "bar"]),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parse_ds_bytes() {
        let s = b"2.78";
        let v = parse_value_bytes(Vr::DS, s, true, &ascii()).unwrap();
        match v {
            Value::Decimals(ds) => assert!((ds[0] - 2.78).abs() < 1e-9),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parse_is_bytes() {
        let s = b"-42";
        let v = parse_value_bytes(Vr::IS, s, true, &ascii()).unwrap();
        match v {
            Value::Ints(is) => assert_eq!(is[0], -42),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parse_ob_bytes() {
        let bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let v = parse_value_bytes(Vr::OB, &bytes, true, &ascii()).unwrap();
        assert_eq!(v.as_bytes(), Some(bytes.as_slice()));
    }

    #[test]
    fn parse_at_bytes() {
        let bytes = [
            0x08, 0x00, 0x20, 0x00, // (0008,0020)
            0x08, 0x00, 0x30, 0x00, // (0008,0030)
        ];
        let v = parse_value_bytes(Vr::AT, &bytes, true, &ascii()).unwrap();
        match v {
            Value::Tags(tags) => {
                assert_eq!(
                    tags,
                    vec![Tag::new(0x0008, 0x0020), Tag::new(0x0008, 0x0030)]
                )
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parse_lo_latin1() {
        // "Müller" in ISO-8859-1
        let bytes = vec![b'M', 0xFC, b'l', b'l', b'e', b'r'];
        let latin1 = DicomCharsetDecoder::new("ISO_IR 100").unwrap();
        let v = parse_value_bytes(Vr::LO, &bytes, true, &latin1).unwrap();
        match v {
            Value::Strings(ss) => assert_eq!(ss, &["Müller"]),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parse_pn_utf8() {
        let name = "田中^太郎";
        let utf8 = DicomCharsetDecoder::new("ISO_IR 192").unwrap();
        let v = parse_value_bytes(Vr::PN, name.as_bytes(), true, &utf8).unwrap();
        match v {
            Value::PersonNames(names) => assert_eq!(names[0].to_string(), "田中^太郎"),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn read_undefined_length_un_as_sequence_cp246() {
        let private_tag = Tag::new(0x7777, 0x0010);

        let mut item = Vec::new();
        item.extend_from_slice(&tags::PATIENT_ID.group.to_le_bytes());
        item.extend_from_slice(&tags::PATIENT_ID.element.to_le_bytes());
        item.extend_from_slice(&4u32.to_le_bytes());
        item.extend_from_slice(b"ABCD");

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&private_tag.group.to_le_bytes());
        bytes.extend_from_slice(&private_tag.element.to_le_bytes());
        bytes.extend_from_slice(b"UN");
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        bytes.extend_from_slice(&[0xFE, 0xFF, 0x00, 0xE0]); // Item
        bytes.extend_from_slice(&(item.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&item);
        bytes.extend_from_slice(&[0xFE, 0xFF, 0xDD, 0xE0, 0, 0, 0, 0]); // Sequence delimitation

        let ds = DicomReader::new(bytes.as_slice())
            .read_dataset("1.2.840.10008.1.2.1")
            .unwrap();

        let elem = ds.get(private_tag).unwrap();
        assert_eq!(elem.vr, Vr::SQ);
        let items = elem.items().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get_string(tags::PATIENT_ID), Some("ABCD"));
    }
}
