//! End-to-end integration tests for the DicomServer framework.
//!
//! Each test spins up a `DicomServer` with in-memory providers and an SCU
//! that talks to it over loopback TCP.  Port 0 is used throughout so the OS
//! assigns a free ephemeral port — tests can run in parallel.

use std::sync::{Arc, Mutex};

use dicom_toolkit_core::uid::sop_class;
use dicom_toolkit_data::io::writer::DicomWriter;
use dicom_toolkit_data::DataSet;
use dicom_toolkit_dict::{tags, Vr};

use dicom_toolkit_net::config::AssociationConfig;
use dicom_toolkit_net::presentation::PresentationContextRq;
use dicom_toolkit_net::server::DicomServer;
use dicom_toolkit_net::services::find::FindRequest;
use dicom_toolkit_net::services::get::GetRequest;
use dicom_toolkit_net::services::r#move::MoveRequest;
use dicom_toolkit_net::services::store::StoreRequest;
use dicom_toolkit_net::{
    c_echo, c_find, c_get, c_move, c_store, Association, FindEvent, FindServiceProvider,
    GetEvent, GetServiceProvider, MoveEvent, MoveServiceProvider, RetrieveItem,
    StaticDestinationLookup, StoreEvent, StoreResult, StoreServiceProvider, STATUS_SUCCESS,
};

// ── Helpers ───────────────────────────────────────────────────────────────────

const TS_EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";

fn encode_dataset(ds: &DataSet) -> Vec<u8> {
    let mut buf = Vec::new();
    DicomWriter::new(&mut buf)
        .write_dataset(ds, TS_EXPLICIT_LE)
        .expect("encode dataset");
    buf
}

fn make_ct_dataset(sop_instance_uid: &str, patient_name: &str) -> DataSet {
    let mut ds = DataSet::new();
    ds.set_string(tags::SOP_CLASS_UID, Vr::UI, sop_class::CT_IMAGE_STORAGE);
    ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, sop_instance_uid);
    ds.set_string(tags::PATIENT_NAME, Vr::PN, patient_name);
    ds
}

fn ct_store_context(id: u8) -> PresentationContextRq {
    PresentationContextRq {
        id,
        abstract_syntax: sop_class::CT_IMAGE_STORAGE.to_string(),
        transfer_syntaxes: vec![TS_EXPLICIT_LE.to_string()],
    }
}

fn echo_context(id: u8) -> PresentationContextRq {
    PresentationContextRq {
        id,
        abstract_syntax: "1.2.840.10008.1.1".to_string(), // Verification SOP Class
        transfer_syntaxes: vec![TS_EXPLICIT_LE.to_string()],
    }
}

fn qr_find_context(id: u8) -> PresentationContextRq {
    PresentationContextRq {
        id,
        abstract_syntax: sop_class::PATIENT_ROOT_QR_FIND.to_string(),
        transfer_syntaxes: vec![TS_EXPLICIT_LE.to_string()],
    }
}

fn qr_get_context(id: u8) -> PresentationContextRq {
    PresentationContextRq {
        id,
        abstract_syntax: sop_class::PATIENT_ROOT_QR_GET.to_string(),
        transfer_syntaxes: vec![TS_EXPLICIT_LE.to_string()],
    }
}

fn qr_move_context(id: u8) -> PresentationContextRq {
    PresentationContextRq {
        id,
        abstract_syntax: sop_class::PATIENT_ROOT_QR_MOVE.to_string(),
        transfer_syntaxes: vec![TS_EXPLICIT_LE.to_string()],
    }
}

fn scu_config() -> AssociationConfig {
    AssociationConfig {
        local_ae_title: "SCU".to_string(),
        ..Default::default()
    }
}

// ── In-memory providers ───────────────────────────────────────────────────────

/// Records every stored instance for test assertions.
#[derive(Clone, Default)]
struct MemoryStore {
    stored: Arc<Mutex<Vec<(String, String)>>>, // (sop_class, sop_instance)
}

impl StoreServiceProvider for MemoryStore {
    async fn on_store(&self, event: StoreEvent) -> StoreResult {
        self.stored
            .lock()
            .unwrap()
            .push((event.sop_class_uid, event.sop_instance_uid));
        StoreResult::success()
    }
}

/// Returns a fixed list of datasets for C-FIND.
struct FixedFindProvider {
    results: Vec<DataSet>,
}

impl FindServiceProvider for FixedFindProvider {
    async fn on_find(&self, _event: FindEvent) -> Vec<DataSet> {
        self.results.clone()
    }
}

/// Returns a fixed list of instances for C-GET.
struct FixedGetProvider {
    items: Vec<RetrieveItem>,
}

impl GetServiceProvider for FixedGetProvider {
    async fn on_get(&self, _event: GetEvent) -> Vec<RetrieveItem> {
        self.items.clone()
    }
}

/// Returns a fixed list of instances for C-MOVE.
struct FixedMoveProvider {
    items: Vec<RetrieveItem>,
}

impl MoveServiceProvider for FixedMoveProvider {
    async fn on_move(&self, _event: MoveEvent) -> Vec<RetrieveItem> {
        self.items.clone()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// C-ECHO to a DicomServer (echo is always built-in).
#[tokio::test]
async fn test_server_echo_builtin() {
    let server = DicomServer::builder()
        .ae_title("ECHOSCP")
        .port(0)
        .build()
        .await
        .expect("build server");

    let addr = server.local_addr().expect("local addr");
    let token = server.cancellation_token();

    tokio::spawn(async move { server.run().await });

    let cfg = scu_config();
    let mut assoc = Association::request(
        &addr.to_string(),
        "ECHOSCP",
        "SCU",
        &[echo_context(1)],
        &cfg,
    )
    .await
    .expect("associate");

    let ctx_id = assoc.find_context("1.2.840.10008.1.1").unwrap().id;
    c_echo(&mut assoc, ctx_id).await.expect("c-echo");
    assoc.release().await.unwrap();

    token.cancel();
}

/// C-STORE to a DicomServer with MemoryStore provider.
#[tokio::test]
async fn test_server_store_loopback() {
    let store = MemoryStore::default();
    let store_check = store.clone();

    let server = DicomServer::builder()
        .ae_title("STORESCP")
        .port(0)
        .store_provider(store)
        .build()
        .await
        .expect("build server");

    let addr = server.local_addr().expect("local addr");
    let token = server.cancellation_token();

    tokio::spawn(async move { server.run().await });

    let cfg = scu_config();
    let mut assoc = Association::request(
        &addr.to_string(),
        "STORESCP",
        "SCU",
        &[ct_store_context(1)],
        &cfg,
    )
    .await
    .expect("associate");

    let ctx_id = assoc.find_context(sop_class::CT_IMAGE_STORAGE).unwrap().id;
    let ds = make_ct_dataset("1.2.3.server.store.1", "Patient^A");
    let rsp = c_store(
        &mut assoc,
        StoreRequest {
            sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
            sop_instance_uid: "1.2.3.server.store.1".to_string(),
            priority: 0,
            dataset_bytes: encode_dataset(&ds),
            context_id: ctx_id,
        },
    )
    .await
    .expect("c-store");

    assert_eq!(rsp.status, STATUS_SUCCESS);
    assoc.release().await.unwrap();

    // Allow server task time to process.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let stored = store_check.stored.lock().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].0, sop_class::CT_IMAGE_STORAGE);
    assert_eq!(stored[0].1, "1.2.3.server.store.1");

    token.cancel();
}

/// C-FIND to a DicomServer with FixedFindProvider.
#[tokio::test]
async fn test_server_find_loopback() {
    let mut result_ds = DataSet::new();
    result_ds.set_string(tags::PATIENT_NAME, Vr::PN, "Find^Result");
    result_ds.set_string(tags::PATIENT_ID, Vr::LO, "PID-001");

    let server = DicomServer::builder()
        .ae_title("FINDSCP")
        .port(0)
        .find_provider(FixedFindProvider {
            results: vec![result_ds.clone()],
        })
        .build()
        .await
        .expect("build server");

    let addr = server.local_addr().expect("local addr");
    let token = server.cancellation_token();

    tokio::spawn(async move { server.run().await });

    let cfg = scu_config();
    let mut assoc = Association::request(
        &addr.to_string(),
        "FINDSCP",
        "SCU",
        &[qr_find_context(1)],
        &cfg,
    )
    .await
    .expect("associate");

    let ctx_id = assoc
        .find_context(sop_class::PATIENT_ROOT_QR_FIND)
        .unwrap()
        .id;

    let query_ds = {
        let mut q = DataSet::new();
        q.set_string(tags::PATIENT_ID, Vr::LO, "");
        q
    };

    let results = c_find(
        &mut assoc,
        FindRequest {
            sop_class_uid: sop_class::PATIENT_ROOT_QR_FIND.to_string(),
            query: encode_dataset(&query_ds),
            context_id: ctx_id,
            priority: 0,
        },
    )
    .await
    .expect("c-find");

    assoc.release().await.unwrap();
    token.cancel();

    assert_eq!(results.len(), 1, "expected 1 match");
}

/// C-GET from a DicomServer — instances delivered via sub-ops on same association.
#[tokio::test]
async fn test_server_get_loopback() {
    let inst_ds = make_ct_dataset("1.2.3.server.get.1", "Patient^Get");
    let inst_bytes = encode_dataset(&inst_ds);

    let server = DicomServer::builder()
        .ae_title("GETSCP")
        .port(0)
        .get_provider(FixedGetProvider {
            items: vec![RetrieveItem {
                sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                sop_instance_uid: "1.2.3.server.get.1".to_string(),
                dataset: inst_bytes,
            }],
        })
        .build()
        .await
        .expect("build server");

    let addr = server.local_addr().expect("local addr");
    let token = server.cancellation_token();

    tokio::spawn(async move { server.run().await });

    let cfg = scu_config();
    let mut assoc = Association::request(
        &addr.to_string(),
        "GETSCP",
        "SCU",
        &[qr_get_context(1), ct_store_context(3)],
        &cfg,
    )
    .await
    .expect("associate");

    let ctx_id = assoc
        .find_context(sop_class::PATIENT_ROOT_QR_GET)
        .unwrap()
        .id;

    let query_ds = {
        let mut q = DataSet::new();
        q.set_string(tags::PATIENT_ID, Vr::LO, "GET-PATIENT");
        q
    };

    let result = c_get(
        &mut assoc,
        GetRequest {
            sop_class_uid: sop_class::PATIENT_ROOT_QR_GET.to_string(),
            query: encode_dataset(&query_ds),
            context_id: ctx_id,
            priority: 0,
        },
    )
    .await
    .expect("c-get");

    assoc.release().await.unwrap();
    token.cancel();

    assert_eq!(result.instances.len(), 1, "expected 1 instance received");
    assert_eq!(result.instances[0].sop_instance_uid, "1.2.3.server.get.1");
    let final_rsp = result.responses.last().unwrap();
    assert_eq!(final_rsp.status & 0xFF00, 0, "final status should be success/warning");
}

/// C-MOVE from a DicomServer — forwards instances to a storage SCP.
#[tokio::test]
async fn test_server_move_loopback() {
    // Storage SCP that records what it receives.
    let storage_store = MemoryStore::default();
    let storage_check = storage_store.clone();
    let storage_server = DicomServer::builder()
        .ae_title("STORESCP")
        .port(0)
        .store_provider(storage_store)
        .build()
        .await
        .expect("build storage server");

    let storage_addr = storage_server.local_addr().expect("storage addr");
    let storage_token = storage_server.cancellation_token();
    tokio::spawn(async move { storage_server.run().await });

    // QR SCP that will move to STORESCP.
    let inst_ds = make_ct_dataset("1.2.3.server.move.1", "Patient^Move");
    let inst_bytes = encode_dataset(&inst_ds);

    let dest_lookup = StaticDestinationLookup::new(vec![(
        "STORESCP".to_string(),
        storage_addr.to_string(),
    )]);

    let qr_server = DicomServer::builder()
        .ae_title("QRSCP")
        .port(0)
        .move_provider(FixedMoveProvider {
            items: vec![RetrieveItem {
                sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                sop_instance_uid: "1.2.3.server.move.1".to_string(),
                dataset: inst_bytes,
            }],
        })
        .move_destination_lookup(dest_lookup)
        .build()
        .await
        .expect("build QR server");

    let qr_addr = qr_server.local_addr().expect("QR addr");
    let qr_token = qr_server.cancellation_token();
    tokio::spawn(async move { qr_server.run().await });

    // SCU issues C-MOVE.
    let cfg = scu_config();
    let mut assoc = Association::request(
        &qr_addr.to_string(),
        "QRSCP",
        "SCU",
        &[qr_move_context(1)],
        &cfg,
    )
    .await
    .expect("associate to QR SCP");

    let ctx_id = assoc
        .find_context(sop_class::PATIENT_ROOT_QR_MOVE)
        .unwrap()
        .id;

    let query_ds = {
        let mut q = DataSet::new();
        q.set_string(tags::PATIENT_ID, Vr::LO, "MOVE-PATIENT");
        q
    };

    let responses = c_move(
        &mut assoc,
        MoveRequest {
            sop_class_uid: sop_class::PATIENT_ROOT_QR_MOVE.to_string(),
            destination: "STORESCP".to_string(),
            query: encode_dataset(&query_ds),
            context_id: ctx_id,
            priority: 0,
        },
    )
    .await
    .expect("c-move");

    assoc.release().await.unwrap();

    // 1 pending + 1 final.
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].status, 0xFF00); // pending
    let final_rsp = responses.last().unwrap();
    assert_eq!(final_rsp.status, 0x0000); // success
    assert_eq!(final_rsp.completed, Some(1));
    assert_eq!(final_rsp.failed, Some(0));

    // Wait for the sub-association and storage to finish.
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let stored = storage_check.stored.lock().unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].1, "1.2.3.server.move.1");

    qr_token.cancel();
    storage_token.cancel();
}

/// Multiple concurrent C-STORE associations to the same DicomServer.
#[tokio::test]
async fn test_server_concurrent_associations() {
    let store = MemoryStore::default();
    let store_check = store.clone();

    let server = DicomServer::builder()
        .ae_title("CONCSCP")
        .port(0)
        .max_associations(10)
        .store_provider(store)
        .build()
        .await
        .expect("build server");

    let addr = server.local_addr().expect("local addr");
    let token = server.cancellation_token();

    tokio::spawn(async move { server.run().await });

    // Spawn 5 concurrent SCU tasks.
    let mut handles = Vec::new();
    for i in 0u32..5 {
        let addr = addr.to_string();
        handles.push(tokio::spawn(async move {
            let cfg = AssociationConfig {
                local_ae_title: format!("SCU{i}"),
                ..Default::default()
            };
            let mut assoc = Association::request(
                &addr,
                "CONCSCP",
                &format!("SCU{i}"),
                &[ct_store_context(1)],
                &cfg,
            )
            .await
            .expect("associate");

            let ctx_id = assoc.find_context(sop_class::CT_IMAGE_STORAGE).unwrap().id;
            let ds = make_ct_dataset(&format!("1.2.3.concurrent.{i}"), "Concurrent^Patient");
            c_store(
                &mut assoc,
                StoreRequest {
                    sop_class_uid: sop_class::CT_IMAGE_STORAGE.to_string(),
                    sop_instance_uid: format!("1.2.3.concurrent.{i}"),
                    priority: 0,
                    dataset_bytes: encode_dataset(&ds),
                    context_id: ctx_id,
                },
            )
            .await
            .expect("c-store");
            assoc.release().await.unwrap();
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let stored = store_check.stored.lock().unwrap();
    assert_eq!(stored.len(), 5, "all 5 concurrent stores should be received");

    token.cancel();
}

/// Graceful shutdown: server stops accepting new connections after cancel.
#[tokio::test]
async fn test_server_graceful_shutdown() {
    let server = DicomServer::builder()
        .ae_title("SHUTDOWNSCP")
        .port(0)
        .build()
        .await
        .expect("build server");

    let addr = server.local_addr().expect("local addr");
    let token = server.cancellation_token();

    let handle = tokio::spawn(async move { server.run().await });

    // Immediately request shutdown.
    token.cancel();

    // Server should complete cleanly.
    let result = handle.await.expect("task did not panic");
    assert!(result.is_ok(), "server run should return Ok after shutdown");

    // New connection attempts should now fail (port no longer listening).
    let cfg = scu_config();
    let connect_result = Association::request(
        &addr.to_string(),
        "SHUTDOWNSCP",
        "SCU",
        &[echo_context(1)],
        &cfg,
    )
    .await;
    assert!(
        connect_result.is_err(),
        "connection after shutdown should fail"
    );
}
