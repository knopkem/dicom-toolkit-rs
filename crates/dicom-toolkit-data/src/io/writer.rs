//! DICOM Part 10 file writer.
//!
//! Writes a `FileFormat` or `DataSet` to a binary DICOM stream.

use crate::dataset::DataSet;
use crate::element::Element;
use crate::file_format::FileFormat;
use crate::io::transfer::TransferSyntaxProperties;
use crate::value::{PixelData, Value};
use dicom_toolkit_core::charset::DicomCharsetDecoder;
use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_dict::{tags, Tag, Vr};
use std::io::Write;

/// DICOM file writer.
pub struct DicomWriter<W: Write> {
    writer: W,
}

impl<W: Write> DicomWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    /// Write a complete DICOM Part 10 file (preamble + meta + dataset).
    pub fn write_file(&mut self, file_format: &FileFormat) -> DcmResult<()> {
        let bytes = encode_file(file_format)?;
        self.writer.write_all(&bytes).map_err(DcmError::Io)
    }

    /// Write a raw dataset with the given transfer syntax (no preamble/meta).
    pub fn write_dataset(&mut self, dataset: &DataSet, ts_uid: &str) -> DcmResult<()> {
        let props = TransferSyntaxProperties::from_uid(ts_uid);
        let bytes = encode_dataset(dataset, props.is_explicit_vr(), props.is_little_endian())?;
        self.writer.write_all(&bytes).map_err(DcmError::Io)
    }
}

// ── File encoding ─────────────────────────────────────────────────────────────

pub(crate) fn encode_file(file_format: &FileFormat) -> DcmResult<Vec<u8>> {
    let ts_uid = &file_format.meta.transfer_syntax_uid;
    let props = TransferSyntaxProperties::from_uid(ts_uid);

    let mut out = Vec::new();

    // 128-byte preamble (zeros) + "DICM" magic
    out.extend_from_slice(&[0u8; 128]);
    out.extend_from_slice(b"DICM");

    // File Meta Information (always explicit VR LE)
    let meta_bytes = encode_meta(&file_format.meta)?;
    out.extend_from_slice(&meta_bytes);

    // Dataset
    let dataset_bytes = if props.is_deflated {
        use flate2::{write::DeflateEncoder, Compression};
        let raw = encode_dataset(&file_format.dataset, true, true)?;
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&raw).map_err(DcmError::Io)?;
        enc.finish().map_err(DcmError::Io)?
    } else {
        encode_dataset(
            &file_format.dataset,
            props.is_explicit_vr(),
            props.is_little_endian(),
        )?
    };
    out.extend_from_slice(&dataset_bytes);

    Ok(out)
}

fn encode_meta(meta: &crate::meta_info::FileMetaInformation) -> DcmResult<Vec<u8>> {
    let meta_ds = meta.to_dataset();
    // Encode meta elements (all are file-meta; do NOT skip them)
    let meta_payload = encode_dataset_impl(&meta_ds, true, true, false)?;

    let group_length = meta_payload.len() as u32;

    let mut out = Vec::new();
    let gl_bytes = encode_element_raw(
        tags::FILE_META_INFORMATION_GROUP_LENGTH,
        Vr::UL,
        &group_length.to_le_bytes(),
        true,
        true,
    )?;
    out.extend_from_slice(&gl_bytes);
    out.extend_from_slice(&meta_payload);

    Ok(out)
}

// ── Dataset encoding ──────────────────────────────────────────────────────────

/// Encode a dataset, skipping file meta elements (for main dataset encoding).
pub(crate) fn encode_dataset(ds: &DataSet, explicit: bool, le: bool) -> DcmResult<Vec<u8>> {
    encode_dataset_impl(ds, explicit, le, true)
}

fn encode_dataset_impl(
    ds: &DataSet,
    explicit: bool,
    le: bool,
    skip_file_meta: bool,
) -> DcmResult<Vec<u8>> {
    // Determine the charset from the dataset for encoding string values.
    let charset = if let Some(elem) = ds.get(tags::SPECIFIC_CHARACTER_SET) {
        if let Value::Strings(ref terms) = elem.value {
            let charset_value = terms.join("\\");
            DicomCharsetDecoder::new(&charset_value)
                .unwrap_or_else(|_| DicomCharsetDecoder::default_ascii())
        } else {
            DicomCharsetDecoder::default_ascii()
        }
    } else {
        DicomCharsetDecoder::default_ascii()
    };

    let mut out = Vec::new();
    for (_, elem) in ds.iter() {
        if skip_file_meta && elem.tag.is_file_meta() {
            continue;
        }
        let bytes = encode_element(elem, explicit, le, &charset)?;
        out.extend_from_slice(&bytes);
    }
    Ok(out)
}

// ── Element encoding ──────────────────────────────────────────────────────────

fn encode_element(
    elem: &Element,
    explicit: bool,
    le: bool,
    charset: &DicomCharsetDecoder,
) -> DcmResult<Vec<u8>> {
    match &elem.value {
        Value::Sequence(items) => encode_sequence_element(elem.tag, items, explicit, le),
        Value::PixelData(PixelData::Encapsulated {
            offset_table,
            fragments,
        }) => encode_encapsulated_pixel(elem.tag, offset_table, fragments, explicit, le),
        _ => {
            let value_bytes = encode_value_bytes(&elem.value, elem.vr, le, charset)?;
            let padded = pad_to_even(value_bytes, elem.vr.padding_byte());
            encode_element_raw(elem.tag, elem.vr, &padded, explicit, le)
        }
    }
}

fn encode_element_raw(
    tag: Tag,
    vr: Vr,
    value_bytes: &[u8],
    explicit: bool,
    le: bool,
) -> DcmResult<Vec<u8>> {
    let len = value_bytes.len() as u32;
    let mut out = Vec::new();

    write_u16(&mut out, tag.group, le);
    write_u16(&mut out, tag.element, le);

    if explicit && !tag.is_delimiter() {
        out.extend_from_slice(&vr.to_bytes());

        if vr.has_long_explicit_length() {
            out.extend_from_slice(&[0u8, 0u8]);
            write_u32(&mut out, len, le);
        } else {
            if len > u16::MAX as u32 {
                return Err(DcmError::InvalidLength {
                    group: tag.group,
                    element: tag.element,
                    length: len as u64,
                });
            }
            write_u16(&mut out, len as u16, le);
        }
    } else {
        // Implicit VR or delimiter: 4-byte length (delimiters always LE)
        let is_delim = tag.is_delimiter();
        write_u32(&mut out, len, if is_delim { true } else { le });
    }

    out.extend_from_slice(value_bytes);
    Ok(out)
}

// ── Sequence encoding ─────────────────────────────────────────────────────────

fn encode_sequence_element(
    tag: Tag,
    items: &[DataSet],
    explicit: bool,
    le: bool,
) -> DcmResult<Vec<u8>> {
    let mut items_buf = Vec::new();
    for item_ds in items {
        let item_bytes = encode_dataset_impl(item_ds, explicit, le, false)?;
        write_u16(&mut items_buf, 0xFFFE, true);
        write_u16(&mut items_buf, 0xE000, true);
        write_u32(&mut items_buf, item_bytes.len() as u32, true);
        items_buf.extend_from_slice(&item_bytes);
    }

    let total_len = items_buf.len() as u32;
    let mut out = Vec::new();

    write_u16(&mut out, tag.group, le);
    write_u16(&mut out, tag.element, le);

    if explicit {
        out.extend_from_slice(b"SQ");
        out.extend_from_slice(&[0u8, 0u8]);
        write_u32(&mut out, total_len, le);
    } else {
        write_u32(&mut out, total_len, le);
    }

    out.extend_from_slice(&items_buf);
    Ok(out)
}

// ── Encapsulated pixel data ───────────────────────────────────────────────────

fn encode_encapsulated_pixel(
    tag: Tag,
    offset_table: &[u32],
    fragments: &[Vec<u8>],
    explicit: bool,
    le: bool,
) -> DcmResult<Vec<u8>> {
    let mut out = Vec::new();

    write_u16(&mut out, tag.group, le);
    write_u16(&mut out, tag.element, le);
    if explicit {
        out.extend_from_slice(b"OB");
        out.extend_from_slice(&[0u8, 0u8]);
    }
    write_u32(&mut out, 0xFFFF_FFFF, le);

    // Basic Offset Table item (FFFE,E000)
    let ot_bytes: Vec<u8> = offset_table.iter().flat_map(|&o| o.to_le_bytes()).collect();
    write_u16(&mut out, 0xFFFE, true);
    write_u16(&mut out, 0xE000, true);
    write_u32(&mut out, ot_bytes.len() as u32, true);
    out.extend_from_slice(&ot_bytes);

    for frag in fragments {
        write_u16(&mut out, 0xFFFE, true);
        write_u16(&mut out, 0xE000, true);
        write_u32(&mut out, frag.len() as u32, true);
        out.extend_from_slice(frag);
    }

    // Sequence delimitation (FFFE,E0DD)
    write_u16(&mut out, 0xFFFE, true);
    write_u16(&mut out, 0xE0DD, true);
    write_u32(&mut out, 0, true);

    Ok(out)
}

// ── Value bytes encoding ──────────────────────────────────────────────────────

pub(crate) fn encode_value_bytes(
    value: &Value,
    _vr: Vr,
    le: bool,
    charset: &DicomCharsetDecoder,
) -> DcmResult<Vec<u8>> {
    match value {
        Value::Empty => Ok(Vec::new()),

        Value::Strings(v) => {
            let joined = v.join("\\");
            charset.encode(&joined).or_else(|_| Ok(joined.into_bytes()))
        }

        Value::PersonNames(v) => {
            let joined = v
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("\\");
            charset.encode(&joined).or_else(|_| Ok(joined.into_bytes()))
        }

        // UIDs are always ASCII — never charset-encoded
        Value::Uid(s) => Ok(s.as_bytes().to_vec()),

        // Date/Time/Numeric values are always ASCII
        Value::Date(v) => Ok(v
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("\\")
            .into_bytes()),

        Value::Time(v) => Ok(v
            .iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join("\\")
            .into_bytes()),

        Value::DateTime(v) => Ok(v
            .iter()
            .map(|dt| dt.to_string())
            .collect::<Vec<_>>()
            .join("\\")
            .into_bytes()),

        Value::Ints(v) => Ok(v
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\\")
            .into_bytes()),

        Value::Decimals(v) => Ok(v
            .iter()
            .map(|n| format_ds(*n))
            .collect::<Vec<_>>()
            .join("\\")
            .into_bytes()),

        Value::U8(v) => Ok(v.clone()),

        Value::U16(v) => {
            let mut buf = Vec::with_capacity(v.len() * 2);
            for &n in v {
                if le {
                    buf.extend_from_slice(&n.to_le_bytes());
                } else {
                    buf.extend_from_slice(&n.to_be_bytes());
                }
            }
            Ok(buf)
        }

        Value::I16(v) => {
            let mut buf = Vec::with_capacity(v.len() * 2);
            for &n in v {
                if le {
                    buf.extend_from_slice(&n.to_le_bytes());
                } else {
                    buf.extend_from_slice(&n.to_be_bytes());
                }
            }
            Ok(buf)
        }

        Value::U32(v) => {
            let mut buf = Vec::with_capacity(v.len() * 4);
            for &n in v {
                if le {
                    buf.extend_from_slice(&n.to_le_bytes());
                } else {
                    buf.extend_from_slice(&n.to_be_bytes());
                }
            }
            Ok(buf)
        }

        Value::I32(v) => {
            let mut buf = Vec::with_capacity(v.len() * 4);
            for &n in v {
                if le {
                    buf.extend_from_slice(&n.to_le_bytes());
                } else {
                    buf.extend_from_slice(&n.to_be_bytes());
                }
            }
            Ok(buf)
        }

        Value::U64(v) => {
            let mut buf = Vec::with_capacity(v.len() * 8);
            for &n in v {
                if le {
                    buf.extend_from_slice(&n.to_le_bytes());
                } else {
                    buf.extend_from_slice(&n.to_be_bytes());
                }
            }
            Ok(buf)
        }

        Value::I64(v) => {
            let mut buf = Vec::with_capacity(v.len() * 8);
            for &n in v {
                if le {
                    buf.extend_from_slice(&n.to_le_bytes());
                } else {
                    buf.extend_from_slice(&n.to_be_bytes());
                }
            }
            Ok(buf)
        }

        Value::F32(v) => {
            let mut buf = Vec::with_capacity(v.len() * 4);
            for &n in v {
                let bits = n.to_bits();
                if le {
                    buf.extend_from_slice(&bits.to_le_bytes());
                } else {
                    buf.extend_from_slice(&bits.to_be_bytes());
                }
            }
            Ok(buf)
        }

        Value::F64(v) => {
            let mut buf = Vec::with_capacity(v.len() * 8);
            for &n in v {
                let bits = n.to_bits();
                if le {
                    buf.extend_from_slice(&bits.to_le_bytes());
                } else {
                    buf.extend_from_slice(&bits.to_be_bytes());
                }
            }
            Ok(buf)
        }

        Value::Tags(v) => {
            let mut buf = Vec::with_capacity(v.len() * 4);
            for &t in v {
                if le {
                    buf.extend_from_slice(&t.group.to_le_bytes());
                    buf.extend_from_slice(&t.element.to_le_bytes());
                } else {
                    buf.extend_from_slice(&t.group.to_be_bytes());
                    buf.extend_from_slice(&t.element.to_be_bytes());
                }
            }
            Ok(buf)
        }

        Value::PixelData(PixelData::Native { bytes }) => Ok(bytes.clone()),

        Value::Sequence(_) | Value::PixelData(PixelData::Encapsulated { .. }) => Err(
            DcmError::Other("encode_value_bytes called on Sequence/Encapsulated".into()),
        ),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn pad_to_even(mut bytes: Vec<u8>, pad: u8) -> Vec<u8> {
    if bytes.len() % 2 != 0 {
        bytes.push(pad);
    }
    bytes
}

fn write_u16(buf: &mut Vec<u8>, v: u16, le: bool) {
    if le {
        buf.extend_from_slice(&v.to_le_bytes());
    } else {
        buf.extend_from_slice(&v.to_be_bytes());
    }
}

fn write_u32(buf: &mut Vec<u8>, v: u32, le: bool) {
    if le {
        buf.extend_from_slice(&v.to_le_bytes());
    } else {
        buf.extend_from_slice(&v.to_be_bytes());
    }
}

/// Format an f64 value as a DICOM DS string (max 16 chars).
fn format_ds(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_string();
    }
    let s = format!("{}", v);
    if s.len() <= 16 {
        return s;
    }
    let s = format!("{:.6E}", v);
    if s.len() <= 16 {
        s
    } else {
        format!("{:.4E}", v)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use dicom_toolkit_dict::Vr;

    fn ascii() -> DicomCharsetDecoder {
        DicomCharsetDecoder::default_ascii()
    }

    #[test]
    fn encode_u16_le() {
        let v = Value::U16(vec![512, 256]);
        let bytes = encode_value_bytes(&v, Vr::US, true, &ascii()).unwrap();
        assert_eq!(bytes, vec![0x00, 0x02, 0x00, 0x01]);
    }

    #[test]
    fn encode_string_single() {
        let v = Value::Strings(vec!["hello".to_string()]);
        let bytes = encode_value_bytes(&v, Vr::LO, true, &ascii()).unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn encode_strings_backslash() {
        let v = Value::Strings(vec!["foo".to_string(), "bar".to_string()]);
        let bytes = encode_value_bytes(&v, Vr::CS, true, &ascii()).unwrap();
        assert_eq!(bytes, b"foo\\bar");
    }

    #[test]
    fn encode_string_latin1() {
        let latin1 = DicomCharsetDecoder::new("ISO_IR 100").unwrap();
        let v = Value::Strings(vec!["Müller".to_string()]);
        let bytes = encode_value_bytes(&v, Vr::LO, true, &latin1).unwrap();
        // "Müller" encoded as ISO-8859-1: ü = 0xFC
        assert_eq!(bytes, vec![b'M', 0xFC, b'l', b'l', b'e', b'r']);
    }

    #[test]
    fn pad_to_even_odd() {
        let bytes = pad_to_even(vec![1, 2, 3], 0x20);
        assert_eq!(bytes, vec![1, 2, 3, 0x20]);
    }

    #[test]
    fn pad_to_even_already_even() {
        let bytes = pad_to_even(vec![1, 2], 0x00);
        assert_eq!(bytes, vec![1, 2]);
    }
}
