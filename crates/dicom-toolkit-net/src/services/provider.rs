//! SCP provider traits for DIMSE services.
//!
//! Implement these traits to plug your own storage, query, and retrieval
//! logic into a [`DicomServer`](crate::server::DicomServer).  The server
//! handles all DICOM protocol mechanics; your provider only sees clean Rust
//! types.
//!
//! # Design
//!
//! * **One trait per DIMSE service** — `StoreServiceProvider`,
//!   `FindServiceProvider`, `GetServiceProvider`, `MoveServiceProvider`.
//! * All methods are `async` and take `&self` so providers can hold
//!   shared state (database pools, in-memory stores, …).
//! * All traits require `Send + Sync + 'static` so they can be shared
//!   across tokio tasks.
//! * The separate [`DestinationLookup`] trait maps AE titles to network
//!   addresses for C-MOVE sub-associations.

use dicom_toolkit_data::DataSet;

// ── Status codes ──────────────────────────────────────────────────────────────

/// DIMSE status: success.
pub const STATUS_SUCCESS: u16 = 0x0000;
/// DIMSE status: pending (C-FIND / C-GET / C-MOVE).
pub const STATUS_PENDING: u16 = 0xFF00;
/// DIMSE status: out-of-resources failure.
pub const STATUS_OUT_OF_RESOURCES: u16 = 0xA700;
/// DIMSE status: dataset does not match SOP class.
pub const STATUS_DATASET_MISMATCH: u16 = 0xA900;
/// DIMSE status: processing failure (generic).
pub const STATUS_PROCESSING_FAILURE: u16 = 0x0110;
/// DIMSE status: unrecognised operation.
pub const STATUS_UNRECOGNISED_OPERATION: u16 = 0x0211;
/// DIMSE status: refused; move destination unknown.
pub const STATUS_MOVE_DESTINATION_UNKNOWN: u16 = 0xA801;
/// DIMSE status: sub-operations completed with one or more failures/warnings.
pub const STATUS_WARNING: u16 = 0xB000;

// ── C-STORE provider ──────────────────────────────────────────────────────────

/// Contextual information delivered to a [`StoreServiceProvider`].
#[derive(Debug, Clone)]
pub struct StoreEvent {
    /// AE title of the calling SCU.
    pub calling_ae: String,
    /// SOP Class UID of the instance being stored.
    pub sop_class_uid: String,
    /// SOP Instance UID of the instance being stored.
    pub sop_instance_uid: String,
    /// The decoded DICOM dataset.
    pub dataset: DataSet,
}

/// Result returned by a [`StoreServiceProvider`] callback.
#[derive(Debug, Clone)]
pub struct StoreResult {
    /// DIMSE status code to return to the SCU.
    ///
    /// Use [`STATUS_SUCCESS`] (0x0000) on success or one of the
    /// `STATUS_*` constants (or a custom code) on failure.
    pub status: u16,
}

impl StoreResult {
    /// Convenience constructor for a successful store.
    pub fn success() -> Self {
        Self {
            status: STATUS_SUCCESS,
        }
    }

    /// Convenience constructor for a processing-failure response.
    pub fn failure(status: u16) -> Self {
        Self { status }
    }
}

/// Trait implemented by SCP back-ends that handle C-STORE requests.
///
/// # Example
///
/// ```rust,no_run
/// use dicom_toolkit_net::services::provider::{StoreEvent, StoreResult, StoreServiceProvider};
///
/// struct MemoryStore {
///     instances: std::sync::Mutex<Vec<dicom_toolkit_data::DataSet>>,
/// }
///
/// impl StoreServiceProvider for MemoryStore {
///     async fn on_store(&self, event: StoreEvent) -> StoreResult {
///         self.instances.lock().unwrap().push(event.dataset);
///         StoreResult::success()
///     }
/// }
/// ```
pub trait StoreServiceProvider: Send + Sync + 'static {
    /// Called when the server receives a C-STORE-RQ.
    ///
    /// The returned status is forwarded to the SCU in the C-STORE-RSP.
    fn on_store(&self, event: StoreEvent) -> impl std::future::Future<Output = StoreResult> + Send;
}

// ── C-FIND provider ───────────────────────────────────────────────────────────

/// Contextual information delivered to a [`FindServiceProvider`].
#[derive(Debug, Clone)]
pub struct FindEvent {
    /// AE title of the calling SCU.
    pub calling_ae: String,
    /// SOP Class UID (identifies which query model is requested).
    pub sop_class_uid: String,
    /// The query identifier dataset supplied by the SCU.
    pub identifier: DataSet,
}

/// Trait implemented by SCP back-ends that handle C-FIND requests.
///
/// Return a `Vec<DataSet>` of matching result identifiers.  An empty
/// `Vec` results in a final C-FIND-RSP with status `0x0000` (success,
/// no matches).
pub trait FindServiceProvider: Send + Sync + 'static {
    /// Called when the server receives a C-FIND-RQ.
    ///
    /// Each `DataSet` in the returned `Vec` is sent as a pending
    /// C-FIND-RSP; a final success response is appended automatically.
    fn on_find(&self, event: FindEvent) -> impl std::future::Future<Output = Vec<DataSet>> + Send;
}

// ── C-GET provider ────────────────────────────────────────────────────────────

/// A single instance to be retrieved during a C-GET or C-MOVE sub-operation.
#[derive(Debug, Clone)]
pub struct RetrieveItem {
    /// SOP Class UID.
    pub sop_class_uid: String,
    /// SOP Instance UID.
    pub sop_instance_uid: String,
    /// Encoded dataset bytes (the pixel data and all other attributes).
    pub dataset: Vec<u8>,
}

/// Contextual information delivered to a [`GetServiceProvider`].
#[derive(Debug, Clone)]
pub struct GetEvent {
    /// AE title of the calling SCU.
    pub calling_ae: String,
    /// SOP Class UID (identifies which query/retrieve model is requested).
    pub sop_class_uid: String,
    /// The query identifier dataset supplied by the SCU.
    pub identifier: DataSet,
}

/// Trait implemented by SCP back-ends that handle C-GET requests.
///
/// Return a `Vec<RetrieveItem>` of instances to send back to the SCU via
/// C-STORE sub-operations on the **same** association.
pub trait GetServiceProvider: Send + Sync + 'static {
    /// Called when the server receives a C-GET-RQ.
    fn on_get(
        &self,
        event: GetEvent,
    ) -> impl std::future::Future<Output = Vec<RetrieveItem>> + Send;
}

// ── C-MOVE provider ───────────────────────────────────────────────────────────

/// Contextual information delivered to a [`MoveServiceProvider`].
#[derive(Debug, Clone)]
pub struct MoveEvent {
    /// AE title of the calling SCU.
    pub calling_ae: String,
    /// AE title of the move destination.
    pub destination: String,
    /// SOP Class UID (identifies which query/retrieve model is requested).
    pub sop_class_uid: String,
    /// The query identifier dataset supplied by the SCU.
    pub identifier: DataSet,
}

/// Trait implemented by SCP back-ends that handle C-MOVE requests.
///
/// Return a `Vec<RetrieveItem>` of instances to forward to the move
/// destination via a sub-association.
pub trait MoveServiceProvider: Send + Sync + 'static {
    /// Called when the server receives a C-MOVE-RQ.
    fn on_move(
        &self,
        event: MoveEvent,
    ) -> impl std::future::Future<Output = Vec<RetrieveItem>> + Send;
}

// ── Destination lookup ────────────────────────────────────────────────────────

/// Maps an AE title to a `"host:port"` address string for C-MOVE
/// sub-associations.
///
/// Implement this trait to manage your AE title registry (config file,
/// database, …).
pub trait DestinationLookup: Send + Sync + 'static {
    /// Return the network address for the given AE title, or `None` if
    /// the destination is unknown (causes the server to reply with
    /// `STATUS_MOVE_DESTINATION_UNKNOWN`).
    fn lookup(&self, ae_title: &str) -> Option<String>;
}

/// A fixed in-memory AE title registry.
///
/// # Example
///
/// ```rust
/// use dicom_toolkit_net::services::provider::StaticDestinationLookup;
///
/// let lookup = StaticDestinationLookup::new(vec![
///     ("STORESCP".to_string(), "127.0.0.1:4242".to_string()),
/// ]);
/// ```
pub struct StaticDestinationLookup {
    entries: Vec<(String, String)>,
}

impl StaticDestinationLookup {
    /// Create a lookup table from a list of `(ae_title, host:port)` pairs.
    pub fn new(entries: Vec<(String, String)>) -> Self {
        Self { entries }
    }
}

impl DestinationLookup for StaticDestinationLookup {
    fn lookup(&self, ae_title: &str) -> Option<String> {
        let upper = ae_title.trim().to_uppercase();
        self.entries
            .iter()
            .find(|(ae, _)| ae.trim().to_uppercase() == upper)
            .map(|(_, addr)| addr.clone())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_destination_lookup_found() {
        let lookup = StaticDestinationLookup::new(vec![
            ("STORESCP".to_string(), "127.0.0.1:4242".to_string()),
            ("ARCHIVE".to_string(), "10.0.0.1:11112".to_string()),
        ]);
        assert_eq!(
            lookup.lookup("STORESCP"),
            Some("127.0.0.1:4242".to_string())
        );
        assert_eq!(lookup.lookup("ARCHIVE"), Some("10.0.0.1:11112".to_string()));
    }

    #[test]
    fn static_destination_lookup_not_found() {
        let lookup = StaticDestinationLookup::new(vec![]);
        assert_eq!(lookup.lookup("UNKNOWN"), None);
    }

    #[test]
    fn static_destination_lookup_case_insensitive() {
        let lookup = StaticDestinationLookup::new(vec![(
            "StOreScp".to_string(),
            "127.0.0.1:4242".to_string(),
        )]);
        assert_eq!(
            lookup.lookup("storescp"),
            Some("127.0.0.1:4242".to_string())
        );
    }

    #[test]
    fn store_result_success() {
        let r = StoreResult::success();
        assert_eq!(r.status, STATUS_SUCCESS);
    }

    #[test]
    fn store_result_failure() {
        let r = StoreResult::failure(STATUS_OUT_OF_RESOURCES);
        assert_eq!(r.status, STATUS_OUT_OF_RESOURCES);
    }
}
