//! DICOM JSON serialization and deserialization (PS3.18 Annex F).
//!
//! Ports DCMTK's `dcjson.h` / `dcjsonrd.h`. The DICOM JSON model encodes each
//! element as `{ "GGGGEEEE": { "vr": "XX", "Value": [...] } }`.

use crate::dataset::DataSet;
use crate::element::Element;
use crate::value::{DicomDate, DicomDateTime, DicomTime, PersonName, PixelData, Value};
use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_dict::{Tag, Vr};

// ── Serialization ─────────────────────────────────────────────────────────────

/// Serialize a `DataSet` to a DICOM JSON string (PS3.18 §F.2).
pub fn to_json(dataset: &DataSet) -> DcmResult<String> {
    let obj = dataset_to_json_object(dataset)?;
    serde_json::to_string(&obj).map_err(|e| DcmError::Other(format!("JSON serialize error: {e}")))
}

/// Serialize a `DataSet` to a pretty-printed DICOM JSON string.
pub fn to_json_pretty(dataset: &DataSet) -> DcmResult<String> {
    let obj = dataset_to_json_object(dataset)?;
    serde_json::to_string_pretty(&obj)
        .map_err(|e| DcmError::Other(format!("JSON serialize error: {e}")))
}

fn dataset_to_json_object(
    dataset: &DataSet,
) -> DcmResult<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::new();
    for (tag, elem) in dataset.iter() {
        // Skip group-length tags and sequence delimiter tags
        if tag.is_group_length() || tag.is_delimiter() {
            continue;
        }
        let key = format!("{:04X}{:04X}", tag.group, tag.element);
        let json_elem = element_to_json(elem)?;
        map.insert(key, json_elem);
    }
    Ok(map)
}

fn element_to_json(elem: &Element) -> DcmResult<serde_json::Value> {
    let vr_str = elem.vr.code().to_string();

    let value_json: Option<serde_json::Value> = match &elem.value {
        Value::Empty => None,

        Value::Strings(v) => {
            if v.is_empty() {
                None
            } else if elem.vr == Vr::PN {
                // PN strings stored as raw "Last^First^Middle^Prefix^Suffix" — convert to JSON PN format
                let arr: Vec<serde_json::Value> = v
                    .iter()
                    .map(|s| {
                        let mut obj = serde_json::Map::new();
                        if !s.is_empty() {
                            obj.insert("Alphabetic".into(), serde_json::Value::String(s.clone()));
                        }
                        serde_json::Value::Object(obj)
                    })
                    .collect();
                Some(serde_json::Value::Array(arr))
            } else {
                Some(serde_json::Value::Array(
                    v.iter()
                        .map(|s| serde_json::Value::String(s.clone()))
                        .collect(),
                ))
            }
        }

        Value::Uid(s) => Some(serde_json::Value::Array(vec![serde_json::Value::String(
            s.clone(),
        )])),

        Value::PersonNames(names) => {
            let arr: Vec<serde_json::Value> = names
                .iter()
                .map(|pn| {
                    let mut obj = serde_json::Map::new();
                    if !pn.alphabetic.is_empty() {
                        obj.insert(
                            "Alphabetic".into(),
                            serde_json::Value::String(pn.alphabetic.clone()),
                        );
                    }
                    if !pn.ideographic.is_empty() {
                        obj.insert(
                            "Ideographic".into(),
                            serde_json::Value::String(pn.ideographic.clone()),
                        );
                    }
                    if !pn.phonetic.is_empty() {
                        obj.insert(
                            "Phonetic".into(),
                            serde_json::Value::String(pn.phonetic.clone()),
                        );
                    }
                    serde_json::Value::Object(obj)
                })
                .collect();
            if arr.is_empty() {
                None
            } else {
                Some(serde_json::Value::Array(arr))
            }
        }

        Value::Date(dates) => Some(serde_json::Value::Array(
            dates
                .iter()
                .map(|d| serde_json::Value::String(d.to_string()))
                .collect(),
        )),
        Value::Time(times) => Some(serde_json::Value::Array(
            times
                .iter()
                .map(|t| serde_json::Value::String(t.to_string()))
                .collect(),
        )),
        Value::DateTime(dts) => Some(serde_json::Value::Array(
            dts.iter()
                .map(|dt| serde_json::Value::String(dt.to_string()))
                .collect(),
        )),

        Value::Ints(v) => Some(serde_json::Value::Array(
            v.iter().map(|n| serde_json::json!(n)).collect(),
        )),
        Value::Decimals(v) => Some(serde_json::Value::Array(
            v.iter()
                .map(|n| {
                    if n.is_finite() {
                        serde_json::json!(n)
                    } else {
                        serde_json::Value::Null
                    }
                })
                .collect(),
        )),

        Value::U16(v) => Some(serde_json::Value::Array(
            v.iter().map(|n| serde_json::json!(n)).collect(),
        )),
        Value::I16(v) => Some(serde_json::Value::Array(
            v.iter().map(|n| serde_json::json!(n)).collect(),
        )),
        Value::U32(v) => Some(serde_json::Value::Array(
            v.iter().map(|n| serde_json::json!(n)).collect(),
        )),
        Value::I32(v) => Some(serde_json::Value::Array(
            v.iter().map(|n| serde_json::json!(n)).collect(),
        )),
        Value::U64(v) => Some(serde_json::Value::Array(
            v.iter().map(|n| serde_json::json!(n)).collect(),
        )),
        Value::I64(v) => Some(serde_json::Value::Array(
            v.iter().map(|n| serde_json::json!(n)).collect(),
        )),
        Value::F32(v) => Some(serde_json::Value::Array(
            v.iter()
                .map(|n| {
                    if n.is_finite() {
                        serde_json::json!(n)
                    } else {
                        serde_json::Value::Null
                    }
                })
                .collect(),
        )),
        Value::F64(v) => Some(serde_json::Value::Array(
            v.iter()
                .map(|n| {
                    if n.is_finite() {
                        serde_json::json!(n)
                    } else {
                        serde_json::Value::Null
                    }
                })
                .collect(),
        )),

        Value::Tags(tags) => Some(serde_json::Value::Array(
            tags.iter()
                .map(|t| serde_json::Value::String(format!("{:04X}{:04X}", t.group, t.element)))
                .collect(),
        )),

        Value::Sequence(items) => {
            let arr: Vec<serde_json::Value> = items
                .iter()
                .map(|item| {
                    dataset_to_json_object(item)
                        .map(serde_json::Value::Object)
                        .unwrap_or(serde_json::Value::Null)
                })
                .collect();
            Some(serde_json::Value::Array(arr))
        }

        // Bulk binary data: encode as base64 InlineBinary
        Value::U8(bytes) => {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            let mut obj = serde_json::Map::new();
            obj.insert("vr".into(), serde_json::Value::String(vr_str.clone()));
            obj.insert("InlineBinary".into(), serde_json::Value::String(b64));
            return Ok(serde_json::Value::Object(obj));
        }

        Value::PixelData(pd) => {
            let bytes = match pd {
                PixelData::Native { bytes } => bytes.as_slice(),
                PixelData::Encapsulated { fragments, .. } => {
                    // Return first fragment for inline; full support would use BulkDataURI
                    fragments.first().map(|f| f.as_slice()).unwrap_or(&[])
                }
            };
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            let mut obj = serde_json::Map::new();
            obj.insert("vr".into(), serde_json::Value::String(vr_str.clone()));
            obj.insert("InlineBinary".into(), serde_json::Value::String(b64));
            return Ok(serde_json::Value::Object(obj));
        }
    };

    let mut obj = serde_json::Map::new();
    obj.insert("vr".into(), serde_json::Value::String(vr_str));
    if let Some(v) = value_json {
        obj.insert("Value".into(), v);
    }
    Ok(serde_json::Value::Object(obj))
}

// ── Deserialization ───────────────────────────────────────────────────────────

/// Deserialize a DICOM JSON string into a `DataSet`.
pub fn from_json(json: &str) -> DcmResult<DataSet> {
    let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(json)
        .map_err(|e| DcmError::Other(format!("JSON parse error: {e}")))?;
    json_object_to_dataset(&map)
}

fn json_object_to_dataset(map: &serde_json::Map<String, serde_json::Value>) -> DcmResult<DataSet> {
    let mut dataset = DataSet::new();
    for (key, val) in map {
        let tag = parse_json_tag(key)?;
        let elem = json_value_to_element(tag, val)?;
        dataset.insert(elem);
    }
    Ok(dataset)
}

fn parse_json_tag(key: &str) -> DcmResult<Tag> {
    if key.len() != 8 {
        return Err(DcmError::Other(format!("invalid JSON tag key: '{key}'")));
    }
    let group = u16::from_str_radix(&key[0..4], 16)
        .map_err(|_| DcmError::Other(format!("invalid tag group: '{}'", &key[0..4])))?;
    let element = u16::from_str_radix(&key[4..8], 16)
        .map_err(|_| DcmError::Other(format!("invalid tag element: '{}'", &key[4..8])))?;
    Ok(Tag::new(group, element))
}

fn json_value_to_element(tag: Tag, val: &serde_json::Value) -> DcmResult<Element> {
    let obj = val
        .as_object()
        .ok_or_else(|| DcmError::Other(format!("expected JSON object for tag {tag}")))?;

    let vr_str = obj
        .get("vr")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DcmError::Other(format!("missing 'vr' in JSON element for tag {tag}")))?;

    let vr = Vr::from_bytes([vr_str.as_bytes()[0], vr_str.as_bytes()[1]])
        .ok_or_else(|| DcmError::Other(format!("unknown VR '{vr_str}' in JSON for tag {tag}")))?;

    // Check for InlineBinary first
    if let Some(b64_val) = obj.get("InlineBinary") {
        use base64::Engine;
        let b64_str = b64_val
            .as_str()
            .ok_or_else(|| DcmError::Other("InlineBinary must be a string".into()))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64_str)
            .map_err(|e| DcmError::Other(format!("base64 decode error: {e}")))?;
        return Ok(Element::bytes(tag, vr, bytes));
    }

    let values_arr = match obj.get("Value") {
        None => return Ok(Element::new(tag, vr, Value::Empty)),
        Some(v) => v
            .as_array()
            .ok_or_else(|| DcmError::Other(format!("'Value' must be array for tag {tag}")))?,
    };

    let value = match vr {
        Vr::SQ => {
            let items: DcmResult<Vec<DataSet>> = values_arr
                .iter()
                .map(|item| {
                    let item_obj = item
                        .as_object()
                        .ok_or_else(|| DcmError::Other("SQ item must be a JSON object".into()))?;
                    json_object_to_dataset(item_obj)
                })
                .collect();
            Value::Sequence(items?)
        }

        Vr::PN => {
            let names: DcmResult<Vec<PersonName>> = values_arr
                .iter()
                .map(|pn_val| {
                    if pn_val.is_null() {
                        return Ok(PersonName::parse(""));
                    }
                    let pn_obj = pn_val
                        .as_object()
                        .ok_or_else(|| DcmError::Other("PN value must be a JSON object".into()))?;
                    let alphabetic = pn_obj
                        .get("Alphabetic")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let ideographic = pn_obj
                        .get("Ideographic")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let phonetic = pn_obj
                        .get("Phonetic")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Ok(PersonName {
                        alphabetic,
                        ideographic,
                        phonetic,
                    })
                })
                .collect();
            Value::PersonNames(names?)
        }

        Vr::DA => {
            let dates: DcmResult<Vec<DicomDate>> = values_arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(DicomDate::parse)
                .collect();
            Value::Date(dates?)
        }

        Vr::TM => {
            let times: DcmResult<Vec<DicomTime>> = values_arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(DicomTime::parse)
                .collect();
            Value::Time(times?)
        }

        Vr::DT => {
            let dts: DcmResult<Vec<DicomDateTime>> = values_arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(DicomDateTime::parse)
                .collect();
            Value::DateTime(dts?)
        }

        Vr::UI => {
            let uid = values_arr
                .first()
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Value::Uid(uid)
        }

        Vr::IS => {
            let ints: Vec<i64> = values_arr.iter().filter_map(|v| v.as_i64()).collect();
            Value::Ints(ints)
        }

        Vr::DS => {
            let decimals: Vec<f64> = values_arr.iter().filter_map(|v| v.as_f64()).collect();
            Value::Decimals(decimals)
        }

        Vr::US | Vr::OW => {
            let vals: Vec<u16> = values_arr
                .iter()
                .filter_map(|v| v.as_u64().map(|n| n as u16))
                .collect();
            Value::U16(vals)
        }

        Vr::SS => {
            let vals: Vec<i16> = values_arr
                .iter()
                .filter_map(|v| v.as_i64().map(|n| n as i16))
                .collect();
            Value::I16(vals)
        }

        Vr::UL | Vr::OL => {
            let vals: Vec<u32> = values_arr
                .iter()
                .filter_map(|v| v.as_u64().map(|n| n as u32))
                .collect();
            Value::U32(vals)
        }

        Vr::SL => {
            let vals: Vec<i32> = values_arr
                .iter()
                .filter_map(|v| v.as_i64().map(|n| n as i32))
                .collect();
            Value::I32(vals)
        }

        Vr::UV | Vr::OV => {
            let vals: Vec<u64> = values_arr.iter().filter_map(|v| v.as_u64()).collect();
            Value::U64(vals)
        }

        Vr::SV => {
            let vals: Vec<i64> = values_arr.iter().filter_map(|v| v.as_i64()).collect();
            Value::I64(vals)
        }

        Vr::FL | Vr::OF => {
            let vals: Vec<f32> = values_arr
                .iter()
                .filter_map(|v| v.as_f64().map(|n| n as f32))
                .collect();
            Value::F32(vals)
        }

        Vr::FD | Vr::OD => {
            let vals: Vec<f64> = values_arr.iter().filter_map(|v| v.as_f64()).collect();
            Value::F64(vals)
        }

        Vr::AT => {
            let tags: DcmResult<Vec<Tag>> = values_arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| {
                    if s.len() != 8 {
                        return Err(DcmError::Other(format!("invalid AT value: '{s}'")));
                    }
                    let g = u16::from_str_radix(&s[0..4], 16)
                        .map_err(|_| DcmError::Other(format!("bad AT group: {s}")))?;
                    let e = u16::from_str_radix(&s[4..8], 16)
                        .map_err(|_| DcmError::Other(format!("bad AT element: {s}")))?;
                    Ok(Tag::new(g, e))
                })
                .collect();
            Value::Tags(tags?)
        }

        // Default: string VRs
        _ => {
            let strings: Vec<String> = values_arr
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            Value::Strings(strings)
        }
    };

    Ok(Element::new(tag, vr, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_dict::tags;

    fn make_dataset() -> DataSet {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_NAME, Vr::PN, "Doe^John");
        ds.set_string(tags::PATIENT_ID, Vr::LO, "PAT-001");
        ds.set_uid(tags::SOP_INSTANCE_UID, "1.2.3.4.5");
        ds.set_u16(tags::SAMPLES_PER_PIXEL, 1);
        ds.set_u16(tags::ROWS, 512);
        ds.set_u16(tags::COLUMNS, 512);
        ds
    }

    #[test]
    fn serialize_basic_dataset() {
        let ds = make_dataset();
        let json = to_json(&ds).unwrap();
        // Should contain our tags as 8-hex-char keys
        assert!(json.contains("00100010"), "should contain PatientName tag");
        assert!(json.contains("00100020"), "should contain PatientID tag");
        assert!(
            json.contains("0020000D") || json.contains("00080018"),
            "should contain UID tag"
        );
    }

    #[test]
    fn roundtrip_string_element() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_ID, Vr::LO, "PAT-123");

        let json = to_json(&ds).unwrap();
        let parsed = from_json(&json).unwrap();

        assert_eq!(parsed.get_string(tags::PATIENT_ID), Some("PAT-123"));
    }

    #[test]
    fn roundtrip_uid_element() {
        let mut ds = DataSet::new();
        ds.set_uid(tags::SOP_INSTANCE_UID, "1.2.840.10008.5.1.4.1.1.2");

        let json = to_json(&ds).unwrap();
        let parsed = from_json(&json).unwrap();

        assert_eq!(
            parsed.get_string(tags::SOP_INSTANCE_UID),
            Some("1.2.840.10008.5.1.4.1.1.2")
        );
    }

    #[test]
    fn roundtrip_u16_element() {
        let mut ds = DataSet::new();
        ds.set_u16(tags::ROWS, 256);

        let json = to_json(&ds).unwrap();
        let parsed = from_json(&json).unwrap();

        assert_eq!(parsed.get_u16(tags::ROWS), Some(256));
    }

    #[test]
    fn roundtrip_sequence() {
        let mut ds = DataSet::new();
        let mut item = DataSet::new();
        item.set_string(tags::PATIENT_ID, Vr::LO, "ITEM-1");
        ds.set_sequence(tags::REFERENCED_SOP_SEQUENCE, vec![item]);

        let json = to_json(&ds).unwrap();
        let parsed = from_json(&json).unwrap();

        let items = parsed.get_items(tags::REFERENCED_SOP_SEQUENCE).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get_string(tags::PATIENT_ID), Some("ITEM-1"));
    }

    #[test]
    fn roundtrip_person_name() {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_NAME, Vr::PN, "Smith^John^^Dr.");

        let json = to_json(&ds).unwrap();
        assert!(json.contains("Alphabetic"), "PN should use Alphabetic key");

        let parsed = from_json(&json).unwrap();
        // PN is stored as PersonName; we get it via the string getter which formats it back
        assert!(parsed.contains(tags::PATIENT_NAME));
    }

    #[test]
    fn invalid_json_returns_error() {
        assert!(from_json("not json").is_err());
        assert!(from_json("[]").is_err(), "array at root should fail");
    }

    #[test]
    fn invalid_tag_key_returns_error() {
        // tag key too short
        assert!(from_json(r#"{"00100": {"vr": "LO"}}"#).is_err());
        // non-hex chars
        assert!(from_json(r#"{"GGGGEEEE": {"vr": "LO"}}"#).is_err());
    }

    #[test]
    fn pretty_print_is_valid_json() {
        let ds = make_dataset();
        let pretty = to_json_pretty(&ds).unwrap();
        // Should be parseable
        let reparsed: serde_json::Value = serde_json::from_str(&pretty).unwrap();
        assert!(reparsed.is_object());
    }
}
