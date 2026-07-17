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

use std::fmt;
use std::pin::Pin;

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_core::uid::Uid;
use dicom_toolkit_data::DataSet;
use futures_core::Stream;
use futures_util::stream;

use crate::dataset_source::DatasetSource;

// ── Status codes ──────────────────────────────────────────────────────────────

/// DIMSE status: success.
pub const STATUS_SUCCESS: u16 = 0x0000;
/// DIMSE status: pending (C-FIND / C-GET / C-MOVE).
pub const STATUS_PENDING: u16 = 0xFF00;
/// DIMSE status: operation cancelled by the requestor.
pub const STATUS_CANCEL: u16 = 0xFE00;
/// DIMSE status: out-of-resources failure.
pub const STATUS_OUT_OF_RESOURCES: u16 = 0xA700;
/// Query/Retrieve status: unable to perform C-STORE sub-operations.
pub const STATUS_UNABLE_TO_PERFORM_SUBOPERATIONS: u16 = 0xA702;
/// DIMSE status: dataset does not match SOP class.
pub const STATUS_DATASET_MISMATCH: u16 = 0xA900;
/// DIMSE status: processing failure (generic).
pub const STATUS_PROCESSING_FAILURE: u16 = 0x0110;
/// Query/Retrieve status: unable to process the request.
pub const STATUS_UNABLE_TO_PROCESS: u16 = 0xC000;
/// DIMSE status: unrecognised operation.
pub const STATUS_UNRECOGNISED_OPERATION: u16 = 0x0211;
/// DIMSE status: refused; move destination unknown.
pub const STATUS_MOVE_DESTINATION_UNKNOWN: u16 = 0xA801;
/// DIMSE status: sub-operations completed with one or more failures/warnings.
pub const STATUS_WARNING: u16 = 0xB000;

/// A boxed asynchronous stream of C-FIND response identifiers.
pub type FindResponseStream = Pin<Box<dyn Stream<Item = DcmResult<DataSet>> + Send + 'static>>;

/// A boxed asynchronous stream of C-GET or C-MOVE sub-operation outcomes.
pub type StreamingRetrieveItemStream =
    Pin<Box<dyn Stream<Item = DcmResult<RetrieveSubOperation>> + Send + 'static>>;

/// Terminal outcome of a streaming C-GET or C-MOVE execution.
///
/// The callback attached to a retrieval plan receives exactly one outcome.
/// `Aborted` covers transport or protocol failures that prevent the service
/// from sending its final DIMSE response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrieveCompletion {
    /// The service reached a terminal DIMSE response.
    Finished {
        /// Number of sub-operations declared by the provider.
        total: u16,
        /// Number of successful C-STORE sub-operations.
        completed: u16,
        /// Number of failed C-STORE sub-operations.
        failed: u16,
        /// Number of C-STORE sub-operations completed with a warning.
        warning: u16,
        /// Whether the requestor cancelled the retrieve.
        cancelled: bool,
        /// Whether the provider stream failed or violated its declaration.
        provider_failed: bool,
        /// Final DIMSE status sent to the requestor.
        final_status: u16,
    },
    /// The association ended before a final DIMSE response could be sent.
    Aborted {
        /// Number of sub-operations declared by the provider.
        total: u16,
    },
}

type RetrieveCompletionCallback = Box<dyn FnOnce(RetrieveCompletion) + Send + 'static>;

/// Convert an in-memory list into a C-FIND response stream.
///
/// This is convenient for small result sets. Providers handling large queries
/// should return a genuinely lazy stream instead.
pub fn find_responses(responses: Vec<DataSet>) -> FindResponseStream {
    Box::pin(stream::iter(responses.into_iter().map(Ok)))
}

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
/// Return a `Vec<DataSet>` of matching result identifiers. An empty `Vec`
/// results in a final C-FIND-RSP with status `0x0000` (success, no matches).
pub trait FindServiceProvider: Send + Sync + 'static {
    /// Called when the server receives a C-FIND-RQ.
    ///
    /// Each `DataSet` in the returned `Vec` is sent as a pending C-FIND-RSP;
    /// a final success response is appended automatically.
    fn on_find(&self, event: FindEvent) -> impl std::future::Future<Output = Vec<DataSet>> + Send;
}

/// Streaming C-FIND provider for large or fallible result sets.
///
/// This trait is additive. [`FindServiceProvider`] remains the compatibility
/// API for existing providers.
pub trait StreamingFindServiceProvider: Send + Sync + 'static {
    /// Called when the server receives a C-FIND-RQ.
    fn on_find_stream(
        &self,
        event: FindEvent,
    ) -> impl std::future::Future<Output = DcmResult<FindResponseStream>> + Send;
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

/// A lazily supplied instance for a streaming C-GET or C-MOVE sub-operation.
#[derive(Debug, Clone)]
pub struct StreamingRetrieveItem {
    /// SOP Class UID.
    pub sop_class_uid: String,
    /// SOP Instance UID.
    pub sop_instance_uid: String,
    /// Transfer Syntax used by the encoded dataset.
    pub transfer_syntax_uid: String,
    /// Encoded dataset source, excluding Part 10 File Meta Information.
    pub dataset: DatasetSource,
}

/// Outcome of one declared retrieve sub-operation.
///
/// Per-instance failures remain in the stream so a provider can continue with
/// independent instances. An outer stream error is reserved for a fatal
/// provider failure that prevents further enumeration.
#[derive(Debug, Clone)]
pub enum RetrieveSubOperation {
    /// The instance is ready to be sent.
    Ready(StreamingRetrieveItem),
    /// The instance failed before a C-STORE sub-operation could complete.
    Failed {
        /// SOP Instance UID reported in the final failed-instance list.
        sop_instance_uid: String,
        /// Stable, human-readable failure detail for logging.
        reason: String,
    },
}

/// A storage presentation context required by a retrieval plan.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RetrievePresentationContext {
    /// Storage SOP Class UID.
    pub sop_class_uid: String,
    /// Transfer Syntax UID of the encoded instances.
    pub transfer_syntax_uid: String,
}

impl RetrievePresentationContext {
    /// Create and validate a storage presentation context.
    pub fn new(
        sop_class_uid: impl Into<String>,
        transfer_syntax_uid: impl Into<String>,
    ) -> DcmResult<Self> {
        let sop_class_uid = sop_class_uid.into();
        let transfer_syntax_uid = transfer_syntax_uid.into();
        Uid::new(sop_class_uid.clone())?;
        Uid::new(transfer_syntax_uid.clone())?;
        Ok(Self {
            sop_class_uid,
            transfer_syntax_uid,
        })
    }
}

/// Metadata and lazy item stream for a C-GET operation.
///
/// Storage presentation contexts are selected from the already negotiated
/// association after each item becomes available.
pub struct GetRetrievePlan {
    total_suboperations: u16,
    items: StreamingRetrieveItemStream,
    completion_callback: Option<RetrieveCompletionCallback>,
}

impl fmt::Debug for GetRetrievePlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GetRetrievePlan")
            .field("total_suboperations", &self.total_suboperations)
            .finish_non_exhaustive()
    }
}

impl GetRetrievePlan {
    /// Create a C-GET plan from a declared total and lazy outcome stream.
    pub fn new(total_suboperations: u16, items: StreamingRetrieveItemStream) -> Self {
        Self {
            total_suboperations,
            items,
            completion_callback: None,
        }
    }

    /// Convert a finite list into a C-GET plan.
    pub fn from_items(items: Vec<StreamingRetrieveItem>) -> DcmResult<Self> {
        let total_suboperations = u16::try_from(items.len()).map_err(|_| {
            DcmError::Other(format!(
                "retrieval contains {} items; DIMSE counters support at most {}",
                items.len(),
                u16::MAX
            ))
        })?;

        for item in &items {
            Uid::new(item.sop_class_uid.clone())?;
            Uid::new(item.sop_instance_uid.clone())?;
            Uid::new(item.transfer_syntax_uid.clone())?;
        }

        Ok(Self::new(
            total_suboperations,
            Box::pin(stream::iter(
                items.into_iter().map(RetrieveSubOperation::Ready).map(Ok),
            )),
        ))
    }

    /// Return the declared number of C-STORE sub-operations.
    pub fn total_suboperations(&self) -> u16 {
        self.total_suboperations
    }

    /// Attach a callback invoked exactly once when execution finishes or aborts.
    pub fn with_completion_callback(
        mut self,
        callback: impl FnOnce(RetrieveCompletion) + Send + 'static,
    ) -> Self {
        self.completion_callback = Some(Box::new(callback));
        self
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        u16,
        StreamingRetrieveItemStream,
        Option<RetrieveCompletionCallback>,
    ) {
        (
            self.total_suboperations,
            self.items,
            self.completion_callback,
        )
    }
}

/// Metadata and lazy item stream for a C-MOVE operation.
///
/// Unlike C-GET, C-MOVE must declare presentation-context candidates before
/// opening the destination association.
pub struct MoveRetrievePlan {
    total_suboperations: u16,
    presentation_contexts: Vec<RetrievePresentationContext>,
    items: StreamingRetrieveItemStream,
    completion_callback: Option<RetrieveCompletionCallback>,
}

impl fmt::Debug for MoveRetrievePlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MoveRetrievePlan")
            .field("total_suboperations", &self.total_suboperations)
            .field("presentation_contexts", &self.presentation_contexts)
            .finish_non_exhaustive()
    }
}

impl MoveRetrievePlan {
    /// Create a C-MOVE plan with explicit presentation-context candidates.
    pub fn new(
        total_suboperations: u16,
        mut presentation_contexts: Vec<RetrievePresentationContext>,
        items: StreamingRetrieveItemStream,
    ) -> DcmResult<Self> {
        validate_retrieve_presentation_contexts(&mut presentation_contexts)?;
        if total_suboperations > 0 && presentation_contexts.is_empty() {
            return Err(DcmError::Other(
                "a non-empty C-MOVE plan requires at least one presentation context".into(),
            ));
        }
        Ok(Self {
            total_suboperations,
            presentation_contexts,
            items,
            completion_callback: None,
        })
    }

    /// Convert a finite list into a C-MOVE plan.
    pub fn from_items(items: Vec<StreamingRetrieveItem>) -> DcmResult<Self> {
        let total_suboperations = u16::try_from(items.len()).map_err(|_| {
            DcmError::Other(format!(
                "retrieval contains {} items; DIMSE counters support at most {}",
                items.len(),
                u16::MAX
            ))
        })?;
        let contexts = items
            .iter()
            .map(|item| RetrievePresentationContext {
                sop_class_uid: item.sop_class_uid.clone(),
                transfer_syntax_uid: item.transfer_syntax_uid.clone(),
            })
            .collect();
        Self::new(
            total_suboperations,
            contexts,
            Box::pin(stream::iter(
                items.into_iter().map(RetrieveSubOperation::Ready).map(Ok),
            )),
        )
    }

    /// Return the declared number of C-STORE sub-operations.
    pub fn total_suboperations(&self) -> u16 {
        self.total_suboperations
    }

    /// Return the candidate storage presentation contexts.
    pub fn presentation_contexts(&self) -> &[RetrievePresentationContext] {
        &self.presentation_contexts
    }

    /// Attach a callback invoked exactly once when execution finishes or aborts.
    pub fn with_completion_callback(
        mut self,
        callback: impl FnOnce(RetrieveCompletion) + Send + 'static,
    ) -> Self {
        self.completion_callback = Some(Box::new(callback));
        self
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        u16,
        Vec<RetrievePresentationContext>,
        StreamingRetrieveItemStream,
        Option<RetrieveCompletionCallback>,
    ) {
        (
            self.total_suboperations,
            self.presentation_contexts,
            self.items,
            self.completion_callback,
        )
    }
}

/// Build and validate the Cartesian product of SOP Classes and transfer syntaxes.
///
/// Product-specific defaults remain the caller's responsibility.
pub fn build_retrieve_presentation_contexts(
    sop_class_uids: &[String],
    transfer_syntax_uids: &[String],
) -> DcmResult<Vec<RetrievePresentationContext>> {
    let mut contexts = Vec::with_capacity(
        sop_class_uids
            .len()
            .saturating_mul(transfer_syntax_uids.len()),
    );
    for sop_class_uid in sop_class_uids {
        for transfer_syntax_uid in transfer_syntax_uids {
            contexts.push(RetrievePresentationContext::new(
                sop_class_uid.clone(),
                transfer_syntax_uid.clone(),
            )?);
        }
    }
    validate_retrieve_presentation_contexts(&mut contexts)?;
    Ok(contexts)
}

fn validate_retrieve_presentation_contexts(
    contexts: &mut Vec<RetrievePresentationContext>,
) -> DcmResult<()> {
    for context in contexts.iter() {
        Uid::new(context.sop_class_uid.clone())?;
        Uid::new(context.transfer_syntax_uid.clone())?;
    }
    contexts.sort();
    contexts.dedup();
    if contexts.len() > 128 {
        return Err(DcmError::Other(format!(
            "retrieval requires {} presentation contexts; DICOM allows at most 128",
            contexts.len()
        )));
    }
    Ok(())
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

/// Streaming C-GET provider with bounded dataset sources and per-item failures.
pub trait StreamingGetServiceProvider: Send + Sync + 'static {
    /// Called when the server receives a C-GET-RQ.
    fn on_get_stream(
        &self,
        event: GetEvent,
    ) -> impl std::future::Future<Output = DcmResult<GetRetrievePlan>> + Send;
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
/// Return a `Vec<RetrieveItem>` of instances to forward to the move destination
/// via a sub-association.
pub trait MoveServiceProvider: Send + Sync + 'static {
    /// Called when the server receives a C-MOVE-RQ.
    fn on_move(
        &self,
        event: MoveEvent,
    ) -> impl std::future::Future<Output = Vec<RetrieveItem>> + Send;
}

/// Streaming C-MOVE provider with an explicit destination negotiation plan.
pub trait StreamingMoveServiceProvider: Send + Sync + 'static {
    /// Called when the server receives a C-MOVE-RQ.
    fn on_move_stream(
        &self,
        event: MoveEvent,
    ) -> impl std::future::Future<Output = DcmResult<MoveRetrievePlan>> + Send;
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

    #[test]
    fn retrieve_plan_deduplicates_presentation_contexts() {
        let items = vec![
            StreamingRetrieveItem {
                sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                sop_instance_uid: "1.2.3.1".to_string(),
                transfer_syntax_uid: "1.2.840.10008.1.2.1".to_string(),
                dataset: Vec::new().into(),
            },
            StreamingRetrieveItem {
                sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".to_string(),
                sop_instance_uid: "1.2.3.2".to_string(),
                transfer_syntax_uid: "1.2.840.10008.1.2.1".to_string(),
                dataset: Vec::new().into(),
            },
        ];

        let plan = MoveRetrievePlan::from_items(items).expect("valid retrieval plan");

        assert_eq!(plan.total_suboperations(), 2);
        assert_eq!(plan.presentation_contexts().len(), 1);
    }

    #[test]
    fn retrieve_plan_rejects_invalid_uids() {
        let item = StreamingRetrieveItem {
            sop_class_uid: "not-a-uid".to_string(),
            sop_instance_uid: "1.2.3".to_string(),
            transfer_syntax_uid: "1.2.840.10008.1.2.1".to_string(),
            dataset: Vec::new().into(),
        };

        let result = MoveRetrievePlan::from_items(vec![item]);

        assert!(matches!(result, Err(DcmError::InvalidUid { .. })));
    }
}
