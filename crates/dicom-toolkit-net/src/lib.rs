//! > ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for diagnostic or therapeutic purposes.
//! Async DICOM networking: association management, DIMSE services.
//!
//! This crate ports DCMTK's `dcmnet` and `dcmtls` modules using `tokio`.

pub mod association;
pub mod config;
pub mod dimse;
pub mod dul;
pub mod pdu;
pub mod presentation;
pub mod services;
pub mod tls;

pub use association::Association;
pub use config::AssociationConfig;
pub use presentation::{PcResult, PresentationContextAc, PresentationContextRq};
pub use services::echo::c_echo;
pub use services::find::{c_find, FindRequest};
pub use services::get::{c_get, GetRequest, GetResponse, GetResult, ReceivedInstance};
pub use services::r#move::{c_move, MoveRequest, MoveResponse};
pub use services::store::{c_store, StoreRequest, StoreResponse};
pub use tls::{connect_tls, make_acceptor, TlsConfig};

