//! Generic DICOM SCP server framework.
//!
//! [`DicomServer`] manages a TCP listener, concurrent association handling,
//! request routing to service providers, and graceful shutdown.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use dicom_toolkit_net::server::{DicomServer, FileStoreProvider};
//!
//! #[tokio::main]
//! async fn main() {
//!     let server = DicomServer::builder()
//!         .ae_title("MYPACS")
//!         .port(4242)
//!         .store_provider(FileStoreProvider::new("/tmp/dicom"))
//!         .build()
//!         .await
//!         .expect("bind port");
//!
//!     server.run().await.expect("server error");
//! }
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::{DataSet, FileFormat};
use dicom_toolkit_dict::tags;

use crate::association::Association;
use crate::config::{AssociationConfig, AssociationOptions};
use crate::services::find::{handle_find_rq, handle_streaming_find_rq};
use crate::services::get::{handle_get_rq, handle_streaming_get_rq};
use crate::services::provider::{
    DestinationLookup, FindEvent, FindResponseStream, FindServiceProvider, GetEvent,
    GetRetrievePlan, GetServiceProvider, MoveEvent, MoveRetrievePlan, MoveServiceProvider,
    RetrieveItem, StaticDestinationLookup, StoreEvent, StoreResult, StoreServiceProvider,
    StreamingFindServiceProvider, StreamingGetServiceProvider, StreamingMoveServiceProvider,
    STATUS_UNRECOGNISED_OPERATION,
};
use crate::services::r#move::{
    handle_move_rq, handle_streaming_move_rq_with_pending_response_interval,
};
use crate::services::store::handle_store_rq;

// ── Service registry ──────────────────────────────────────────────────────────

/// Holds optional provider implementations for each DIMSE service.
struct ServiceRegistry {
    store: Option<Arc<dyn AnyStoreProvider>>,
    find: Option<FindProviderRegistration>,
    get: Option<GetProviderRegistration>,
    r#move: Option<MoveProviderRegistration>,
    dest_lookup: Arc<dyn DestinationLookup>,
    local_ae: String,
}

// ── Type-erased provider wrappers ─────────────────────────────────────────────

// We need object-safe versions of the provider traits because they use
// `impl Future` return types which aren't object-safe. We use a small
// wrapper that boxes the futures.

trait AnyStoreProvider: Send + Sync + 'static {
    fn on_store<'a>(
        &'a self,
        event: StoreEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = StoreResult> + Send + 'a>>;
}

impl<P: StoreServiceProvider> AnyStoreProvider for P {
    fn on_store<'a>(
        &'a self,
        event: StoreEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = StoreResult> + Send + 'a>> {
        Box::pin(StoreServiceProvider::on_store(self, event))
    }
}

trait AnyFindProvider: Send + Sync + 'static {
    fn on_find<'a>(
        &'a self,
        event: FindEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<DataSet>> + Send + 'a>>;
}

impl<P: FindServiceProvider> AnyFindProvider for P {
    fn on_find<'a>(
        &'a self,
        event: FindEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<DataSet>> + Send + 'a>> {
        Box::pin(FindServiceProvider::on_find(self, event))
    }
}

trait AnyStreamingFindProvider: Send + Sync + 'static {
    fn on_find_stream<'a>(
        &'a self,
        event: FindEvent,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = DcmResult<FindResponseStream>> + Send + 'a>,
    >;
}

impl<P: StreamingFindServiceProvider> AnyStreamingFindProvider for P {
    fn on_find_stream<'a>(
        &'a self,
        event: FindEvent,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = DcmResult<FindResponseStream>> + Send + 'a>,
    > {
        Box::pin(StreamingFindServiceProvider::on_find_stream(self, event))
    }
}

trait AnyGetProvider: Send + Sync + 'static {
    fn on_get<'a>(
        &'a self,
        event: GetEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<RetrieveItem>> + Send + 'a>>;
}

impl<P: GetServiceProvider> AnyGetProvider for P {
    fn on_get<'a>(
        &'a self,
        event: GetEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<RetrieveItem>> + Send + 'a>> {
        Box::pin(GetServiceProvider::on_get(self, event))
    }
}

trait AnyStreamingGetProvider: Send + Sync + 'static {
    fn on_get_stream<'a>(
        &'a self,
        event: GetEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = DcmResult<GetRetrievePlan>> + Send + 'a>>;
}

impl<P: StreamingGetServiceProvider> AnyStreamingGetProvider for P {
    fn on_get_stream<'a>(
        &'a self,
        event: GetEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = DcmResult<GetRetrievePlan>> + Send + 'a>>
    {
        Box::pin(StreamingGetServiceProvider::on_get_stream(self, event))
    }
}

trait AnyMoveProvider: Send + Sync + 'static {
    fn on_move<'a>(
        &'a self,
        event: MoveEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<RetrieveItem>> + Send + 'a>>;
}

impl<P: MoveServiceProvider> AnyMoveProvider for P {
    fn on_move<'a>(
        &'a self,
        event: MoveEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<RetrieveItem>> + Send + 'a>> {
        Box::pin(MoveServiceProvider::on_move(self, event))
    }
}

trait AnyStreamingMoveProvider: Send + Sync + 'static {
    fn on_move_stream<'a>(
        &'a self,
        event: MoveEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = DcmResult<MoveRetrievePlan>> + Send + 'a>>;
}

impl<P: StreamingMoveServiceProvider> AnyStreamingMoveProvider for P {
    fn on_move_stream<'a>(
        &'a self,
        event: MoveEvent,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = DcmResult<MoveRetrievePlan>> + Send + 'a>>
    {
        Box::pin(StreamingMoveServiceProvider::on_move_stream(self, event))
    }
}

enum FindProviderRegistration {
    Legacy(Arc<dyn AnyFindProvider>),
    Streaming(Arc<dyn AnyStreamingFindProvider>),
}

enum GetProviderRegistration {
    Legacy(Arc<dyn AnyGetProvider>),
    Streaming(Arc<dyn AnyStreamingGetProvider>),
}

enum MoveProviderRegistration {
    Legacy(Arc<dyn AnyMoveProvider>),
    Streaming(Arc<dyn AnyStreamingMoveProvider>),
}

// ── Type-erased adapters for SCP handler functions ────────────────────────────

struct DynStoreAdapter(Arc<dyn AnyStoreProvider>);
impl StoreServiceProvider for DynStoreAdapter {
    async fn on_store(&self, event: StoreEvent) -> StoreResult {
        self.0.on_store(event).await
    }
}

struct DynFindAdapter(Arc<dyn AnyFindProvider>);
impl FindServiceProvider for DynFindAdapter {
    async fn on_find(&self, event: FindEvent) -> Vec<DataSet> {
        self.0.on_find(event).await
    }
}

struct DynStreamingFindAdapter(Arc<dyn AnyStreamingFindProvider>);
impl StreamingFindServiceProvider for DynStreamingFindAdapter {
    async fn on_find_stream(&self, event: FindEvent) -> DcmResult<FindResponseStream> {
        self.0.on_find_stream(event).await
    }
}

struct DynGetAdapter(Arc<dyn AnyGetProvider>);
impl GetServiceProvider for DynGetAdapter {
    async fn on_get(&self, event: GetEvent) -> Vec<RetrieveItem> {
        self.0.on_get(event).await
    }
}

struct DynStreamingGetAdapter(Arc<dyn AnyStreamingGetProvider>);
impl StreamingGetServiceProvider for DynStreamingGetAdapter {
    async fn on_get_stream(&self, event: GetEvent) -> DcmResult<GetRetrievePlan> {
        self.0.on_get_stream(event).await
    }
}

struct DynMoveAdapter(Arc<dyn AnyMoveProvider>);
impl MoveServiceProvider for DynMoveAdapter {
    async fn on_move(&self, event: MoveEvent) -> Vec<RetrieveItem> {
        self.0.on_move(event).await
    }
}

struct DynStreamingMoveAdapter(Arc<dyn AnyStreamingMoveProvider>);
impl StreamingMoveServiceProvider for DynStreamingMoveAdapter {
    async fn on_move_stream(&self, event: MoveEvent) -> DcmResult<MoveRetrievePlan> {
        self.0.on_move_stream(event).await
    }
}

// ── DicomServer ───────────────────────────────────────────────────────────────

/// A generic async DICOM SCP server.
///
/// Build with [`DicomServer::builder()`], then call [`DicomServer::run()`]
/// to accept and dispatch connections.  Graceful shutdown is achieved by
/// calling [`DicomServer::shutdown()`] from another task.
pub struct DicomServer {
    listener: TcpListener,
    registry: Arc<ServiceRegistry>,
    config: Arc<AssociationConfig>,
    association_options: Arc<AssociationOptions>,
    use_association_options: bool,
    move_destination_config: Arc<AssociationConfig>,
    move_destination_options: Arc<AssociationOptions>,
    streaming_move_pending_response_interval: Option<Duration>,
    streaming_move_destination_associations: usize,
    max_associations: usize,
    graceful_shutdown_timeout: Option<Duration>,
    token: CancellationToken,
}

impl DicomServer {
    /// Create a [`DicomServerBuilder`].
    pub fn builder() -> DicomServerBuilder {
        DicomServerBuilder::default()
    }

    /// Return the local address the server is listening on.
    pub fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.listener.local_addr()
    }

    /// Return a [`CancellationToken`] that, when cancelled, causes
    /// [`run()`](Self::run) to stop accepting new connections and return.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Stop accepting new connections.
    ///
    /// Active associations are detached by default. Configure
    /// [`DicomServerBuilder::graceful_shutdown_timeout`] to drain them for a
    /// bounded period instead.
    pub fn shutdown(&self) {
        self.token.cancel();
    }

    /// Run the server until [`shutdown()`](Self::shutdown) is called.
    ///
    /// Returns `Ok(())` when shutdown is clean, or an error if the listener
    /// fails.
    pub async fn run(self) -> DcmResult<()> {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_associations));
        let mut connection_tasks = tokio::task::JoinSet::new();
        info!(
            ae = %self.registry.local_ae,
            addr = ?self.listener.local_addr(),
            "DICOM server listening"
        );

        loop {
            tokio::select! {
                _ = self.token.cancelled() => {
                    info!("DICOM server shutting down");
                    break;
                }
                result = self.listener.accept() => {
                    match result {
                        Err(e) => {
                            error!("accept error: {}", e);
                            continue;
                        }
                        Ok((stream, peer_addr)) => {
                            let permit = match semaphore.clone().try_acquire_owned() {
                                Ok(p) => p,
                                Err(_) => {
                                    warn!(%peer_addr, "max associations reached, rejecting");
                                    drop(stream);
                                    continue;
                                }
                            };

                            let registry = Arc::clone(&self.registry);
                            let config = Arc::clone(&self.config);
                            let association_options = Arc::clone(&self.association_options);
                            let use_association_options = self.use_association_options;
                            let move_destination_config = Arc::clone(&self.move_destination_config);
                            let move_destination_options = Arc::clone(&self.move_destination_options);
                            let streaming_move_pending_response_interval =
                                self.streaming_move_pending_response_interval;
                            let streaming_move_destination_associations =
                                self.streaming_move_destination_associations;

                            connection_tasks.spawn(async move {
                                let _permit = permit;
                                match handle_connection(
                                    stream,
                                    &registry,
                                    ConnectionConfiguration {
                                        association: &config,
                                        association_options: &association_options,
                                        use_association_options,
                                        move_destination: &move_destination_config,
                                        move_destination_options: &move_destination_options,
                                        streaming_move_pending_response_interval,
                                        streaming_move_destination_associations,
                                    },
                                )
                                .await
                                {
                                    Ok(()) => {}
                                    Err(e) => {
                                        warn!(%peer_addr, "connection error: {}", e);
                                    }
                                }
                            });
                        }
                    }
                }
                result = connection_tasks.join_next(), if !connection_tasks.is_empty() => {
                    if let Some(Err(error)) = result {
                        warn!(%error, "DICOM connection task failed");
                    }
                }
            }
        }

        let Some(graceful_shutdown_timeout) = self.graceful_shutdown_timeout else {
            connection_tasks.detach_all();
            return Ok(());
        };

        let drain_connections = async {
            while let Some(result) = connection_tasks.join_next().await {
                if let Err(error) = result {
                    warn!(%error, "DICOM connection task failed while draining");
                }
            }
        };
        if tokio::time::timeout(graceful_shutdown_timeout, drain_connections)
            .await
            .is_err()
        {
            warn!(
                timeout_seconds = graceful_shutdown_timeout.as_secs(),
                "graceful shutdown timed out; aborting remaining associations"
            );
            connection_tasks.shutdown().await;
        }
        Ok(())
    }
}

// ── Connection handler ────────────────────────────────────────────────────────

struct ConnectionConfiguration<'a> {
    association: &'a AssociationConfig,
    association_options: &'a AssociationOptions,
    use_association_options: bool,
    move_destination: &'a AssociationConfig,
    move_destination_options: &'a AssociationOptions,
    streaming_move_pending_response_interval: Option<Duration>,
    streaming_move_destination_associations: usize,
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    registry: &ServiceRegistry,
    configuration: ConnectionConfiguration<'_>,
) -> DcmResult<()> {
    let peer = stream.peer_addr().ok();
    if let Some(addr) = peer {
        info!(%addr, "association accepted");
    }

    let mut assoc = if configuration.use_association_options {
        Association::accept_with_options(
            stream,
            configuration.association,
            configuration.association_options,
        )
        .await?
    } else {
        Association::accept(stream, configuration.association).await?
    };

    info!(
        peer_addr = %assoc.peer_addr,
        calling_ae = %assoc.calling_ae.trim(),
        called_ae = %assoc.called_ae.trim(),
        accepted_presentation_context_count = assoc.presentation_contexts.len(),
        negotiated_role_selection_count = assoc.role_selections.len(),
        maximum_peer_pdu_length = assoc.max_pdu_length,
        "association negotiation completed"
    );
    for context in &assoc.presentation_contexts {
        debug!(
            peer_addr = %assoc.peer_addr,
            calling_ae = %assoc.calling_ae.trim(),
            called_ae = %assoc.called_ae.trim(),
            presentation_context_id = context.id,
            abstract_syntax_uid = %context.abstract_syntax,
            transfer_syntax_uid = %context.transfer_syntax,
            local_scu_role = assoc.local_scu_role(&context.abstract_syntax),
            local_scp_role = assoc.local_scp_role(&context.abstract_syntax),
            "presentation context accepted"
        );
    }
    for role in &assoc.role_selections {
        debug!(
            peer_addr = %assoc.peer_addr,
            calling_ae = %assoc.calling_ae.trim(),
            called_ae = %assoc.called_ae.trim(),
            sop_class_uid = %role.sop_class_uid,
            requestor_scu_role = role.scu_role,
            requestor_scp_role = role.scp_role,
            "SCP/SCU role selection negotiated"
        );
    }

    loop {
        let (ctx_id, cmd) = match assoc.recv_dimse_command().await {
            Ok(c) => c,
            Err(_) => break,
        };

        let command_field = cmd.get_u16(tags::COMMAND_FIELD).unwrap_or(0);

        match command_field {
            // C-ECHO-RQ — always handled, no provider required.
            0x0030 => {
                let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);
                let sop_class = cmd
                    .get_string(tags::AFFECTED_SOP_CLASS_UID)
                    .unwrap_or("1.2.840.10008.1.1")
                    .trim_end_matches('\0')
                    .to_string();
                let mut rsp = DataSet::new();
                rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
                rsp.set_u16(tags::COMMAND_FIELD, 0x8030); // C-ECHO-RSP
                rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
                rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
                rsp.set_u16(tags::STATUS, 0x0000);
                assoc.send_dimse_command(ctx_id, &rsp).await?;
            }

            // C-STORE-RQ
            0x0001 => {
                if let Some(provider) = &registry.store {
                    let adapter = DynStoreAdapter(Arc::clone(provider));
                    handle_store_rq(&mut assoc, ctx_id, &cmd, &adapter).await?;
                } else {
                    send_refused(&mut assoc, ctx_id, &cmd, 0x8001).await?;
                }
            }

            // C-FIND-RQ
            0x0020 => {
                if let Some(provider) = &registry.find {
                    match provider {
                        FindProviderRegistration::Legacy(provider) => {
                            let adapter = DynFindAdapter(Arc::clone(provider));
                            handle_find_rq(&mut assoc, ctx_id, &cmd, &adapter).await?;
                        }
                        FindProviderRegistration::Streaming(provider) => {
                            let adapter = DynStreamingFindAdapter(Arc::clone(provider));
                            handle_streaming_find_rq(&mut assoc, ctx_id, &cmd, &adapter).await?;
                        }
                    }
                } else {
                    send_refused(&mut assoc, ctx_id, &cmd, 0x8020).await?;
                }
            }

            // C-GET-RQ
            0x0010 => {
                if let Some(provider) = &registry.get {
                    match provider {
                        GetProviderRegistration::Legacy(provider) => {
                            let adapter = DynGetAdapter(Arc::clone(provider));
                            handle_get_rq(&mut assoc, ctx_id, &cmd, &adapter).await?;
                        }
                        GetProviderRegistration::Streaming(provider) => {
                            let adapter = DynStreamingGetAdapter(Arc::clone(provider));
                            handle_streaming_get_rq(&mut assoc, ctx_id, &cmd, &adapter).await?;
                        }
                    }
                } else {
                    send_refused(&mut assoc, ctx_id, &cmd, 0x8010).await?;
                }
            }

            // C-MOVE-RQ
            0x0021 => {
                if let Some(provider) = &registry.r#move {
                    match provider {
                        MoveProviderRegistration::Legacy(provider) => {
                            let adapter = DynMoveAdapter(Arc::clone(provider));
                            handle_move_rq(
                                &mut assoc,
                                ctx_id,
                                &cmd,
                                &adapter,
                                registry.dest_lookup.as_ref(),
                                &registry.local_ae,
                            )
                            .await?;
                        }
                        MoveProviderRegistration::Streaming(provider) => {
                            let adapter = DynStreamingMoveAdapter(Arc::clone(provider));
                            handle_streaming_move_rq_with_pending_response_interval(
                                &mut assoc,
                                ctx_id,
                                &cmd,
                                &adapter,
                                registry.dest_lookup.as_ref(),
                                &registry.local_ae,
                                configuration.move_destination,
                                configuration.move_destination_options,
                                configuration.streaming_move_pending_response_interval,
                                configuration.streaming_move_destination_associations,
                            )
                            .await?;
                        }
                    }
                } else {
                    send_refused(&mut assoc, ctx_id, &cmd, 0x8021).await?;
                }
            }

            // A response from a C-STORE interrupted by C-CANCEL may arrive
            // after the final retrieve response. It is already accounted for.
            0x8001 | 0x0FFF => {
                warn!(command_field, "ignoring stale DIMSE response or cancel");
            }

            _ => {
                // Unrecognised command — send failure and continue.
                warn!(command_field, "unrecognised DIMSE command");
                break;
            }
        }
    }

    Ok(())
}

/// Send a failure response when no provider is registered for the service.
async fn send_refused(
    assoc: &mut Association,
    ctx_id: u8,
    cmd: &DataSet,
    rsp_command_field: u16,
) -> DcmResult<()> {
    let sop_class = cmd
        .get_string(tags::AFFECTED_SOP_CLASS_UID)
        .unwrap_or_default()
        .trim_end_matches('\0')
        .to_string();
    let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

    let mut rsp = DataSet::new();
    rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, &sop_class);
    rsp.set_u16(tags::COMMAND_FIELD, rsp_command_field);
    rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
    rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
    rsp.set_u16(tags::STATUS, STATUS_UNRECOGNISED_OPERATION);
    assoc.send_dimse_command(ctx_id, &rsp).await
}

// ── DicomServerBuilder ────────────────────────────────────────────────────────

/// Builder for [`DicomServer`].
pub struct DicomServerBuilder {
    ae_title: String,
    port: u16,
    max_associations: usize,
    graceful_shutdown_timeout: Option<Duration>,
    config: Option<AssociationConfig>,
    association_options: Option<AssociationOptions>,
    move_destination_config: Option<AssociationConfig>,
    move_destination_options: Option<AssociationOptions>,
    streaming_move_pending_response_interval: Option<Duration>,
    streaming_move_destination_associations: usize,
    store: Option<Arc<dyn AnyStoreProvider>>,
    find: Option<FindProviderRegistration>,
    get: Option<GetProviderRegistration>,
    r#move: Option<MoveProviderRegistration>,
    dest_lookup: Option<Arc<dyn DestinationLookup>>,
}

impl Default for DicomServerBuilder {
    fn default() -> Self {
        Self {
            ae_title: "DICOMRS".to_string(),
            port: 4242,
            max_associations: 100,
            graceful_shutdown_timeout: None,
            config: None,
            association_options: None,
            move_destination_config: None,
            move_destination_options: None,
            streaming_move_pending_response_interval: None,
            streaming_move_destination_associations: 1,
            store: None,
            find: None,
            get: None,
            r#move: None,
            dest_lookup: None,
        }
    }
}

impl DicomServerBuilder {
    /// Set the server's AE title.
    pub fn ae_title(mut self, ae: impl Into<String>) -> Self {
        self.ae_title = ae.into();
        self
    }

    /// Set the TCP port to listen on.
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Maximum number of simultaneous associations.
    pub fn max_associations(mut self, n: usize) -> Self {
        self.max_associations = n;
        self
    }

    /// Set how long shutdown waits for active associations before aborting them.
    ///
    /// Without this option, active association tasks remain detached on
    /// shutdown, preserving the legacy server behavior.
    pub fn graceful_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.graceful_shutdown_timeout = Some(timeout);
        self
    }

    /// Override the full [`AssociationConfig`].
    pub fn config(mut self, cfg: AssociationConfig) -> Self {
        self.config = Some(cfg);
        self
    }

    /// Configure bounded resources and extended association negotiation.
    pub fn association_options(mut self, options: AssociationOptions) -> Self {
        self.association_options = Some(options);
        self
    }

    /// Override the association configuration used for C-MOVE destinations.
    ///
    /// This is separate from the inbound SCP configuration because requestor
    /// role selections and asynchronous-operation proposals are directional.
    pub fn move_destination_config(mut self, config: AssociationConfig) -> Self {
        self.move_destination_config = Some(config);
        self
    }

    /// Configure bounded resources for C-MOVE destination associations.
    pub fn move_destination_options(mut self, options: AssociationOptions) -> Self {
        self.move_destination_options = Some(options);
        self
    }

    /// Set the interval between C-MOVE Pending responses while a streaming
    /// provider or an active C-STORE sub-operation makes no visible progress.
    ///
    /// This is disabled by default to preserve earlier behavior. A bounded
    /// interval keeps the request association active while a large object is
    /// downloaded or delivered over the separate Storage association.
    pub fn streaming_move_pending_response_interval(mut self, interval: Duration) -> Self {
        self.streaming_move_pending_response_interval = Some(interval);
        self
    }

    /// Set the maximum number of synchronous Storage associations used by one
    /// streaming C-MOVE operation.
    ///
    /// The default is one, preserving the serial behavior of earlier
    /// releases. Values greater than one allow bounded parallel C-STORE
    /// delivery when the destination accepts multiple associations.
    pub fn streaming_move_destination_associations(mut self, maximum: usize) -> Self {
        self.streaming_move_destination_associations = maximum;
        self
    }

    /// Register a C-STORE provider.
    pub fn store_provider(mut self, p: impl StoreServiceProvider) -> Self {
        self.store = Some(Arc::new(p));
        self
    }

    /// Register a C-FIND provider.
    pub fn find_provider(mut self, p: impl FindServiceProvider) -> Self {
        self.find = Some(FindProviderRegistration::Legacy(Arc::new(p)));
        self
    }

    /// Register a lazy, fallible C-FIND provider.
    pub fn streaming_find_provider(mut self, provider: impl StreamingFindServiceProvider) -> Self {
        self.find = Some(FindProviderRegistration::Streaming(Arc::new(provider)));
        self
    }

    /// Register a C-GET provider.
    pub fn get_provider(mut self, p: impl GetServiceProvider) -> Self {
        self.get = Some(GetProviderRegistration::Legacy(Arc::new(p)));
        self
    }

    /// Register a bounded-source C-GET provider.
    pub fn streaming_get_provider(mut self, provider: impl StreamingGetServiceProvider) -> Self {
        self.get = Some(GetProviderRegistration::Streaming(Arc::new(provider)));
        self
    }

    /// Register a C-MOVE provider.
    pub fn move_provider(mut self, p: impl MoveServiceProvider) -> Self {
        self.r#move = Some(MoveProviderRegistration::Legacy(Arc::new(p)));
        self
    }

    /// Register a bounded-source C-MOVE provider.
    pub fn streaming_move_provider(mut self, provider: impl StreamingMoveServiceProvider) -> Self {
        self.r#move = Some(MoveProviderRegistration::Streaming(Arc::new(provider)));
        self
    }

    /// Register a destination lookup for C-MOVE sub-associations.
    pub fn move_destination_lookup(mut self, l: impl DestinationLookup) -> Self {
        self.dest_lookup = Some(Arc::new(l));
        self
    }

    /// Build the [`DicomServer`], binding the TCP listener immediately.
    ///
    /// # Errors
    ///
    /// Returns an error if the port cannot be bound.
    pub async fn build(self) -> DcmResult<DicomServer> {
        if self
            .streaming_move_pending_response_interval
            .is_some_and(|interval| interval.is_zero())
        {
            return Err(dicom_toolkit_core::error::DcmError::Other(
                "streaming C-MOVE Pending response interval must be greater than zero".into(),
            ));
        }
        if self.streaming_move_destination_associations == 0 {
            return Err(dicom_toolkit_core::error::DcmError::Other(
                "streaming C-MOVE destination association count must be greater than zero".into(),
            ));
        }
        let ae = self.ae_title.clone();
        let config = self.config.unwrap_or_else(|| AssociationConfig {
            local_ae_title: ae.clone(),
            accept_all_transfer_syntaxes: true,
            ..Default::default()
        });

        let has_streaming_get = matches!(
            self.get.as_ref(),
            Some(GetProviderRegistration::Streaming(_))
        );
        let uses_streaming_provider = matches!(
            self.find.as_ref(),
            Some(FindProviderRegistration::Streaming(_))
        ) || has_streaming_get
            || matches!(
                self.r#move.as_ref(),
                Some(MoveProviderRegistration::Streaming(_))
            );
        let use_association_options = uses_streaming_provider || self.association_options.is_some();
        let mut association_options = self.association_options.unwrap_or_else(|| {
            if uses_streaming_provider {
                AssociationOptions::default()
            } else {
                AssociationOptions {
                    maximum_incoming_pdu_length: 0,
                    maximum_outgoing_pdu_length: 0,
                    ..AssociationOptions::default()
                }
            }
        });

        if association_options
            .maximum_asynchronous_operations_window
            .invoked
            != 1
            || association_options
                .maximum_asynchronous_operations_window
                .performed
                != 1
        {
            return Err(dicom_toolkit_core::error::DcmError::Other(
                "DicomServer currently supports only the synchronous asynchronous-operations window (1, 1)"
                    .into(),
            ));
        }

        if has_streaming_get {
            association_options.accept_requestor_scp_role = true;
        }

        let move_destination_config = self
            .move_destination_config
            .unwrap_or_else(|| config.clone());
        let move_destination_options = self.move_destination_options.unwrap_or_else(|| {
            let mut destination_options = association_options.clone();
            destination_options.requested_role_selections.clear();
            destination_options
        });

        let listener = TcpListener::bind(("0.0.0.0", self.port)).await?;

        let dest_lookup: Arc<dyn DestinationLookup> = self
            .dest_lookup
            .unwrap_or_else(|| Arc::new(StaticDestinationLookup::new(vec![])));

        let registry = Arc::new(ServiceRegistry {
            store: self.store,
            find: self.find,
            get: self.get,
            r#move: self.r#move,
            dest_lookup,
            local_ae: ae,
        });

        Ok(DicomServer {
            listener,
            registry,
            config: Arc::new(config),
            association_options: Arc::new(association_options),
            use_association_options,
            move_destination_config: Arc::new(move_destination_config),
            move_destination_options: Arc::new(move_destination_options),
            streaming_move_pending_response_interval: self.streaming_move_pending_response_interval,
            streaming_move_destination_associations: self.streaming_move_destination_associations,
            max_associations: self.max_associations,
            graceful_shutdown_timeout: self.graceful_shutdown_timeout,
            token: CancellationToken::new(),
        })
    }
}

// ── Built-in providers ────────────────────────────────────────────────────────

/// A ready-to-use [`StoreServiceProvider`] that saves received DICOM
/// instances as `.dcm` files in a given directory.
///
/// # Example
///
/// ```rust,no_run
/// use dicom_toolkit_net::server::{DicomServer, FileStoreProvider};
///
/// # async fn run() {
/// let server = DicomServer::builder()
///     .store_provider(FileStoreProvider::new("/tmp/dicom"))
///     .build()
///     .await
///     .unwrap();
/// # }
/// ```
pub struct FileStoreProvider {
    dir: PathBuf,
}

impl FileStoreProvider {
    /// Create a new `FileStoreProvider` that stores files in `dir`.
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }
}

impl StoreServiceProvider for FileStoreProvider {
    async fn on_store(&self, event: StoreEvent) -> StoreResult {
        let destination_directory = self.dir.clone();
        let StoreEvent {
            sop_class_uid,
            sop_instance_uid,
            dataset,
            ..
        } = event;

        let result = tokio::task::spawn_blocking(move || {
            let file_format = FileFormat::from_dataset(&sop_class_uid, &sop_instance_uid, dataset);
            let safe_instance_uid: String = sop_instance_uid
                .chars()
                .map(|character| {
                    if character.is_alphanumeric() || character == '.' {
                        character
                    } else {
                        '_'
                    }
                })
                .collect();
            let destination = destination_directory.join(format!("{safe_instance_uid}.dcm"));
            file_format.save(&destination).map(|()| destination)
        })
        .await;

        match result {
            Ok(Ok(destination)) => {
                info!(path = %destination.display(), "stored instance");
                StoreResult::success()
            }
            Ok(Err(error)) => {
                error!(%error, "failed to save instance");
                StoreResult::failure(crate::services::provider::STATUS_PROCESSING_FAILURE)
            }
            Err(error) => {
                error!(%error, "file store worker failed");
                StoreResult::failure(crate::services::provider::STATUS_PROCESSING_FAILURE)
            }
        }
    }
}
