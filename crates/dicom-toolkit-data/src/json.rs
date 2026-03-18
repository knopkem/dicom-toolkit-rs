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

/// Controls how binary values are represented when serializing DICOM JSON.
pub enum BinaryValueMode<'a> {
    /// Keep binary data inline using the existing JSON behavior.
    InlineBinary,
    /// Emit `BulkDataURI` for eligible tags when the callback provides a URI.
    ///
    /// If encapsulated Pixel Data is encountered and the callback does not
    /// provide a URI, serialization returns an error instead of emitting only
    /// the first fragment.
    BulkDataUri(&'a dyn Fn(Tag) -> Option<String>),
}

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

/// Serialize a `DataSet` to a DICOM JSON string using an explicit binary-value policy.
pub fn to_json_with_binary_mode(dataset: &DataSet, mode: BinaryValueMode<'_>) -> DcmResult<String> {
    let obj = dataset_to_json_object_with_binary_mode(dataset, &mode)?;
    serde_json::to_string(&obj).map_err(|e| DcmError::Other(format!("JSON serialize error: {e}")))
}

fn dataset_to_json_object(
    dataset: &DataSet,
) -> DcmResult<serde_json::Map<String, serde_json::Value>> {
    dataset_to_json_object_internal(dataset, None)
}

fn dataset_to_json_object_with_binary_mode(
    dataset: &DataSet,
    mode: &BinaryValueMode<'_>,
) -> DcmResult<serde_json::Map<String, serde_json::Value>> {
    dataset_to_json_object_internal(dataset, Some(mode))
}

fn dataset_to_json_object_internal(
    dataset: &DataSet,
    binary_mode: Option<&BinaryValueMode<'_>>,
) -> DcmResult<serde_json::Map<String, serde_json::Value>> {
    let mut map = serde_json::Map::new();
    for (tag, elem) in dataset.iter() {
        // Skip group-length tags and sequence delimiter tags
        if tag.is_group_length() || tag.is_delimiter() {
            continue;
        }
        let key = format!("{:04X}{:04X}", tag.group, tag.element);
        let json_elem = element_to_json_internal(elem, binary_mode)?;
        map.insert(key, json_elem);
    }
    Ok(map)
}

fn element_to_json_internal(
    elem: &Element,
    binary_mode: Option<&BinaryValueMode<'_>>,
) -> DcmResult<serde_json::Value> {
    let vr_str = elem.vr.code().to_string();

    if let Some(json) = binary_value_to_json(elem, binary_mode, &vr_str)? {
        return Ok(json);
    }

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
                    dataset_to_json_object_internal(item, binary_mode)
                        .map(serde_json::Value::Object)
                        .unwrap_or(serde_json::Value::Null)
                })
                .collect();
            Some(serde_json::Value::Array(arr))
        }
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

fn binary_value_to_json(
    elem: &Element,
    binary_mode: Option<&BinaryValueMode<'_>>,
    vr_str: &str,
) -> DcmResult<Option<serde_json::Value>> {
    let Some(binary_mode) = binary_mode else {
        return Ok(None);
    };

    if !is_bulk_data_eligible(elem) {
        return Ok(None);
    }

    match binary_mode {
        BinaryValueMode::BulkDataUri(resolve_uri) => {
            if let Some(uri) = resolve_uri(elem.tag) {
                return Ok(Some(json_bulk_data_uri(vr_str, uri)));
            }

            if matches!(elem.value, Value::PixelData(PixelData::Encapsulated { .. })) {
                return Err(DcmError::Other(format!(
                    "encapsulated Pixel Data tag {} requires BulkDataURI in to_json_with_binary_mode",
                    elem.tag
                )));
            }

            Ok(None)
        }
        BinaryValueMode::InlineBinary => {
            if let Value::PixelData(PixelData::Encapsulated { .. }) = &elem.value {
                return Err(DcmError::Other(format!(
                    "encapsulated Pixel Data tag {} requires BulkDataURI in to_json_with_binary_mode",
                    elem.tag
                )));
            }
            Ok(None)
        }
    }
}

fn is_bulk_data_eligible(elem: &Element) -> bool {
    matches!(elem.value, Value::PixelData(_))
        || matches!(
            elem.vr,
            Vr::OB | Vr::OD | Vr::OF | Vr::OL | Vr::OV | Vr::OW | Vr::UN
        )
}

fn json_bulk_data_uri(vr_str: &str, uri: String) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("vr".into(), serde_json::Value::String(vr_str.to_string()));
    obj.insert("BulkDataURI".into(), serde_json::Value::String(uri));
    serde_json::Value::Object(obj)
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

fn json_scalar_token_string(tag: Tag, vr: Vr, value: &serde_json::Value) -> DcmResult<String> {
    match value {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Null => Ok(String::new()),
        _ => Err(DcmError::Other(format!(
            "invalid JSON value for tag {tag} VR {}: expected number, string or null",
            vr.code()
        ))),
    }
}

fn json_value_tokens(tag: Tag, vr: Vr, values_arr: &[serde_json::Value]) -> DcmResult<Vec<String>> {
    values_arr
        .iter()
        .map(|value| json_scalar_token_string(tag, vr, value))
        .collect()
}

fn tokens_are_all_empty(tokens: &[String]) -> bool {
    tokens.iter().all(|token| token.is_empty())
}

fn reject_mixed_empty_tokens(tag: Tag, vr: Vr, tokens: &[String]) -> DcmResult<()> {
    let has_empty = tokens.iter().any(|token| token.is_empty());
    let has_non_empty = tokens.iter().any(|token| !token.is_empty());

    if has_empty && has_non_empty {
        return Err(DcmError::Other(format!(
            "tag {tag} VR {} cannot represent mixed empty and non-empty JSON values",
            vr.code()
        )));
    }

    Ok(())
}

fn parse_numeric_tokens<T>(
    tag: Tag,
    vr: Vr,
    values_arr: &[serde_json::Value],
) -> DcmResult<Option<Vec<T>>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let tokens = json_value_tokens(tag, vr, values_arr)?;
    if tokens_are_all_empty(&tokens) {
        return Ok(None);
    }

    reject_mixed_empty_tokens(tag, vr, &tokens)?;

    let values = tokens
        .iter()
        .map(|token| {
            token.parse::<T>().map_err(|err| {
                DcmError::Other(format!(
                    "invalid {} value '{}' for tag {tag}: {err}",
                    vr.code(),
                    token
                ))
            })
        })
        .collect::<DcmResult<Vec<T>>>()?;

    Ok(Some(values))
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

    if let Some(uri_val) = obj.get("BulkDataURI") {
        let _uri = uri_val
            .as_str()
            .ok_or_else(|| DcmError::Other("BulkDataURI must be a string".into()))?;
        return Err(DcmError::Other(format!(
            "BulkDataURI deserialization is not supported for tag {tag}"
        )));
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
            let tokens = json_value_tokens(tag, vr, values_arr)?;
            if tokens_are_all_empty(&tokens) {
                Value::Empty
            } else {
                reject_mixed_empty_tokens(tag, vr, &tokens)?;
                let dates: DcmResult<Vec<DicomDate>> =
                    tokens.iter().map(|token| DicomDate::parse(token)).collect();
                Value::Date(dates?)
            }
        }

        Vr::TM => {
            let tokens = json_value_tokens(tag, vr, values_arr)?;
            if tokens_are_all_empty(&tokens) {
                Value::Empty
            } else {
                reject_mixed_empty_tokens(tag, vr, &tokens)?;
                let times: DcmResult<Vec<DicomTime>> =
                    tokens.iter().map(|token| DicomTime::parse(token)).collect();
                Value::Time(times?)
            }
        }

        Vr::DT => {
            let tokens = json_value_tokens(tag, vr, values_arr)?;
            if tokens_are_all_empty(&tokens) {
                Value::Empty
            } else {
                reject_mixed_empty_tokens(tag, vr, &tokens)?;
                let dts: DcmResult<Vec<DicomDateTime>> = tokens
                    .iter()
                    .map(|token| DicomDateTime::parse(token))
                    .collect();
                Value::DateTime(dts?)
            }
        }

        Vr::UI => {
            let tokens = json_value_tokens(tag, vr, values_arr)?;
            if tokens_are_all_empty(&tokens) {
                Value::Empty
            } else {
                reject_mixed_empty_tokens(tag, vr, &tokens)?;
                let uid = tokens.first().cloned().unwrap_or_default();
                Value::Uid(uid)
            }
        }

        Vr::IS => match parse_numeric_tokens::<i64>(tag, vr, values_arr)? {
            Some(ints) => Value::Ints(ints),
            None => Value::Empty,
        },

        Vr::DS => match parse_numeric_tokens::<f64>(tag, vr, values_arr)? {
            Some(decimals) => Value::Decimals(decimals),
            None => Value::Empty,
        },

        Vr::US | Vr::OW => match parse_numeric_tokens::<u16>(tag, vr, values_arr)? {
            Some(vals) => Value::U16(vals),
            None => Value::Empty,
        },

        Vr::SS => match parse_numeric_tokens::<i16>(tag, vr, values_arr)? {
            Some(vals) => Value::I16(vals),
            None => Value::Empty,
        },

        Vr::UL | Vr::OL => match parse_numeric_tokens::<u32>(tag, vr, values_arr)? {
            Some(vals) => Value::U32(vals),
            None => Value::Empty,
        },

        Vr::SL => match parse_numeric_tokens::<i32>(tag, vr, values_arr)? {
            Some(vals) => Value::I32(vals),
            None => Value::Empty,
        },

        Vr::UV | Vr::OV => match parse_numeric_tokens::<u64>(tag, vr, values_arr)? {
            Some(vals) => Value::U64(vals),
            None => Value::Empty,
        },

        Vr::SV => match parse_numeric_tokens::<i64>(tag, vr, values_arr)? {
            Some(vals) => Value::I64(vals),
            None => Value::Empty,
        },

        Vr::FL | Vr::OF => match parse_numeric_tokens::<f32>(tag, vr, values_arr)? {
            Some(vals) => Value::F32(vals),
            None => Value::Empty,
        },

        Vr::FD | Vr::OD => match parse_numeric_tokens::<f64>(tag, vr, values_arr)? {
            Some(vals) => Value::F64(vals),
            None => Value::Empty,
        },

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
            let strings = json_value_tokens(tag, vr, values_arr)?;
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

    #[test]
    fn bulk_data_uri_mode_uses_uri_for_binary_vrs() {
        let binary_tag = Tag::new(0x5400, 0x1010);
        let mut ds = DataSet::new();
        ds.insert(Element::new(
            binary_tag,
            Vr::OB,
            Value::U8(vec![1, 2, 3, 4]),
        ));

        let json = to_json_with_binary_mode(
            &ds,
            BinaryValueMode::BulkDataUri(&|tag| {
                (tag == binary_tag).then_some("https://example.test/bulk/54001010".to_string())
            }),
        )
        .unwrap();

        assert!(json.contains("\"BulkDataURI\":\"https://example.test/bulk/54001010\""));
        assert!(!json.contains("InlineBinary"));
    }

    #[test]
    fn bulk_data_uri_mode_uses_uri_for_pixel_data() {
        let mut ds = DataSet::new();
        ds.insert(Element::new(
            tags::PIXEL_DATA,
            Vr::OB,
            Value::PixelData(PixelData::Encapsulated {
                offset_table: vec![0],
                fragments: vec![vec![1, 2], vec![3, 4]],
            }),
        ));

        let json = to_json_with_binary_mode(
            &ds,
            BinaryValueMode::BulkDataUri(&|tag| {
                (tag == tags::PIXEL_DATA).then_some("https://example.test/bulk/pixel".to_string())
            }),
        )
        .unwrap();

        assert!(json.contains("\"BulkDataURI\":\"https://example.test/bulk/pixel\""));
        assert!(!json.contains("InlineBinary"));
    }

    #[test]
    fn bulk_data_uri_mode_rejects_encapsulated_pixel_data_without_uri() {
        let mut ds = DataSet::new();
        ds.insert(Element::new(
            tags::PIXEL_DATA,
            Vr::OB,
            Value::PixelData(PixelData::Encapsulated {
                offset_table: vec![0],
                fragments: vec![vec![1, 2], vec![3, 4]],
            }),
        ));

        let err =
            to_json_with_binary_mode(&ds, BinaryValueMode::BulkDataUri(&|_| None)).unwrap_err();
        assert!(err.to_string().contains("requires BulkDataURI"));
    }

    #[test]
    fn from_json_accepts_dcmtk_style_scalar_tokens() {
        let json = r#"{
            "00100020": {"vr": "LO", "Value": [123]},
            "00200013": {"vr": "IS", "Value": ["42"]},
            "00180050": {"vr": "DS", "Value": ["2.5"]},
            "00280010": {"vr": "US", "Value": ["256"]}
        }"#;

        let parsed = from_json(json).unwrap();

        assert_eq!(parsed.get_string(tags::PATIENT_ID), Some("123"));
        assert_eq!(
            parsed.get(Tag::new(0x0020, 0x0013)).unwrap().value,
            Value::Ints(vec![42])
        );
        match &parsed.get(Tag::new(0x0018, 0x0050)).unwrap().value {
            Value::Decimals(values) => assert!((values[0] - 2.5).abs() < 1e-9),
            other => panic!("unexpected value: {:?}", other),
        }
        assert_eq!(parsed.get_u16(tags::ROWS), Some(256));
    }

    #[test]
    fn from_json_rejects_non_scalar_value_entries() {
        let err = from_json(r#"{"00100020":{"vr":"LO","Value":[{}]}}"#).unwrap_err();
        assert!(err.to_string().contains("expected number, string or null"));
    }

    #[test]
    fn from_json_rejects_bulk_data_uri_without_loader() {
        let err =
            from_json(r#"{"7FE00010":{"vr":"OB","BulkDataURI":"https://example.test/pixel"}}"#)
                .unwrap_err();
        assert!(err.to_string().contains("BulkDataURI"));
    }
}
