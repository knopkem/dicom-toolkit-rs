//! C-MOVE (Query/Retrieve — Move Service) — PS3.4 §C.4.2.

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::{io::reader::DicomReader, DataSet};
use dicom_toolkit_dict::{tags, Vr};

use crate::association::Association;
use crate::config::AssociationConfig;
use crate::presentation::PresentationContextRq;
use crate::services::provider::{DestinationLookup, MoveEvent, MoveServiceProvider};
use crate::services::store::c_store;

// ── Public types ──────────────────────────────────────────────────────────────

/// Parameters for a C-MOVE-RQ.
#[derive(Debug, Clone)]
pub struct MoveRequest {
    /// Affected SOP Class UID (e.g. Patient Root Query/Retrieve – MOVE).
    pub sop_class_uid: String,
    /// AE Title of the destination SCP that should store the retrieved data.
    pub destination: String,
    /// Pre-encoded query identifier dataset bytes.
    pub query: Vec<u8>,
    /// Presentation context ID negotiated for this SOP class.
    pub context_id: u8,
    /// Priority: 0 = medium, 1 = high, 2 = low.
    pub priority: u16,
}

/// A single C-MOVE-RSP received from the SCP.
#[derive(Debug, Clone)]
pub struct MoveResponse {
    /// DIMSE status code.
    ///
    /// * `0xFF00` — pending (sub-operations in progress).
    /// * `0x0000` — success (all sub-operations completed).
    /// * `0xB000` — warning (one or more sub-operations failed or warned).
    /// * Other — failure.
    pub status: u16,
    /// Number of sub-operations remaining.
    pub remaining: Option<u16>,
    /// Number of sub-operations completed successfully.
    pub completed: Option<u16>,
    /// Number of sub-operations that failed.
    pub failed: Option<u16>,
    /// Number of sub-operations that completed with a warning.
    pub warning: Option<u16>,
}

// ── C-MOVE ────────────────────────────────────────────────────────────────────

/// Execute a C-MOVE operation and collect all responses.
///
/// Sends a C-MOVE-RQ, then collects all pending C-MOVE-RSP messages
/// (status `0xFF00`) plus the final response.  Returns all responses
/// in the order they were received.
pub async fn c_move(assoc: &mut Association, req: MoveRequest) -> DcmResult<Vec<MoveResponse>> {
    let msg_id = next_message_id();

    // Build C-MOVE-RQ command dataset.
    let mut cmd = DataSet::new();
    cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, &req.sop_class_uid);
    cmd.set_u16(tags::COMMAND_FIELD, 0x0021); // C-MOVE-RQ
    cmd.set_u16(tags::MESSAGE_ID, msg_id);
    cmd.set_u16(tags::PRIORITY, req.priority);
    cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000); // identifier dataset present
    cmd.set_string(tags::MOVE_DESTINATION, Vr::AE, &req.destination);

    assoc.send_dimse_command(req.context_id, &cmd).await?;
    assoc.send_dimse_data(req.context_id, &req.query).await?;

    let mut responses = Vec::new();

    loop {
        let (_ctx, rsp_cmd) = assoc.recv_dimse_command().await?;
        let status = rsp_cmd.get_u16(tags::STATUS).unwrap_or(0xFFFF);

        // The final failure response may carry a dataset listing failed instances.
        let has_dataset = rsp_cmd
            .get_u16(tags::COMMAND_DATA_SET_TYPE)
            .map(|v| v != 0x0101)
            .unwrap_or(false);

        if has_dataset {
            // Consume (and discard) the accompanying dataset.
            let _ = assoc.recv_dimse_data().await?;
        }

        responses.push(MoveResponse {
            status,
            remaining: rsp_cmd.get_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS),
            completed: rsp_cmd.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
            failed: rsp_cmd.get_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS),
            warning: rsp_cmd.get_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS),
        });

        // Pending responses: continue collecting.  Anything else: final response.
        let is_pending = status == 0xFF00 || status == 0xFF01;
        if !is_pending {
            break;
        }
    }

    Ok(responses)
}

fn next_message_id() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static ID: AtomicU16 = AtomicU16::new(1);
    ID.fetch_add(1, Ordering::Relaxed)
}

const TS_EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";

// ── SCP handler ───────────────────────────────────────────────────────────────

/// Handle a C-MOVE-RQ received on an SCP association.
///
/// Reads the query identifier and move destination, calls the provider's
/// [`on_move`](MoveServiceProvider::on_move) callback, opens a
/// **sub-association** to the destination, forwards the instances via
/// C-STORE, then sends pending and final C-MOVE-RSP messages back to the
/// requesting SCU.
///
/// `ctx_id` and `cmd` are the values returned by
/// [`Association::recv_dimse_command`].
pub async fn handle_move_rq<P, L>(
    assoc: &mut Association,
    ctx_id: u8,
    cmd: &DataSet,
    provider: &P,
    dest_lookup: &L,
    local_ae: &str,
) -> DcmResult<()>
where
    P: MoveServiceProvider,
    L: DestinationLookup + ?Sized,
{
    let sop_class = cmd
        .get_string(tags::AFFECTED_SOP_CLASS_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);
    let destination = cmd
        .get_string(tags::MOVE_DESTINATION)
        .unwrap_or_default()
        .trim()
        .to_string();

    let query_bytes = assoc.recv_dimse_data().await?;

    let ts = assoc
        .context_by_id(ctx_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());

    let identifier = DicomReader::new(query_bytes.as_slice())
        .read_dataset(&ts)
        .unwrap_or_else(|_| DataSet::new());

    // Resolve destination AE title.
    let dest_addr = match dest_lookup.lookup(&destination) {
        Some(addr) => addr,
        None => {
            let mut rsp = DataSet::new();
            rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
            rsp.set_u16(tags::COMMAND_FIELD, 0x8021); // C-MOVE-RSP
            rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
            rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
            rsp.set_u16(tags::STATUS, 0xA801); // refused: move destination unknown
            return assoc.send_dimse_command(ctx_id, &rsp).await;
        }
    };

    let event = MoveEvent {
        calling_ae: assoc.calling_ae.clone(),
        destination: destination.clone(),
        sop_class_uid: sop_class.clone(),
        identifier,
    };

    let items = provider.on_move(event).await;

    if items.is_empty() {
        // Nothing to move — send final success immediately.
        let mut rsp = DataSet::new();
        rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
        rsp.set_u16(tags::COMMAND_FIELD, 0x8021);
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        rsp.set_u16(tags::STATUS, 0x0000);
        rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, 0);
        rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, 0);
        rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, 0);
        rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
        return assoc.send_dimse_command(ctx_id, &rsp).await;
    }

    // Collect unique SOP class UIDs to build presentation contexts.
    let mut unique_sop_classes: Vec<String> = items
        .iter()
        .map(|i| i.sop_class_uid.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    unique_sop_classes.sort();

    let sub_contexts: Vec<PresentationContextRq> = unique_sop_classes
        .iter()
        .enumerate()
        .map(|(i, sc)| PresentationContextRq {
            id: (i * 2 + 1) as u8,
            abstract_syntax: sc.clone(),
            transfer_syntaxes: vec![TS_EXPLICIT_LE.to_string()],
        })
        .collect();

    let sub_cfg = AssociationConfig {
        local_ae_title: local_ae.to_string(),
        accept_all_transfer_syntaxes: true,
        ..Default::default()
    };

    let mut sub_assoc =
        match Association::request(&dest_addr, &destination, local_ae, &sub_contexts, &sub_cfg)
            .await
        {
            Ok(a) => a,
            Err(_) => {
                // Could not open sub-association — report all as failed.
                let total = items.len() as u16;
                let mut rsp = DataSet::new();
                rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
                rsp.set_u16(tags::COMMAND_FIELD, 0x8021);
                rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
                rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
                rsp.set_u16(tags::STATUS, 0xA801);
                rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, 0);
                rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, 0);
                rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, total);
                rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
                return assoc.send_dimse_command(ctx_id, &rsp).await;
            }
        };

    let total = items.len() as u16;
    let mut completed: u16 = 0;
    let mut failed: u16 = 0;

    for item in &items {
        let remaining = total.saturating_sub(completed + failed + 1);

        if let Some(store_ctx) = sub_assoc.find_context(&item.sop_class_uid) {
            use crate::services::store::StoreRequest;
            let req = StoreRequest {
                sop_class_uid: item.sop_class_uid.clone(),
                sop_instance_uid: item.sop_instance_uid.clone(),
                priority: 0,
                dataset_bytes: item.dataset.clone(),
                context_id: store_ctx.id,
            };
            match c_store(&mut sub_assoc, req).await {
                Ok(rsp) if rsp.status == 0x0000 => completed += 1,
                _ => failed += 1,
            }
        } else {
            failed += 1;
        }

        // Send pending C-MOVE-RSP.
        let mut pending_rsp = DataSet::new();
        pending_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
        pending_rsp.set_u16(tags::COMMAND_FIELD, 0x8021); // C-MOVE-RSP
        pending_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
        pending_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101); // no dataset
        pending_rsp.set_u16(tags::STATUS, 0xFF00); // pending
        pending_rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, remaining);
        pending_rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, completed);
        pending_rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, failed);
        pending_rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
        assoc.send_dimse_command(ctx_id, &pending_rsp).await?;
    }

    let _ = sub_assoc.release().await;

    let final_status: u16 = if failed > 0 { 0xB000 } else { 0x0000 };

    let mut final_rsp = DataSet::new();
    final_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
    final_rsp.set_u16(tags::COMMAND_FIELD, 0x8021); // C-MOVE-RSP
    final_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    final_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101); // no dataset
    final_rsp.set_u16(tags::STATUS, final_status);
    final_rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, 0);
    final_rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, completed);
    final_rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, failed);
    final_rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
    assoc.send_dimse_command(ctx_id, &final_rsp).await
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::dimse;
    use dicom_toolkit_data::DataSet;
    use dicom_toolkit_dict::{tags, Vr};

    #[test]
    fn c_move_rq_command_build() {
        let mut cmd = DataSet::new();
        cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.2.1.2");
        cmd.set_u16(tags::COMMAND_FIELD, 0x0021); // C-MOVE-RQ
        cmd.set_u16(tags::MESSAGE_ID, 3);
        cmd.set_u16(tags::PRIORITY, 0);
        cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000);
        cmd.set_string(tags::MOVE_DESTINATION, Vr::AE, "STORAGESCU");

        let bytes = dimse::encode_command_dataset(&cmd);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::COMMAND_FIELD), Some(0x0021));
        assert_eq!(decoded.get_u16(tags::MESSAGE_ID), Some(3));
        assert_eq!(
            decoded.get_string(tags::MOVE_DESTINATION),
            Some("STORAGESCU")
        );
    }

    #[test]
    fn c_move_rsp_pending_has_counts() {
        let mut rsp = DataSet::new();
        rsp.set_u16(tags::COMMAND_FIELD, 0x8021); // C-MOVE-RSP
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, 3);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101); // no dataset
        rsp.set_u16(tags::STATUS, 0xFF00); // pending
        rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, 10);
        rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, 3);
        rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, 0);
        rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 1);

        let bytes = dimse::encode_command_dataset(&rsp);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::STATUS), Some(0xFF00));
        assert_eq!(
            decoded.get_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS),
            Some(10)
        );
        assert_eq!(
            decoded.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
            Some(3)
        );
        assert_eq!(
            decoded.get_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS),
            Some(1)
        );
    }

    #[test]
    fn c_move_rsp_final_success() {
        let mut rsp = DataSet::new();
        rsp.set_u16(tags::COMMAND_FIELD, 0x8021);
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, 3);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        rsp.set_u16(tags::STATUS, 0x0000);
        rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, 13);

        let bytes = dimse::encode_command_dataset(&rsp);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::STATUS), Some(0x0000));
        assert_eq!(
            decoded.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
            Some(13)
        );
    }
}
