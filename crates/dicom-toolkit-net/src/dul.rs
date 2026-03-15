//! DUL (DICOM Upper Layer) transport.
//!
//! A thin layer over `tokio::net::TcpStream` that reads and writes PDUs.
//! Higher-level state management lives in [`crate::association`].

use tokio::net::TcpStream;
use dicom_toolkit_core::error::DcmResult;
use crate::pdu::{self, Pdu};

// ── DulTransport ──────────────────────────────────────────────────────────────

/// Wraps a `TcpStream` with PDU-level framing helpers.
pub struct DulTransport {
    pub(crate) stream: TcpStream,
}

impl DulTransport {
    pub fn new(stream: TcpStream) -> Self {
        Self { stream }
    }

    /// Read the next PDU from the underlying stream.
    pub async fn read_pdu(&mut self) -> DcmResult<Pdu> {
        pdu::read_pdu(&mut self.stream).await
    }

    /// Write a pre-encoded PDU byte buffer to the stream.
    pub async fn write_raw(&mut self, data: &[u8]) -> DcmResult<()> {
        pdu::write_pdu(&mut self.stream, data).await
    }
}
