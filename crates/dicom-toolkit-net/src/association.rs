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
    /// Maximum PDU length the peer advertised for inbound PDUs we send.
    pub max_pdu_length: u32,
    /// Remote socket address.
    pub peer_addr: SocketAddr,
    /// Buffered PDVs from the most recently read P-DATA-TF PDU.
    ///
    /// DICOM PS3.8 §9.3.4 allows multiple PDVs per P-DATA-TF.
    /// The original DCMTK C++ buffers all PDVs in the DUL layer
    /// (`PRIVATE_ASSOCIATIONKEY::pdvIndex/pdvCount`). This queue
    /// replicates that behaviour so that no PDVs are silently lost.
    pdv_queue: std::collections::VecDeque<pdu::Pdv>,
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
                    pdv_queue: std::collections::VecDeque::new(),
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
            max_pdu_length: rq.max_pdu_length,
            peer_addr,
            pdv_queue: std::collections::VecDeque::new(),
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

        let max_data = max_pdv_data_length(self.max_pdu_length, data.len());

        let send_empty = data.is_empty();
        let chunks: Vec<&[u8]> = if send_empty {
            vec![&[]]
        } else {
            data.chunks(max_data).collect()
        };

        let n = chunks.len();
        for (i, chunk) in chunks.iter().enumerate() {
            let last_fragment = is_last && (i == n - 1);
            // DICOM PS3.8 §9.3.1: bit 0 = command, bit 1 = last
            let mut ctrl: u8 = 0;
            if is_command {
                ctrl |= 0x01;
            }
            if last_fragment {
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
    ///
    /// When a P-DATA-TF contains multiple PDVs (allowed by DICOM PS3.8 §9.3.4),
    /// the remaining PDVs are buffered internally and returned by subsequent
    /// calls without additional network I/O — matching the DCMTK C++ DUL layer
    /// behaviour (`DUL_NextPDV` / `DUL_ReadPDVs`).
    pub async fn recv_pdata(&mut self) -> DcmResult<(u8, bool, bool, Vec<u8>)> {
        self.ensure_established()?;

        self.fill_pdv_queue().await?;

        if let Some(pdv) = self.pdv_queue.pop_front() {
            return Ok((pdv.context_id, pdv.is_command(), pdv.is_last(), pdv.data));
        }

        Err(DcmError::Other(
            "expected a P-DATA-TF PDU but none was available".into(),
        ))
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

    /// Collect data PDVs if present, but tolerate peers that immediately send the
    /// next DIMSE command instead of a dataset PDV.
    ///
    /// Returns:
    /// - `Ok(Some(bytes))` if one or more data PDVs were received
    /// - `Ok(None)` if the next queued PDV was another DIMSE command
    pub async fn recv_optional_dimse_data(&mut self) -> DcmResult<Option<Vec<u8>>> {
        self.ensure_established()?;
        let mut all_data = Vec::new();
        let mut saw_data_pdv = false;

        loop {
            self.fill_pdv_queue().await?;

            if self.pdv_queue.front().is_some_and(Pdv::is_command) {
                return if saw_data_pdv {
                    Ok(Some(all_data))
                } else {
                    Ok(None)
                };
            }

            let Some(pdv) = self.pdv_queue.pop_front() else {
                return if saw_data_pdv {
                    Ok(Some(all_data))
                } else {
                    Ok(None)
                };
            };

            saw_data_pdv = true;
            all_data.extend_from_slice(&pdv.data);
            if pdv.is_last() {
                return Ok(Some(all_data));
            }
        }
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

    async fn fill_pdv_queue(&mut self) -> DcmResult<()> {
        self.ensure_established()?;
        if !self.pdv_queue.is_empty() {
            return Ok(());
        }

        loop {
            let pdu = pdu::read_pdu(&mut self.stream).await?;

            match pdu {
                Pdu::PDataTf(pd) => {
                    if pd.pdvs.is_empty() {
                        continue;
                    }
                    self.pdv_queue.extend(pd.pdvs);
                    return Ok(());
                }
                Pdu::AAbort(abort) => {
                    self.state = AssociationState::Closed;
                    return Err(DcmError::AssociationAborted {
                        abort_source: abort.source.to_string(),
                        reason: abort.reason.to_string(),
                    });
                }
                Pdu::ReleaseRq => {
                    let _ = self.stream.write_all(&pdu::encode_release_rp()).await;
                    self.state = AssociationState::Closed;
                    return Err(DcmError::Other("association released by peer".into()));
                }
                _ => {}
            }
        }
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
        return choose_preferred_ts(offered, &config.preferred_transfer_syntaxes)
            .or_else(|| offered.first().cloned());
    }

    let allowed: Vec<&String> = if config.accepted_transfer_syntaxes.is_empty() {
        offered.iter().collect()
    } else {
        offered
            .iter()
            .filter(|ts| {
                config
                    .accepted_transfer_syntaxes
                    .iter()
                    .any(|allowed| allowed == *ts)
            })
            .collect()
    };

    if allowed.is_empty() {
        return None;
    }

    choose_preferred_ts_refs(&allowed, &config.preferred_transfer_syntaxes).or_else(|| {
        if config.accepted_transfer_syntaxes.is_empty() {
            choose_default_uncompressed_ts(&allowed)
        } else {
            allowed.first().map(|ts| (*ts).clone())
        }
    })
}

fn choose_preferred_ts(offered: &[String], preferred: &[String]) -> Option<String> {
    preferred.iter().find_map(|candidate| {
        offered
            .iter()
            .find(|offered_ts| *offered_ts == candidate)
            .cloned()
    })
}

fn choose_preferred_ts_refs(offered: &[&String], preferred: &[String]) -> Option<String> {
    preferred.iter().find_map(|candidate| {
        offered
            .iter()
            .find(|offered_ts| ***offered_ts == *candidate)
            .map(|ts| (*ts).clone())
    })
}

fn max_pdv_data_length(max_pdu_length: u32, data_len: usize) -> usize {
    const PDU_OVERHEAD: usize = 12;

    if max_pdu_length == 0 {
        return data_len.max(1);
    }

    (max_pdu_length as usize)
        .saturating_sub(PDU_OVERHEAD)
        .max(1)
}

fn choose_default_uncompressed_ts(offered: &[&String]) -> Option<String> {
    for preferred in &[TS_EXPLICIT_VR_LE, TS_IMPLICIT_VR_LE] {
        if let Some(ts) = offered
            .iter()
            .find(|offered_ts| ***offered_ts == *preferred)
        {
            return Some((*ts).clone());
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimse;
    use crate::pdu::{self, AssociateRq, Pdu, Pdv, PresentationContextRqItem};
    use dicom_toolkit_core::uid::sop_class;
    use tokio::{
        io::AsyncWriteExt,
        net::{TcpListener, TcpStream},
        sync::oneshot,
    };

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
    fn negotiate_pc_respects_accepted_transfer_syntaxes() {
        let config = AssociationConfig {
            accepted_transfer_syntaxes: vec!["1.2.840.10008.1.2.4.50".to_string()],
            ..Default::default()
        };
        let pc = PresentationContextRqItem {
            id: 1,
            abstract_syntax: "1.2.840.10008.1.1".to_string(),
            transfer_syntaxes: vec![
                TS_EXPLICIT_VR_LE.to_string(),
                "1.2.840.10008.1.2.4.50".to_string(),
            ],
        };
        let (result, ts) = negotiate_pc(&pc, &config);
        assert_eq!(result, 0);
        assert_eq!(ts, "1.2.840.10008.1.2.4.50");
    }

    #[test]
    fn negotiate_pc_prefers_custom_transfer_syntax_order() {
        let config = AssociationConfig {
            accepted_transfer_syntaxes: vec![
                TS_EXPLICIT_VR_LE.to_string(),
                "1.2.840.10008.1.2.4.50".to_string(),
            ],
            preferred_transfer_syntaxes: vec![
                "1.2.840.10008.1.2.4.50".to_string(),
                TS_EXPLICIT_VR_LE.to_string(),
            ],
            ..Default::default()
        };
        let pc = PresentationContextRqItem {
            id: 1,
            abstract_syntax: "1.2.840.10008.1.1".to_string(),
            transfer_syntaxes: vec![
                TS_EXPLICIT_VR_LE.to_string(),
                "1.2.840.10008.1.2.4.50".to_string(),
            ],
        };
        let (result, ts) = negotiate_pc(&pc, &config);
        assert_eq!(result, 0);
        assert_eq!(ts, "1.2.840.10008.1.2.4.50");
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

    fn find_context_item() -> PresentationContextRqItem {
        PresentationContextRqItem {
            id: 1,
            abstract_syntax: sop_class::PATIENT_ROOT_QR_FIND.to_string(),
            transfer_syntaxes: vec![TS_EXPLICIT_VR_LE.to_string()],
        }
    }

    fn associate_rq(max_pdu_length: u32) -> AssociateRq {
        AssociateRq {
            called_ae_title: "SCP".into(),
            calling_ae_title: "SCU".into(),
            application_context: APP_CONTEXT_UID.to_string(),
            presentation_contexts: vec![find_context_item()],
            max_pdu_length,
            implementation_class_uid: "1.2.826.0.1.3680043.8.498".into(),
            implementation_version_name: "TEST".into(),
        }
    }

    fn find_command(command_data_set_type: u16) -> DataSet {
        use dicom_toolkit_dict::tags;

        let mut cmd = DataSet::new();
        cmd.set_uid(
            tags::AFFECTED_SOP_CLASS_UID,
            sop_class::PATIENT_ROOT_QR_FIND,
        );
        cmd.set_u16(tags::COMMAND_FIELD, 0x0020);
        cmd.set_u16(tags::MESSAGE_ID, 1);
        cmd.set_u16(tags::PRIORITY, 0);
        cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, command_data_set_type);
        cmd
    }

    fn echo_command() -> DataSet {
        use dicom_toolkit_dict::tags;

        let mut cmd = DataSet::new();
        cmd.set_uid(tags::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.1.1");
        cmd.set_u16(tags::COMMAND_FIELD, 0x0030);
        cmd.set_u16(tags::MESSAGE_ID, 2);
        cmd.set_u16(tags::COMMAND_DATA_SET_TYPE, 0x0101);
        cmd
    }

    async fn connect_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("listener addr");
        let client = tokio::spawn(async move { TcpStream::connect(addr).await.expect("connect") });
        let (server, _) = listener.accept().await.expect("accept");
        let client = client.await.expect("join client task");
        (server, client)
    }

    #[test]
    fn max_pdv_data_length_honors_peer_limit() {
        assert_eq!(max_pdv_data_length(0, 128), 128);
        assert_eq!(max_pdv_data_length(16_384, 32_768), 16_372);
        assert_eq!(max_pdv_data_length(8, 64), 1);
    }

    #[tokio::test]
    async fn accept_uses_requestor_max_pdu_length_for_outbound_limit() {
        let (server_stream, mut client_stream) = connect_pair().await;
        let (done_tx, done_rx) = oneshot::channel();

        tokio::spawn(async move {
            let assoc = Association::accept(server_stream, &AssociationConfig::default())
                .await
                .expect("accept association");
            done_tx
                .send(assoc.max_pdu_length)
                .expect("send negotiated max pdu");
        });

        client_stream
            .write_all(&pdu::encode_associate_rq(&associate_rq(16_384)))
            .await
            .expect("send associate-rq");
        match pdu::read_pdu(&mut client_stream)
            .await
            .expect("read associate-ac")
        {
            Pdu::AssociateAc(_) => {}
            other => panic!("expected AssociateAc, got {other:?}"),
        }

        assert_eq!(done_rx.await.expect("receive max pdu"), 16_384);
    }

    #[tokio::test]
    async fn recv_optional_dimse_data_keeps_next_command_queued() {
        let (server_stream, mut client_stream) = connect_pair().await;

        let (done_tx, done_rx) = oneshot::channel();
        tokio::spawn(async move {
            let mut assoc = Association::accept(server_stream, &AssociationConfig::default())
                .await
                .expect("accept association");

            let (ctx_id, find_cmd) = assoc.recv_dimse_command().await.expect("receive command");
            assert_eq!(ctx_id, 1);
            assert_eq!(
                find_cmd.get_u16(dicom_toolkit_dict::tags::COMMAND_FIELD),
                Some(0x0020)
            );

            let query_bytes = assoc
                .recv_optional_dimse_data()
                .await
                .expect("receive optional query data");
            assert!(query_bytes.is_none());

            let (_, next_cmd) = assoc
                .recv_dimse_command()
                .await
                .expect("receive queued follow-up command");
            done_tx
                .send(next_cmd.get_u16(dicom_toolkit_dict::tags::COMMAND_FIELD))
                .expect("send command field");
        });

        client_stream
            .write_all(&pdu::encode_associate_rq(&associate_rq(16_384)))
            .await
            .expect("send associate-rq");
        match pdu::read_pdu(&mut client_stream)
            .await
            .expect("read associate-ac")
        {
            Pdu::AssociateAc(_) => {}
            other => panic!("expected AssociateAc, got {other:?}"),
        }

        let pdus = pdu::encode_p_data_tf(&[
            Pdv {
                context_id: 1,
                msg_control: 0x03,
                data: dimse::encode_command_dataset(&find_command(0x0000)),
            },
            Pdv {
                context_id: 1,
                msg_control: 0x03,
                data: dimse::encode_command_dataset(&echo_command()),
            },
        ]);
        client_stream
            .write_all(&pdus)
            .await
            .expect("send back-to-back commands");

        assert_eq!(done_rx.await.expect("receive next command"), Some(0x0030));
    }

    #[tokio::test]
    async fn recv_optional_dimse_data_tolerates_empty_data_pdv_before_next_command() {
        let (server_stream, mut client_stream) = connect_pair().await;

        let (done_tx, done_rx) = oneshot::channel();
        tokio::spawn(async move {
            let mut assoc = Association::accept(server_stream, &AssociationConfig::default())
                .await
                .expect("accept association");

            let (_, store_cmd) = assoc.recv_dimse_command().await.expect("receive command");
            assert_eq!(
                store_cmd.get_u16(dicom_toolkit_dict::tags::COMMAND_FIELD),
                Some(0x0001)
            );

            let data = assoc
                .recv_optional_dimse_data()
                .await
                .expect("receive optional store data");
            let (_, next_cmd) = assoc
                .recv_dimse_command()
                .await
                .expect("receive queued follow-up command");

            done_tx
                .send((
                    data,
                    next_cmd.get_u16(dicom_toolkit_dict::tags::COMMAND_FIELD),
                ))
                .expect("send result");
        });

        client_stream
            .write_all(&pdu::encode_associate_rq(&associate_rq(16_384)))
            .await
            .expect("send associate-rq");
        match pdu::read_pdu(&mut client_stream)
            .await
            .expect("read associate-ac")
        {
            Pdu::AssociateAc(_) => {}
            other => panic!("expected AssociateAc, got {other:?}"),
        }

        let mut store_cmd = DataSet::new();
        store_cmd.set_uid(
            dicom_toolkit_dict::tags::AFFECTED_SOP_CLASS_UID,
            sop_class::CT_IMAGE_STORAGE,
        );
        store_cmd.set_u16(dicom_toolkit_dict::tags::COMMAND_FIELD, 0x0001);
        store_cmd.set_u16(dicom_toolkit_dict::tags::MESSAGE_ID, 1);
        store_cmd.set_u16(dicom_toolkit_dict::tags::PRIORITY, 0);
        store_cmd.set_u16(dicom_toolkit_dict::tags::COMMAND_DATA_SET_TYPE, 0x0000);
        store_cmd.set_uid(
            dicom_toolkit_dict::tags::AFFECTED_SOP_INSTANCE_UID,
            "1.2.3.4.5",
        );

        let pdus = pdu::encode_p_data_tf(&[
            Pdv {
                context_id: 1,
                msg_control: 0x03,
                data: dimse::encode_command_dataset(&store_cmd),
            },
            Pdv {
                context_id: 1,
                msg_control: 0x00,
                data: Vec::new(),
            },
            Pdv {
                context_id: 1,
                msg_control: 0x03,
                data: dimse::encode_command_dataset(&echo_command()),
            },
        ]);
        client_stream
            .write_all(&pdus)
            .await
            .expect("send store command, empty data PDV, then next command");

        let (data, next_command_field) = done_rx.await.expect("receive result");
        assert_eq!(data, Some(Vec::new()));
        assert_eq!(next_command_field, Some(0x0030));
    }
}
