//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for diagnostic or therapeutic purposes.
//!
//! Async DICOM networking: association management, DIMSE services.
//!
//! This crate ports DCMTK's `dcmnet` and `dcmtls` modules using `tokio`.

pub mod association;
pub mod config;
pub mod dataset_source;
pub mod dimse;
pub mod dul;
pub mod pdu;
pub mod presentation;
pub mod server;
pub mod services;
pub mod tls;

pub use association::Association;
pub use config::{AssociationConfig, AssociationOptions};
pub use dataset_source::{DatasetSource, FileDataset};
pub use pdu::{AssociationUserInformation, AsynchronousOperationsWindow, ScpScuRoleSelection};
pub use presentation::{PcResult, PresentationContextAc, PresentationContextRq};
pub use server::{DicomServer, DicomServerBuilder, FileStoreProvider};
pub use services::echo::c_echo;
pub use services::find::{c_find, handle_find_rq, handle_streaming_find_rq, FindRequest};
pub use services::get::{
    c_get, handle_get_rq, handle_streaming_get_rq, GetRequest, GetResponse, GetResult,
    ReceivedInstance,
};
pub use services::provider::{
    build_retrieve_presentation_contexts, find_responses, DestinationLookup, FindEvent,
    FindResponseStream, FindServiceProvider, GetEvent, GetRetrievePlan, GetServiceProvider,
    MoveEvent, MoveRetrievePlan, MoveServiceProvider, RetrieveCompletion, RetrieveItem,
    RetrievePresentationContext, RetrieveSubOperation, StaticDestinationLookup, StoreEvent,
    StoreResult, StoreServiceProvider, StreamingFindServiceProvider, StreamingGetServiceProvider,
    StreamingMoveServiceProvider, StreamingRetrieveItem, StreamingRetrieveItemStream,
    STATUS_CANCEL, STATUS_PENDING, STATUS_PROCESSING_FAILURE, STATUS_SUCCESS,
    STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS, STATUS_UNABLE_TO_PROCESS, STATUS_WARNING,
};
pub use services::r#move::{
    c_move, handle_move_rq, handle_streaming_move_rq, MoveRequest, MoveResponse,
};
pub use services::store::{
    c_store, c_store_source, handle_store_rq, StoreRequest, StoreResponse, StoreSourceRequest,
};
pub use tls::{connect_tls, make_acceptor, TlsConfig};
