//! DICOM association state machine.
//!
//! Implements the SCU (request) and SCP (accept) sides of an association
//! as defined in PS3.8 §7 and §9.

use std::net::SocketAddr;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

use dicom_toolkit_core::error::{DcmError, DcmResult};
use dicom_toolkit_data::DataSet;

use crate::config::AssociationConfig;
use crate::dimse;
use crate::pdu::{
    self, AAbort, AssociateAc, AssociateRq, Pdu, Pdv, PresentationContextAcItem,
    PresentationContextRqItem,
};
use crate::presentation::{PcResult, PresentationContextAc, PresentationContextRq};

// ── Well-known transfer syntax UIDs ──────────────────────────────────────────

const TS_IMPLICIT_VR_LE: &str = "1.2.840.10008.1.2";
const TS_EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";
const APP_CONTEXT_UID: &str = "1.2.840.10008.3.1.1.1";

// ── AssociationState ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AssociationState {
    #[allow(dead_code)]
    Idle,
    #[allow(dead_code)]
    RequestSent,
    Established,
    ReleaseRequested,
    Closed,
}

// ── Association ───────────────────────────────────────────────────────────────

/// An established DICOM association over a TCP connection.
pub struct Association {
    stream: TcpStream,
    state: AssociationState,
    /// AE title of the remote called entity.
    pub called_ae: String,
    /// AE title of the initiating (calling) entity.
    pub calling_ae: String,
    /// Negotiated presentation contexts.
    pub presentation_contexts: Vec<PresentationContextAc>,
    /// Maximum PDU length negotiated with the peer.
    pub max_pdu_length: u32,
    /// Remote socket address.
    pub peer_addr: SocketAddr,
}

impl Association {
    // ── SCU side ─────────────────────────────────────────────────────────────

    /// Connect to `addr` and perform the A-ASSOCIATE-RQ / AC handshake.
    ///
    /// `addr` may be a `"host:port"` string accepted by `TcpStream::connect`.
    pub async fn request(
        addr: &str,
        called_ae: &str,
        calling_ae: &str,
        contexts: &[PresentationContextRq],
        config: &AssociationConfig,
    ) -> DcmResult<Self> {
        let stream = TcpStream::connect(addr).await?;
        let peer_addr = stream.peer_addr()?;
        let mut stream = stream;

        let rq = AssociateRq {
            called_ae_title: called_ae.to_string(),
            calling_ae_title: calling_ae.to_string(),
            application_context: APP_CONTEXT_UID.to_string(),
            presentation_contexts: contexts
                .iter()
                .map(|pc| PresentationContextRqItem {
                    id: pc.id,
                    abstract_syntax: pc.abstract_syntax.clone(),
                    transfer_syntaxes: pc.transfer_syntaxes.clone(),
                })
                .collect(),
            max_pdu_length: config.max_pdu_length,
            implementation_class_uid: config.implementation_class_uid.clone(),
            implementation_version_name: config.implementation_version_name.clone(),
        };

        stream.write_all(&pdu::encode_associate_rq(&rq)).await?;

        let response = timeout(
            Duration::from_secs(config.dimse_timeout_secs),
            pdu::read_pdu(&mut stream),
        )
        .await
        .map_err(|_| DcmError::Timeout {
            seconds: config.dimse_timeout_secs,
        })??;

        match response {
            Pdu::AssociateAc(ac) => {
                // Map raw AC items back to our typed PresentationContextAc,
                // joining with the original abstract syntaxes from the RQ.
                let pcs = ac
                    .presentation_contexts
                    .iter()
                    .map(|ac_item| {
                        let abs = contexts
                            .iter()
                            .find(|rq| rq.id == ac_item.id)
                            .map(|rq| rq.abstract_syntax.clone())
                            .unwrap_or_default();
                        PresentationContextAc {
                            id: ac_item.id,
                            result: PcResult::from_u8(ac_item.result),
                            transfer_syntax: ac_item.transfer_syntax.clone(),
                            abstract_syntax: abs,
                        }
                    })
                    .collect();

                Ok(Association {
                    stream,
                    state: AssociationState::Established,
                    called_ae: called_ae.to_string(),
                    calling_ae: calling_ae.to_string(),
                    presentation_contexts: pcs,
                    max_pdu_length: ac.max_pdu_length,
                    peer_addr,
                })
            }
            Pdu::AssociateRj(rj) => Err(DcmError::AssociationRejected {
                reason: format!(
                    "result={}, source={}, reason={}",
                    rj.result, rj.source, rj.reason
                ),
            }),
            _ => Err(DcmError::Other(
                "unexpected PDU type during association negotiation".into(),
            )),
        }
    }

    // ── SCP side ─────────────────────────────────────────────────────────────

    /// Accept an incoming TCP connection and complete the A-ASSOCIATE-AC
    /// handshake according to `config`.
    pub async fn accept(stream: TcpStream, config: &AssociationConfig) -> DcmResult<Self> {
        let peer_addr = stream.peer_addr()?;
        let mut stream = stream;

        let incoming = timeout(
            Duration::from_secs(config.dimse_timeout_secs),
            pdu::read_pdu(&mut stream),
        )
        .await
        .map_err(|_| DcmError::Timeout {
            seconds: config.dimse_timeout_secs,
        })??;

        let rq = match incoming {
            Pdu::AssociateRq(rq) => rq,
            _ => {
                return Err(DcmError::Other(
                    "expected A-ASSOCIATE-RQ as first PDU".into(),
                ))
            }
        };

        // Negotiate each proposed presentation context
        let mut accepted_pcs: Vec<PresentationContextAc> = Vec::new();
        let mut ac_items: Vec<PresentationContextAcItem> = Vec::new();

        for pc in &rq.presentation_contexts {
            let (result_byte, ts) = negotiate_pc(pc, config);
            ac_items.push(PresentationContextAcItem {
                id: pc.id,
                result: result_byte,
                transfer_syntax: ts.clone(),
            });
            if result_byte == 0 {
                accepted_pcs.push(PresentationContextAc {
                    id: pc.id,
                    result: PcResult::Acceptance,
                    transfer_syntax: ts,
                    abstract_syntax: pc.abstract_syntax.clone(),
                });
            }
        }

        let app_ctx = if rq.application_context.is_empty() {
            APP_CONTEXT_UID.to_string()
        } else {
            rq.application_context.clone()
        };

        let ac = AssociateAc {
            called_ae_title: rq.called_ae_title.clone(),
            calling_ae_title: rq.calling_ae_title.clone(),
            application_context: app_ctx,
            presentation_contexts: ac_items,
            max_pdu_length: config.max_pdu_length,
            implementation_class_uid: config.implementation_class_uid.clone(),
            implementation_version_name: config.implementation_version_name.clone(),
        };

        stream.write_all(&pdu::encode_associate_ac(&ac)).await?;

        Ok(Association {
            stream,
            state: AssociationState::Established,
            called_ae: rq.called_ae_title,
            calling_ae: rq.calling_ae_title,
            presentation_contexts: accepted_pcs,
            max_pdu_length: config.max_pdu_length,
            peer_addr,
        })
    }

    // ── P-DATA transfer ───────────────────────────────────────────────────────

    /// Send data as one or more P-DATA-TF PDUs.
    ///
    /// Large payloads are automatically fragmented to fit within
    /// `max_pdu_length`.  The `is_last` flag is set on the final fragment.
    pub async fn send_pdata(
        &mut self,
        context_id: u8,
        data: &[u8],
        is_command: bool,
        is_last: bool,
    ) -> DcmResult<()> {
        self.ensure_established()?;

        // PDU overhead: 6-byte PDU header + 4-byte PDV item-length + 2-byte PDV header
        const PDU_OVERHEAD: usize = 12;
        let max_data = (self.max_pdu_length as usize)
            .saturating_sub(PDU_OVERHEAD)
            .max(1); // never divide into 0-byte fragments

        let send_empty = data.is_empty();
        let chunks: Vec<&[u8]> = if send_empty {
            vec![&[]]
        } else {
            data.chunks(max_data).collect()
        };

        let n = chunks.len();
        for (i, chunk) in chunks.iter().enumerate() {
            let last_fragment = is_last && (i == n - 1);
            let mut ctrl: u8 = 0;
            if last_fragment {
                ctrl |= 0x01;
            }
            if is_command {
                ctrl |= 0x02;
            }
            let pdv = Pdv {
                context_id,
                msg_control: ctrl,
                data: chunk.to_vec(),
            };
            self.stream
                .write_all(&pdu::encode_p_data_tf(&[pdv]))
                .await?;
        }
        Ok(())
    }

    /// Receive the next P-DATA PDV.
    ///
    /// Returns `(context_id, is_command, is_last, data)`.
    /// Handles A-ABORT and A-RELEASE-RQ from the peer transparently.
    pub async fn recv_pdata(&mut self) -> DcmResult<(u8, bool, bool, Vec<u8>)> {
        self.ensure_established()?;

        loop {
            let pdu = pdu::read_pdu(&mut self.stream).await?;

            match pdu {
                Pdu::PDataTf(pd) => {
                    if let Some(pdv) = pd.pdvs.into_iter().next() {
                        return Ok((pdv.context_id, pdv.is_command(), pdv.is_last(), pdv.data));
                    }
                    // empty P-DATA-TF — keep waiting
                }
                Pdu::AAbort(abort) => {
                    self.state = AssociationState::Closed;
                    return Err(DcmError::AssociationAborted {
                        abort_source: abort.source.to_string(),
                        reason: abort.reason.to_string(),
                    });
                }
                Pdu::ReleaseRq => {
                    // Respond with A-RELEASE-RP then close
                    let _ = self.stream.write_all(&pdu::encode_release_rp()).await;
                    self.state = AssociationState::Closed;
                    return Err(DcmError::Other("association released by peer".into()));
                }
                _ => {} // ignore other unexpected PDUs
            }
        }
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Gracefully release the association (SCU-initiated A-RELEASE).
    pub async fn release(&mut self) -> DcmResult<()> {
        if self.state != AssociationState::Established {
            return Ok(());
        }
        self.state = AssociationState::ReleaseRequested;
        self.stream.write_all(&pdu::encode_release_rq()).await?;

        // Wait for A-RELEASE-RP with a short timeout
        let result = timeout(Duration::from_secs(30), pdu::read_pdu(&mut self.stream)).await;

        self.state = AssociationState::Closed;

        match result {
            Ok(Ok(Pdu::ReleaseRp)) | Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Ok(()), // timeout during release is acceptable
        }
    }

    /// Immediately abort the association by sending an A-ABORT PDU.
    pub async fn abort(&mut self) -> DcmResult<()> {
        let _ = self
            .stream
            .write_all(&pdu::encode_a_abort(&AAbort {
                source: 0,
                reason: 0,
            }))
            .await;
        self.state = AssociationState::Closed;
        Ok(())
    }

    // ── Presentation context lookup ───────────────────────────────────────────

    /// Find an accepted presentation context by its Abstract Syntax UID.
    pub fn find_context(&self, abstract_syntax: &str) -> Option<&PresentationContextAc> {
        self.presentation_contexts
            .iter()
            .find(|pc| pc.result.is_accepted() && pc.abstract_syntax == abstract_syntax)
    }

    /// Find a presentation context by its context ID.
    pub fn context_by_id(&self, id: u8) -> Option<&PresentationContextAc> {
        self.presentation_contexts.iter().find(|pc| pc.id == id)
    }

    // ── DIMSE helpers ─────────────────────────────────────────────────────────

    /// Encode and send a DIMSE command dataset as command PDVs.
    pub async fn send_dimse_command(&mut self, context_id: u8, command: &DataSet) -> DcmResult<()> {
        let bytes = dimse::encode_command_dataset(command);
        self.send_pdata(context_id, &bytes, true, true).await
    }

    /// Send pre-encoded DIMSE data (e.g. an SOP instance) as data PDVs.
    pub async fn send_dimse_data(&mut self, context_id: u8, data: &[u8]) -> DcmResult<()> {
        self.send_pdata(context_id, data, false, true).await
    }

    /// Collect command PDVs until the last fragment and decode them.
    ///
    /// Returns `(context_id, command_dataset)`.
    pub async fn recv_dimse_command(&mut self) -> DcmResult<(u8, DataSet)> {
        let mut all_data: Vec<u8> = Vec::new();
        // Initialised to 0; overwritten by the first command PDV received.
        #[allow(unused_assignments)]
        let mut ctx_id = 0u8;

        loop {
            let (cid, is_cmd, is_last, data) = self.recv_pdata().await?;
            if is_cmd {
                ctx_id = cid;
                all_data.extend_from_slice(&data);
                if is_last {
                    break;
                }
            }
            // Non-command PDVs are skipped — they should not appear before the
            // command is complete, but we handle them defensively.
        }

        let ds = dimse::decode_command_dataset(&all_data)?;
        Ok((ctx_id, ds))
    }

    /// Collect data PDVs until the last fragment and return the raw bytes.
    pub async fn recv_dimse_data(&mut self) -> DcmResult<Vec<u8>> {
        let mut all_data: Vec<u8> = Vec::new();

        loop {
            let (_, is_cmd, is_last, data) = self.recv_pdata().await?;
            if !is_cmd {
                all_data.extend_from_slice(&data);
                if is_last {
                    break;
                }
            }
        }
        Ok(all_data)
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn ensure_established(&self) -> DcmResult<()> {
        if self.state != AssociationState::Established {
            return Err(DcmError::Other(
                "operation requires an established association".into(),
            ));
        }
        Ok(())
    }
}

// ── SCP negotiation helpers ───────────────────────────────────────────────────

/// Decide whether to accept a proposed presentation context and which TS to use.
///
/// Returns `(result_byte, accepted_transfer_syntax)`.
fn negotiate_pc(pc: &PresentationContextRqItem, config: &AssociationConfig) -> (u8, String) {
    // Check abstract syntax acceptability
    if !config.accepted_abstract_syntaxes.is_empty()
        && !config
            .accepted_abstract_syntaxes
            .iter()
            .any(|a| a == &pc.abstract_syntax)
    {
        return (3, TS_IMPLICIT_VR_LE.to_string()); // abstract syntax not supported
    }

    let ts = choose_ts(&pc.transfer_syntaxes, config);
    match ts {
        Some(t) => (0, t),
        None => (4, TS_IMPLICIT_VR_LE.to_string()), // transfer syntaxes not supported
    }
}

/// Choose the best transfer syntax from an offered list based on config policy.
fn choose_ts(offered: &[String], config: &AssociationConfig) -> Option<String> {
    if config.accept_all_transfer_syntaxes {
        return offered.first().cloned();
    }
    // Prefer Explicit VR LE, then Implicit VR LE
    for preferred in &[TS_EXPLICIT_VR_LE, TS_IMPLICIT_VR_LE] {
        if offered.iter().any(|ts| ts == preferred) {
            return Some(preferred.to_string());
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiate_pc_accept_all() {
        let config = AssociationConfig {
            accept_all_transfer_syntaxes: true,
            ..Default::default()
        };

        let pc = PresentationContextRqItem {
            id: 1,
            abstract_syntax: "1.2.840.10008.1.1".to_string(),
            transfer_syntaxes: vec!["1.2.840.10008.1.2".to_string()],
        };
        let (result, ts) = negotiate_pc(&pc, &config);
        assert_eq!(result, 0);
        assert_eq!(ts, "1.2.840.10008.1.2");
    }

    #[test]
    fn negotiate_pc_prefer_explicit_le() {
        let config = AssociationConfig::default();
        let pc = PresentationContextRqItem {
            id: 1,
            abstract_syntax: "1.2.840.10008.1.1".to_string(),
            transfer_syntaxes: vec![TS_IMPLICIT_VR_LE.to_string(), TS_EXPLICIT_VR_LE.to_string()],
        };
        let (result, ts) = negotiate_pc(&pc, &config);
        assert_eq!(result, 0);
        assert_eq!(ts, TS_EXPLICIT_VR_LE);
    }

    #[test]
    fn negotiate_pc_unsupported_ts() {
        let config = AssociationConfig::default(); // accept_all = false
        let pc = PresentationContextRqItem {
            id: 1,
            abstract_syntax: "1.2.840.10008.1.1".to_string(),
            transfer_syntaxes: vec!["1.2.840.10008.1.2.4.50".to_string()], // JPEG Baseline only
        };
        let (result, _) = negotiate_pc(&pc, &config);
        assert_eq!(result, 4); // transfer syntaxes not supported
    }

    #[test]
    fn negotiate_pc_unsupported_abstract_syntax() {
        let config = AssociationConfig {
            accepted_abstract_syntaxes: vec!["1.2.840.10008.1.1".to_string()],
            ..Default::default()
        };

        let pc = PresentationContextRqItem {
            id: 1,
            abstract_syntax: "1.2.840.10008.5.1.4.1.1.2".to_string(), // CT Image Storage
            transfer_syntaxes: vec![TS_EXPLICIT_VR_LE.to_string()],
        };
        let (result, _) = negotiate_pc(&pc, &config);
        assert_eq!(result, 3); // abstract syntax not supported
    }

    // ── Loopback integration test ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_echo_loopback() {
        use crate::services::echo::c_echo;
        use dicom_toolkit_dict::tags;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // ── SCP task ────────────────────────────────────────────────────────
        let scp_handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let config = AssociationConfig {
                accept_all_transfer_syntaxes: true,
                ..Default::default()
            };

            let mut assoc = Association::accept(stream, &config).await.unwrap();

            // Receive C-ECHO-RQ
            let (ctx_id, cmd) = assoc.recv_dimse_command().await.unwrap();
            let msg_id = cmd.get_u16(tags::MESSAGE_ID).unwrap_or(1);

            // Build and send C-ECHO-RSP
            let mut rsp = DataSet::new();
            rsp.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.1.1");
            rsp.set_u16(tags::COMMAND_FIELD, 0x8030); // C-ECHO-RSP
            rsp.set_u16(tags::MESSAGE_ID_BEING_RESPONDED_TO, msg_id);
            rsp.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
            rsp.set_u16(tags::STATUS, 0x0000);
            assoc.send_dimse_command(ctx_id, &rsp).await.unwrap();

            // Absorb the incoming A-RELEASE-RQ and respond automatically
            let _ = assoc.recv_pdata().await;
        });

        // ── SCU side ────────────────────────────────────────────────────────
        let config = AssociationConfig::default();
        let contexts = vec![PresentationContextRq {
            id: 1,
            abstract_syntax: "1.2.840.10008.1.1".to_string(),
            transfer_syntaxes: vec![TS_EXPLICIT_VR_LE.to_string()],
        }];

        let mut assoc = Association::request(&addr.to_string(), "SCP", "SCU", &contexts, &config)
            .await
            .unwrap();

        let ctx_id = assoc
            .find_context("1.2.840.10008.1.1")
            .expect("context not found")
            .id;

        c_echo(&mut assoc, ctx_id).await.unwrap();
        assoc.release().await.unwrap();

        scp_handle.await.unwrap();
    }
}
