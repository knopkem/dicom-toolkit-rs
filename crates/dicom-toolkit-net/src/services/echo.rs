//! C-ECHO (Verification Service Class) — PS3.4 §A.

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_data::DataSet;
use dicom_toolkit_dict::tags;

use crate::association::Association;

// ── C-ECHO ────────────────────────────────────────────────────────────────────

/// Send a C-ECHO-RQ on `context_id` and verify the SCP responds with status 0x0000.
///
/// `context_id` must correspond to the Verification SOP Class
/// (`1.2.840.10008.1.1`) accepted during association negotiation.
pub async fn c_echo(assoc: &mut Association, context_id: u8) -> DcmResult<()> {
    // Build C-ECHO-RQ command dataset
    let mut cmd = DataSet::new();
    cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.1.1");
    cmd.set_u16(tags::COMMAND_FIELD, 0x0030); // C-ECHO-RQ
    cmd.set_u16(tags::MESSAGE_ID, next_message_id());
    cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101); // no dataset

    assoc.send_dimse_command(context_id, &cmd).await?;

    // Receive C-ECHO-RSP
    let (_ctx, rsp) = assoc.recv_dimse_command().await?;
    let status = rsp.get_u16(tags::STATUS).unwrap_or(0xFFFF);

    if status != 0x0000 {
        return Err(DcmError::DimseError {
            status,
            description: format!("C-ECHO failed with status 0x{:04X}", status),
        });
    }
    Ok(())
}

/// Returns a simple monotonic message ID (wraps at u16::MAX).
///
/// In production code a per-association counter would be used; for a
/// single-threaded async context this simple approach is sufficient.
fn next_message_id() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static ID: AtomicU16 = AtomicU16::new(1);
    ID.fetch_add(1, Ordering::Relaxed)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimse;
    use dicom_toolkit_data::DataSet;

    #[test]
    fn echo_rq_command_fields() {
        let mut cmd = DataSet::new();
        cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.1.1");
        cmd.set_u16(tags::COMMAND_FIELD, 0x0030);
        cmd.set_u16(tags::MESSAGE_ID, 1);
        cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);

        let bytes = dimse::encode_command_dataset(&cmd);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::COMMAND_FIELD), Some(0x0030));
        assert_eq!(
            decoded.get_string(tags::AFFECTED_SOP_CLASS_UID),
            Some("1.2.840.10008.1.1")
        );
        assert_eq!(decoded.get_u16(tags::COMMAND_DATA_SET_TYPE), Some(0x0101));
    }

    #[test]
    fn echo_rsp_command_fields() {
        let mut rsp = DataSet::new();
        rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.1.1");
        rsp.set_u16(tags::COMMAND_FIELD, 0x8030); // C-ECHO-RSP
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, 1);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        rsp.set_u16(tags::STATUS, 0x0000);

        let bytes = dimse::encode_command_dataset(&rsp);
        let decoded = dimse::decode_command_dataset(&bytes).unwrap();

        assert_eq!(decoded.get_u16(tags::COMMAND_FIELD), Some(0x8030));
        assert_eq!(decoded.get_u16(tags::STATUS), Some(0x0000));
    }
}
