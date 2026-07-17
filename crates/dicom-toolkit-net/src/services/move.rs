//! C-MOVE (Query/Retrieve — Move Service) — PS3.4 §C.4.2.

use std::collections::VecDeque;
use std::future::pending;
use std::time::Duration;

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::{io::reader::DicomReader, io::writer::DicomWriter, DataSet};
use dicom_toolkit_dict::{tags, Tag, Vr};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::association::Association;
use crate::config::AssociationConfig;
use crate::presentation::PresentationContextRq;
use crate::services::provider::{
    DestinationLookup, MoveEvent, MoveRetrievePlan, MoveServiceProvider, RetrieveCompletion,
    RetrieveSubOperation, StreamingMoveServiceProvider, StreamingRetrieveItem, STATUS_CANCEL,
    STATUS_DATASET_MISMATCH, STATUS_SUCCESS, STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS,
    STATUS_UNABLE_TO_PROCESS, STATUS_WARNING,
};
use crate::services::recv_command_data_bytes;
use crate::services::store::{
    c_store, c_store_source_abort_on_cancel, CancellableStoreOutcome, StoreRequest, StoreResponse,
    StoreSourceRequest,
};
use crate::services::SubOperationCounts;

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

/// Handle a C-MOVE-RQ with the compatibility provider API.
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
    let query_bytes = recv_command_data_bytes(assoc, cmd).await?;
    let transfer_syntax = assoc
        .context_by_id(ctx_id)
        .map(|context| context.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());
    let identifier = DicomReader::new(query_bytes.as_slice())
        .read_dataset(&transfer_syntax)
        .unwrap_or_else(|_| DataSet::new());
    let destination_address = match dest_lookup.lookup(&destination) {
        Some(address) => address,
        None => {
            let mut response = DataSet::new();
            response.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
            response.set_u16(tags::COMMAND_FIELD, 0x8021);
            response.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
            response.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
            response.set_u16(tags::STATUS, 0xA801);
            return assoc.send_dimse_command(ctx_id, &response).await;
        }
    };
    let items = provider
        .on_move(MoveEvent {
            calling_ae: assoc.calling_ae.clone(),
            destination: destination.clone(),
            sop_class_uid: sop_class.clone(),
            identifier,
        })
        .await;
    if items.is_empty() {
        return send_final_response(
            assoc,
            ctx_id,
            &sop_class,
            msg_id,
            STATUS_SUCCESS,
            SubOperationCounts::default(),
        )
        .await;
    }

    let mut unique_sop_classes = items
        .iter()
        .map(|item| item.sop_class_uid.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    unique_sop_classes.sort();
    let sub_contexts = unique_sop_classes
        .iter()
        .enumerate()
        .map(|(index, sop_class_uid)| PresentationContextRq {
            id: (index * 2 + 1) as u8,
            abstract_syntax: sop_class_uid.clone(),
            transfer_syntaxes: vec![TS_EXPLICIT_LE.to_string()],
        })
        .collect::<Vec<_>>();
    let sub_config = AssociationConfig {
        local_ae_title: local_ae.to_string(),
        accept_all_transfer_syntaxes: true,
        ..Default::default()
    };
    let mut sub_association = match Association::request(
        &destination_address,
        &destination,
        local_ae,
        &sub_contexts,
        &sub_config,
    )
    .await
    {
        Ok(association) => association,
        Err(_) => {
            return send_final_response(
                assoc,
                ctx_id,
                &sop_class,
                msg_id,
                0xA801,
                SubOperationCounts {
                    remaining: 0,
                    completed: 0,
                    failed: items.len() as u16,
                    warning: 0,
                },
            )
            .await;
        }
    };

    let total = items.len() as u16;
    let mut completed = 0u16;
    let mut failed = 0u16;
    for item in &items {
        let remaining = total.saturating_sub(completed.saturating_add(failed).saturating_add(1));
        if let Some(context_id) = sub_association
            .find_context(&item.sop_class_uid)
            .map(|context| context.id)
        {
            let request = StoreRequest {
                sop_class_uid: item.sop_class_uid.clone(),
                sop_instance_uid: item.sop_instance_uid.clone(),
                priority: 0,
                dataset_bytes: item.dataset.clone(),
                context_id,
            };
            match c_store(&mut sub_association, request).await {
                Ok(response) if response.status == STATUS_SUCCESS => completed += 1,
                _ => failed += 1,
            }
        } else {
            failed += 1;
        }
        send_pending_response(
            assoc,
            &sop_class,
            ctx_id,
            msg_id,
            SubOperationCounts {
                remaining,
                completed,
                failed,
                warning: 0,
            },
        )
        .await?;
    }
    let _ = sub_association.release().await;
    send_final_response(
        assoc,
        ctx_id,
        &sop_class,
        msg_id,
        if failed > 0 {
            STATUS_WARNING
        } else {
            STATUS_SUCCESS
        },
        SubOperationCounts {
            remaining: 0,
            completed,
            failed,
            warning: 0,
        },
    )
    .await
}

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
#[allow(clippy::too_many_arguments)]
pub async fn handle_streaming_move_rq<P, L>(
    assoc: &mut Association,
    ctx_id: u8,
    cmd: &DataSet,
    provider: &P,
    dest_lookup: &L,
    local_ae: &str,
    association_config: &AssociationConfig,
    association_options: &crate::config::AssociationOptions,
) -> DcmResult<()>
where
    P: StreamingMoveServiceProvider,
    L: DestinationLookup + ?Sized,
{
    handle_streaming_move_rq_with_pending_response_interval(
        assoc,
        ctx_id,
        cmd,
        provider,
        dest_lookup,
        local_ae,
        association_config,
        association_options,
        None,
        1,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_streaming_move_rq_with_pending_response_interval<P, L>(
    assoc: &mut Association,
    ctx_id: u8,
    cmd: &DataSet,
    provider: &P,
    dest_lookup: &L,
    local_ae: &str,
    association_config: &AssociationConfig,
    association_options: &crate::config::AssociationOptions,
    pending_response_interval: Option<Duration>,
    maximum_destination_associations: usize,
) -> DcmResult<()>
where
    P: StreamingMoveServiceProvider,
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

    let query_bytes = recv_command_data_bytes(assoc, cmd).await?;

    let ts = assoc
        .context_by_id(ctx_id)
        .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
        .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());

    let identifier = match DicomReader::new(query_bytes.as_slice()).read_dataset(&ts) {
        Ok(identifier) => identifier,
        Err(error) => {
            warn!(%error, "failed to decode C-MOVE identifier");
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

    let plan = match provider.on_move_stream(event).await {
        Ok(plan) => plan,
        Err(error) => {
            warn!(%error, "C-MOVE provider failed before producing a retrieval plan");
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
    if maximum_destination_associations > 1 {
        return execute_parallel_move_plan(
            assoc,
            ctx_id,
            &sop_class,
            msg_id,
            &destination,
            &dest_addr,
            local_ae,
            association_config,
            association_options,
            pending_response_interval,
            maximum_destination_associations,
            plan,
        )
        .await;
    }
    let (total, storage_contexts, mut items, completion_callback) = plan.into_parts();
    let mut completion_notifier = RetrieveCompletionNotifier::new(total, completion_callback);

    if total == 0 {
        let response_result = send_final_response(
            assoc,
            ctx_id,
            &sop_class,
            msg_id,
            STATUS_SUCCESS,
            SubOperationCounts::default(),
        )
        .await;
        if response_result.is_ok() {
            completion_notifier.finish(RetrieveCompletion::Finished {
                total,
                completed: 0,
                failed: 0,
                warning: 0,
                cancelled: false,
                provider_failed: false,
                final_status: STATUS_SUCCESS,
            });
        }
        return response_result;
    }

    info!(
        dimse_service = "C-MOVE",
        calling_ae = %assoc.calling_ae.trim(),
        called_ae = %assoc.called_ae.trim(),
        destination_ae = %destination,
        destination_address = %dest_addr,
        information_model = %sop_class,
        total_suboperations = total,
        proposed_storage_presentation_context_count = storage_contexts.len(),
        "C-MOVE retrieval plan execution started"
    );

    send_pending_response(
        assoc,
        &sop_class,
        ctx_id,
        msg_id,
        SubOperationCounts {
            remaining: total,
            completed: 0,
            failed: 0,
            warning: 0,
        },
    )
    .await?;

    let declared_contexts = storage_contexts
        .iter()
        .map(|context| {
            (
                context.sop_class_uid.clone(),
                context.transfer_syntax_uid.clone(),
            )
        })
        .collect::<std::collections::HashSet<_>>();

    let sub_contexts: Vec<PresentationContextRq> = storage_contexts
        .iter()
        .enumerate()
        .map(|(index, context)| PresentationContextRq {
            id: (index * 2 + 1) as u8,
            abstract_syntax: context.sop_class_uid.clone(),
            transfer_syntaxes: vec![context.transfer_syntax_uid.clone()],
        })
        .collect();

    let mut sub_config = association_config.clone();
    sub_config.local_ae_title = local_ae.to_string();

    let mut sub_assoc = match Association::request_with_options(
        &dest_addr,
        &destination,
        local_ae,
        &sub_contexts,
        &sub_config,
        association_options,
    )
    .await
    {
        Ok(a) => a,
        Err(error) => {
            warn!(%error, destination = %destination, "C-MOVE destination association failed");
            let response_result = send_final_response(
                assoc,
                ctx_id,
                &sop_class,
                msg_id,
                STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS,
                SubOperationCounts {
                    remaining: 0,
                    completed: 0,
                    failed: total,
                    warning: 0,
                },
            )
            .await;
            if response_result.is_ok() {
                completion_notifier.finish(RetrieveCompletion::Finished {
                    total,
                    completed: 0,
                    failed: total,
                    warning: 0,
                    cancelled: false,
                    provider_failed: true,
                    final_status: STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS,
                });
            }
            return response_result;
        }
    };

    info!(
        dimse_service = "C-MOVE",
        destination_ae = %destination,
        destination_address = %dest_addr,
        accepted_storage_presentation_context_count = sub_assoc.presentation_contexts.len(),
        negotiated_role_selection_count = sub_assoc.role_selections.len(),
        "C-MOVE destination association negotiation completed"
    );
    for context in &sub_assoc.presentation_contexts {
        debug!(
            dimse_service = "C-MOVE",
            destination_ae = %destination,
            presentation_context_id = context.id,
            sop_class_uid = %context.abstract_syntax,
            transfer_syntax_uid = %context.transfer_syntax,
            local_scu_role = sub_assoc.local_scu_role(&context.abstract_syntax),
            "C-MOVE destination presentation context accepted"
        );
    }

    let mut completed: u16 = 0;
    let mut failed: u16 = 0;
    let mut warning: u16 = 0;
    let mut provider_failed = false;
    let mut cancelled = false;
    let mut failed_sop_instance_uids = Vec::new();

    loop {
        let counts = sub_operation_counts(total, completed, failed, warning);
        let pending_context = MovePendingResponseContext {
            retrieve_message_id: msg_id,
            sop_class: &sop_class,
            context_id: ctx_id,
            counts,
            interval: pending_response_interval,
            phase: "waiting_for_retrieve_item",
            sop_instance_uid: None,
        };
        let next_outcome = next_move_outcome_or_cancel(assoc, &mut items, pending_context).await;
        let result = match next_outcome {
            Err(error) => {
                log_request_association_interruption(
                    assoc,
                    &destination,
                    &dest_addr,
                    "waiting_for_retrieve_item",
                    None,
                    counts,
                    &error,
                );
                abort_storage_after_request_failure(&mut sub_assoc, &destination, &error).await;
                return Err(error);
            }
            Ok(NextMoveOutcome::Item(Some(result))) => result,
            Ok(NextMoveOutcome::Item(None)) => break,
            Ok(NextMoveOutcome::Cancelled) => {
                cancelled = true;
                break;
            }
        };
        if completed.saturating_add(failed).saturating_add(warning) >= total {
            warn!(total, "C-MOVE provider yielded more items than declared");
            provider_failed = true;
            break;
        }

        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                warn!(%error, "C-MOVE retrieval stream failed");
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
                warn!(%sop_instance_uid, %reason, "C-MOVE instance failed before C-STORE");
                failed = failed.saturating_add(1);
                failed_sop_instance_uids.push(sop_instance_uid);
                let remaining =
                    total.saturating_sub(completed.saturating_add(failed).saturating_add(warning));
                send_pending_response(
                    assoc,
                    &sop_class,
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
                log_move_progress(total, completed, failed, warning, &destination, &dest_addr);
                continue;
            }
        };

        let context_key = (item.sop_class_uid.clone(), item.transfer_syntax_uid.clone());

        if let Some(store_context_id) = declared_contexts
            .contains(&context_key)
            .then(|| {
                sub_assoc
                    .find_context_for_scu_with_transfer_syntax(
                        &item.sop_class_uid,
                        &item.transfer_syntax_uid,
                    )
                    .map(|context| context.id)
            })
            .flatten()
        {
            let failed_sop_instance_uid = item.sop_instance_uid.clone();
            debug!(
                dimse_service = "C-STORE",
                parent_dimse_service = "C-MOVE",
                destination_ae = %destination,
                presentation_context_id = store_context_id,
                sop_class_uid = %item.sop_class_uid,
                sop_instance_uid = %item.sop_instance_uid,
                transfer_syntax_uid = %item.transfer_syntax_uid,
                "sending C-STORE sub-operation"
            );
            let req = StoreSourceRequest {
                sop_class_uid: item.sop_class_uid,
                sop_instance_uid: item.sop_instance_uid,
                priority: 0,
                dataset: item.dataset,
                context_id: store_context_id,
            };
            let store_outcome = move_store_or_cancel(
                assoc,
                &mut sub_assoc,
                req,
                MovePendingResponseContext {
                    phase: "waiting_for_c_store_response",
                    sop_instance_uid: Some(&failed_sop_instance_uid),
                    ..pending_context
                },
            )
            .await;
            let store_outcome = match store_outcome {
                Ok(outcome) => outcome,
                Err(error) => {
                    log_request_association_interruption(
                        assoc,
                        &destination,
                        &dest_addr,
                        "waiting_for_c_store_response",
                        Some(&failed_sop_instance_uid),
                        counts,
                        &error,
                    );
                    abort_storage_after_request_failure(&mut sub_assoc, &destination, &error).await;
                    return Err(error);
                }
            };
            match store_outcome {
                MoveStoreOutcome::Response(Ok(response)) if response.status == STATUS_SUCCESS => {
                    completed += 1;
                    debug!(
                        dimse_service = "C-STORE",
                        parent_dimse_service = "C-MOVE",
                        sop_instance_uid = %failed_sop_instance_uid,
                        status = format_args!("0x{:04X}", response.status),
                        "C-STORE sub-operation completed"
                    );
                }
                MoveStoreOutcome::Response(Ok(response)) if is_warning_status(response.status) => {
                    warning += 1;
                    warn!(
                        dimse_service = "C-STORE",
                        parent_dimse_service = "C-MOVE",
                        sop_instance_uid = %failed_sop_instance_uid,
                        status = format_args!("0x{:04X}", response.status),
                        "C-STORE sub-operation completed with warning"
                    );
                }
                MoveStoreOutcome::Response(response) => {
                    match &response {
                        Ok(response) => warn!(
                            dimse_service = "C-STORE",
                            parent_dimse_service = "C-MOVE",
                            sop_instance_uid = %failed_sop_instance_uid,
                            status = format_args!("0x{:04X}", response.status),
                            "C-STORE sub-operation failed"
                        ),
                        Err(error) => warn!(
                            dimse_service = "C-STORE",
                            parent_dimse_service = "C-MOVE",
                            sop_instance_uid = %failed_sop_instance_uid,
                            %error,
                            "C-STORE sub-operation failed"
                        ),
                    }
                    failed += 1;
                    failed_sop_instance_uids.push(failed_sop_instance_uid);
                }
                MoveStoreOutcome::Cancelled(response) => {
                    match response {
                        Some(Ok(response)) if response.status == STATUS_SUCCESS => completed += 1,
                        Some(Ok(response)) if is_warning_status(response.status) => warning += 1,
                        Some(Err(error)) => {
                            warn!(%error, "cancelled C-MOVE storage sub-operation failed");
                            failed += 1;
                            failed_sop_instance_uids.push(failed_sop_instance_uid);
                        }
                        Some(Ok(_)) | None => {
                            failed += 1;
                            failed_sop_instance_uids.push(failed_sop_instance_uid);
                        }
                    }
                    cancelled = true;
                    break;
                }
            }
        } else {
            let was_declared = declared_contexts.contains(&context_key);
            let local_scu_role = sub_assoc.local_scu_role(&item.sop_class_uid);
            let accepted_transfer_syntax_uids = sub_assoc
                .presentation_contexts
                .iter()
                .filter(|context| {
                    context.result.is_accepted() && context.abstract_syntax == item.sop_class_uid
                })
                .map(|context| context.transfer_syntax.as_str())
                .collect::<Vec<_>>();
            let reason = if !was_declared {
                "the retrieve plan did not declare the source SOP Class and Transfer Syntax pair"
            } else if !local_scu_role {
                "local Storage SCU role was not negotiated"
            } else if accepted_transfer_syntax_uids.is_empty() {
                "the destination accepted no Storage presentation context for the SOP Class"
            } else {
                "the destination did not accept the source Transfer Syntax for the SOP Class"
            };
            warn!(
                dimse_service = "C-MOVE",
                destination_ae = %destination,
                destination_address = %dest_addr,
                sop_class_uid = %item.sop_class_uid,
                sop_instance_uid = %item.sop_instance_uid,
                source_transfer_syntax_uid = %item.transfer_syntax_uid,
                was_declared,
                local_scu_role,
                ?accepted_transfer_syntax_uids,
                reason,
                "C-MOVE instance has no compatible negotiated C-STORE presentation context"
            );
            failed += 1;
            failed_sop_instance_uids.push(item.sop_instance_uid);
        }

        let remaining =
            total.saturating_sub(completed.saturating_add(failed).saturating_add(warning));
        send_pending_response(
            assoc,
            &sop_class,
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
        log_move_progress(total, completed, failed, warning, &destination, &dest_addr);
    }

    if cancelled {
        if let Err(error) = sub_assoc.abort_strict().await {
            warn!(%error, "failed to send A-ABORT on cancelled C-MOVE storage association");
        }
    } else {
        if let Err(error) = sub_assoc.release_strict().await {
            warn!(
                dimse_service = "C-MOVE",
                destination_ae = %destination,
                destination_address = %dest_addr,
                %error,
                "C-MOVE destination association release failed"
            );
        }
    }

    let accounted = completed.saturating_add(failed).saturating_add(warning);
    if accounted < total && !cancelled {
        failed = failed.saturating_add(total - accounted);
        provider_failed = true;
        warn!(
            total,
            completed, failed, warning, "C-MOVE provider stream ended early"
        );
    }

    let final_status = if cancelled {
        STATUS_CANCEL
    } else if provider_failed || (failed > 0 && completed == 0 && warning == 0) {
        STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS
    } else if failed > 0 || warning > 0 {
        STATUS_WARNING
    } else {
        STATUS_SUCCESS
    };

    info!(
        dimse_service = "C-MOVE",
        calling_ae = %assoc.calling_ae.trim(),
        called_ae = %assoc.called_ae.trim(),
        destination_ae = %destination,
        destination_address = %dest_addr,
        information_model = %sop_class,
        total_suboperations = total,
        completed_suboperations = completed,
        failed_suboperations = failed,
        warning_suboperations = warning,
        cancelled,
        provider_failed,
        final_status = format_args!("0x{final_status:04X}"),
        "C-MOVE retrieval plan execution completed"
    );

    let response_result = send_final_response_with_failures(
        assoc,
        ctx_id,
        &sop_class,
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

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn execute_parallel_move_plan(
    request_association: &mut Association,
    context_id: u8,
    sop_class: &str,
    message_id: u16,
    destination: &str,
    destination_address: &str,
    local_ae: &str,
    association_config: &AssociationConfig,
    association_options: &crate::config::AssociationOptions,
    pending_response_interval: Option<Duration>,
    maximum_destination_associations: usize,
    plan: MoveRetrievePlan,
) -> DcmResult<()> {
    let (total, storage_contexts, mut items, completion_callback) = plan.into_parts();
    let mut completion_notifier = RetrieveCompletionNotifier::new(total, completion_callback);
    if total == 0 {
        let response_result = send_final_response(
            request_association,
            context_id,
            sop_class,
            message_id,
            STATUS_SUCCESS,
            SubOperationCounts::default(),
        )
        .await;
        if response_result.is_ok() {
            completion_notifier.finish(RetrieveCompletion::Finished {
                total,
                completed: 0,
                failed: 0,
                warning: 0,
                cancelled: false,
                provider_failed: false,
                final_status: STATUS_SUCCESS,
            });
        }
        return response_result;
    }

    send_pending_response(
        request_association,
        sop_class,
        context_id,
        message_id,
        SubOperationCounts {
            remaining: total,
            completed: 0,
            failed: 0,
            warning: 0,
        },
    )
    .await?;

    let sub_contexts: Vec<PresentationContextRq> = storage_contexts
        .iter()
        .enumerate()
        .map(|(index, context)| PresentationContextRq {
            id: (index * 2 + 1) as u8,
            abstract_syntax: context.sop_class_uid.clone(),
            transfer_syntaxes: vec![context.transfer_syntax_uid.clone()],
        })
        .collect();
    let mut sub_config = association_config.clone();
    sub_config.local_ae_title = local_ae.to_string();
    let cancellation = CancellationToken::new();
    let (result_sender, mut result_receiver) = mpsc::unbounded_channel();
    let mut worker_tasks = JoinSet::new();
    let mut worker_senders = Vec::new();
    let requested_worker_count = maximum_destination_associations.min(usize::from(total));

    for worker_index in 0..requested_worker_count {
        match Association::request_with_options(
            destination_address,
            destination,
            local_ae,
            &sub_contexts,
            &sub_config,
            association_options,
        )
        .await
        {
            Ok(association) => {
                let (task_sender, task_receiver) = mpsc::channel(1);
                worker_senders.push(Some(task_sender));
                worker_tasks.spawn(run_storage_worker(
                    worker_index,
                    association,
                    task_receiver,
                    result_sender.clone(),
                    cancellation.clone(),
                    destination.to_owned(),
                    destination_address.to_owned(),
                ));
            }
            Err(error) if worker_senders.is_empty() => {
                warn!(
                    %error,
                    destination,
                    destination_address,
                    "C-MOVE destination association failed"
                );
                let response_result = send_final_response(
                    request_association,
                    context_id,
                    sop_class,
                    message_id,
                    STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS,
                    SubOperationCounts {
                        remaining: 0,
                        completed: 0,
                        failed: total,
                        warning: 0,
                    },
                )
                .await;
                if response_result.is_ok() {
                    completion_notifier.finish(RetrieveCompletion::Finished {
                        total,
                        completed: 0,
                        failed: total,
                        warning: 0,
                        cancelled: false,
                        provider_failed: true,
                        final_status: STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS,
                    });
                }
                return response_result;
            }
            Err(error) => {
                warn!(
                    %error,
                    worker_index,
                    destination,
                    destination_address,
                    "additional C-MOVE destination association failed; using smaller pool"
                );
                break;
            }
        }
    }
    drop(result_sender);

    info!(
        dimse_service = "C-MOVE",
        calling_ae = %request_association.calling_ae.trim(),
        called_ae = %request_association.called_ae.trim(),
        destination_ae = destination,
        destination_address,
        information_model = sop_class,
        total_suboperations = total,
        requested_storage_association_count = requested_worker_count,
        active_storage_association_count = worker_senders.len(),
        proposed_storage_presentation_context_count = storage_contexts.len(),
        "parallel C-MOVE retrieval plan execution started"
    );

    let mut idle_workers = (0..worker_senders.len()).collect::<VecDeque<_>>();
    let mut active_suboperations = 0_usize;
    let mut stream_ended = false;
    let mut completed = 0_u16;
    let mut failed = 0_u16;
    let mut warning = 0_u16;
    let mut cancelled = false;
    let mut provider_failed = false;
    let mut failed_sop_instance_uids = Vec::new();
    let mut request_error = None;

    while active_suboperations != 0 || !stream_ended {
        if idle_workers.is_empty() && active_suboperations == 0 && !stream_ended {
            provider_failed = true;
            break;
        }

        let counts = sub_operation_counts(total, completed, failed, warning);
        tokio::select! {
            item = items.next(), if !stream_ended && !idle_workers.is_empty() => {
                match item {
                    Some(Ok(RetrieveSubOperation::Ready(item))) => {
                        let worker_index = idle_workers.pop_front().expect("idle worker available");
                        let task = StorageWorkerTask { item };
                        let send_result = match worker_senders
                            .get(worker_index)
                            .and_then(Option::as_ref)
                        {
                            Some(sender) => sender.send(task).await,
                            None => Err(mpsc::error::SendError(task)),
                        };
                        if let Err(error) = send_result {
                            failed = failed.saturating_add(1);
                            failed_sop_instance_uids.push(error.0.item.sop_instance_uid);
                            if let Some(sender) = worker_senders.get_mut(worker_index) {
                                *sender = None;
                            }
                        } else {
                            active_suboperations += 1;
                        }
                    }
                    Some(Ok(RetrieveSubOperation::Failed { sop_instance_uid, reason })) => {
                        warn!(%sop_instance_uid, %reason, "C-MOVE instance failed before C-STORE");
                        failed = failed.saturating_add(1);
                        failed_sop_instance_uids.push(sop_instance_uid);
                    }
                    Some(Err(error)) => {
                        warn!(%error, "parallel C-MOVE retrieval stream failed");
                        provider_failed = true;
                        stream_ended = true;
                    }
                    None => stream_ended = true,
                }
                if failed > counts.failed {
                    if let Err(error) = send_pending_response(
                        request_association,
                        sop_class,
                        context_id,
                        message_id,
                        sub_operation_counts(total, completed, failed, warning),
                    ).await {
                        request_error = Some(error);
                        break;
                    }
                }
            }
            result = result_receiver.recv(), if active_suboperations != 0 => {
                let Some(result) = result else {
                    provider_failed = true;
                    break;
                };
                active_suboperations = active_suboperations.saturating_sub(1);
                if result.worker_reusable {
                    idle_workers.push_back(result.worker_index);
                } else if let Some(sender) = worker_senders.get_mut(result.worker_index) {
                    *sender = None;
                }
                match result.outcome {
                    StorageWorkerOutcome::Success => completed = completed.saturating_add(1),
                    StorageWorkerOutcome::Warning => warning = warning.saturating_add(1),
                    StorageWorkerOutcome::Failed(reason) => {
                        warn!(
                            sop_instance_uid = %result.sop_instance_uid,
                            %reason,
                            "parallel C-MOVE C-STORE sub-operation failed"
                        );
                        failed = failed.saturating_add(1);
                        failed_sop_instance_uids.push(result.sop_instance_uid);
                    }
                    StorageWorkerOutcome::Cancelled => {
                        if !cancelled {
                            failed = failed.saturating_add(1);
                            failed_sop_instance_uids.push(result.sop_instance_uid);
                        }
                    }
                }
                if let Err(error) = send_pending_response(
                    request_association,
                    sop_class,
                    context_id,
                    message_id,
                    sub_operation_counts(total, completed, failed, warning),
                ).await {
                    request_error = Some(error);
                    break;
                }
            }
            readiness = request_association.wait_for_incoming_data() => {
                match readiness {
                    Err(error) => {
                        request_error = Some(error);
                        break;
                    }
                    Ok(()) => {
                        loop {
                            match request_association.try_recv_dimse_command() {
                                Err(error) => {
                                    request_error = Some(error);
                                    break;
                                }
                                Ok(Some((_incoming_context_id, command)))
                                    if is_matching_cancel(&command, message_id) =>
                                {
                                    cancelled = true;
                                    stream_ended = true;
                                    cancellation.cancel();
                                }
                                Ok(Some((_incoming_context_id, command)))
                                    if command.get_u16(tags::COMMAND_FIELD) == Some(0x0FFF) => {}
                                Ok(Some((incoming_context_id, command))) => {
                                    request_association.queue_dimse_command(incoming_context_id, command);
                                    request_error = Some(dicom_toolkit_core::error::DcmError::Other(
                                        "unexpected DIMSE command during active parallel C-MOVE".into(),
                                    ));
                                    break;
                                }
                                Ok(None) => break,
                            }
                        }
                        if request_error.is_some() || cancelled {
                            break;
                        }
                    }
                }
            }
            () = wait_for_pending_heartbeat(pending_response_interval) => {
                if let Err(error) = send_pending_response(
                    request_association,
                    sop_class,
                    context_id,
                    message_id,
                    counts,
                ).await {
                    request_error = Some(error);
                    break;
                }
                log_pending_heartbeat(
                    request_association,
                    "parallel_c_store",
                    None,
                    counts,
                );
            }
        }
    }

    if cancelled || request_error.is_some() {
        cancellation.cancel();
    }
    drop(worker_senders);
    while let Some(result) = worker_tasks.join_next().await {
        if let Err(error) = result {
            warn!(%error, "parallel C-MOVE Storage worker failed");
        }
    }

    if let Some(error) = request_error {
        return Err(error);
    }

    let accounted = completed.saturating_add(failed).saturating_add(warning);
    if accounted < total && !cancelled {
        failed = failed.saturating_add(total - accounted);
        provider_failed = true;
    }
    let final_status = if cancelled {
        STATUS_CANCEL
    } else if provider_failed || (failed > 0 && completed == 0 && warning == 0) {
        STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS
    } else if failed > 0 || warning > 0 {
        STATUS_WARNING
    } else {
        STATUS_SUCCESS
    };

    info!(
        dimse_service = "C-MOVE",
        calling_ae = %request_association.calling_ae.trim(),
        called_ae = %request_association.called_ae.trim(),
        destination_ae = destination,
        destination_address,
        information_model = sop_class,
        total_suboperations = total,
        completed_suboperations = completed,
        failed_suboperations = failed,
        warning_suboperations = warning,
        cancelled,
        provider_failed,
        final_status = format_args!("0x{final_status:04X}"),
        "parallel C-MOVE retrieval plan execution completed"
    );

    let response_result = send_final_response_with_failures(
        request_association,
        context_id,
        sop_class,
        message_id,
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

struct StorageWorkerTask {
    item: StreamingRetrieveItem,
}

struct StorageWorkerResult {
    worker_index: usize,
    sop_instance_uid: String,
    outcome: StorageWorkerOutcome,
    worker_reusable: bool,
}

enum StorageWorkerOutcome {
    Success,
    Warning,
    Failed(String),
    Cancelled,
}

async fn run_storage_worker(
    worker_index: usize,
    mut association: Association,
    mut task_receiver: mpsc::Receiver<StorageWorkerTask>,
    result_sender: mpsc::UnboundedSender<StorageWorkerResult>,
    cancellation: CancellationToken,
    destination: String,
    destination_address: String,
) {
    info!(
        worker_index,
        destination_ae = %destination,
        destination_address = %destination_address,
        accepted_storage_presentation_context_count = association.presentation_contexts.len(),
        "parallel C-MOVE Storage association ready"
    );
    let mut aborted = false;
    loop {
        let task = tokio::select! {
            () = cancellation.cancelled() => {
                aborted = true;
                break;
            }
            task = task_receiver.recv() => {
                let Some(task) = task else { break; };
                task
            }
        };
        let sop_instance_uid = task.item.sop_instance_uid.clone();
        let context_id = association
            .find_context_for_scu_with_transfer_syntax(
                &task.item.sop_class_uid,
                &task.item.transfer_syntax_uid,
            )
            .map(|context| context.id);
        let Some(context_id) = context_id else {
            let _ = result_sender.send(StorageWorkerResult {
                worker_index,
                sop_instance_uid,
                outcome: StorageWorkerOutcome::Failed(
                    "destination accepted no compatible presentation context".to_owned(),
                ),
                worker_reusable: true,
            });
            continue;
        };
        let request = StoreSourceRequest {
            sop_class_uid: task.item.sop_class_uid,
            sop_instance_uid: task.item.sop_instance_uid,
            priority: 0,
            dataset: task.item.dataset,
            context_id,
        };
        let (outcome, worker_reusable) =
            match c_store_source_abort_on_cancel(&mut association, request, &cancellation).await {
                Ok(CancellableStoreOutcome::Response(response))
                    if response.status == STATUS_SUCCESS =>
                {
                    (StorageWorkerOutcome::Success, true)
                }
                Ok(CancellableStoreOutcome::Response(response))
                    if is_warning_status(response.status) =>
                {
                    (StorageWorkerOutcome::Warning, true)
                }
                Ok(CancellableStoreOutcome::Response(response)) => (
                    StorageWorkerOutcome::Failed(format!(
                        "destination returned C-STORE status 0x{:04X}",
                        response.status
                    )),
                    true,
                ),
                Ok(CancellableStoreOutcome::Aborted) => {
                    aborted = true;
                    (StorageWorkerOutcome::Cancelled, false)
                }
                Err(error) => {
                    aborted = true;
                    (StorageWorkerOutcome::Failed(error.to_string()), false)
                }
            };
        let _ = result_sender.send(StorageWorkerResult {
            worker_index,
            sop_instance_uid,
            outcome,
            worker_reusable,
        });
        if !worker_reusable {
            break;
        }
    }

    if aborted || cancellation.is_cancelled() {
        let _ = association.abort_strict().await;
    } else if let Err(error) = association.release_strict().await {
        warn!(
            worker_index,
            destination_ae = %destination,
            destination_address = %destination_address,
            %error,
            "parallel C-MOVE Storage association release failed"
        );
    }
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

enum NextMoveOutcome {
    Item(Option<DcmResult<RetrieveSubOperation>>),
    Cancelled,
}

#[derive(Clone, Copy)]
struct MovePendingResponseContext<'a> {
    retrieve_message_id: u16,
    sop_class: &'a str,
    context_id: u8,
    counts: SubOperationCounts,
    interval: Option<Duration>,
    phase: &'static str,
    sop_instance_uid: Option<&'a str>,
}

async fn next_move_outcome_or_cancel(
    assoc: &mut Association,
    items: &mut crate::services::provider::StreamingRetrieveItemStream,
    pending_context: MovePendingResponseContext<'_>,
) -> DcmResult<NextMoveOutcome> {
    loop {
        tokio::select! {
            item = items.next() => return Ok(NextMoveOutcome::Item(item)),
            readiness = assoc.wait_for_incoming_data() => {
                readiness?;
                while let Some((context_id, command)) = assoc.try_recv_dimse_command()? {
                    if is_matching_cancel(&command, pending_context.retrieve_message_id) {
                        return Ok(NextMoveOutcome::Cancelled);
                    }
                    match command.get_u16(tags::COMMAND_FIELD) {
                        Some(0x0FFF) => continue,
                        _ => {
                            assoc.queue_dimse_command(context_id, command);
                            return Err(dicom_toolkit_core::error::DcmError::Other(
                                "unexpected DIMSE command during active C-MOVE".into(),
                            ));
                        }
                    }
                }
            }
            () = wait_for_pending_heartbeat(pending_context.interval) => {
                send_pending_response(
                    assoc,
                    pending_context.sop_class,
                    pending_context.context_id,
                    pending_context.retrieve_message_id,
                    pending_context.counts,
                ).await?;
                log_pending_heartbeat(
                    assoc,
                    pending_context.phase,
                    pending_context.sop_instance_uid,
                    pending_context.counts,
                );
            }
        }
    }
}

enum MoveStoreOutcome {
    Response(DcmResult<StoreResponse>),
    Cancelled(Option<DcmResult<StoreResponse>>),
}

async fn move_store_or_cancel(
    assoc: &mut Association,
    sub_association: &mut Association,
    request: StoreSourceRequest,
    pending_context: MovePendingResponseContext<'_>,
) -> DcmResult<MoveStoreOutcome> {
    let cancellation_token = tokio_util::sync::CancellationToken::new();
    let store = c_store_source_abort_on_cancel(sub_association, request, &cancellation_token);
    tokio::pin!(store);
    loop {
        tokio::select! {
            response = &mut store => {
                return Ok(match response {
                    Ok(CancellableStoreOutcome::Response(response)) => {
                        MoveStoreOutcome::Response(Ok(response))
                    }
                    Ok(CancellableStoreOutcome::Aborted) => MoveStoreOutcome::Cancelled(None),
                    Err(error) => MoveStoreOutcome::Response(Err(error)),
                });
            }
            readiness = assoc.wait_for_incoming_data() => {
                readiness?;
                while let Some((context_id, command)) = assoc.try_recv_dimse_command()? {
                    if is_matching_cancel(&command, pending_context.retrieve_message_id) {
                        cancellation_token.cancel();
                        let result = store.await;
                        return Ok(match result {
                            Ok(CancellableStoreOutcome::Response(response)) => {
                                MoveStoreOutcome::Cancelled(Some(Ok(response)))
                            }
                            Ok(CancellableStoreOutcome::Aborted) => {
                                MoveStoreOutcome::Cancelled(None)
                            }
                            Err(error) => MoveStoreOutcome::Cancelled(Some(Err(error))),
                        });
                    }
                    match command.get_u16(tags::COMMAND_FIELD) {
                        Some(0x0FFF) => continue,
                        _ => {
                            assoc.queue_dimse_command(context_id, command);
                            return Err(dicom_toolkit_core::error::DcmError::Other(
                                "unexpected DIMSE command during active C-MOVE sub-operation".into(),
                            ));
                        }
                    }
                }
            }
            () = wait_for_pending_heartbeat(pending_context.interval) => {
                send_pending_response(
                    assoc,
                    pending_context.sop_class,
                    pending_context.context_id,
                    pending_context.retrieve_message_id,
                    pending_context.counts,
                ).await?;
                log_pending_heartbeat(
                    assoc,
                    pending_context.phase,
                    pending_context.sop_instance_uid,
                    pending_context.counts,
                );
            }
        }
    }
}

async fn wait_for_pending_heartbeat(interval: Option<Duration>) {
    match interval {
        Some(interval) => tokio::time::sleep(interval).await,
        None => pending::<()>().await,
    }
}

fn sub_operation_counts(
    total: u16,
    completed: u16,
    failed: u16,
    warning: u16,
) -> SubOperationCounts {
    SubOperationCounts {
        remaining: total.saturating_sub(completed.saturating_add(failed).saturating_add(warning)),
        completed,
        failed,
        warning,
    }
}

fn log_pending_heartbeat(
    association: &Association,
    phase: &'static str,
    sop_instance_uid: Option<&str>,
    counts: SubOperationCounts,
) {
    info!(
        dimse_service = "C-MOVE",
        calling_ae = %association.calling_ae.trim(),
        called_ae = %association.called_ae.trim(),
        phase,
        sop_instance_uid,
        remaining_suboperations = counts.remaining,
        completed_suboperations = counts.completed,
        failed_suboperations = counts.failed,
        warning_suboperations = counts.warning,
        "C-MOVE Pending heartbeat sent"
    );
}

fn log_move_progress(
    total: u16,
    completed: u16,
    failed: u16,
    warning: u16,
    destination: &str,
    destination_address: &str,
) {
    let accounted = completed.saturating_add(failed).saturating_add(warning);
    if accounted == 0 || accounted == total || accounted % 100 != 0 {
        return;
    }
    info!(
        dimse_service = "C-MOVE",
        destination_ae = %destination,
        destination_address,
        total_suboperations = total,
        remaining_suboperations = total.saturating_sub(accounted),
        completed_suboperations = completed,
        failed_suboperations = failed,
        warning_suboperations = warning,
        "C-MOVE retrieval progress"
    );
}

fn log_request_association_interruption(
    association: &Association,
    destination: &str,
    destination_address: &str,
    phase: &'static str,
    sop_instance_uid: Option<&str>,
    counts: SubOperationCounts,
    error: &dicom_toolkit_core::error::DcmError,
) {
    warn!(
        dimse_service = "C-MOVE",
        calling_ae = %association.calling_ae.trim(),
        called_ae = %association.called_ae.trim(),
        destination_ae = %destination,
        destination_address,
        phase,
        sop_instance_uid,
        remaining_suboperations = counts.remaining,
        completed_suboperations = counts.completed,
        failed_suboperations = counts.failed,
        warning_suboperations = counts.warning,
        %error,
        "C-MOVE request association interrupted during active retrieval"
    );
}

async fn abort_storage_after_request_failure(
    storage_association: &mut Association,
    destination: &str,
    request_error: &dicom_toolkit_core::error::DcmError,
) {
    if let Err(storage_abort_error) = storage_association.abort_strict().await {
        warn!(
            dimse_service = "C-MOVE",
            destination_ae = %destination,
            %request_error,
            %storage_abort_error,
            "failed to abort C-MOVE Storage association after request association failure"
        );
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
    response.set_u16(tags::COMMAND_FIELD, 0x8021);
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
    final_rsp.set_u16(tags::COMMAND_FIELD, 0x8021);
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
