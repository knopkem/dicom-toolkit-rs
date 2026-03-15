//! C-FIND (Query/Retrieve — Query Service) — PS3.4 §C.

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::{io::reader::DicomReader, io::writer::DicomWriter, DataSet};
use dicom_toolkit_dict::tags;

use crate::association::Association;
use crate::services::provider::{FindEvent, FindServiceProvider};

// ── Public types ──────────────────────────────────────────────────────────────

/// Parameters for a C-FIND-RQ.
#[derive(Debug, Clone)]
pub struct FindRequest {
    /// Affected SOP Class UID (e.g. Patient Root Query/Retrieve).
    pub sop_class_uid: String,
    /// Pre-encoded query identifier dataset bytes.
    pub query: Vec<u8>,
    /// Presentation context ID to use.
    pub context_id: u8,
    /// Priority: 0=medium, 1=high, 2=low.
    pub priority: u16,
}

// ── C-FIND ────────────────────────────────────────────────────────────────────

/// Execute a C-FIND operation and return the result identifier datasets.
///
/// Collects all pending C-FIND-RSP responses (status 0xFF00 / 0xFF01) and
/// returns their encoded data set bytes.  The final success or failure status
/// is silently consumed.
pub async fn c_find(assoc: &mut Association, req: FindRequest) -> DcmResult<Vec<Vec<u8>>> {
    let msg_id = next_message_id();

    // Build C-FIND-RQ command dataset
    let mut cmd = DataSet::new();
    cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, &req.sop_class_uid);
    cmd.set_u16(tags::COMMAND_FIELD, 0x0020); // C-FIND-RQ
    cmd.set_u16(tags::MESSAGE_ID, msg_id);
    cmd.set_u16(tags::PRIORITY, req.priority);
    cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000); // query dataset present

    assoc.send_dimse_command(req.context_id, &cmd).await?;
    assoc.send_dimse_data(req.context_id, &req.query).await?;

    // Collect pending responses
    let mut results: Vec<Vec<u8>> = Vec::new();

    loop {
        let (_ctx, rsp_cmd) = assoc.recv_dimse_command().await?;
        let status = rsp_cmd.get_u16(tags::STATUS).unwrap_or(0xFFFF);

        // CommandDataSetType != 0x0101 means a result dataset follows
        let has_dataset = rsp_cmd
            .get_u16(tags::COMMAND_DATA_SET_TYPE)
            .map(|v| v != 0x0101)
            .unwrap_or(false);

        if has_dataset {
            let data = assoc.recv_dimse_data().await?;
            results.push(data);
        }

        // Pending: 0xFF00 or 0xFF01 — continue collecting
        // Anything else: final response (success 0x0000 or failure) — stop
        let is_pending = status == 0xFF00 || status == 0xFF01;
        if !is_pending {
            break;
        }
    }

    Ok(results)
}

fn next_message_id() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static ID: AtomicU16 = AtomicU16::new(1);
    ID.fetch_add(1, Ordering::Relaxed)
}

// ── Encode helpers ────────────────────────────────────────────────────────────

const TS_EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";

fn encode_dataset(ds: &DataSet) -> Vec<u8> {
    let mut buf = Vec::new();
    DicomWriter::new(&mut buf)
        .write_dataset(ds, TS_EXPLICIT_LE)
        .unwrap_or_default();
    buf
}

// ── SCP handler ───────────────────────────────────────────────────────────────

/// Handle a C-FIND-RQ received on an SCP association.
///
/// Reads the query identifier dataset, calls the provider's
/// [`on_find`](FindServiceProvider::on_find) callback, and streams the
/// results back as pending C-FIND-RSP messages followed by a final
/// success response.
///
/// `ctx_id` and `cmd` are the values returned by
/// [`Association::recv_dimse_command`].
pub async fn handle_find_rq<P>(
    assoc: &mut Association,
    ctx_id: u8,
    cmd: &DataSet,
    provider: &P,
) -> DcmResult<()>
where
    P: FindServiceProvider,
{
    let sop_class = cmd
        .get_string(tags::AFFECTED_SOP_CLASS_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

    let query_bytes = assoc.recv_dimse_data().await?;

    // Decode using the negotiated transfer syntax.
    let ts = assoc
        .context_by_id(ctx_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());

    let identifier = DicomReader::new(query_bytes.as_slice())
        .read_dataset(&ts)
        .unwrap_or_else(|_| DataSet::new());

    let event = FindEvent {
        calling_ae: assoc.calling_ae.clone(),
        sop_class_uid: sop_class.clone(),
        identifier,
    };

    let matches = provider.on_find(event).await;

    // Send one pending RSP per match.
    for result_ds in &matches {
        let result_bytes = encode_dataset(result_ds);

        let mut rsp = DataSet::new();
        rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
        rsp.set_u16(tags::COMMAND_FIELD, 0x8020); // C-FIND-RSP
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000); // dataset present
        rsp.set_u16(tags::STATUS, 0xFF00); // pending

        assoc.send_dimse_command(ctx_id, &rsp).await?;
        assoc.send_dimse_data(ctx_id, &result_bytes).await?;
    }

    // Send final success response (no dataset).
    let mut final_rsp = DataSet::new();
    final_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
    final_rsp.set_u16(tags::COMMAND_FIELD, 0x8020); // C-FIND-RSP
    final_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    final_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101); // no dataset
    final_rsp.set_u16(tags::STATUS, 0x0000); // success

    assoc.send_dimse_command(ctx_id, &final_rsp).await
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use crate::dimse;
    use dicom_toolkit_data::DataSet;
    use dicom_toolkit_dict::tags;

    #[test]
    fn c_find_rq_command_build() {
        let mut cmd = DataSet::new();
        cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.2.1.1");
        cmd.set_u16(tags::COMMAND_FIELD, 0x0020);
        cmd.set_u16(tags::MESSAGE_ID, 1);
        cmd.set_u16(tags::PRIORITY, 0);
        cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000);

        let bytes = dimse::encode_command_dataset(&cmd);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::COMMAND_FIELD), Some(0x0020));
        assert_eq!(decoded.get_u16(tags::PRIORITY), Some(0));
        assert_eq!(decoded.get_u16(tags::COMMAND_DATA_SET_TYPE), Some(0x0000));
    }

    #[test]
    fn c_find_rsp_pending() {
        let mut rsp = DataSet::new();
        rsp.set_u16(tags::COMMAND_FIELD, 0x8020); // C-FIND-RSP
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, 1);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000); // has dataset
        rsp.set_u16(tags::STATUS, 0xFF00); // pending

        let bytes = dimse::encode_command_dataset(&rsp);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::STATUS), Some(0xFF00));
        // has_dataset = CommandDataSetType != 0x0101
        assert!(decoded.get_u16(tags::COMMAND_DATA_SET_TYPE).unwrap() != 0x0101);
    }

    #[test]
    fn c_find_rsp_final_success() {
        let mut rsp = DataSet::new();
        rsp.set_u16(tags::COMMAND_FIELD, 0x8020);
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, 1);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101); // no dataset
        rsp.set_u16(tags::STATUS, 0x0000); // success

        let bytes = dimse::encode_command_dataset(&rsp);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::STATUS), Some(0x0000));
        assert_eq!(decoded.get_u16(tags::COMMAND_DATA_SET_TYPE), Some(0x0101));
    }
}
