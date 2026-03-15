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

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::{DataSet, FileFormat};
use dicom_toolkit_dict::tags;

use crate::association::Association;
use crate::config::AssociationConfig;
use crate::services::find::handle_find_rq;
use crate::services::get::handle_get_rq;
use crate::services::provider::{
    DestinationLookup, FindEvent, FindServiceProvider, GetEvent, GetServiceProvider, MoveEvent,
    MoveServiceProvider, RetrieveItem, StaticDestinationLookup, StoreEvent, StoreResult,
    StoreServiceProvider, STATUS_UNRECOGNISED_OPERATION,
};
use crate::services::r#move::handle_move_rq;
use crate::services::store::handle_store_rq;

// ── Service registry ──────────────────────────────────────────────────────────

/// Holds optional provider implementations for each DIMSE service.
struct ServiceRegistry {
    store: Option<Arc<dyn AnyStoreProvider>>,
    find: Option<Arc<dyn AnyFindProvider>>,
    get: Option<Arc<dyn AnyGetProvider>>,
    r#move: Option<Arc<dyn AnyMoveProvider>>,
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

struct DynGetAdapter(Arc<dyn AnyGetProvider>);
impl GetServiceProvider for DynGetAdapter {
    async fn on_get(&self, event: GetEvent) -> Vec<RetrieveItem> {
        self.0.on_get(event).await
    }
}

struct DynMoveAdapter(Arc<dyn AnyMoveProvider>);
impl MoveServiceProvider for DynMoveAdapter {
    async fn on_move(&self, event: MoveEvent) -> Vec<RetrieveItem> {
        self.0.on_move(event).await
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
    max_associations: usize,
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

    /// Stop the server gracefully.  In-flight associations complete normally.
    pub fn shutdown(&self) {
        self.token.cancel();
    }

    /// Run the server until [`shutdown()`](Self::shutdown) is called.
    ///
    /// Returns `Ok(())` when shutdown is clean, or an error if the listener
    /// fails.
    pub async fn run(self) -> DcmResult<()> {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_associations));
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

                            tokio::spawn(async move {
                                let _permit = permit;
                                match handle_connection(stream, &registry, &config).await {
                                    Ok(()) => {}
                                    Err(e) => {
                                        warn!(%peer_addr, "connection error: {}", e);
                                    }
                                }
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

// ── Connection handler ────────────────────────────────────────────────────────

async fn handle_connection(
    stream: tokio::net::TcpStream,
    registry: &ServiceRegistry,
    config: &AssociationConfig,
) -> DcmResult<()> {
    let peer = stream.peer_addr().ok();
    if let Some(addr) = peer {
        info!(%addr, "association accepted");
    }

    let mut assoc = Association::accept(stream, config).await?;

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
                    let adapter = DynFindAdapter(Arc::clone(provider));
                    handle_find_rq(&mut assoc, ctx_id, &cmd, &adapter).await?;
                } else {
                    send_refused(&mut assoc, ctx_id, &cmd, 0x8020).await?;
                }
            }

            // C-GET-RQ
            0x0010 => {
                if let Some(provider) = &registry.get {
                    let adapter = DynGetAdapter(Arc::clone(provider));
                    handle_get_rq(&mut assoc, ctx_id, &cmd, &adapter).await?;
                } else {
                    send_refused(&mut assoc, ctx_id, &cmd, 0x8010).await?;
                }
            }

            // C-MOVE-RQ
            0x0021 => {
                if let Some(provider) = &registry.r#move {
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
                } else {
                    send_refused(&mut assoc, ctx_id, &cmd, 0x8021).await?;
                }
            }

            _ => {
                // Unrecognised command — send failure and continue.
                warn!(command_field, "unrecognised DIMSE command");
                break;
            }
        }
    }

    let _ = assoc.release().await;
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
    config: Option<AssociationConfig>,
    store: Option<Arc<dyn AnyStoreProvider>>,
    find: Option<Arc<dyn AnyFindProvider>>,
    get: Option<Arc<dyn AnyGetProvider>>,
    r#move: Option<Arc<dyn AnyMoveProvider>>,
    dest_lookup: Option<Arc<dyn DestinationLookup>>,
}

impl Default for DicomServerBuilder {
    fn default() -> Self {
        Self {
            ae_title: "DICOMRS".to_string(),
            port: 4242,
            max_associations: 100,
            config: None,
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

    /// Override the full [`AssociationConfig`].
    pub fn config(mut self, cfg: AssociationConfig) -> Self {
        self.config = Some(cfg);
        self
    }

    /// Register a C-STORE provider.
    pub fn store_provider(mut self, p: impl StoreServiceProvider) -> Self {
        self.store = Some(Arc::new(p));
        self
    }

    /// Register a C-FIND provider.
    pub fn find_provider(mut self, p: impl FindServiceProvider) -> Self {
        self.find = Some(Arc::new(p));
        self
    }

    /// Register a C-GET provider.
    pub fn get_provider(mut self, p: impl GetServiceProvider) -> Self {
        self.get = Some(Arc::new(p));
        self
    }

    /// Register a C-MOVE provider.
    pub fn move_provider(mut self, p: impl MoveServiceProvider) -> Self {
        self.r#move = Some(Arc::new(p));
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
        let ae = self.ae_title.clone();
        let config = self.config.unwrap_or_else(|| AssociationConfig {
            local_ae_title: ae.clone(),
            accept_all_transfer_syntaxes: true,
            ..Default::default()
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
            max_associations: self.max_associations,
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
        let ff =
            FileFormat::from_dataset(&event.sop_class_uid, &event.sop_instance_uid, event.dataset);

        let safe: String = event
            .sop_instance_uid
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        let dest = self.dir.join(format!("{safe}.dcm"));

        match ff.save(&dest) {
            Ok(()) => {
                info!(path = %dest.display(), "stored instance");
                StoreResult::success()
            }
            Err(e) => {
                error!(path = %dest.display(), error = %e, "failed to save instance");
                StoreResult::failure(crate::services::provider::STATUS_PROCESSING_FAILURE)
            }
        }
    }
}
