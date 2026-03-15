//! Logging setup for dcmtk-rs.
//!
//! Re-exports `tracing` macros and provides initialization helpers, replacing
//! DCMTK's `oflog` module (log4cplus wrapper).
//!
//! # Usage
//!
//! ```rust
//! use dicom_toolkit_core::log::init_logging;
//! use tracing::{info, debug};
//!
//! init_logging();
//! info!("DICOM toolkit initialized");
//! debug!(tag = "0008,0010", "reading element");
//! ```

pub use tracing::{debug, error, info, trace, warn};

/// Initializes the default logging subscriber.
///
/// Uses the `RUST_LOG` environment variable for filtering (e.g.,
/// `RUST_LOG=dicom_toolkit_data=debug,dicom_toolkit_net=trace`).
///
/// Call this once at application startup. Library code should not call this.
pub fn init_logging() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .init();
}

/// Initializes logging for tests (with test output capture support).
pub fn init_test_logging() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::TRACE)
        .try_init();
}
