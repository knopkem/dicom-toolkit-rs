use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_core::uid::sop_class;
use dicom_toolkit_data::{DataSet, DicomReader, DicomWriter};
use dicom_toolkit_dict::{tags, Tag, Vr};
use dicom_toolkit_net::PresentationContextRq;
use dicom_toolkit_net::{
    c_get, Association, AssociationConfig, AssociationOptions, DatasetSource, DicomServer,
    GetEvent, GetRequest, GetRetrievePlan, MoveEvent, MoveRetrievePlan,
    RetrievePresentationContext, RetrieveSubOperation, ScpScuRoleSelection,
    StaticDestinationLookup, StoreEvent, StoreResult, StoreServiceProvider,
    StreamingGetServiceProvider, StreamingMoveServiceProvider, StreamingRetrieveItem,
    StreamingRetrieveItemStream, STATUS_CANCEL, STATUS_WARNING,
};
use futures_util::stream;
use tokio::net::TcpListener;
use tokio::sync::{oneshot, Barrier};
use tokio::time::timeout;

const TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN: &str = "1.2.840.10008.1.2.1";
const TRANSFER_SYNTAX_IMPLICIT_VR_LITTLE_ENDIAN: &str = "1.2.840.10008.1.2";

#[tokio::test]
async fn rejects_a_zero_streaming_move_pending_response_interval() {
    let result = DicomServer::builder()
        .port(0)
        .streaming_move_pending_response_interval(Duration::ZERO)
        .build()
        .await;

    assert!(result.is_err());
}

fn loopback_address(address: SocketAddr) -> SocketAddr {
    match address {
        SocketAddr::V4(address) if address.ip().is_unspecified() => {
            SocketAddr::from((Ipv4Addr::LOCALHOST, address.port()))
        }
        SocketAddr::V6(address) if address.ip().is_unspecified() => {
            SocketAddr::from((Ipv6Addr::LOCALHOST, address.port()))
        }
        _ => address,
    }
}

fn query_retrieve_get_context(identifier: u8) -> PresentationContextRq {
    PresentationContextRq {
        id: identifier,
        abstract_syntax: sop_class::PATIENT_ROOT_QR_GET.to_string(),
        transfer_syntaxes: vec![TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string()],
    }
}

fn storage_context(identifier: u8) -> PresentationContextRq {
    PresentationContextRq {
        id: identifier,
        abstract_syntax: sop_class::CT_IMAGE_STORAGE.to_string(),
        transfer_syntaxes: vec![TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string()],
    }
}

fn query_retrieve_move_context(identifier: u8) -> PresentationContextRq {
    PresentationContextRq {
        id: identifier,
        abstract_syntax: sop_class::PATIENT_ROOT_QR_MOVE.to_string(),
        transfer_syntaxes: vec![TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string()],
    }
}

fn requestor_options() -> AssociationOptions {
    AssociationOptions {
        requested_role_selections: vec![ScpScuRoleSelection {
            sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
            scu_role: false,
            scp_role: true,
        }],
        ..Default::default()
    }
}

fn encode_dataset(dataset: &DataSet) -> Vec<u8> {
    let mut encoded = Vec::new();
    DicomWriter::new(&mut encoded)
        .write_dataset(dataset, TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN)
        .expect("encode dataset");
    encoded
}

fn encoded_instance(sop_instance_uid: &str) -> Vec<u8> {
    let mut dataset = DataSet::new();
    dataset.set_string(tags::SOP_CLASS_UID, Vr::UI, sop_class::CT_IMAGE_STORAGE);
    dataset.set_string(tags::SOP_INSTANCE_UID, Vr::UI, sop_instance_uid);
    encode_dataset(&dataset)
}

fn query_identifier() -> Vec<u8> {
    let mut dataset = DataSet::new();
    dataset.set_string(tags::PATIENT_ID, Vr::LO, "STREAMING-TEST");
    encode_dataset(&dataset)
}

fn get_request_command(message_id: u16) -> DataSet {
    let mut command = DataSet::new();
    command.set_uid(tags::AFFECTED_SOP_CLASS_UID, sop_class::PATIENT_ROOT_QR_GET);
    command.set_u16(tags::COMMAND_FIELD, 0x0010);
    command.set_u16(tags::MESSAGE_ID, message_id);
    command.set_u16(tags::PRIORITY, 0);
    command.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000);
    command
}

fn cancel_command(message_id_being_responded_to: u16) -> DataSet {
    let mut command = DataSet::new();
    command.set_u16(tags::COMMAND_FIELD, 0x0FFF);
    command.set_u16(
        tags::MESSAGE_ID_BEING_RESPONDED_TO,
        message_id_being_responded_to,
    );
    command.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
    command
}

fn successful_store_response(store_request: &DataSet) -> DataSet {
    let mut response = DataSet::new();
    response.set_uid(
        tags::AFFECTED_SOP_CLASS_UID,
        store_request
            .get_string(tags::AFFECTED_SOP_CLASS_UID)
            .expect("C-STORE SOP Class UID"),
    );
    response.set_u16(tags::COMMAND_FIELD, 0x8001);
    response.set_u16(
        tags::MESSAGE_ID_BEING_RESPONDED_TO,
        store_request
            .get_u16(tags::MESSAGE_ID)
            .expect("C-STORE message ID"),
    );
    response.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
    response.set_u16(tags::STATUS, 0x0000);
    response
}

async fn receive_initial_pending_response(
    association: &mut Association,
    expected_command_field: u16,
    expected_remaining_suboperations: u16,
) {
    let (_, response) = association
        .recv_dimse_command()
        .await
        .expect("receive initial retrieve pending response");
    assert_eq!(
        response.get_u16(tags::COMMAND_FIELD),
        Some(expected_command_field)
    );
    assert_eq!(response.get_u16(tags::STATUS), Some(0xFF00));
    assert_eq!(
        response.get_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS),
        Some(expected_remaining_suboperations)
    );
    assert_eq!(
        response.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
        Some(0)
    );
    assert_eq!(
        response.get_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS),
        Some(0)
    );
    assert_eq!(
        response.get_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS),
        Some(0)
    );
}

fn move_request_command(message_id: u16, destination: &str) -> DataSet {
    let mut command = DataSet::new();
    command.set_uid(
        tags::AFFECTED_SOP_CLASS_UID,
        sop_class::PATIENT_ROOT_QR_MOVE,
    );
    command.set_u16(tags::COMMAND_FIELD, 0x0021);
    command.set_u16(tags::MESSAGE_ID, message_id);
    command.set_u16(tags::PRIORITY, 0);
    command.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000);
    command.set_string(tags::MOVE_DESTINATION, Vr::AE, destination);
    command
}

struct FixedStreamingGetProvider {
    items: Vec<StreamingRetrieveItem>,
}

impl StreamingGetServiceProvider for FixedStreamingGetProvider {
    async fn on_get_stream(&self, _event: GetEvent) -> DcmResult<GetRetrievePlan> {
        GetRetrievePlan::from_items(self.items.clone())
    }
}

struct DropSignal(Arc<AtomicBool>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct PendingStreamingGetProvider {
    stream_dropped: Arc<AtomicBool>,
}

struct PendingStreamingMoveProvider {
    stream_dropped: Arc<AtomicBool>,
}

struct FixedStreamingMoveProvider {
    items: Vec<StreamingRetrieveItem>,
}

#[derive(Clone)]
struct ConcurrentStoreProvider {
    synchronization_barrier: Arc<Barrier>,
    active_operations: Arc<AtomicUsize>,
    maximum_active_operations: Arc<AtomicUsize>,
    stored_operations: Arc<AtomicUsize>,
}

impl StoreServiceProvider for ConcurrentStoreProvider {
    async fn on_store(&self, _event: StoreEvent) -> StoreResult {
        let active_operations = self.active_operations.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum_active_operations
            .fetch_max(active_operations, Ordering::SeqCst);
        self.synchronization_barrier.wait().await;
        self.stored_operations.fetch_add(1, Ordering::SeqCst);
        self.active_operations.fetch_sub(1, Ordering::SeqCst);
        StoreResult::success()
    }
}

impl StreamingMoveServiceProvider for FixedStreamingMoveProvider {
    async fn on_move_stream(&self, _event: MoveEvent) -> DcmResult<MoveRetrievePlan> {
        MoveRetrievePlan::from_items(self.items.clone())
    }
}

impl StreamingMoveServiceProvider for PendingStreamingMoveProvider {
    async fn on_move_stream(&self, _event: MoveEvent) -> DcmResult<MoveRetrievePlan> {
        let drop_signal = DropSignal(Arc::clone(&self.stream_dropped));
        let items: StreamingRetrieveItemStream =
            Box::pin(stream::unfold(drop_signal, |drop_signal| async move {
                let _drop_signal = drop_signal;
                std::future::pending::<Option<(DcmResult<RetrieveSubOperation>, DropSignal)>>()
                    .await
            }));
        MoveRetrievePlan::new(
            3,
            vec![RetrievePresentationContext::new(
                sop_class::CT_IMAGE_STORAGE,
                TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN,
            )?],
            items,
        )
    }
}

impl StreamingGetServiceProvider for PendingStreamingGetProvider {
    async fn on_get_stream(&self, _event: GetEvent) -> DcmResult<GetRetrievePlan> {
        let drop_signal = DropSignal(Arc::clone(&self.stream_dropped));
        let items: StreamingRetrieveItemStream =
            Box::pin(stream::unfold(drop_signal, |drop_signal| async move {
                let _drop_signal = drop_signal;
                std::future::pending::<Option<(DcmResult<RetrieveSubOperation>, DropSignal)>>()
                    .await
            }));
        Ok(GetRetrievePlan::new(3, items))
    }
}

#[tokio::test]
async fn parallel_move_uses_the_configured_destination_association_pool() {
    const DESTINATION_ASSOCIATIONS: usize = 3;
    const INSTANCE_COUNT: usize = 6;

    let active_operations = Arc::new(AtomicUsize::new(0));
    let maximum_active_operations = Arc::new(AtomicUsize::new(0));
    let stored_operations = Arc::new(AtomicUsize::new(0));
    let destination_server = DicomServer::builder()
        .ae_title("DESTINATION")
        .port(0)
        .store_provider(ConcurrentStoreProvider {
            synchronization_barrier: Arc::new(Barrier::new(DESTINATION_ASSOCIATIONS)),
            active_operations: Arc::clone(&active_operations),
            maximum_active_operations: Arc::clone(&maximum_active_operations),
            stored_operations: Arc::clone(&stored_operations),
        })
        .build()
        .await
        .expect("build destination server");
    let destination_address = loopback_address(
        destination_server
            .local_addr()
            .expect("destination address"),
    );
    let destination_cancellation_token = destination_server.cancellation_token();
    let destination_server_task = tokio::spawn(async move { destination_server.run().await });

    let items = (1..=INSTANCE_COUNT)
        .map(|sequence| {
            let sop_instance_uid = format!("1.2.826.0.1.20.{sequence}");
            StreamingRetrieveItem {
                sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                sop_instance_uid: sop_instance_uid.clone(),
                transfer_syntax_uid: TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string(),
                dataset: DatasetSource::bytes(encoded_instance(&sop_instance_uid)),
            }
        })
        .collect();
    let query_retrieve_server = DicomServer::builder()
        .ae_title("MOVESCP")
        .port(0)
        .streaming_move_provider(FixedStreamingMoveProvider { items })
        .streaming_move_destination_associations(DESTINATION_ASSOCIATIONS)
        .move_destination_lookup(StaticDestinationLookup::new(vec![(
            "DESTINATION".to_string(),
            destination_address.to_string(),
        )]))
        .build()
        .await
        .expect("build query/retrieve server");
    let query_retrieve_address = loopback_address(
        query_retrieve_server
            .local_addr()
            .expect("query/retrieve address"),
    );
    let query_retrieve_cancellation_token = query_retrieve_server.cancellation_token();
    let query_retrieve_server_task = tokio::spawn(async move { query_retrieve_server.run().await });

    let mut association = Association::request(
        &query_retrieve_address.to_string(),
        "MOVESCP",
        "MOVESCU",
        &[query_retrieve_move_context(1)],
        &AssociationConfig::default(),
    )
    .await
    .expect("associate");
    let query_context_id = association
        .find_context(sop_class::PATIENT_ROOT_QR_MOVE)
        .expect("query context")
        .id;
    association
        .send_dimse_command(query_context_id, &move_request_command(49, "DESTINATION"))
        .await
        .expect("send C-MOVE-RQ");
    association
        .send_dimse_data(query_context_id, &query_identifier())
        .await
        .expect("send identifier");

    let final_response = timeout(Duration::from_secs(5), async {
        loop {
            let (_, response) = association
                .recv_dimse_command()
                .await
                .expect("receive C-MOVE-RSP");
            if response.get_u16(tags::STATUS) != Some(0xFF00) {
                break response;
            }
        }
    })
    .await
    .expect("parallel C-MOVE completes before timeout");

    assert_eq!(final_response.get_u16(tags::STATUS), Some(0x0000));
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
        Some(INSTANCE_COUNT as u16)
    );
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS),
        Some(0)
    );
    assert_eq!(stored_operations.load(Ordering::SeqCst), INSTANCE_COUNT);
    assert_eq!(
        maximum_active_operations.load(Ordering::SeqCst),
        DESTINATION_ASSOCIATIONS
    );

    association
        .release()
        .await
        .expect("release request association");
    query_retrieve_cancellation_token.cancel();
    destination_cancellation_token.cancel();
    query_retrieve_server_task
        .await
        .expect("query/retrieve server task")
        .expect("query/retrieve server run");
    destination_server_task
        .await
        .expect("destination server task")
        .expect("destination server run");
}

#[tokio::test]
async fn late_bound_get_continues_after_an_unnegotiated_transfer_syntax() {
    let successful_instance_uids = ["1.2.826.0.1.1", "1.2.826.0.1.3"];
    let failed_instance_uid = "1.2.826.0.1.2";
    let server = DicomServer::builder()
        .ae_title("GETSCP")
        .port(0)
        .streaming_get_provider(FixedStreamingGetProvider {
            items: vec![
                StreamingRetrieveItem {
                    sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                    sop_instance_uid: successful_instance_uids[0].to_string(),
                    transfer_syntax_uid: TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string(),
                    dataset: DatasetSource::bytes(encoded_instance(successful_instance_uids[0])),
                },
                StreamingRetrieveItem {
                    sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                    sop_instance_uid: failed_instance_uid.to_string(),
                    transfer_syntax_uid: TRANSFER_SYNTAX_IMPLICIT_VR_LITTLE_ENDIAN.to_string(),
                    dataset: DatasetSource::bytes(encoded_instance(failed_instance_uid)),
                },
                StreamingRetrieveItem {
                    sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                    sop_instance_uid: successful_instance_uids[1].to_string(),
                    transfer_syntax_uid: TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string(),
                    dataset: DatasetSource::bytes(encoded_instance(successful_instance_uids[1])),
                },
            ],
        })
        .build()
        .await
        .expect("build server");
    let address = loopback_address(server.local_addr().expect("local address"));
    let cancellation_token = server.cancellation_token();
    let server_task = tokio::spawn(async move { server.run().await });

    let association_config = AssociationConfig::default();
    let mut association = Association::request_with_options(
        &address.to_string(),
        "GETSCP",
        "GETSCU",
        &[query_retrieve_get_context(1), storage_context(3)],
        &association_config,
        &requestor_options(),
    )
    .await
    .expect("associate");
    let query_context_id = association
        .find_context(sop_class::PATIENT_ROOT_QR_GET)
        .expect("query context")
        .id;

    let result = c_get(
        &mut association,
        GetRequest {
            sop_class_uid: sop_class::PATIENT_ROOT_QR_GET.to_string(),
            query: query_identifier(),
            context_id: query_context_id,
            priority: 0,
        },
    )
    .await
    .expect("C-GET");

    let received_instance_uids = result
        .instances
        .iter()
        .map(|instance| instance.sop_instance_uid.as_str())
        .collect::<Vec<_>>();
    assert_eq!(received_instance_uids, successful_instance_uids);
    let final_response = result.responses.last().expect("final response");
    assert_eq!(final_response.status, STATUS_WARNING);
    assert_eq!(final_response.completed, Some(2));
    assert_eq!(final_response.failed, Some(1));
    let failed_identifier = final_response.dataset.as_ref().expect("failed UID list");
    let failed_identifier = DicomReader::new(failed_identifier.as_slice())
        .read_dataset(TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN)
        .expect("decode failed UID list");
    assert_eq!(
        failed_identifier.get_string(Tag::new(0x0008, 0x0058)),
        Some(failed_instance_uid)
    );

    association.release().await.expect("release");
    cancellation_token.cancel();
    server_task.await.expect("server task").expect("server run");
}

#[tokio::test]
async fn matching_cancel_drops_a_pending_provider_stream_and_ignores_a_mismatch() {
    let stream_dropped = Arc::new(AtomicBool::new(false));
    let server = DicomServer::builder()
        .ae_title("GETSCP")
        .port(0)
        .streaming_get_provider(PendingStreamingGetProvider {
            stream_dropped: Arc::clone(&stream_dropped),
        })
        .build()
        .await
        .expect("build server");
    let address = loopback_address(server.local_addr().expect("local address"));
    let cancellation_token = server.cancellation_token();
    let server_task = tokio::spawn(async move { server.run().await });

    let association_config = AssociationConfig::default();
    let mut association = Association::request_with_options(
        &address.to_string(),
        "GETSCP",
        "GETSCU",
        &[query_retrieve_get_context(1), storage_context(3)],
        &association_config,
        &requestor_options(),
    )
    .await
    .expect("associate");
    let query_context_id = association
        .find_context(sop_class::PATIENT_ROOT_QR_GET)
        .expect("query context")
        .id;
    let retrieve_message_id = 41;
    association
        .send_dimse_command(query_context_id, &get_request_command(retrieve_message_id))
        .await
        .expect("send C-GET-RQ");
    association
        .send_dimse_data(query_context_id, &query_identifier())
        .await
        .expect("send identifier");
    receive_initial_pending_response(&mut association, 0x8010, 3).await;
    association
        .send_dimse_command(query_context_id, &cancel_command(retrieve_message_id + 1))
        .await
        .expect("send mismatched C-CANCEL-RQ");
    association
        .send_dimse_command(query_context_id, &cancel_command(retrieve_message_id))
        .await
        .expect("send matching C-CANCEL-RQ");

    let (_, final_response) = association
        .recv_dimse_command()
        .await
        .expect("receive final C-GET-RSP");
    assert_eq!(final_response.get_u16(tags::COMMAND_FIELD), Some(0x8010));
    assert_eq!(final_response.get_u16(tags::STATUS), Some(STATUS_CANCEL));
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS),
        Some(3)
    );
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
        Some(0)
    );
    assert!(stream_dropped.load(Ordering::SeqCst));
    if final_response.get_u16(tags::COMMAND_DATA_SET_TYPE) != Some(0x0101) {
        association
            .recv_dimse_data()
            .await
            .expect("receive cancel identifier");
    }

    association.release().await.expect("release");
    cancellation_token.cancel();
    server_task.await.expect("server task").expect("server run");
}

#[tokio::test]
async fn matching_move_cancel_drops_a_pending_provider_stream() {
    let destination_server = DicomServer::builder()
        .ae_title("DESTINATION")
        .port(0)
        .build()
        .await
        .expect("build destination server");
    let destination_address = loopback_address(
        destination_server
            .local_addr()
            .expect("destination address"),
    );
    let destination_cancellation_token = destination_server.cancellation_token();
    let destination_server_task = tokio::spawn(async move { destination_server.run().await });

    let stream_dropped = Arc::new(AtomicBool::new(false));
    let query_retrieve_server = DicomServer::builder()
        .ae_title("MOVESCP")
        .port(0)
        .streaming_move_provider(PendingStreamingMoveProvider {
            stream_dropped: Arc::clone(&stream_dropped),
        })
        .streaming_move_pending_response_interval(Duration::from_millis(25))
        .move_destination_lookup(StaticDestinationLookup::new(vec![(
            "DESTINATION".to_string(),
            destination_address.to_string(),
        )]))
        .build()
        .await
        .expect("build query/retrieve server");
    let query_retrieve_address = loopback_address(
        query_retrieve_server
            .local_addr()
            .expect("query/retrieve address"),
    );
    let query_retrieve_cancellation_token = query_retrieve_server.cancellation_token();
    let query_retrieve_server_task = tokio::spawn(async move { query_retrieve_server.run().await });

    let association_config = AssociationConfig::default();
    let mut association = Association::request(
        &query_retrieve_address.to_string(),
        "MOVESCP",
        "MOVESCU",
        &[query_retrieve_move_context(1)],
        &association_config,
    )
    .await
    .expect("associate");
    let query_context_id = association
        .find_context(sop_class::PATIENT_ROOT_QR_MOVE)
        .expect("query context")
        .id;
    let retrieve_message_id = 43;
    association
        .send_dimse_command(
            query_context_id,
            &move_request_command(retrieve_message_id, "DESTINATION"),
        )
        .await
        .expect("send C-MOVE-RQ");
    association
        .send_dimse_data(query_context_id, &query_identifier())
        .await
        .expect("send identifier");
    receive_initial_pending_response(&mut association, 0x8021, 3).await;
    timeout(
        Duration::from_secs(1),
        receive_initial_pending_response(&mut association, 0x8021, 3),
    )
    .await
    .expect("receive C-MOVE Pending heartbeat while provider is waiting");
    association
        .send_dimse_command(query_context_id, &cancel_command(retrieve_message_id))
        .await
        .expect("send C-CANCEL-RQ");

    let (_, final_response) = association
        .recv_dimse_command()
        .await
        .expect("receive final C-MOVE-RSP");
    assert_eq!(final_response.get_u16(tags::COMMAND_FIELD), Some(0x8021));
    assert_eq!(final_response.get_u16(tags::STATUS), Some(STATUS_CANCEL));
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS),
        Some(3)
    );
    assert!(stream_dropped.load(Ordering::SeqCst));
    if final_response.get_u16(tags::COMMAND_DATA_SET_TYPE) != Some(0x0101) {
        association
            .recv_dimse_data()
            .await
            .expect("receive cancel identifier");
    }

    association.release().await.expect("release");
    query_retrieve_cancellation_token.cancel();
    destination_cancellation_token.cancel();
    query_retrieve_server_task
        .await
        .expect("query/retrieve server task")
        .expect("query/retrieve server run");
    destination_server_task
        .await
        .expect("destination server task")
        .expect("destination server run");
}

#[tokio::test]
async fn cancel_during_active_move_store_aborts_the_storage_association_and_preserves_the_request_association(
) {
    const DATASET_LENGTH: u64 = 64 * 1024 * 1024;
    let temporary_file = tempfile::NamedTempFile::new().expect("create temporary file");
    temporary_file
        .as_file()
        .set_len(DATASET_LENGTH)
        .expect("create sparse dataset");

    let destination_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("bind destination listener");
    let destination_address = destination_listener
        .local_addr()
        .expect("destination listener address");
    let (store_request_received_sender, store_request_received_receiver) = oneshot::channel();
    let (allow_dataset_read_sender, allow_dataset_read_receiver) = oneshot::channel();
    let destination_task = tokio::spawn(async move {
        let (stream, _) = destination_listener
            .accept()
            .await
            .expect("accept destination association");
        let mut association = Association::accept(stream, &AssociationConfig::default())
            .await
            .expect("accept destination association protocol");
        let (_, store_request) = association
            .recv_dimse_command()
            .await
            .expect("receive active C-STORE-RQ");
        assert_eq!(store_request.get_u16(tags::COMMAND_FIELD), Some(0x0001));
        store_request_received_sender
            .send(())
            .expect("signal active C-STORE-RQ");

        allow_dataset_read_receiver
            .await
            .expect("allow destination dataset read");
        let error = association
            .recv_dimse_data()
            .await
            .expect_err("C-MOVE cancellation must abort the Storage association");
        assert!(matches!(
            error,
            dicom_toolkit_core::error::DcmError::AssociationAborted { .. }
        ));
    });

    let first_sop_instance_uid = "1.2.826.0.1.7";
    let query_retrieve_server = DicomServer::builder()
        .ae_title("MOVESCP")
        .port(0)
        .streaming_move_provider(FixedStreamingMoveProvider {
            items: vec![
                StreamingRetrieveItem {
                    sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                    sop_instance_uid: first_sop_instance_uid.to_string(),
                    transfer_syntax_uid: TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string(),
                    dataset: DatasetSource::file_region(temporary_file.path(), 0, DATASET_LENGTH),
                },
                StreamingRetrieveItem {
                    sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                    sop_instance_uid: "1.2.826.0.1.8".to_string(),
                    transfer_syntax_uid: TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string(),
                    dataset: DatasetSource::bytes(encoded_instance("1.2.826.0.1.8")),
                },
            ],
        })
        .streaming_move_pending_response_interval(Duration::from_millis(25))
        .move_destination_lookup(StaticDestinationLookup::new(vec![(
            "DESTINATION".to_string(),
            destination_address.to_string(),
        )]))
        .build()
        .await
        .expect("build query/retrieve server");
    let query_retrieve_address = loopback_address(
        query_retrieve_server
            .local_addr()
            .expect("query/retrieve address"),
    );
    let query_retrieve_cancellation_token = query_retrieve_server.cancellation_token();
    let query_retrieve_server_task = tokio::spawn(async move { query_retrieve_server.run().await });

    let mut association = Association::request(
        &query_retrieve_address.to_string(),
        "MOVESCP",
        "MOVESCU",
        &[query_retrieve_move_context(1)],
        &AssociationConfig::default(),
    )
    .await
    .expect("associate");
    let query_context_id = association
        .find_context(sop_class::PATIENT_ROOT_QR_MOVE)
        .expect("query context")
        .id;
    let retrieve_message_id = 45;
    association
        .send_dimse_command(
            query_context_id,
            &move_request_command(retrieve_message_id, "DESTINATION"),
        )
        .await
        .expect("send C-MOVE-RQ");
    association
        .send_dimse_data(query_context_id, &query_identifier())
        .await
        .expect("send identifier");
    receive_initial_pending_response(&mut association, 0x8021, 2).await;

    timeout(Duration::from_secs(5), store_request_received_receiver)
        .await
        .expect("C-STORE-RQ must become active before cancellation")
        .expect("destination must observe active C-STORE-RQ");
    timeout(
        Duration::from_secs(1),
        receive_initial_pending_response(&mut association, 0x8021, 2),
    )
    .await
    .expect("receive C-MOVE Pending heartbeat during active C-STORE");
    association
        .send_dimse_command(query_context_id, &cancel_command(retrieve_message_id))
        .await
        .expect("send C-CANCEL-MOVE-RQ during active C-STORE");
    allow_dataset_read_sender
        .send(())
        .expect("allow destination to observe the abort");

    let (_, final_response) = timeout(Duration::from_secs(5), association.recv_dimse_command())
        .await
        .expect("receive final C-MOVE-RSP before timeout")
        .expect("receive final C-MOVE-RSP");
    assert_eq!(final_response.get_u16(tags::COMMAND_FIELD), Some(0x8021));
    assert_eq!(final_response.get_u16(tags::STATUS), Some(STATUS_CANCEL));
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS),
        Some(1)
    );
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
        Some(0)
    );
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS),
        Some(1)
    );
    let failed_identifier = association
        .recv_dimse_data()
        .await
        .expect("receive failed SOP Instance UID List");
    let failed_identifier = DicomReader::new(failed_identifier.as_slice())
        .read_dataset(TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN)
        .expect("decode failed SOP Instance UID List");
    assert_eq!(
        failed_identifier.get_string(Tag::new(0x0008, 0x0058)),
        Some(first_sop_instance_uid)
    );

    association
        .release()
        .await
        .expect("the C-MOVE request association remains usable after cancellation");
    timeout(Duration::from_secs(5), destination_task)
        .await
        .expect("destination task completes before timeout")
        .expect("destination task");
    query_retrieve_cancellation_token.cancel();
    query_retrieve_server_task
        .await
        .expect("query/retrieve server task")
        .expect("query/retrieve server run");
}

#[tokio::test]
async fn request_association_abort_during_active_move_aborts_the_storage_association() {
    const DATASET_LENGTH: u64 = 64 * 1024 * 1024;
    let temporary_file = tempfile::NamedTempFile::new().expect("create temporary file");
    temporary_file
        .as_file()
        .set_len(DATASET_LENGTH)
        .expect("create sparse dataset");

    let destination_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("bind destination listener");
    let destination_address = destination_listener
        .local_addr()
        .expect("destination listener address");
    let (store_request_received_sender, store_request_received_receiver) = oneshot::channel();
    let (allow_dataset_read_sender, allow_dataset_read_receiver) = oneshot::channel();
    let destination_task = tokio::spawn(async move {
        let (stream, _) = destination_listener
            .accept()
            .await
            .expect("accept destination association");
        let mut association = Association::accept(stream, &AssociationConfig::default())
            .await
            .expect("accept destination association protocol");
        let (_, store_request) = association
            .recv_dimse_command()
            .await
            .expect("receive active C-STORE-RQ");
        assert_eq!(store_request.get_u16(tags::COMMAND_FIELD), Some(0x0001));
        store_request_received_sender
            .send(())
            .expect("signal active C-STORE-RQ");

        allow_dataset_read_receiver
            .await
            .expect("allow destination dataset read");
        let error = association
            .recv_dimse_data()
            .await
            .expect_err("request-association failure must abort the Storage association");
        assert!(matches!(
            error,
            dicom_toolkit_core::error::DcmError::AssociationAborted { .. }
        ));
    });

    let query_retrieve_server = DicomServer::builder()
        .ae_title("MOVESCP")
        .port(0)
        .streaming_move_provider(FixedStreamingMoveProvider {
            items: vec![StreamingRetrieveItem {
                sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                sop_instance_uid: "1.2.826.0.1.9".to_string(),
                transfer_syntax_uid: TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string(),
                dataset: DatasetSource::file_region(temporary_file.path(), 0, DATASET_LENGTH),
            }],
        })
        .streaming_move_pending_response_interval(Duration::from_millis(25))
        .move_destination_lookup(StaticDestinationLookup::new(vec![(
            "DESTINATION".to_string(),
            destination_address.to_string(),
        )]))
        .build()
        .await
        .expect("build query/retrieve server");
    let query_retrieve_address = loopback_address(
        query_retrieve_server
            .local_addr()
            .expect("query/retrieve address"),
    );
    let query_retrieve_cancellation_token = query_retrieve_server.cancellation_token();
    let query_retrieve_server_task = tokio::spawn(async move { query_retrieve_server.run().await });

    let mut association = Association::request(
        &query_retrieve_address.to_string(),
        "MOVESCP",
        "MOVESCU",
        &[query_retrieve_move_context(1)],
        &AssociationConfig::default(),
    )
    .await
    .expect("associate");
    let query_context_id = association
        .find_context(sop_class::PATIENT_ROOT_QR_MOVE)
        .expect("query context")
        .id;
    association
        .send_dimse_command(query_context_id, &move_request_command(47, "DESTINATION"))
        .await
        .expect("send C-MOVE-RQ");
    association
        .send_dimse_data(query_context_id, &query_identifier())
        .await
        .expect("send identifier");
    receive_initial_pending_response(&mut association, 0x8021, 1).await;

    timeout(Duration::from_secs(5), store_request_received_receiver)
        .await
        .expect("C-STORE-RQ must become active before abort")
        .expect("destination must observe active C-STORE-RQ");
    association
        .abort_strict()
        .await
        .expect("abort request association");
    allow_dataset_read_sender
        .send(())
        .expect("allow destination to observe Storage abort");

    timeout(Duration::from_secs(5), destination_task)
        .await
        .expect("destination task completes before timeout")
        .expect("destination task");
    query_retrieve_cancellation_token.cancel();
    query_retrieve_server_task
        .await
        .expect("query/retrieve server task")
        .expect("query/retrieve server run");
}

#[tokio::test]
async fn cancel_during_c_get_finishes_the_current_store_before_returning_cancel() {
    const DATASET_LENGTH: u64 = 32 * 1024 * 1024;
    let temporary_file = tempfile::NamedTempFile::new().expect("create temporary file");
    temporary_file
        .as_file()
        .set_len(DATASET_LENGTH)
        .expect("create sparse dataset");
    let sop_instance_uid = "1.2.826.0.1.4";
    let server = DicomServer::builder()
        .ae_title("GETSCP")
        .port(0)
        .streaming_get_provider(FixedStreamingGetProvider {
            items: vec![
                StreamingRetrieveItem {
                    sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                    sop_instance_uid: sop_instance_uid.to_string(),
                    transfer_syntax_uid: TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string(),
                    dataset: DatasetSource::file_region(temporary_file.path(), 0, DATASET_LENGTH),
                },
                StreamingRetrieveItem {
                    sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                    sop_instance_uid: "1.2.826.0.1.5".to_string(),
                    transfer_syntax_uid: TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string(),
                    dataset: DatasetSource::bytes(encoded_instance("1.2.826.0.1.5")),
                },
            ],
        })
        .build()
        .await
        .expect("build server");
    let address = loopback_address(server.local_addr().expect("local address"));
    let cancellation_token = server.cancellation_token();
    let server_task = tokio::spawn(async move { server.run().await });

    let association_config = AssociationConfig::default();
    let mut association = Association::request_with_options(
        &address.to_string(),
        "GETSCP",
        "GETSCU",
        &[query_retrieve_get_context(1), storage_context(3)],
        &association_config,
        &requestor_options(),
    )
    .await
    .expect("associate");
    let query_context_id = association
        .find_context(sop_class::PATIENT_ROOT_QR_GET)
        .expect("query context")
        .id;
    let retrieve_message_id = 42;
    association
        .send_dimse_command(query_context_id, &get_request_command(retrieve_message_id))
        .await
        .expect("send C-GET-RQ");
    association
        .send_dimse_data(query_context_id, &query_identifier())
        .await
        .expect("send identifier");
    receive_initial_pending_response(&mut association, 0x8010, 2).await;

    let (store_context_id, store_request) = association
        .recv_dimse_command()
        .await
        .expect("receive C-STORE-RQ");
    assert_eq!(store_request.get_u16(tags::COMMAND_FIELD), Some(0x0001));
    association
        .send_dimse_command(query_context_id, &cancel_command(retrieve_message_id))
        .await
        .expect("send C-CANCEL-RQ");
    let complete_dataset = association
        .recv_dimse_data()
        .await
        .expect("receive complete current C-STORE dataset");
    assert_eq!(complete_dataset.len(), DATASET_LENGTH as usize);
    association
        .send_dimse_command(store_context_id, &successful_store_response(&store_request))
        .await
        .expect("send C-STORE-RSP after complete dataset");

    let (_, final_response) = association
        .recv_dimse_command()
        .await
        .expect("receive final C-GET-RSP");
    assert_eq!(final_response.get_u16(tags::STATUS), Some(STATUS_CANCEL));
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS),
        Some(1)
    );
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
        Some(1)
    );
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS),
        Some(0)
    );
    assert_eq!(
        final_response.get_u16(tags::COMMAND_DATA_SET_TYPE),
        Some(0x0101),
        "an empty Failed SOP Instance UID List must be absent"
    );

    association.release().await.expect("release");
    cancellation_token.cancel();
    server_task.await.expect("server task").expect("server run");
}

#[tokio::test]
async fn cancel_while_waiting_for_store_response_still_accounts_that_response() {
    let sop_instance_uid = "1.2.826.0.1.6";
    let encoded_dataset = encoded_instance(sop_instance_uid);
    let expected_dataset = encoded_dataset.clone();
    let server = DicomServer::builder()
        .ae_title("GETSCP")
        .port(0)
        .streaming_get_provider(FixedStreamingGetProvider {
            items: vec![StreamingRetrieveItem {
                sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                sop_instance_uid: sop_instance_uid.to_string(),
                transfer_syntax_uid: TRANSFER_SYNTAX_EXPLICIT_VR_LITTLE_ENDIAN.to_string(),
                dataset: DatasetSource::bytes(encoded_dataset),
            }],
        })
        .build()
        .await
        .expect("build server");
    let address = loopback_address(server.local_addr().expect("local address"));
    let cancellation_token = server.cancellation_token();
    let server_task = tokio::spawn(async move { server.run().await });

    let mut association = Association::request_with_options(
        &address.to_string(),
        "GETSCP",
        "GETSCU",
        &[query_retrieve_get_context(1), storage_context(3)],
        &AssociationConfig::default(),
        &requestor_options(),
    )
    .await
    .expect("associate");
    let query_context_id = association
        .find_context(sop_class::PATIENT_ROOT_QR_GET)
        .expect("query context")
        .id;
    let retrieve_message_id = 44;
    association
        .send_dimse_command(query_context_id, &get_request_command(retrieve_message_id))
        .await
        .expect("send C-GET-RQ");
    association
        .send_dimse_data(query_context_id, &query_identifier())
        .await
        .expect("send identifier");
    receive_initial_pending_response(&mut association, 0x8010, 1).await;

    let (store_context_id, store_request) = association
        .recv_dimse_command()
        .await
        .expect("receive C-STORE-RQ");
    let received_dataset = association
        .recv_dimse_data()
        .await
        .expect("receive complete C-STORE dataset");
    assert_eq!(received_dataset, expected_dataset);

    association
        .send_dimse_command(query_context_id, &cancel_command(retrieve_message_id))
        .await
        .expect("send C-CANCEL-RQ while SCP awaits C-STORE-RSP");
    association
        .send_dimse_command(store_context_id, &successful_store_response(&store_request))
        .await
        .expect("send matching C-STORE-RSP");

    let (_, final_response) = association
        .recv_dimse_command()
        .await
        .expect("receive final C-GET-RSP");
    assert_eq!(final_response.get_u16(tags::STATUS), Some(STATUS_CANCEL));
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS),
        Some(0)
    );
    assert_eq!(
        final_response.get_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
        Some(1)
    );
    assert_eq!(
        final_response.get_u16(tags::COMMAND_DATA_SET_TYPE),
        Some(0x0101)
    );

    association.release().await.expect("release");
    cancellation_token.cancel();
    server_task.await.expect("server task").expect("server run");
}
