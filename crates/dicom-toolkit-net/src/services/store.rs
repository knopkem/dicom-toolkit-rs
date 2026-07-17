//! C-STORE (Storage Service Class) — PS3.4 §B.

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::{io::reader::DicomReader, DataSet};
use dicom_toolkit_dict::tags;
use tokio_util::sync::CancellationToken;

use crate::association::Association;
use crate::dataset_source::DatasetSource;
use crate::services::provider::{StoreEvent, StoreServiceProvider};
use crate::services::recv_command_data_bytes;

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

/// Parameters for a bounded-source C-STORE-RQ.
#[derive(Debug, Clone)]
pub struct StoreSourceRequest {
    /// Affected SOP Class UID.
    pub sop_class_uid: String,
    /// Affected SOP Instance UID.
    pub sop_instance_uid: String,
    /// Priority: 0=medium, 1=high, 2=low.
    pub priority: u16,
    /// Pre-encoded dataset source, excluding Part 10 File Meta Information.
    pub dataset: DatasetSource,
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
    let source_request = StoreSourceRequest {
        sop_class_uid: req.sop_class_uid,
        sop_instance_uid: req.sop_instance_uid,
        priority: req.priority,
        dataset: DatasetSource::from(req.dataset_bytes),
        context_id: req.context_id,
    };
    send_store_source(assoc, source_request, false).await
}

/// Send a C-STORE-RQ from a bounded dataset source.
pub async fn c_store_source(
    assoc: &mut Association,
    req: StoreSourceRequest,
) -> DcmResult<StoreResponse> {
    send_store_source(assoc, req, true).await
}

pub(crate) enum CancellableStoreOutcome {
    Response(StoreResponse),
    Aborted,
}

pub(crate) async fn c_store_source_abort_on_cancel(
    assoc: &mut Association,
    req: StoreSourceRequest,
    cancellation_token: &CancellationToken,
) -> DcmResult<CancellableStoreOutcome> {
    let context = assoc.context_by_id(req.context_id).ok_or_else(|| {
        dicom_toolkit_core::error::DcmError::NoPresentationContext {
            sop_class_uid: req.sop_class_uid.clone(),
        }
    })?;
    if context.abstract_syntax != req.sop_class_uid || !assoc.local_scu_role(&req.sop_class_uid) {
        return Err(dicom_toolkit_core::error::DcmError::NoPresentationContext {
            sop_class_uid: req.sop_class_uid.clone(),
        });
    }

    let message_id = next_message_id();
    let mut command = DataSet::new();
    command.set_uid(tags::AFFECTED_SOP_CLASS_UID, &req.sop_class_uid);
    command.set_u16(tags::COMMAND_FIELD, 0x0001);
    command.set_u16(tags::MESSAGE_ID, message_id);
    command.set_u16(tags::PRIORITY, req.priority);
    command.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000);
    command.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, &req.sop_instance_uid);

    assoc.send_dimse_command(req.context_id, &command).await?;
    let dataset_completed = assoc
        .send_dimse_data_source_abort_on_cancel(req.context_id, &req.dataset, cancellation_token)
        .await?;
    if !dataset_completed {
        return Ok(CancellableStoreOutcome::Aborted);
    }

    let (_response_context_id, response) = tokio::select! {
        response = assoc.recv_dimse_command() => response?,
        () = cancellation_token.cancelled() => {
            assoc.abort_strict().await?;
            return Ok(CancellableStoreOutcome::Aborted);
        }
    };
    let command_field = response.get_u16(tags::COMMAND_FIELD).unwrap_or_default();
    let response_message_id = response
        .get_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO)
        .unwrap_or_default();
    if command_field != 0x8001 || response_message_id != message_id {
        return Err(dicom_toolkit_core::error::DcmError::Other(format!(
            "expected C-STORE-RSP for message {message_id}, received command 0x{command_field:04X} for message {response_message_id}"
        )));
    }

    Ok(CancellableStoreOutcome::Response(StoreResponse {
        status: response.get_u16(tags::STATUS).unwrap_or(0xFFFF),
        message_id,
    }))
}

async fn send_store_source(
    assoc: &mut Association,
    req: StoreSourceRequest,
    validate_context_and_response: bool,
) -> DcmResult<StoreResponse> {
    if validate_context_and_response {
        let context = assoc.context_by_id(req.context_id).ok_or_else(|| {
            dicom_toolkit_core::error::DcmError::NoPresentationContext {
                sop_class_uid: req.sop_class_uid.clone(),
            }
        })?;
        if context.abstract_syntax != req.sop_class_uid || !assoc.local_scu_role(&req.sop_class_uid)
        {
            return Err(dicom_toolkit_core::error::DcmError::NoPresentationContext {
                sop_class_uid: req.sop_class_uid.clone(),
            });
        }
    }

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
    assoc
        .send_dimse_data_source(req.context_id, &req.dataset)
        .await?;

    // Receive C-STORE-RSP
    let (_ctx, rsp) = assoc.recv_dimse_command().await?;
    if validate_context_and_response {
        let command_field = rsp.get_u16(tags::COMMAND_FIELD).unwrap_or_default();
        let response_message_id = rsp
            .get_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO)
            .unwrap_or_default();
        if command_field != 0x8001 || response_message_id != msg_id {
            return Err(dicom_toolkit_core::error::DcmError::Other(format!(
                "expected C-STORE-RSP for message {msg_id}, received command 0x{command_field:04X} for message {response_message_id}"
            )));
        }
    }
    let status = rsp.get_u16(tags::STATUS).unwrap_or(0xFFFF);

    Ok(StoreResponse {
        status,
        message_id: msg_id,
    })
}

fn next_message_id() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static ID: AtomicU16 = AtomicU16::new(1);
    ID.fetch_add(1, Ordering::Relaxed)
}

// ── SCP handler ───────────────────────────────────────────────────────────────

/// Handle a C-STORE-RQ received on an SCP association.
///
/// Reads the incoming data set, decodes it, calls the provider's
/// [`on_store`](StoreServiceProvider::on_store) callback, and sends the
/// C-STORE-RSP back to the SCU.
///
/// `ctx_id` and `cmd` are the values returned by
/// [`Association::recv_dimse_command`].
pub async fn handle_store_rq<P>(
    assoc: &mut Association,
    ctx_id: u8,
    cmd: &DataSet,
    provider: &P,
) -> DcmResult<()>
where
    P: StoreServiceProvider,
{
    let sop_class = cmd
        .get_string(tags::AFFECTED_SOP_CLASS_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let sop_instance = cmd
        .get_string(tags::AFFECTED_SOP_INSTANCE_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

    let data = recv_command_data_bytes(assoc, cmd).await?;

    // Decode dataset using the negotiated transfer syntax.
    let ts_uid = assoc
        .context_by_id(ctx_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| "1.2.840.10008.1.2.1".to_string());

    let dataset = DicomReader::new(data.as_slice())
        .read_dataset(&ts_uid)
        .unwrap_or_else(|_| DataSet::new());

    let event = StoreEvent {
        calling_ae: assoc.calling_ae.clone(),
        sop_class_uid: sop_class.clone(),
        sop_instance_uid: sop_instance.clone(),
        dataset,
    };

    let result = provider.on_store(event).await;

    send_store_response(
        assoc,
        ctx_id,
        &sop_class,
        &sop_instance,
        msg_id,
        result.status,
    )
    .await
}

async fn send_store_response(
    assoc: &mut Association,
    ctx_id: u8,
    sop_class: &str,
    sop_instance: &str,
    msg_id: u16,
    status: u16,
) -> DcmResult<()> {
    let mut rsp = DataSet::new();
    rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, sop_class);
    rsp.set_u16(tags::COMMAND_FIELD, 0x8001); // C-STORE-RSP
    rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101); // no dataset
    rsp.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, sop_instance);
    rsp.set_u16(tags::STATUS, status);

    assoc.send_dimse_command(ctx_id, &rsp).await
}

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
