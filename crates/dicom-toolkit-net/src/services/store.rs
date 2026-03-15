//! C-STORE (Storage Service Class) — PS3.4 §B.

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::DataSet;
use dicom_toolkit_dict::tags;

use crate::association::Association;

// ── Public types ──────────────────────────────────────────────────────────────

/// Parameters for a C-STORE-RQ.
#[derive(Debug, Clone)]
pub struct StoreRequest {
    /// Affected SOP Class UID.
    pub sop_class_uid: String,
    /// Affected SOP Instance UID.
    pub sop_instance_uid: String,
    /// Priority: 0=medium, 1=high, 2=low.
    pub priority: u16,
    /// Pre-encoded data set bytes (the DICOM object to store).
    pub dataset_bytes: Vec<u8>,
    /// Presentation context ID to use.
    pub context_id: u8,
}

/// Response received from the SCP for a C-STORE operation.
#[derive(Debug, Clone)]
pub struct StoreResponse {
    /// DIMSE status code (0x0000 = success).
    pub status: u16,
    /// The message ID echoed from the request.
    pub message_id: u16,
}

// ── C-STORE ───────────────────────────────────────────────────────────────────

/// Send a C-STORE-RQ followed by the data set and wait for the C-STORE-RSP.
pub async fn c_store(assoc: &mut Association, req: StoreRequest) -> DcmResult<StoreResponse> {
    let msg_id = next_message_id();

    // Build C-STORE-RQ command dataset
    let mut cmd = DataSet::new();
    cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, &req.sop_class_uid);
    cmd.set_u16(tags::COMMAND_FIELD, 0x0001); // C-STORE-RQ
    cmd.set_u16(tags::MESSAGE_ID, msg_id);
    cmd.set_u16(tags::PRIORITY, req.priority);
    cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000); // dataset present
    cmd.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, &req.sop_instance_uid);

    assoc.send_dimse_command(req.context_id, &cmd).await?;
    assoc.send_dimse_data(req.context_id, &req.dataset_bytes).await?;

    // Receive C-STORE-RSP
    let (_ctx, rsp) = assoc.recv_dimse_command().await?;
    let status = rsp.get_u16(tags::STATUS).unwrap_or(0xFFFF);

    Ok(StoreResponse { status, message_id: msg_id })
}

fn next_message_id() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static ID: AtomicU16 = AtomicU16::new(1);
    ID.fetch_add(1, Ordering::Relaxed)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::dimse;
    use dicom_toolkit_data::DataSet;
    use dicom_toolkit_dict::tags;

    #[test]
    fn c_store_rq_command_build() {
        let mut cmd = DataSet::new();
        cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.1.2");
        cmd.set_u16(tags::COMMAND_FIELD, 0x0001);
        cmd.set_u16(tags::MESSAGE_ID, 7);
        cmd.set_u16(tags::PRIORITY, 0);
        cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000);
        cmd.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, "1.2.3.4.5.6.7");

        let bytes = dimse::encode_command_dataset(&cmd);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::COMMAND_FIELD), Some(0x0001));
        assert_eq!(decoded.get_u16(tags::MESSAGE_ID), Some(7));
        assert_eq!(decoded.get_u16(tags::PRIORITY), Some(0));
        assert_eq!(decoded.get_u16(tags::COMMAND_DATA_SET_TYPE), Some(0x0000));
        assert_eq!(
            decoded.get_string(tags::AFFECTED_SOP_INSTANCE_UID),
            Some("1.2.3.4.5.6.7")
        );
    }

    #[test]
    fn c_store_rsp_success_status() {
        let mut rsp = DataSet::new();
        rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.1.2");
        rsp.set_u16(tags::COMMAND_FIELD, 0x8001); // C-STORE-RSP
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, 7);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        rsp.set_u16(tags::STATUS, 0x0000);

        let bytes = dimse::encode_command_dataset(&rsp);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();
        assert_eq!(decoded.get_u16(tags::STATUS), Some(0x0000));
        assert_eq!(decoded.get_u16(tags::COMMAND_FIELD), Some(0x8001));
    }
}
