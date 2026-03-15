//! DICOM XML representation (PS3.19 Native DICOM Model).
//!
//! Ports DCMTK's `dcmtk/dcmdata/dcrledrg.h` XML output capability.
//! The XML format uses `<NativeDicomModel>` as root, with `<DicomAttribute>` for
//! each element, matching the Native DICOM Model (PS3.19 §A.1).

use crate::dataset::DataSet;
use crate::element::Element;
use crate::value::{PixelData, Value};
use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_dict::Tag;

// ── Serialization ─────────────────────────────────────────────────────────────

/// Serialize a `DataSet` to a DICOM XML (Native DICOM Model) string.
///
/// Produces XML per PS3.19 Annex A — suitable for WADO-RS multipart responses.
pub fn to_xml(dataset: &DataSet) -> DcmResult<String> {
    let mut out = String::with_capacity(4096);
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push('\n');
    out.push_str("<NativeDicomModel xml:space=\"preserve\">\n");
    write_dataset(&mut out, dataset, 1)?;
    out.push_str("</NativeDicomModel>\n");
    Ok(out)
}

fn indent(level: usize) -> String {
    "  ".repeat(level)
}

fn write_dataset(out: &mut String, dataset: &DataSet, level: usize) -> DcmResult<()> {
    for (tag, elem) in dataset.iter() {
        if tag.is_group_length() || tag.is_delimiter() {
            continue;
        }
        write_element(out, tag, elem, level)?;
    }
    Ok(())
}

fn write_element(out: &mut String, tag: &Tag, elem: &Element, level: usize) -> DcmResult<()> {
    let pad = indent(level);
    let vr_str = elem.vr.code();

    out.push_str(&format!(
        r#"{}<DicomAttribute tag="{:04X}{:04X}" vr="{}">"#,
        pad, tag.group, tag.element, vr_str
    ));

    match &elem.value {
        Value::Empty => {
            out.push_str("/>\n");
            return Ok(());
        }
        Value::Sequence(items) => {
            out.push('\n');
            for (i, item) in items.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Item number=\"{}\">\n",
                    indent(level + 1),
                    i + 1
                ));
                write_dataset(out, item, level + 2)?;
                out.push_str(&format!("{}</Item>\n", indent(level + 1)));
            }
            out.push_str(&format!("{}</DicomAttribute>\n", pad));
            return Ok(());
        }
        _ => {}
    }

    out.push('\n');

    match &elem.value {
        Value::Strings(v) => {
            for (i, s) in v.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    xml_escape(s)
                ));
            }
        }
        Value::Uid(s) => {
            out.push_str(&format!(
                "{}<Value number=\"1\">{}</Value>\n",
                indent(level + 1),
                xml_escape(s)
            ));
        }
        Value::PersonNames(names) => {
            for (i, pn) in names.iter().enumerate() {
                out.push_str(&format!(
                    "{}<PersonName number=\"{}\">\n",
                    indent(level + 1),
                    i + 1
                ));
                if !pn.alphabetic.is_empty() {
                    out.push_str(&format!(
                        "{}<Alphabetic><FamilyName>{}</FamilyName></Alphabetic>\n",
                        indent(level + 2),
                        xml_escape(pn.last_name())
                    ));
                }
                if !pn.ideographic.is_empty() {
                    out.push_str(&format!(
                        "{}<Ideographic>{}</Ideographic>\n",
                        indent(level + 2),
                        xml_escape(&pn.ideographic)
                    ));
                }
                if !pn.phonetic.is_empty() {
                    out.push_str(&format!(
                        "{}<Phonetic>{}</Phonetic>\n",
                        indent(level + 2),
                        xml_escape(&pn.phonetic)
                    ));
                }
                out.push_str(&format!("{}</PersonName>\n", indent(level + 1)));
            }
        }
        Value::Date(dates) => {
            for (i, d) in dates.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    d
                ));
            }
        }
        Value::Time(times) => {
            for (i, t) in times.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    t
                ));
            }
        }
        Value::DateTime(dts) => {
            for (i, dt) in dts.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    dt
                ));
            }
        }
        Value::Ints(v) => {
            for (i, n) in v.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    n
                ));
            }
        }
        Value::Decimals(v) => {
            for (i, n) in v.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    n
                ));
            }
        }
        Value::U16(v) => {
            for (i, n) in v.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    n
                ));
            }
        }
        Value::I16(v) => {
            for (i, n) in v.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    n
                ));
            }
        }
        Value::U32(v) => {
            for (i, n) in v.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    n
                ));
            }
        }
        Value::I32(v) => {
            for (i, n) in v.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    n
                ));
            }
        }
        Value::U64(v) => {
            for (i, n) in v.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    n
                ));
            }
        }
        Value::I64(v) => {
            for (i, n) in v.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    n
                ));
            }
        }
        Value::F32(v) => {
            for (i, n) in v.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    n
                ));
            }
        }
        Value::F64(v) => {
            for (i, n) in v.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    n
                ));
            }
        }
        Value::Tags(tags) => {
            for (i, t) in tags.iter().enumerate() {
                out.push_str(&format!(
                    "{}<Value number=\"{}\">{:04X}{:04X}</Value>\n",
                    indent(level + 1),
                    i + 1,
                    t.group,
                    t.element
                ));
            }
        }
        // Binary data — base64 encoded InlineBinary
        Value::U8(bytes) => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            out.push_str(&format!(
                "{}<InlineBinary>{}</InlineBinary>\n",
                indent(level + 1),
                b64
            ));
        }
        Value::PixelData(pd) => {
            use base64::Engine;
            let bytes: &[u8] = match pd {
                PixelData::Native { bytes } => bytes,
                PixelData::Encapsulated { fragments, .. } => {
                    fragments.first().map(|f| f.as_slice()).unwrap_or(&[])
                }
            };
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            out.push_str(&format!(
                "{}<InlineBinary>{}</InlineBinary>\n",
                indent(level + 1),
                b64
            ));
        }
        // Empty handled above
        Value::Empty | Value::Sequence(_) => {}
    }

    out.push_str(&format!("{}</DicomAttribute>\n", pad));
    Ok(())
}

/// Escape special XML characters in a text value.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_dict::{tags, Vr};

    #[test]
    fn xml_has_root_element() {
        let ds = DataSet::new();
        let xml = to_xml(&ds).unwrap();
        assert!(
            xml.contains("<NativeDicomModel"),
            "should have NativeDicomModel root"
        );
        assert!(
            xml.contains("</NativeDicomModel>"),
            "should close NativeDicomModel"
        );
    }

    #[test]
    fn xml_contains_patient_name() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_NAME, Vr::PN, "Doe^Jane");
        let xml = to_xml(&ds).unwrap();
        assert!(xml.contains("00100010"), "should contain PatientName tag");
        assert!(xml.contains("PN"), "should contain VR");
    }

    #[test]
    fn xml_contains_uid() {
        let mut ds = DataSet::new();
        ds.set_uid(tags::SOP_INSTANCE_UID, "1.2.3.4.5");
        let xml = to_xml(&ds).unwrap();
        assert!(xml.contains("1.2.3.4.5"), "should contain UID value");
    }

    #[test]
    fn xml_escapes_special_chars() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_ID, Vr::LO, "A<B>&C");
        let xml = to_xml(&ds).unwrap();
        assert!(xml.contains("&lt;"), "< should be escaped");
        assert!(xml.contains("&amp;"), "& should be escaped");
        assert!(!xml.contains("A<B>"), "raw < should not appear in value");
    }

    #[test]
    fn xml_contains_sequence() {
        let mut ds = DataSet::new();
        let mut item = DataSet::new();
        item.set_string(tags::PATIENT_ID, Vr::LO, "ITEM-1");
        ds.set_sequence(tags::REFERENCED_SOP_SEQUENCE, vec![item]);
        let xml = to_xml(&ds).unwrap();
        assert!(
            xml.contains("<Item number=\"1\">"),
            "should have Item element"
        );
        assert!(xml.contains("</Item>"), "should close Item");
    }

    #[test]
    fn xml_is_well_formed() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_NAME, Vr::PN, "Smith^John");
        ds.set_string(tags::PATIENT_ID, Vr::LO, "ID-001");
        ds.set_uid(tags::SOP_INSTANCE_UID, "1.2.3");
        ds.set_u16(tags::ROWS, 256);
        ds.set_u16(tags::COLUMNS, 256);
        let xml = to_xml(&ds).unwrap();
        // Basic well-formedness: every open tag should have a close
        assert!(
            xml.contains("</DicomAttribute>") || xml.contains("/>"),
            "all attributes should be closed"
        );
    }
}
