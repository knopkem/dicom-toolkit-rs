//! C-GET (Query/Retrieve — Retrieve Service to initiating AE) — PS3.4 §C.4.3.

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::DataSet;
use dicom_toolkit_dict::tags;

use crate::association::Association;

// ── Public types ──────────────────────────────────────────────────────────────

/// Parameters for a C-GET-RQ.
#[derive(Debug, Clone)]
pub struct GetRequest {
    /// Affected SOP Class UID (e.g. Patient Root Query/Retrieve – GET).
    pub sop_class_uid: String,
    /// Pre-encoded query identifier dataset bytes (the set of attributes to match).
    pub query: Vec<u8>,
    /// Presentation context ID negotiated for this SOP class.
    pub context_id: u8,
    /// Priority: 0 = medium, 1 = high, 2 = low.
    pub priority: u16,
}

/// A single C-GET-RSP received from the SCP.
#[derive(Debug, Clone)]
pub struct GetResponse {
    /// DIMSE status code.
    ///
    /// * `0xFF00` / `0xFF01` — pending (more sub-operations in progress).
    /// * `0x0000` — success (all sub-operations completed).
    /// * Other — warning or failure.
    pub status: u16,
    /// Number of sub-operations remaining.
    pub remaining: Option<u16>,
    /// Number of sub-operations completed successfully.
    pub completed: Option<u16>,
    /// Number of sub-operations that failed.
    pub failed: Option<u16>,
    /// Number of sub-operations that completed with a warning.
    pub warning: Option<u16>,
    /// Dataset returned with the response (present on the final response
    /// when failures occurred, listing the failed SOP instance UIDs).
    pub dataset: Option<Vec<u8>>,
}

/// A DICOM instance delivered by the SCP via a C-STORE sub-operation
/// during a C-GET exchange.
#[derive(Debug, Clone)]
pub struct ReceivedInstance {
    /// SOP Class UID of the received instance.
    pub sop_class_uid: String,
    /// SOP Instance UID of the received instance.
    pub sop_instance_uid: String,
    /// Raw encoded dataset bytes (use `DicomReader::read_dataset` to decode).
    pub dataset: Vec<u8>,
}

/// Result of a C-GET operation.
#[derive(Debug)]
pub struct GetResult {
    /// All C-GET-RSP status messages received (pending + final).
    pub responses: Vec<GetResponse>,
    /// Instances delivered by the SCP via C-STORE sub-operations on this
    /// association.  Ordered as received.
    pub instances: Vec<ReceivedInstance>,
}

// ── C-GET ─────────────────────────────────────────────────────────────────────

/// Execute a C-GET operation and collect all responses and received instances.
///
/// Sends a C-GET-RQ, then drives the interleaved protocol:
///
/// * **C-STORE-RQ** sub-operations sent by the SCP on this association are
///   received, stored in [`GetResult::instances`], and acknowledged with a
///   `C-STORE-RSP` (status `0x0000`).
/// * **C-GET-RSP** messages are collected into [`GetResult::responses`];
///   pending responses (`0xFF00` / `0xFF01`) continue the loop and the final
///   response terminates it.
pub async fn c_get(assoc: &mut Association, req: GetRequest) -> DcmResult<GetResult> {
    let msg_id = next_message_id();

    // Build C-GET-RQ command dataset.
    let mut cmd = DataSet::new();
    cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, &req.sop_class_uid);
    cmd.set_u16(tags::COMMAND_FIELD, 0x0010); // C-GET-RQ
    cmd.set_u16(tags::MESSAGE_ID, msg_id);
    cmd.set_u16(tags::PRIORITY, req.priority);
    cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000); // identifier dataset present

    assoc.send_dimse_command(req.context_id, &cmd).await?;
    assoc.send_dimse_data(req.context_id, &req.query).await?;

    let mut responses = Vec::new();
    let mut instances = Vec::new();

    loop {
        let (ctx_id, rsp_cmd) = assoc.recv_dimse_command().await?;
        let command_field = rsp_cmd.get_u16(tags::COMMAND_FIELD).unwrap_or(0);

        match command_field {
            0x0001 => {
                // C-STORE-RQ sub-operation from the SCP — receive dataset and
                // acknowledge so the SCP can proceed with the next sub-op.
                let sop_class = rsp_cmd
                    .get_string(tags::AFFECTED_SOP_CLASS_UID)
                    .unwrap_or_default()
                    .trim_end_matches('\0')
                    .to_string();
                let sop_instance = rsp_cmd
                    .get_string(tags::AFFECTED_SOP_INSTANCE_UID)
                    .unwrap_or_default()
                    .trim_end_matches('\0')
                    .to_string();
                let store_msg_id = rsp_cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

                let data = assoc.recv_dimse_data().await?;
                instances.push(ReceivedInstance {
                    sop_class_uid: sop_class.clone(),
                    sop_instance_uid: sop_instance.clone(),
                    dataset: data,
                });

                // Send C-STORE-RSP with success status.
                let mut store_rsp = DataSet::new();
                store_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
                store_rsp.set_u16(tags::COMMAND_FIELD, 0x8001); // C-STORE-RSP
                store_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, store_msg_id);
                store_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101); // no dataset
                store_rsp.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, &sop_instance);
                store_rsp.set_u16(tags::STATUS, 0x0000); // success
                assoc.send_dimse_command(ctx_id, &store_rsp).await?;
            }

            0x8010 => {
                // C-GET-RSP.
                let status = rsp_cmd.get_u16(tags::STATUS).unwrap_or(0xFFFF);

                let has_dataset = rsp_cmd
                    .get_u16(tags::COMMAND_DATA_SET_TYPE)
                    .map(|v| v != 0x0101)
                    .unwrap_or(false);

                let dataset = if has_dataset {
                    Some(assoc.recv_dimse_data().await?)
                } else {
                    None
                };

                responses.push(GetResponse {
                    status,
                    remaining: rsp_cmd.get_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS),
                    completed: rsp_cmd.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
                    failed: rsp_cmd.get_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS),
                    warning: rsp_cmd.get_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS),
                    dataset,
                });

                let is_pending = status == 0xFF00 || status == 0xFF01;
                if !is_pending {
                    break;
                }
            }

            _ => {
                // Unknown command — stop processing.
                break;
            }
        }
    }

    Ok(GetResult {
        responses,
        instances,
    })
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
    fn c_get_rq_command_build() {
        let mut cmd = DataSet::new();
        cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.2.1.3");
        cmd.set_u16(tags::COMMAND_FIELD, 0x0010); // C-GET-RQ
        cmd.set_u16(tags::MESSAGE_ID, 1);
        cmd.set_u16(tags::PRIORITY, 0);
        cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000);

        let bytes = dimse::encode_command_dataset(&cmd);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::COMMAND_FIELD), Some(0x0010));
        assert_eq!(decoded.get_u16(tags::PRIORITY), Some(0));
        assert_eq!(decoded.get_u16(tags::COMMAND_DATA_SET_TYPE), Some(0x0000));
    }

    #[test]
    fn c_get_rsp_pending_has_sub_operation_counts() {
        let mut rsp = DataSet::new();
        rsp.set_u16(tags::COMMAND_FIELD, 0x8010); // C-GET-RSP
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, 1);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101); // no dataset
        rsp.set_u16(tags::STATUS, 0xFF00); // pending
        rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, 5);
        rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, 2);
        rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, 0);
        rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);

        let bytes = dimse::encode_command_dataset(&rsp);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::STATUS), Some(0xFF00));
        assert_eq!(
            decoded.get_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS),
            Some(5)
        );
        assert_eq!(
            decoded.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
            Some(2)
        );
    }

    #[test]
    fn c_get_rsp_final_success() {
        let mut rsp = DataSet::new();
        rsp.set_u16(tags::COMMAND_FIELD, 0x8010);
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, 1);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        rsp.set_u16(tags::STATUS, 0x0000);
        rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, 7);

        let bytes = dimse::encode_command_dataset(&rsp);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::STATUS), Some(0x0000));
        assert_eq!(
            decoded.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
            Some(7)
        );
    }
}
