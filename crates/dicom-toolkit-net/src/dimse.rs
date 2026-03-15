//! DIMSE (DICOM Message Service Element) dataset encoding and decoding.
//!
//! DIMSE command datasets are always encoded in **Implicit VR Little Endian**
//! (PS3.7 §6.3.1).  Only group-0000 elements appear in command datasets.
//!
//! Each element on the wire:
//! ```text
//! [2B group  LE][2B element LE][4B value-length LE][value-length bytes]
//! ```

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_data::{DataSet, Element, Value};
use dicom_toolkit_dict::{Tag, Vr};

// ── Encoding ──────────────────────────────────────────────────────────────────

/// Encode a DIMSE command `DataSet` into Implicit VR LE bytes.
///
/// This function:
/// 1. Encodes every group-0000 element (other groups are ignored).
/// 2. Prepends the `CommandGroupLength` (0000,0000) element set to the total
///    byte length of the remaining encoded elements.
pub fn encode_command_dataset(ds: &DataSet) -> Vec<u8> {
    // Encode all command elements except (0000,0000) in order
    let mut body = Vec::new();
    for (tag, elem) in ds.iter() {
        if tag.group != 0x0000 || tag.element == 0x0000 {
            continue;
        }
        encode_element_implicit_le(&mut body, elem);
    }

    // Prepend CommandGroupLength = byte length of everything that follows
    let group_length = body.len() as u32;
    let mut result = Vec::with_capacity(8 + body.len());
    result.extend_from_slice(&0x0000u16.to_le_bytes()); // group
    result.extend_from_slice(&0x0000u16.to_le_bytes()); // element
    result.extend_from_slice(&4u32.to_le_bytes()); // value length = 4
    result.extend_from_slice(&group_length.to_le_bytes()); // UL value
    result.extend_from_slice(&body);
    result
}

/// Encode a single element in Implicit VR LE.
pub(crate) fn encode_element_implicit_le(buf: &mut Vec<u8>, elem: &Element) {
    let value_bytes = encode_value_implicit(&elem.value, elem.vr);
    buf.extend_from_slice(&elem.tag.group.to_le_bytes());
    buf.extend_from_slice(&elem.tag.element.to_le_bytes());
    buf.extend_from_slice(&(value_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(&value_bytes);
}

/// Serialize an element's `Value` to raw bytes suitable for Implicit VR LE.
fn encode_value_implicit(value: &Value, vr: Vr) -> Vec<u8> {
    match value {
        Value::U16(vals) => {
            let mut buf = Vec::with_capacity(2 * vals.len());
            for v in vals {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf
        }
        Value::U32(vals) => {
            let mut buf = Vec::with_capacity(4 * vals.len());
            for v in vals {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf
        }
        Value::I32(vals) => {
            let mut buf = Vec::with_capacity(4 * vals.len());
            for v in vals {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf
        }
        Value::I16(vals) => {
            let mut buf = Vec::with_capacity(2 * vals.len());
            for v in vals {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf
        }
        Value::F64(vals) => {
            let mut buf = Vec::with_capacity(8 * vals.len());
            for v in vals {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf
        }
        Value::F32(vals) => {
            let mut buf = Vec::with_capacity(4 * vals.len());
            for v in vals {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            buf
        }
        Value::Uid(uid) => {
            // UI is null-padded to even length
            let mut b = uid.as_bytes().to_vec();
            if b.len() % 2 != 0 {
                b.push(0x00);
            }
            b
        }
        Value::Strings(ss) => {
            let s = ss.join("\\");
            let mut b = s.into_bytes();
            // String VRs are space-padded to even length
            let pad = if vr == Vr::UI { 0x00 } else { b' ' };
            if b.len() % 2 != 0 {
                b.push(pad);
            }
            b
        }
        Value::U8(bytes) => {
            let mut b = bytes.clone();
            if b.len() % 2 != 0 {
                b.push(0x00);
            }
            b
        }
        Value::Empty => Vec::new(),
        _ => Vec::new(), // other value types not used in DIMSE command datasets
    }
}

// ── Decoding ──────────────────────────────────────────────────────────────────

/// Decode a DIMSE command dataset from Implicit VR LE bytes.
///
/// Uses a built-in VR lookup table for the known group-0000 tags.
pub fn decode_command_dataset(bytes: &[u8]) -> DcmResult<DataSet> {
    let mut ds = DataSet::new();
    let mut pos = 0;

    while pos + 8 <= bytes.len() {
        let group = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]);
        let element = u16::from_le_bytes([bytes[pos + 2], bytes[pos + 3]]);
        let len =
            u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]])
                as usize;
        pos += 8;

        if pos + len > bytes.len() {
            return Err(DcmError::Other(format!(
                "DIMSE element ({:04X},{:04X}) value truncated: need {} bytes, have {}",
                group,
                element,
                len,
                bytes.len() - pos + len,
            )));
        }
        let val_bytes = &bytes[pos..pos + len];
        pos += len;

        let tag = Tag::new(group, element);
        if let Some(elem) = decode_dimse_element(tag, val_bytes) {
            ds.insert(elem);
        }
    }
    Ok(ds)
}

/// Look up the VR for a known group-0000 DIMSE command tag and build an Element.
fn decode_dimse_element(tag: Tag, bytes: &[u8]) -> Option<Element> {
    match (tag.group, tag.element) {
        // ── UL (4 bytes) ──────────────────────────────────────────────────
        (0x0000, 0x0000) => {
            // CommandGroupLength
            bytes
                .get(..4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .map(|v| Element::u32(tag, v))
        }

        // ── US (2 bytes) ──────────────────────────────────────────────────
        (0x0000, 0x0100  // CommandField
            | 0x0110     // MessageID
            | 0x0120     // MessageIDBeingRespondedTo
            | 0x0700     // Priority
            | 0x0800     // CommandDataSetType
            | 0x0900     // Status
            | 0x1020     // NumberOfRemainingSubOperations
            | 0x1021     // NumberOfCompletedSubOperations
            | 0x1022     // NumberOfFailedSubOperations
            | 0x1023) => { // NumberOfWarningSubOperations
            bytes
                .get(..2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .map(|v| Element::u16(tag, v))
        }

        // ── UI (variable) ─────────────────────────────────────────────────
        (0x0000, 0x0002  // AffectedSOPClassUID
            | 0x0003     // RequestedSOPClassUID
            | 0x1000     // AffectedSOPInstanceUID
            | 0x1001) => { // RequestedSOPInstanceUID
            let uid = crate::pdu::decode_uid_bytes(bytes);
            Some(Element::uid(tag, &uid))
        }

        // ── AE (variable) ─────────────────────────────────────────────────
        (0x0000, 0x0600) => {
            // MoveDestination
            let ae = String::from_utf8_lossy(bytes).trim().to_string();
            Some(Element::string(tag, Vr::AE, &ae))
        }

        _ => None, // unknown tag silently skipped
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_dict::tags;

    fn build_echo_rq() -> DataSet {
        let mut ds = DataSet::new();
        ds.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.1.1");
        ds.set_u16(tags::COMMAND_FIELD, 0x0030);
        ds.set_u16(tags::MESSAGE_ID, 42);
        ds.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        ds
    }

    #[test]
    fn echo_rq_encode_decode_roundtrip() {
        let ds = build_echo_rq();
        let bytes = encode_command_dataset(&ds);

        // Must start with CommandGroupLength (0000,0000)
        assert_eq!(&bytes[0..4], &[0x00, 0x00, 0x00, 0x00]);

        let decoded = decode_command_dataset(&bytes).unwrap();
        assert_eq!(
            decoded.get_string(tags::AFFECTED_SOP_CLASS_UID),
            Some("1.2.840.10008.1.1")
        );
        assert_eq!(decoded.get_u16(tags::COMMAND_FIELD), Some(0x0030));
        assert_eq!(decoded.get_u16(tags::MESSAGE_ID), Some(42));
        assert_eq!(decoded.get_u16(tags::COMMAND_DATA_SET_TYPE), Some(0x0101));
    }

    #[test]
    fn c_store_rq_roundtrip() {
        let mut ds = DataSet::new();
        ds.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.1.2");
        ds.set_u16(tags::COMMAND_FIELD, 0x0001); // C-STORE-RQ
        ds.set_u16(tags::MESSAGE_ID, 1);
        ds.set_u16(tags::PRIORITY, 0); // medium
        ds.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000); // has dataset
        ds.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, "1.2.3.4.5.6.7.8.9");

        let bytes = encode_command_dataset(&ds);
        let decoded = decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::COMMAND_FIELD), Some(0x0001));
        assert_eq!(
            decoded.get_string(tags::AFFECTED_SOP_CLASS_UID),
            Some("1.2.840.10008.5.1.4.1.1.2")
        );
        assert_eq!(
            decoded.get_string(tags::AFFECTED_SOP_INSTANCE_UID),
            Some("1.2.3.4.5.6.7.8.9")
        );
        assert_eq!(decoded.get_u16(tags::PRIORITY), Some(0));
    }

    #[test]
    fn command_group_length_is_correct() {
        let ds = build_echo_rq();
        let bytes = encode_command_dataset(&ds);
        let decoded = decode_command_dataset(&bytes).unwrap();

        // CommandGroupLength = byte length of everything after (0000,0000)
        let group_len = decoded.get_u32(tags::COMMAND_GROUP_LENGTH).unwrap();
        // The CommandGroupLength element itself is 12 bytes (4 tag + 4 len + 4 value).
        let remaining = bytes.len() - 12;
        assert_eq!(group_len as usize, remaining);
    }

    #[test]
    fn empty_dataset_encodes_only_group_length() {
        let ds = DataSet::new();
        let bytes = encode_command_dataset(&ds);
        // 8 bytes: tag(4) + len(4) + value(4) = 12 bytes for CommandGroupLength alone
        assert_eq!(bytes.len(), 12);
        let decoded = decode_command_dataset(&bytes).unwrap();
        assert_eq!(decoded.get_u32(tags::COMMAND_GROUP_LENGTH), Some(0));
    }

    #[test]
    fn uid_padded_to_even_length() {
        // UID "1.2.840.10008.1.1" has 17 chars (odd) — should be null-padded
        let uid = "1.2.840.10008.1.1";
        assert_eq!(uid.len() % 2, 1); // confirm it's odd

        let mut ds = DataSet::new();
        ds.set_uid(tags::AFFECTED_SOP_CLASS_UID, uid);
        let bytes = encode_command_dataset(&ds);

        // Each element: 4 (tag) + 4 (len) + N (value, even)
        // CommandGroupLength: 12 bytes
        // UID element: 4 + 4 + 18 (padded from 17) = 26 bytes
        assert_eq!(bytes.len(), 12 + 26);

        let decoded = decode_command_dataset(&bytes).unwrap();
        assert_eq!(
            decoded.get_string(tags::AFFECTED_SOP_CLASS_UID),
            Some(uid)
        );
    }

    #[test]
    fn move_destination_roundtrip() {
        let mut ds = DataSet::new();
        ds.set_string(tags::MOVE_DESTINATION, Vr::AE, "DEST_AE");
        let bytes = encode_command_dataset(&ds);
        let decoded = decode_command_dataset(&bytes).unwrap();
        assert_eq!(decoded.get_string(tags::MOVE_DESTINATION), Some("DEST_AE"));
    }
}
