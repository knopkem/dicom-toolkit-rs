//! End-to-end integration tests for DICOM networking services.
//!
//! Each test spins up in-process tokio tasks for both SCU and SCP sides,
//! communicates over loopback TCP, and verifies protocol correctness.
//!
//! Port 0 is used everywhere so the OS assigns a free port — tests are safe
//! to run in parallel.

use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::Notify;

use dicom_toolkit_data::DataSet;
use dicom_toolkit_data::io::writer::DicomWriter;
use dicom_toolkit_data::io::reader::DicomReader;
use dicom_toolkit_dict::tags;
use dicom_toolkit_core::uid::sop_class;

use dicom_toolkit_net::association::Association;
use dicom_toolkit_net::config::AssociationConfig;
use dicom_toolkit_net::presentation::PresentationContextRq;
use dicom_toolkit_net::services::store::{c_store, StoreRequest};
use dicom_toolkit_net::services::find::{c_find, FindRequest};
use dicom_toolkit_net::services::get::{c_get, GetRequest};
use dicom_toolkit_net::services::r#move::{c_move, MoveRequest};

// ── Helpers ───────────────────────────────────────────────────────────────────

const TS_EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";

/// Encode a DataSet to bytes using Explicit VR Little Endian.
fn encode_dataset(ds: &DataSet) -> Vec<u8> {
    let mut buf = Vec::new();
    DicomWriter::new(&mut buf)
        .write_dataset(ds, TS_EXPLICIT_LE)
        .expect("encode dataset");
    buf
}

/// Decode bytes to a DataSet using Explicit VR Little Endian.
fn decode_dataset(bytes: &[u8]) -> DataSet {
    DicomReader::new(bytes)
        .read_dataset(TS_EXPLICIT_LE)
        .expect("decode dataset")
}

/// Build a minimal CT image dataset good enough for DIMSE transmission tests.
fn make_ct_dataset(sop_instance_uid: &str, patient_name: &str) -> DataSet {
    let mut ds = DataSet::new();
    ds.set_string(tags::SOP_CLASS_UID,    dicom_toolkit_dict::Vr::UI, sop_class::CT_IMAGE_STORAGE);
    ds.set_string(tags::SOP_INSTANCE_UID, dicom_toolkit_dict::Vr::UI, sop_instance_uid);
    ds.set_string(tags::PATIENT_NAME,     dicom_toolkit_dict::Vr::PN, patient_name);
    ds.set_u16(tags::ROWS,    512);
    ds.set_u16(tags::COLUMNS, 512);
    ds
}

/// Build a minimal query dataset for QR tests.
fn make_query_dataset(patient_id: &str) -> DataSet {
    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_ID, dicom_toolkit_dict::Vr::LO, patient_id);
    ds
}

/// SCP association config that accepts any presented SOP class / transfer syntax.
fn open_scp_config(ae_title: &str) -> AssociationConfig {
    let mut cfg = AssociationConfig::default();
    cfg.local_ae_title = ae_title.to_string();
    cfg.accept_all_transfer_syntaxes = true;
    cfg
}

fn ct_store_context(id: u8) -> PresentationContextRq {
    PresentationContextRq {
        id,
        abstract_syntax: sop_class::CT_IMAGE_STORAGE.to_string(),
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

// ── Test 1: C-STORE loopback ──────────────────────────────────────────────────

/// SCU stores one CT instance → SCP receives, decodes, stores in a channel →
/// SCU verifies success status and the decoded patient name round-trips.
#[tokio::test]
async fn test_store_loopback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let ready = Arc::new(Notify::new());
    let ready_clone = ready.clone();

    // ── SCP task ─────────────────────────────────────────────────────────────
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DataSet>(1);
    tokio::spawn(async move {
        ready_clone.notify_one();
        let (stream, _) = listener.accept().await.unwrap();
        let mut assoc = Association::accept(stream, &open_scp_config("STORESCP"))
            .await
            .unwrap();

        let (ctx_id, cmd) = assoc.recv_dimse_command().await.unwrap();
        assert_eq!(cmd.get_u16(tags::COMMAND_FIELD), Some(0x0001)); // C-STORE-RQ

        let sop_class_uid = cmd
            .get_string(tags::AFFECTED_SOP_CLASS_UID)
            .unwrap_or_default()
            .trim_end_matches('\0')
            .to_string();
        let sop_instance_uid = cmd
            .get_string(tags::AFFECTED_SOP_INSTANCE_UID)
            .unwrap_or_default()
            .trim_end_matches('\0')
            .to_string();
        let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap();

        let data = assoc.recv_dimse_data().await.unwrap();
        let ts_uid = assoc
            .context_by_id(ctx_id)
            .map(|pc| pc.transfer_syntax.trim_end_matches('\0').to_string())
            .unwrap_or_else(|| TS_EXPLICIT_LE.to_string());
        let ds = DicomReader::new(data.as_slice())
            .read_dataset(&ts_uid)
            .unwrap();
        tx.send(ds).await.unwrap();

        let mut rsp = DataSet::new();
        rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class_uid);
        rsp.set_u16(tags::COMMAND_FIELD, 0x8001);
        rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
        rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        rsp.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, &sop_instance_uid);
        rsp.set_u16(tags::STATUS, 0x0000);
        assoc.send_dimse_command(ctx_id, &rsp).await.unwrap();
        let _ = assoc.release().await;
    });

    // ── SCU task ─────────────────────────────────────────────────────────────
    ready.notified().await;
    let scu_cfg = AssociationConfig::default();
    let ctx = ct_store_context(1);
    let mut assoc = Association::request(
        &format!("127.0.0.1:{port}"),
        "STORESCP", "STORESCU",
        &[ctx], &scu_cfg,
    ).await.unwrap();

    let ctx_id = assoc.find_context(sop_class::CT_IMAGE_STORAGE).unwrap().id;
    let ds = make_ct_dataset("1.2.3.4.5.6", "Loopback^Test");
    let dataset_bytes = encode_dataset(&ds);

    let req = StoreRequest {
        sop_class_uid:    sop_class::CT_IMAGE_STORAGE.to_string(),
        sop_instance_uid: "1.2.3.4.5.6".to_string(),
        priority: 0,
        dataset_bytes,
        context_id: ctx_id,
    };
    let rsp = c_store(&mut assoc, req).await.unwrap();
    assert_eq!(rsp.status, 0x0000, "C-STORE should succeed");
    assoc.release().await.unwrap();

    // Verify the SCP received and decoded the dataset correctly.
    let received = rx.recv().await.unwrap();
    assert_eq!(
        received.get_string(tags::PATIENT_NAME).as_deref(),
        Some("Loopback^Test"),
        "patient name must round-trip"
    );
    assert_eq!(received.get_u16(tags::ROWS), Some(512));
}

// ── Test 2: C-FIND loopback ───────────────────────────────────────────────────

/// SCU sends a C-FIND query → SCP returns 2 pending result datasets + final →
/// SCU collects all 2 result datasets and verifies their content.
#[tokio::test]
async fn test_find_loopback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let ready = Arc::new(Notify::new());
    let ready_clone = ready.clone();

    // ── SCP (mock QR SCP) ─────────────────────────────────────────────────────
    tokio::spawn(async move {
        ready_clone.notify_one();
        let (stream, _) = listener.accept().await.unwrap();
        let mut assoc = Association::accept(stream, &open_scp_config("FINDSCP"))
            .await
            .unwrap();

        let (ctx_id, cmd) = assoc.recv_dimse_command().await.unwrap();
        assert_eq!(cmd.get_u16(tags::COMMAND_FIELD), Some(0x0020)); // C-FIND-RQ

        let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap();
        let _query_bytes = assoc.recv_dimse_data().await.unwrap();

        // Two "database" records to return.
        let patients: &[(&str, &str, &str)] = &[
            ("1.1.1.1", "Smith^John", "P001"),
            ("2.2.2.2", "Jones^Mary", "P002"),
        ];

        for (uid, name, pid) in patients {
            let mut result = DataSet::new();
            result.set_string(tags::SOP_INSTANCE_UID, dicom_toolkit_dict::Vr::UI, uid);
            result.set_string(tags::PATIENT_NAME, dicom_toolkit_dict::Vr::PN, name);
            result.set_string(tags::PATIENT_ID,   dicom_toolkit_dict::Vr::LO, pid);
            let result_bytes = encode_dataset(&result);

            let mut rsp = DataSet::new();
            rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, sop_class::PATIENT_ROOT_QR_FIND);
            rsp.set_u16(tags::COMMAND_FIELD, 0x8020);
            rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
            rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000); // dataset follows
            rsp.set_u16(tags::STATUS, 0xFF00); // pending
            assoc.send_dimse_command(ctx_id, &rsp).await.unwrap();
            assoc.send_dimse_data(ctx_id, &result_bytes).await.unwrap();
        }

        // Final C-FIND-RSP (success, no dataset).
        let mut final_rsp = DataSet::new();
        final_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, sop_class::PATIENT_ROOT_QR_FIND);
        final_rsp.set_u16(tags::COMMAND_FIELD, 0x8020);
        final_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
        final_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        final_rsp.set_u16(tags::STATUS, 0x0000);
        assoc.send_dimse_command(ctx_id, &final_rsp).await.unwrap();
        let _ = assoc.release().await;
    });

    // ── SCU ───────────────────────────────────────────────────────────────────
    ready.notified().await;
    let scu_cfg = AssociationConfig::default();
    let mut assoc = Association::request(
        &format!("127.0.0.1:{port}"),
        "FINDSCP", "FINDSCU",
        &[qr_find_context(1)], &scu_cfg,
    ).await.unwrap();

    let ctx_id = assoc.find_context(sop_class::PATIENT_ROOT_QR_FIND).unwrap().id;
    let query = encode_dataset(&make_query_dataset(""));

    let results = c_find(&mut assoc, FindRequest {
        sop_class_uid: sop_class::PATIENT_ROOT_QR_FIND.to_string(),
        query,
        context_id: ctx_id,
        priority: 0,
    }).await.unwrap();

    assoc.release().await.unwrap();

    assert_eq!(results.len(), 2, "should receive 2 result datasets");

    let r0 = decode_dataset(&results[0]);
    let r1 = decode_dataset(&results[1]);
    assert_eq!(r0.get_string(tags::PATIENT_NAME).as_deref(), Some("Smith^John"));
    assert_eq!(r0.get_string(tags::PATIENT_ID).as_deref(),   Some("P001"));
    assert_eq!(r1.get_string(tags::PATIENT_NAME).as_deref(), Some("Jones^Mary"));
    assert_eq!(r1.get_string(tags::PATIENT_ID).as_deref(),   Some("P002"));
}

// ── Test 3: C-GET loopback ────────────────────────────────────────────────────

/// SCU sends C-GET-RQ → SCP pushes 2 instances back via C-STORE sub-ops on
/// the same association → SCU handles sub-ops and collects instances →
/// SCP sends pending + final C-GET-RSP → verify all counts and content.
#[tokio::test]
async fn test_get_loopback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let ready = Arc::new(Notify::new());
    let ready_clone = ready.clone();

    // Pre-build the two instances the SCP will push.
    let inst1_bytes = encode_dataset(&make_ct_dataset("1.2.3.101", "Get^PatientA"));
    let inst2_bytes = encode_dataset(&make_ct_dataset("1.2.3.102", "Get^PatientA"));

    // ── SCP (mock QR-GET SCP) ─────────────────────────────────────────────────
    let inst1_c = inst1_bytes.clone();
    let inst2_c = inst2_bytes.clone();
    tokio::spawn(async move {
        ready_clone.notify_one();
        let (stream, _) = listener.accept().await.unwrap();
        let mut assoc = Association::accept(stream, &open_scp_config("GETSCP"))
            .await
            .unwrap();

        let (ctx_id, cmd) = assoc.recv_dimse_command().await.unwrap();
        assert_eq!(cmd.get_u16(tags::COMMAND_FIELD), Some(0x0010)); // C-GET-RQ
        let get_msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap();
        let sop_class_uid = cmd
            .get_string(tags::AFFECTED_SOP_CLASS_UID)
            .unwrap_or_default()
            .trim_end_matches('\0')
            .to_string();
        let _query = assoc.recv_dimse_data().await.unwrap();

        let instances: &[(&str, &str, &[u8])] = &[
            (sop_class::CT_IMAGE_STORAGE, "1.2.3.101", &inst1_c),
            (sop_class::CT_IMAGE_STORAGE, "1.2.3.102", &inst2_c),
        ];

        for (i, (sc, si, data)) in instances.iter().enumerate() {
            let remaining = (instances.len() - 1 - i) as u16;

            // Pending C-GET-RSP before each sub-op.
            let mut get_rsp = DataSet::new();
            get_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class_uid);
            get_rsp.set_u16(tags::COMMAND_FIELD, 0x8010);
            get_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, get_msg_id);
            get_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
            get_rsp.set_u16(tags::STATUS, 0xFF00);
            get_rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, remaining);
            get_rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, i as u16);
            get_rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, 0);
            get_rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
            assoc.send_dimse_command(ctx_id, &get_rsp).await.unwrap();

            // C-STORE-RQ sub-operation.
            let store_msg_id = (100 + i) as u16;
            let mut store_rq = DataSet::new();
            store_rq.set_uid(tags::AFFECTED_SOP_CLASS_UID, sc);
            store_rq.set_u16(tags::COMMAND_FIELD, 0x0001);
            store_rq.set_u16(tags::MESSAGE_ID, store_msg_id);
            store_rq.set_u16(tags::PRIORITY, 0);
            store_rq.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0000); // dataset follows
            store_rq.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, si);
            assoc.send_dimse_command(ctx_id, &store_rq).await.unwrap();
            assoc.send_dimse_data(ctx_id, data).await.unwrap();

            // Wait for C-STORE-RSP from the SCU.
            let (_, store_rsp) = assoc.recv_dimse_command().await.unwrap();
            assert_eq!(store_rsp.get_u16(tags::COMMAND_FIELD), Some(0x8001));
            assert_eq!(store_rsp.get_u16(tags::STATUS), Some(0x0000));
        }

        // Final C-GET-RSP.
        let mut final_rsp = DataSet::new();
        final_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class_uid);
        final_rsp.set_u16(tags::COMMAND_FIELD, 0x8010);
        final_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, get_msg_id);
        final_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        final_rsp.set_u16(tags::STATUS, 0x0000);
        final_rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, 2);
        final_rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, 0);
        final_rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, 0);
        final_rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
        assoc.send_dimse_command(ctx_id, &final_rsp).await.unwrap();
        let _ = assoc.release().await;
    });

    // ── SCU ───────────────────────────────────────────────────────────────────
    ready.notified().await;
    let scu_cfg = AssociationConfig::default();
    let mut assoc = Association::request(
        &format!("127.0.0.1:{port}"),
        "GETSCP", "GETSCU",
        &[qr_get_context(1), ct_store_context(3)],
        &scu_cfg,
    ).await.unwrap();

    let ctx_id = assoc.find_context(sop_class::PATIENT_ROOT_QR_GET).unwrap().id;
    let query = encode_dataset(&make_query_dataset("P-GET"));

    let result = c_get(&mut assoc, GetRequest {
        sop_class_uid: sop_class::PATIENT_ROOT_QR_GET.to_string(),
        query,
        context_id: ctx_id,
        priority: 0,
    }).await.unwrap();

    assoc.release().await.unwrap();

    // 2 pending + 1 final.
    assert_eq!(result.responses.len(), 3, "2 pending + 1 final response");
    assert!(result.responses[..2].iter().all(|r| r.status == 0xFF00));
    assert_eq!(result.responses.last().unwrap().status, 0x0000);
    assert_eq!(result.responses.last().unwrap().completed, Some(2));

    // 2 instances delivered via C-STORE sub-ops.
    assert_eq!(result.instances.len(), 2, "should receive 2 instances");
    assert_eq!(result.instances[0].sop_instance_uid, "1.2.3.101");
    assert_eq!(result.instances[1].sop_instance_uid, "1.2.3.102");

    let ds0 = decode_dataset(&result.instances[0].dataset);
    let ds1 = decode_dataset(&result.instances[1].dataset);
    assert_eq!(ds0.get_string(tags::PATIENT_NAME).as_deref(), Some("Get^PatientA"));
    assert_eq!(ds1.get_u16(tags::ROWS), Some(512));
}

// ── Test 4: C-MOVE loopback ───────────────────────────────────────────────────

/// Three-party C-MOVE:
///   SCU → C-MOVE-RQ → QR SCP  (port A)
///   QR SCP → C-STORE ×2 → Storage SCP  (port B, AE "STORESCP")
///   QR SCP → C-MOVE-RSP (pending ×2 + final) → SCU
///
/// Verifies sub-operation counts, final status, and that the Storage SCP
/// received the two instances.
#[tokio::test]
async fn test_move_loopback() {
    // Storage SCP on its own port.
    let store_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let store_port = store_listener.local_addr().unwrap().port();

    // QR SCP on its own port.
    let qr_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let qr_port = qr_listener.local_addr().unwrap().port();

    let ready = Arc::new(Notify::new());
    let ready_clone = ready.clone();

    // Pre-build instances the QR SCP will forward.
    let inst1_bytes = encode_dataset(&make_ct_dataset("1.2.3.201", "Move^PatientA"));
    let inst2_bytes = encode_dataset(&make_ct_dataset("1.2.3.202", "Move^PatientA"));

    // ── Storage SCP task ──────────────────────────────────────────────────────
    let (store_tx, mut store_rx) = tokio::sync::mpsc::channel::<String>(4);
    tokio::spawn(async move {
        let (stream, _) = store_listener.accept().await.unwrap();
        let mut assoc = Association::accept(stream, &open_scp_config("STORESCP"))
            .await
            .unwrap();
        loop {
            let (ctx_id, cmd) = match assoc.recv_dimse_command().await {
                Ok(c) => c,
                Err(_) => break,
            };
            if cmd.get_u16(tags::COMMAND_FIELD) != Some(0x0001) { break; }

            let sop_class_uid = cmd.get_string(tags::AFFECTED_SOP_CLASS_UID)
                .unwrap_or_default().trim_end_matches('\0').to_string();
            let sop_instance_uid = cmd.get_string(tags::AFFECTED_SOP_INSTANCE_UID)
                .unwrap_or_default().trim_end_matches('\0').to_string();
            let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

            let _data = assoc.recv_dimse_data().await.unwrap();
            store_tx.send(sop_instance_uid.clone()).await.unwrap();

            let mut rsp = DataSet::new();
            rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class_uid);
            rsp.set_u16(tags::COMMAND_FIELD, 0x8001);
            rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
            rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
            rsp.set_uid(tags::AFFECTED_SOP_INSTANCE_UID, &sop_instance_uid);
            rsp.set_u16(tags::STATUS, 0x0000);
            assoc.send_dimse_command(ctx_id, &rsp).await.unwrap();
        }
        let _ = assoc.release().await;
    });

    // ── QR SCP task ───────────────────────────────────────────────────────────
    let inst1_c = inst1_bytes.clone();
    let inst2_c = inst2_bytes.clone();
    tokio::spawn(async move {
        ready_clone.notify_one();
        let (stream, _) = qr_listener.accept().await.unwrap();
        let mut assoc = Association::accept(stream, &open_scp_config("QRSCP"))
            .await
            .unwrap();

        let (ctx_id, cmd) = assoc.recv_dimse_command().await.unwrap();
        assert_eq!(cmd.get_u16(tags::COMMAND_FIELD), Some(0x0021)); // C-MOVE-RQ
        let move_msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap();
        let destination = cmd.get_string(tags::MOVE_DESTINATION)
            .unwrap_or_default().trim().to_string();
        assert_eq!(destination, "STORESCP");
        let sop_class_uid = cmd.get_string(tags::AFFECTED_SOP_CLASS_UID)
            .unwrap_or_default().trim_end_matches('\0').to_string();
        let _query = assoc.recv_dimse_data().await.unwrap();

        // Sub-associate to Storage SCP and forward both instances.
        let store_cfg = AssociationConfig::default();
        let mut sub = Association::request(
            &format!("127.0.0.1:{store_port}"),
            "STORESCP", "QRSCP",
            &[ct_store_context(1)],
            &store_cfg,
        ).await.unwrap();

        let instances: &[(&str, &str, &[u8])] = &[
            (sop_class::CT_IMAGE_STORAGE, "1.2.3.201", &inst1_c),
            (sop_class::CT_IMAGE_STORAGE, "1.2.3.202", &inst2_c),
        ];

        let mut completed = 0u16;
        for (i, (sc, si, data)) in instances.iter().enumerate() {
            let store_ctx = sub.find_context(sc).unwrap().id;
            let req = StoreRequest {
                sop_class_uid:    sc.to_string(),
                sop_instance_uid: si.to_string(),
                priority: 0,
                dataset_bytes: data.to_vec(),
                context_id: store_ctx,
            };
            let store_rsp = c_store(&mut sub, req).await.unwrap();
            if store_rsp.status == 0x0000 { completed += 1; }

            let remaining = (instances.len() - 1 - i) as u16;
            let mut move_rsp = DataSet::new();
            move_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class_uid);
            move_rsp.set_u16(tags::COMMAND_FIELD, 0x8021);
            move_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, move_msg_id);
            move_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
            move_rsp.set_u16(tags::STATUS, 0xFF00);
            move_rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, remaining);
            move_rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, completed);
            move_rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, 0);
            move_rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
            assoc.send_dimse_command(ctx_id, &move_rsp).await.unwrap();
        }

        sub.release().await.unwrap();

        let mut final_rsp = DataSet::new();
        final_rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class_uid);
        final_rsp.set_u16(tags::COMMAND_FIELD, 0x8021);
        final_rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, move_msg_id);
        final_rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        final_rsp.set_u16(tags::STATUS, 0x0000);
        final_rsp.set_u16(tags::NUMBER_OF_COMPLETED_SUB_OPERATIONS, completed);
        final_rsp.set_u16(tags::NUMBER_OF_FAILED_SUB_OPERATIONS, 0);
        final_rsp.set_u16(tags::NUMBER_OF_REMAINING_SUB_OPERATIONS, 0);
        final_rsp.set_u16(tags::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
        assoc.send_dimse_command(ctx_id, &final_rsp).await.unwrap();
        let _ = assoc.release().await;
    });

    // ── SCU ───────────────────────────────────────────────────────────────────
    ready.notified().await;
    let scu_cfg = AssociationConfig::default();
    let mut assoc = Association::request(
        &format!("127.0.0.1:{qr_port}"),
        "QRSCP", "MOVESCU",
        &[qr_move_context(1)],
        &scu_cfg,
    ).await.unwrap();

    let ctx_id = assoc.find_context(sop_class::PATIENT_ROOT_QR_MOVE).unwrap().id;
    let query = encode_dataset(&make_query_dataset("P-MOVE"));

    let responses = c_move(&mut assoc, MoveRequest {
        sop_class_uid: sop_class::PATIENT_ROOT_QR_MOVE.to_string(),
        destination: "STORESCP".to_string(),
        query,
        context_id: ctx_id,
        priority: 0,
    }).await.unwrap();

    assoc.release().await.unwrap();

    // 2 pending + 1 final.
    assert_eq!(responses.len(), 3, "2 pending + 1 final response");
    assert!(responses[..2].iter().all(|r| r.status == 0xFF00));
    let final_r = responses.last().unwrap();
    assert_eq!(final_r.status, 0x0000);
    assert_eq!(final_r.completed, Some(2));
    assert_eq!(final_r.failed, Some(0));

    // Storage SCP received both SOP instance UIDs.
    let uid1 = store_rx.recv().await.unwrap();
    let uid2 = store_rx.recv().await.unwrap();
    let mut received_uids = [uid1, uid2];
    received_uids.sort();
    assert_eq!(received_uids, ["1.2.3.201", "1.2.3.202"]);
}
