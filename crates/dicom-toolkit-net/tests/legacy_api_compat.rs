//! Source-compatibility fixture for the public `dicom-toolkit-net` 0.5 API.
//!
//! This integration test is compiled as an external consumer. It deliberately
//! uses public struct literals and the original provider signatures so an
//! accidental required field or signature change fails at compile time.

use dicom_toolkit_data::DataSet;
use dicom_toolkit_net::pdu::{
    AAbort, AssociateAc, AssociateRq, PDataTf, Pdv, PresentationContextAcItem,
    PresentationContextRqItem,
};
use dicom_toolkit_net::{
    Association, AssociationConfig, DestinationLookup, FindEvent, FindServiceProvider, GetEvent,
    GetServiceProvider, MoveEvent, MoveServiceProvider, RetrieveItem, StoreEvent, StoreRequest,
    StoreResult, StoreServiceProvider,
};

struct LegacyProvider;

impl StoreServiceProvider for LegacyProvider {
    async fn on_store(&self, _event: StoreEvent) -> StoreResult {
        StoreResult::success()
    }
}

impl FindServiceProvider for LegacyProvider {
    async fn on_find(&self, _event: FindEvent) -> Vec<DataSet> {
        Vec::new()
    }
}

impl GetServiceProvider for LegacyProvider {
    async fn on_get(&self, _event: GetEvent) -> Vec<RetrieveItem> {
        Vec::new()
    }
}

impl MoveServiceProvider for LegacyProvider {
    async fn on_move(&self, _event: MoveEvent) -> Vec<RetrieveItem> {
        Vec::new()
    }
}

impl DestinationLookup for LegacyProvider {
    fn lookup(&self, _ae_title: &str) -> Option<String> {
        None
    }
}

fn legacy_config_literal() -> AssociationConfig {
    AssociationConfig {
        local_ae_title: "LEGACY".to_string(),
        max_pdu_length: 65_536,
        dimse_timeout_secs: 30,
        accept_all_transfer_syntaxes: false,
        accepted_transfer_syntaxes: Vec::new(),
        preferred_transfer_syntaxes: Vec::new(),
        implementation_class_uid: "1.2.826.0.1".to_string(),
        implementation_version_name: "LEGACY_050".to_string(),
        accepted_abstract_syntaxes: Vec::new(),
    }
}

#[test]
fn legacy_public_struct_literals_compile_unchanged() {
    let _retrieve_item = RetrieveItem {
        sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
        sop_instance_uid: "1.2.826.0.1.1".to_string(),
        dataset: Vec::new(),
    };
    let _store_request = StoreRequest {
        sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
        sop_instance_uid: "1.2.826.0.1.1".to_string(),
        priority: 0,
        dataset_bytes: Vec::new(),
        context_id: 1,
    };
    let request_context = PresentationContextRqItem {
        id: 1,
        abstract_syntax: "1.2.840.10008.1.1".to_string(),
        transfer_syntaxes: vec!["1.2.840.10008.1.2.1".to_string()],
    };
    let accept_context = PresentationContextAcItem {
        id: 1,
        result: 0,
        transfer_syntax: "1.2.840.10008.1.2.1".to_string(),
    };
    let _associate_request = AssociateRq {
        called_ae_title: "CALLED".to_string(),
        calling_ae_title: "CALLING".to_string(),
        application_context: "1.2.840.10008.3.1.1.1".to_string(),
        presentation_contexts: vec![request_context],
        max_pdu_length: 65_536,
        implementation_class_uid: "1.2.826.0.1".to_string(),
        implementation_version_name: "LEGACY_050".to_string(),
    };
    let _associate_accept = AssociateAc {
        called_ae_title: "CALLED".to_string(),
        calling_ae_title: "CALLING".to_string(),
        application_context: "1.2.840.10008.3.1.1.1".to_string(),
        presentation_contexts: vec![accept_context],
        max_pdu_length: 65_536,
        implementation_class_uid: "1.2.826.0.1".to_string(),
        implementation_version_name: "LEGACY_050".to_string(),
    };
    let _data = PDataTf {
        pdvs: vec![Pdv {
            context_id: 1,
            msg_control: 0x02,
            data: Vec::new(),
        }],
    };
    let _abort = AAbort {
        source: 0,
        reason: 0,
    };
    let _config = legacy_config_literal();
}

#[allow(dead_code)]
async fn legacy_handler_calls_compile(
    association: &mut Association,
    context_id: u8,
    command: &DataSet,
) {
    let provider = LegacyProvider;
    let _ = dicom_toolkit_net::handle_find_rq(association, context_id, command, &provider).await;
    let _ = dicom_toolkit_net::handle_get_rq(association, context_id, command, &provider).await;
    let _ = dicom_toolkit_net::handle_move_rq(
        association,
        context_id,
        command,
        &provider,
        &provider,
        "LEGACY",
    )
    .await;
    let _ = dicom_toolkit_net::handle_store_rq(association, context_id, command, &provider).await;
}
