//! C-GET (Query/Retrieve — Retrieve Service to initiating AE) — PS3.4 §C.4.3.

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::{io::reader::DicomReader, io::writer::DicomWriter, DataSet};
use dicom_toolkit_dict::{tags, Tag, Vr};
use futures_util::{stream, StreamExt};
use tracing::{debug, info, warn};

use crate::association::Association;
use crate::services::provider::{
    GetEvent, GetRetrievePlan, GetServiceProvider, RetrieveCompletion, RetrieveSubOperation,
    StreamingGetServiceProvider, StreamingRetrieveItem, StreamingRetrieveItemStream, STATUS_CANCEL,
    STATUS_DATASET_MISMATCH, STATUS_SUCCESS, STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS,
    STATUS_UNABLE_TO_PROCESS, STATUS_WARNING,
};
use crate::services::recv_command_data_bytes;
use crate::services::SubOperationCounts;

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
    /// Transfer Syntax UID negotiated for the C-STORE sub-operation that
    /// delivered this dataset.
    pub transfer_syntax_uid: String,
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
                let transfer_syntax_uid = assoc
                    .context_by_id(ctx_id)
                    .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
                    .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());

                let data = assoc.recv_dimse_data().await?;
                instances.push(ReceivedInstance {
                    sop_class_uid: sop_class.clone(),
                    sop_instance_uid: sop_instance.clone(),
                    transfer_syntax_uid,
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

const TS_EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";

// ── SCP handler ───────────────────────────────────────────────────────────────

/// Handle a C-GET-RQ received on an SCP association.
///
/// Reads the query identifier, calls the provider's
/// [`on_get`](GetServiceProvider::on_get) callback, then sends each
/// retrieved instance to the SCU as a C-STORE sub-operation on the
/// **same** association, interleaved with pending C-GET-RSP status
/// messages.  A final C-GET-RSP is sent when all sub-operations complete.
///
/// `ctx_id` and `cmd` are the values returned by
/// [`Association::recv_dimse_command`].
pub async fn handle_get_rq<P>(
    assoc: &mut Association,
    ctx_id: u8,
    cmd: &DataSet,
    provider: &P,
) -> DcmResult<()>
where
    P: GetServiceProvider,
{
    let sop_class = cmd
        .get_string(tags::AFFECTED_SOP_CLASS_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

    let query_bytes = recv_command_data_bytes(assoc, cmd).await?;

    let ts = assoc
        .context_by_id(ctx_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());

    let identifier = match DicomReader::new(query_bytes.as_slice()).read_dataset(&ts) {
        Ok(identifier) => identifier,
        Err(error) => {
            warn!(%error, "failed to decode C-GET identifier");
            return send_final_response(
                assoc,
                ctx_id,
                &sop_class,
                msg_id,
                STATUS_DATASET_MISMATCH,
                SubOperationCounts::default(),
            )
            .await;
        }
    };

    let event = GetEvent {
        calling_ae: assoc.calling_ae.clone(),
        sop_class_uid: sop_class.clone(),
        identifier,
    };

    let legacy_items = provider.on_get(event).await;
    let total = u16::try_from(legacy_items.len()).map_err(|_| {
        dicom_toolkit_core::error::DcmError::Other(format!(
            "C-GET provider returned {} items; DIMSE counters support at most {}",
            legacy_items.len(),
            u16::MAX
        ))
    })?;
    let streaming_items = legacy_items
        .into_iter()
        .map(|item| {
            let transfer_syntax_uid = assoc
                .find_context(&item.sop_class_uid)
                .map(|context| context.transfer_syntax.clone())
                .unwrap_or_default();
            RetrieveSubOperation::Ready(StreamingRetrieveItem {
                sop_class_uid: item.sop_class_uid,
                sop_instance_uid: item.sop_instance_uid,
                transfer_syntax_uid,
                dataset: item.dataset.into(),
            })
        })
        .collect::<Vec<_>>();
    let items: StreamingRetrieveItemStream =
        Box::pin(stream::iter(streaming_items.into_iter().map(Ok)));
    execute_get_plan(
        assoc,
        ctx_id,
        &sop_class,
        msg_id,
        GetRetrievePlan::new(total, items),
        false,
    )
    .await
}

/// Handle a C-GET-RQ with late-bound presentation contexts and bounded sources.
pub async fn handle_streaming_get_rq<P>(
    assoc: &mut Association,
    ctx_id: u8,
    cmd: &DataSet,
    provider: &P,
) -> DcmResult<()>
where
    P: StreamingGetServiceProvider,
{
    let sop_class = cmd
        .get_string(tags::AFFECTED_SOP_CLASS_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);
    let query_bytes = recv_command_data_bytes(assoc, cmd).await?;
    let transfer_syntax = assoc
        .context_by_id(ctx_id)
        .map(|context| context.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());
    let identifier = match DicomReader::new(query_bytes.as_slice()).read_dataset(&transfer_syntax) {
        Ok(identifier) => identifier,
        Err(error) => {
            warn!(%error, "failed to decode streaming C-GET identifier");
            return send_final_response(
                assoc,
                ctx_id,
                &sop_class,
                msg_id,
                STATUS_DATASET_MISMATCH,
                SubOperationCounts::default(),
            )
            .await;
        }
    };
    let event = GetEvent {
        calling_ae: assoc.calling_ae.clone(),
        sop_class_uid: sop_class.clone(),
        identifier,
    };
    let plan = match provider.on_get_stream(event).await {
        Ok(plan) => plan,
        Err(error) => {
            warn!(%error, "streaming C-GET provider failed before producing a plan");
            return send_final_response(
                assoc,
                ctx_id,
                &sop_class,
                msg_id,
                STATUS_UNABLE_TO_PROCESS,
                SubOperationCounts::default(),
            )
            .await;
        }
    };
    execute_get_plan(assoc, ctx_id, &sop_class, msg_id, plan, true).await
}

async fn execute_get_plan(
    assoc: &mut Association,
    ctx_id: u8,
    sop_class: &str,
    msg_id: u16,
    plan: GetRetrievePlan,
    enforce_negotiated_role: bool,
) -> DcmResult<()> {
    let (total, mut items, completion_callback) = plan.into_parts();
    let mut completion_notifier = RetrieveCompletionNotifier::new(total, completion_callback);
    let mut completed: u16 = 0;
    let mut failed: u16 = 0;
    let mut warning: u16 = 0;
    let mut provider_failed = false;
    let mut cancelled = false;
    let mut failed_sop_instance_uids = Vec::new();

    info!(
        dimse_service = "C-GET",
        calling_ae = %assoc.calling_ae.trim(),
        called_ae = %assoc.called_ae.trim(),
        information_model = %sop_class,
        total_suboperations = total,
        enforce_negotiated_role,
        "C-GET retrieval plan execution started"
    );

    if total > 0 {
        send_pending_response(
            assoc,
            sop_class,
            ctx_id,
            msg_id,
            SubOperationCounts {
                remaining: total,
                completed,
                failed,
                warning,
            },
        )
        .await?;
    }

    loop {
        let result = match next_get_outcome_or_cancel(assoc, &mut items, msg_id).await? {
            NextGetOutcome::Item(Some(result)) => result,
            NextGetOutcome::Item(None) => break,
            NextGetOutcome::Cancelled => {
                cancelled = true;
                break;
            }
        };
        if completed.saturating_add(failed).saturating_add(warning) >= total {
            warn!(total, "C-GET provider yielded more items than declared");
            provider_failed = true;
            break;
        }

        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                warn!(%error, "C-GET retrieval stream failed");
                failed = total.saturating_sub(completed.saturating_add(warning));
                provider_failed = true;
                break;
            }
        };

        let item = match outcome {
            RetrieveSubOperation::Ready(item) => item,
            RetrieveSubOperation::Failed {
                sop_instance_uid,
                reason,
            } => {
                warn!(%sop_instance_uid, %reason, "C-GET instance failed before C-STORE");
                failed = failed.saturating_add(1);
                failed_sop_instance_uids.push(sop_instance_uid);
                let remaining =
                    total.saturating_sub(completed.saturating_add(failed).saturating_add(warning));
                send_pending_response(
                    assoc,
                    sop_class,
                    ctx_id,
                    msg_id,
                    SubOperationCounts {
                        remaining,
                        completed,
                        failed,
                        warning,
                    },
                )
                .await?;
                continue;
            }
        };

        // Find a suitable presentation context for the SOP class.
        let store_ctx = if enforce_negotiated_role {
            assoc.find_context_for_scu_with_transfer_syntax(
                &item.sop_class_uid,
                &item.transfer_syntax_uid,
            )
        } else {
            assoc.find_context_with_transfer_syntax(&item.sop_class_uid, &item.transfer_syntax_uid)
        }
        .map(|context| context.id);

        if let Some(store_ctx_id) = store_ctx {
            let sub_msg_id = next_message_id();

            debug!(
                dimse_service = "C-STORE",
                parent_dimse_service = "C-GET",
                calling_ae = %assoc.calling_ae.trim(),
                called_ae = %assoc.called_ae.trim(),
                presentation_context_id = store_ctx_id,
                sop_class_uid = %item.sop_class_uid,
                sop_instance_uid = %item.sop_instance_uid,
                transfer_syntax_uid = %item.transfer_syntax_uid,
                message_id = sub_msg_id,
                "sending C-STORE sub-operation"
            );

            let mut store_rq = DataSet::new();
            store_rq.set_uid(tags::AFFECTED_SOP_CLASS_UID, &item.sop_class_uid);
            store_rq.set_u16(tags::COMMAND_FIELD, 0x0001); // C-STORE-RQ
            store_rq.set_u16(tags::MESSAGE_ID, sub_msg_id);
            store_rq.set_u16(tags::PRIORITY, 0);
            store_rq.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000); // dataset present
            store_rq.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, &item.sop_instance_uid);

            assoc.send_dimse_command(store_ctx_id, &store_rq).await?;
            let cancel_observed_while_sending = assoc
                .send_dimse_data_source_interruptible(store_ctx_id, &item.dataset, msg_id)
                .await?;
            if cancel_observed_while_sending {
                cancelled = true;
            }

            // Wait for C-STORE-RSP from SCU.
            let store_rsp = loop {
                let (_response_context_id, response) = assoc.recv_dimse_command().await?;
                if is_matching_cancel(&response, msg_id) {
                    cancelled = true;
                    continue;
                }
                let response_message_id = response
                    .get_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO)
                    .unwrap_or_default();
                if response.get_u16(tags::COMMAND_FIELD) == Some(0x8001)
                    && response_message_id == sub_msg_id
                {
                    break response;
                }
                if response.get_u16(tags::COMMAND_FIELD) == Some(0x0FFF) {
                    continue;
                }
                return Err(dicom_toolkit_core::error::DcmError::Other(format!(
                    "unexpected command while waiting for C-STORE-RSP: 0x{:04X}",
                    response.get_u16(tags::COMMAND_FIELD).unwrap_or_default()
                )));
            };
            let store_status = store_rsp.get_u16(tags::STATUS).unwrap_or(0xFFFF);
            let response_message_id = store_rsp
                .get_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO)
                .unwrap_or(0);
            let response_command = store_rsp.get_u16(tags::COMMAND_FIELD).unwrap_or(0);
            if store_status == STATUS_SUCCESS
                && response_message_id == sub_msg_id
                && response_command == 0x8001
            {
                completed += 1;
                debug!(
                    dimse_service = "C-STORE",
                    parent_dimse_service = "C-GET",
                    sop_instance_uid = %item.sop_instance_uid,
                    status = format_args!("0x{store_status:04X}"),
                    "C-STORE sub-operation completed"
                );
            } else if is_warning_status(store_status)
                && response_message_id == sub_msg_id
                && response_command == 0x8001
            {
                warning += 1;
                warn!(
                    dimse_service = "C-STORE",
                    parent_dimse_service = "C-GET",
                    sop_instance_uid = %item.sop_instance_uid,
                    status = format_args!("0x{store_status:04X}"),
                    response_command = format_args!("0x{response_command:04X}"),
                    response_message_id,
                    expected_message_id = sub_msg_id,
                    "C-STORE sub-operation completed with warning"
                );
            } else {
                failed += 1;
                failed_sop_instance_uids.push(item.sop_instance_uid.clone());
                warn!(
                    dimse_service = "C-STORE",
                    parent_dimse_service = "C-GET",
                    sop_class_uid = %item.sop_class_uid,
                    sop_instance_uid = %item.sop_instance_uid,
                    transfer_syntax_uid = %item.transfer_syntax_uid,
                    status = format_args!("0x{store_status:04X}"),
                    response_command = format_args!("0x{response_command:04X}"),
                    response_message_id,
                    expected_message_id = sub_msg_id,
                    "C-STORE sub-operation failed"
                );
            }
        } else {
            let local_scu_role = assoc.local_scu_role(&item.sop_class_uid);
            let accepted_transfer_syntax_uids = assoc
                .presentation_contexts
                .iter()
                .filter(|context| {
                    context.result.is_accepted() && context.abstract_syntax == item.sop_class_uid
                })
                .map(|context| context.transfer_syntax.as_str())
                .collect::<Vec<_>>();
            let requestor_role_selection = assoc
                .role_selections
                .iter()
                .find(|role| role.sop_class_uid == item.sop_class_uid);
            let reason = if !local_scu_role {
                "local Storage SCU role was not negotiated"
            } else if accepted_transfer_syntax_uids.is_empty() {
                "no Storage presentation context was accepted for the SOP Class"
            } else {
                "the source Transfer Syntax was not accepted for the SOP Class"
            };
            warn!(
                dimse_service = "C-GET",
                calling_ae = %assoc.calling_ae.trim(),
                called_ae = %assoc.called_ae.trim(),
                sop_class_uid = %item.sop_class_uid,
                sop_instance_uid = %item.sop_instance_uid,
                source_transfer_syntax_uid = %item.transfer_syntax_uid,
                local_scu_role,
                requestor_scu_role = requestor_role_selection.map(|role| role.scu_role),
                requestor_scp_role = requestor_role_selection.map(|role| role.scp_role),
                ?accepted_transfer_syntax_uids,
                reason,
                "C-GET instance has no compatible negotiated C-STORE presentation context"
            );
            failed += 1;
            failed_sop_instance_uids.push(item.sop_instance_uid.clone());
        }

        if cancelled {
            break;
        }

        let remaining =
            total.saturating_sub(completed.saturating_add(failed).saturating_add(warning));
        send_pending_response(
            assoc,
            sop_class,
            ctx_id,
            msg_id,
            SubOperationCounts {
                remaining,
                completed,
                failed,
                warning,
            },
        )
        .await?;
    }

    let accounted = completed.saturating_add(failed).saturating_add(warning);
    if accounted < total && !cancelled {
        failed = failed.saturating_add(total - accounted);
        provider_failed = true;
        warn!(
            total,
            completed, failed, warning, "C-GET provider stream ended early"
        );
    }

    let final_status = if cancelled {
        STATUS_CANCEL
    } else if provider_failed {
        STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS
    } else if failed > 0 || warning > 0 {
        if completed > 0 || warning > 0 {
            STATUS_WARNING
        } else {
            STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS
        }
    } else {
        STATUS_SUCCESS
    };

    info!(
        dimse_service = "C-GET",
        calling_ae = %assoc.calling_ae.trim(),
        called_ae = %assoc.called_ae.trim(),
        information_model = %sop_class,
        total_suboperations = total,
        completed_suboperations = completed,
        failed_suboperations = failed,
        warning_suboperations = warning,
        cancelled,
        provider_failed,
        final_status = format_args!("0x{final_status:04X}"),
        "C-GET retrieval plan execution completed"
    );

    let response_result = send_final_response_with_failures(
        assoc,
        ctx_id,
        sop_class,
        msg_id,
        final_status,
        SubOperationCounts {
            remaining: if cancelled {
                total.saturating_sub(completed.saturating_add(failed).saturating_add(warning))
            } else {
                0
            },
            completed,
            failed,
            warning,
        },
        &failed_sop_instance_uids,
    )
    .await;
    if response_result.is_ok() {
        completion_notifier.finish(RetrieveCompletion::Finished {
            total,
            completed,
            failed,
            warning,
            cancelled,
            provider_failed,
            final_status,
        });
    }
    response_result
}

struct RetrieveCompletionNotifier {
    total: u16,
    callback: Option<Box<dyn FnOnce(RetrieveCompletion) + Send + 'static>>,
}

impl RetrieveCompletionNotifier {
    fn new(
        total: u16,
        callback: Option<Box<dyn FnOnce(RetrieveCompletion) + Send + 'static>>,
    ) -> Self {
        Self { total, callback }
    }

    fn finish(&mut self, completion: RetrieveCompletion) {
        if let Some(callback) = self.callback.take() {
            callback(completion);
        }
    }
}

impl Drop for RetrieveCompletionNotifier {
    fn drop(&mut self) {
        if let Some(callback) = self.callback.take() {
            callback(RetrieveCompletion::Aborted { total: self.total });
        }
    }
}

enum NextGetOutcome {
    Item(Option<DcmResult<RetrieveSubOperation>>),
    Cancelled,
}

async fn next_get_outcome_or_cancel(
    assoc: &mut Association,
    items: &mut StreamingRetrieveItemStream,
    retrieve_message_id: u16,
) -> DcmResult<NextGetOutcome> {
    loop {
        tokio::select! {
            item = items.next() => return Ok(NextGetOutcome::Item(item)),
            readiness = assoc.wait_for_incoming_data() => {
                readiness?;
                while let Some((context_id, command)) = assoc.try_recv_dimse_command()? {
                    if is_matching_cancel(&command, retrieve_message_id) {
                        return Ok(NextGetOutcome::Cancelled);
                    }
                    match command.get_u16(tags::COMMAND_FIELD) {
                        Some(0x0FFF) | Some(0x8001) => continue,
                        _ => {
                            assoc.queue_dimse_command(context_id, command);
                            return Err(dicom_toolkit_core::error::DcmError::Other(
                                "unexpected DIMSE command during active C-GET".into(),
                            ));
                        }
                    }
                }
            }
        }
    }
}

fn is_matching_cancel(command: &DataSet, retrieve_message_id: u16) -> bool {
    command.get_u16(tags::COMMAND_FIELD) == Some(0x0FFF)
        && command
            .get_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO)
            .is_some_and(|message_id| message_id == retrieve_message_id)
}

async fn send_pending_response(
    assoc: &mut Association,
    sop_class: &str,
    ctx_id: u8,
    msg_id: u16,
    counts: SubOperationCounts,
) -> DcmResult<()> {
    let mut response = DataSet::new();
    response.set_uid(tags::AFFECTED_SOP_CLASS_UID, sop_class);
    response.set_u16(tags::COMMAND_FIELD, 0x8010);
    response.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    response.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
    response.set_u16(tags::STATUS, 0xFF00);
    response.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, counts.remaining);
    response.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, counts.completed);
    response.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, counts.failed);
    response.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, counts.warning);
    assoc.send_dimse_command(ctx_id, &response).await
}

async fn send_final_response(
    assoc: &mut Association,
    ctx_id: u8,
    sop_class: &str,
    msg_id: u16,
    status: u16,
    counts: SubOperationCounts,
) -> DcmResult<()> {
    send_final_response_with_failures(assoc, ctx_id, sop_class, msg_id, status, counts, &[]).await
}

async fn send_final_response_with_failures(
    assoc: &mut Association,
    ctx_id: u8,
    sop_class: &str,
    msg_id: u16,
    status: u16,
    counts: SubOperationCounts,
    failed_sop_instance_uids: &[String],
) -> DcmResult<()> {
    let response_identifier = if failed_sop_instance_uids.is_empty() {
        None
    } else {
        let mut identifier = DataSet::new();
        identifier.set_string(
            Tag::new(0x0008, 0x0058),
            Vr::UI,
            &failed_sop_instance_uids.join("\\"),
        );
        let transfer_syntax = assoc
            .context_by_id(ctx_id)
            .map(|context| context.transfer_syntax.trim_end_matches('\0'))
            .unwrap_or(TS_EXPLICIT_LE);
        let mut encoded = Vec::new();
        DicomWriter::new(&mut encoded).write_dataset(&identifier, transfer_syntax)?;
        Some(encoded)
    };

    let mut final_rsp = DataSet::new();
    final_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, sop_class);
    final_rsp.set_u16(tags::COMMAND_FIELD, 0x8010); // C-GET-RSP
    final_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    final_rsp.set_u16(
        tags::COMMAND_DATA_SET_TYPE,
        if response_identifier.is_some() {
            0x0000
        } else {
            0x0101
        },
    );
    final_rsp.set_u16(tags::STATUS, status);
    if status == STATUS_CANCEL {
        final_rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, counts.remaining);
    }
    final_rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, counts.completed);
    final_rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, counts.failed);
    final_rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, counts.warning);
    assoc.send_dimse_command(ctx_id, &final_rsp).await?;
    if let Some(identifier) = response_identifier {
        assoc.send_dimse_data(ctx_id, &identifier).await?;
    }
    Ok(())
}

fn is_warning_status(status: u16) -> bool {
    status == 0x0001 || status & 0xF000 == 0xB000
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::RetrieveCompletionNotifier;
    use crate::dimse;
    use crate::services::provider::{RetrieveCompletion, STATUS_SUCCESS};
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

    #[test]
    fn completion_notifier_reports_abort_when_execution_exits_early() {
        let observed = Arc::new(Mutex::new(None));
        let callback_observed = Arc::clone(&observed);
        {
            let _notifier = RetrieveCompletionNotifier::new(
                7,
                Some(Box::new(move |completion| {
                    *callback_observed.lock().unwrap() = Some(completion);
                })),
            );
        }

        assert_eq!(
            *observed.lock().unwrap(),
            Some(RetrieveCompletion::Aborted { total: 7 })
        );
    }

    #[test]
    fn completion_notifier_invokes_callback_only_once_after_success() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let callback_observed = Arc::clone(&observed);
        {
            let mut notifier = RetrieveCompletionNotifier::new(
                3,
                Some(Box::new(move |completion| {
                    callback_observed.lock().unwrap().push(completion);
                })),
            );
            notifier.finish(RetrieveCompletion::Finished {
                total: 3,
                completed: 3,
                failed: 0,
                warning: 0,
                cancelled: false,
                provider_failed: false,
                final_status: STATUS_SUCCESS,
            });
        }

        assert_eq!(observed.lock().unwrap().len(), 1);
    }
}
