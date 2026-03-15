//! DICOM Upper Layer Protocol PDU types, encoding, and decoding.
//!
//! Implements PS3.8 §9 — Upper Layer Service and Protocol.
//!
//! Every PDU is preceded by a 6-byte header:
//! ```text
//! [1B type][1B reserved=0][4B body-length (u32 BE)]
//! ```

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use dicom_toolkit_core::error::{DcmError, DcmResult};

// ── PDU type constants ────────────────────────────────────────────────────────

pub const PDU_ASSOCIATE_RQ: u8 = 0x01;
pub const PDU_ASSOCIATE_AC: u8 = 0x02;
pub const PDU_ASSOCIATE_RJ: u8 = 0x03;
pub const PDU_P_DATA_TF: u8 = 0x04;
pub const PDU_RELEASE_RQ: u8 = 0x05;
pub const PDU_RELEASE_RP: u8 = 0x06;
pub const PDU_A_ABORT: u8 = 0x07;

// ── Sub-item type constants ───────────────────────────────────────────────────

const ITEM_APPLICATION_CONTEXT: u8 = 0x10;
const ITEM_PRESENTATION_CONTEXT_RQ: u8 = 0x20;
const ITEM_PRESENTATION_CONTEXT_AC: u8 = 0x21;
const ITEM_ABSTRACT_SYNTAX: u8 = 0x30;
const ITEM_TRANSFER_SYNTAX: u8 = 0x40;
const ITEM_USER_INFORMATION: u8 = 0x50;
const ITEM_MAX_PDU_LENGTH: u8 = 0x51;
const ITEM_IMPLEMENTATION_CLASS_UID: u8 = 0x52;
#[allow(dead_code)]
const ITEM_ASYNC_OPS_WINDOW: u8 = 0x53;
const ITEM_IMPLEMENTATION_VERSION_NAME: u8 = 0x55;

// ── PDU struct types ──────────────────────────────────────────────────────────

/// Presentation context item carried in an A-ASSOCIATE-RQ.
#[derive(Debug, Clone)]
pub struct PresentationContextRqItem {
    pub id: u8,
    pub abstract_syntax: String,
    pub transfer_syntaxes: Vec<String>,
}

/// Presentation context item carried in an A-ASSOCIATE-AC.
#[derive(Debug, Clone)]
pub struct PresentationContextAcItem {
    pub id: u8,
    /// 0=acceptance, 1=user-reject, 2=no-reason,
    /// 3=abstract-not-supported, 4=ts-not-supported
    pub result: u8,
    pub transfer_syntax: String,
}

/// A-ASSOCIATE-RQ PDU body.
#[derive(Debug, Clone)]
pub struct AssociateRq {
    pub called_ae_title: String,
    pub calling_ae_title: String,
    pub application_context: String,
    pub presentation_contexts: Vec<PresentationContextRqItem>,
    pub max_pdu_length: u32,
    pub implementation_class_uid: String,
    pub implementation_version_name: String,
}

/// A-ASSOCIATE-AC PDU body.
#[derive(Debug, Clone)]
pub struct AssociateAc {
    pub called_ae_title: String,
    pub calling_ae_title: String,
    pub application_context: String,
    pub presentation_contexts: Vec<PresentationContextAcItem>,
    pub max_pdu_length: u32,
    pub implementation_class_uid: String,
    pub implementation_version_name: String,
}

/// A-ASSOCIATE-RJ PDU body.
#[derive(Debug, Clone)]
pub struct AssociateRj {
    /// 1=rejected-permanent, 2=rejected-transient.
    pub result: u8,
    /// 1=service-user, 2=service-provider-ACSE, 3=service-provider-presentation.
    pub source: u8,
    /// Reason code (source-dependent).
    pub reason: u8,
}

/// A single PDV item within a P-DATA-TF PDU.
#[derive(Debug, Clone)]
pub struct Pdv {
    pub context_id: u8,
    /// Bit 0: last fragment; bit 1: command (1) / data (0).
    pub msg_control: u8,
    pub data: Vec<u8>,
}

impl Pdv {
    /// Returns `true` if this is the last fragment of the message.
    pub fn is_last(&self) -> bool {
        self.msg_control & 0x01 != 0
    }

    /// Returns `true` if this PDV carries a DIMSE command dataset.
    pub fn is_command(&self) -> bool {
        self.msg_control & 0x02 != 0
    }
}

/// P-DATA-TF PDU body (one or more PDV items).
#[derive(Debug, Clone)]
pub struct PDataTf {
    pub pdvs: Vec<Pdv>,
}

/// A-ABORT PDU body.
#[derive(Debug, Clone)]
pub struct AAbort {
    /// 0=service-user, 2=service-provider.
    pub source: u8,
    pub reason: u8,
}

/// All supported PDU variants.
#[derive(Debug, Clone)]
pub enum Pdu {
    AssociateRq(AssociateRq),
    AssociateAc(AssociateAc),
    AssociateRj(AssociateRj),
    PDataTf(PDataTf),
    ReleaseRq,
    ReleaseRp,
    AAbort(AAbort),
}

// ── Encoding helpers ──────────────────────────────────────────────────────────

/// Encode a 16-byte space-padded AE title.
fn write_ae_title(buf: &mut Vec<u8>, title: &str) {
    let mut bytes = [b' '; 16];
    let src = title.as_bytes();
    let len = src.len().min(16);
    bytes[..len].copy_from_slice(&src[..len]);
    buf.extend_from_slice(&bytes);
}

/// Read a 16-byte space-padded AE title, trimming trailing spaces.
fn read_ae_title(data: &[u8]) -> String {
    std::str::from_utf8(data).unwrap_or("").trim().to_string()
}

/// Encode a sub-item whose value is a UID byte string.
fn encode_uid_sub_item(item_type: u8, uid: &str) -> Vec<u8> {
    let uid_bytes = uid.as_bytes();
    let len = uid_bytes.len() as u16;
    let mut buf = Vec::with_capacity(4 + uid_bytes.len());
    buf.push(item_type);
    buf.push(0); // reserved
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(uid_bytes);
    buf
}

/// Decode a UID byte string, stripping null padding.
pub(crate) fn decode_uid_bytes(data: &[u8]) -> String {
    let trimmed = if let Some(pos) = data.iter().position(|&b| b == 0) {
        &data[..pos]
    } else {
        data
    };
    String::from_utf8_lossy(trimmed).trim().to_string()
}

fn encode_presentation_context_rq(pc: &PresentationContextRqItem) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(pc.id);
    body.push(0); // reserved
    body.push(0); // reserved
    body.push(0); // reserved
    body.extend_from_slice(&encode_uid_sub_item(ITEM_ABSTRACT_SYNTAX, &pc.abstract_syntax));
    for ts in &pc.transfer_syntaxes {
        body.extend_from_slice(&encode_uid_sub_item(ITEM_TRANSFER_SYNTAX, ts));
    }
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.push(ITEM_PRESENTATION_CONTEXT_RQ);
    buf.push(0);
    buf.extend_from_slice(&(body.len() as u16).to_be_bytes());
    buf.extend_from_slice(&body);
    buf
}

fn encode_presentation_context_ac(pc: &PresentationContextAcItem) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(pc.id);
    body.push(0); // reserved
    body.push(pc.result);
    body.push(0); // reserved
    body.extend_from_slice(&encode_uid_sub_item(ITEM_TRANSFER_SYNTAX, &pc.transfer_syntax));
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.push(ITEM_PRESENTATION_CONTEXT_AC);
    buf.push(0);
    buf.extend_from_slice(&(body.len() as u16).to_be_bytes());
    buf.extend_from_slice(&body);
    buf
}

fn encode_user_information(max_pdu: u32, impl_uid: &str, impl_version: &str) -> Vec<u8> {
    let mut user_data = Vec::new();

    // Max PDU Length sub-item (0x51)
    user_data.push(ITEM_MAX_PDU_LENGTH);
    user_data.push(0);
    user_data.extend_from_slice(&4u16.to_be_bytes());
    user_data.extend_from_slice(&max_pdu.to_be_bytes());

    // Implementation Class UID (0x52)
    user_data.extend_from_slice(&encode_uid_sub_item(ITEM_IMPLEMENTATION_CLASS_UID, impl_uid));

    // Implementation Version Name (0x55), only if non-empty
    if !impl_version.is_empty() {
        let vb = impl_version.as_bytes();
        user_data.push(ITEM_IMPLEMENTATION_VERSION_NAME);
        user_data.push(0);
        user_data.extend_from_slice(&(vb.len() as u16).to_be_bytes());
        user_data.extend_from_slice(vb);
    }

    let mut buf = Vec::with_capacity(4 + user_data.len());
    buf.push(ITEM_USER_INFORMATION);
    buf.push(0);
    buf.extend_from_slice(&(user_data.len() as u16).to_be_bytes());
    buf.extend_from_slice(&user_data);
    buf
}

fn encode_associate_header(
    buf: &mut Vec<u8>,
    called: &str,
    calling: &str,
    app_ctx: &str,
) {
    buf.extend_from_slice(&1u16.to_be_bytes()); // protocol version
    buf.extend_from_slice(&0u16.to_be_bytes()); // reserved
    write_ae_title(buf, called);
    write_ae_title(buf, calling);
    buf.extend_from_slice(&[0u8; 32]); // reserved
    buf.extend_from_slice(&encode_uid_sub_item(ITEM_APPLICATION_CONTEXT, app_ctx));
}

fn raw_pdu(pdu_type: u8, body: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(6 + body.len());
    buf.push(pdu_type);
    buf.push(0); // reserved
    buf.extend_from_slice(&(body.len() as u32).to_be_bytes());
    buf.extend_from_slice(body);
    buf
}

// ── Public encoding functions ─────────────────────────────────────────────────

/// Encode an A-ASSOCIATE-RQ PDU into a byte buffer ready to be sent.
pub fn encode_associate_rq(rq: &AssociateRq) -> Vec<u8> {
    let mut body = Vec::new();
    encode_associate_header(
        &mut body,
        &rq.called_ae_title,
        &rq.calling_ae_title,
        &rq.application_context,
    );
    for pc in &rq.presentation_contexts {
        body.extend_from_slice(&encode_presentation_context_rq(pc));
    }
    body.extend_from_slice(&encode_user_information(
        rq.max_pdu_length,
        &rq.implementation_class_uid,
        &rq.implementation_version_name,
    ));
    raw_pdu(PDU_ASSOCIATE_RQ, &body)
}

/// Encode an A-ASSOCIATE-AC PDU.
pub fn encode_associate_ac(ac: &AssociateAc) -> Vec<u8> {
    let mut body = Vec::new();
    encode_associate_header(
        &mut body,
        &ac.called_ae_title,
        &ac.calling_ae_title,
        &ac.application_context,
    );
    for pc in &ac.presentation_contexts {
        body.extend_from_slice(&encode_presentation_context_ac(pc));
    }
    body.extend_from_slice(&encode_user_information(
        ac.max_pdu_length,
        &ac.implementation_class_uid,
        &ac.implementation_version_name,
    ));
    raw_pdu(PDU_ASSOCIATE_AC, &body)
}

/// Encode an A-ASSOCIATE-RJ PDU.
pub fn encode_associate_rj(rj: &AssociateRj) -> Vec<u8> {
    raw_pdu(PDU_ASSOCIATE_RJ, &[0, rj.result, rj.source, rj.reason])
}

/// Encode a P-DATA-TF PDU from a slice of PDV items.
pub fn encode_p_data_tf(pdvs: &[Pdv]) -> Vec<u8> {
    let mut body = Vec::new();
    for pdv in pdvs {
        // item-length = context_id (1) + msg_control (1) + data
        let item_len = (2 + pdv.data.len()) as u32;
        body.extend_from_slice(&item_len.to_be_bytes());
        body.push(pdv.context_id);
        body.push(pdv.msg_control);
        body.extend_from_slice(&pdv.data);
    }
    raw_pdu(PDU_P_DATA_TF, &body)
}

/// Encode an A-RELEASE-RQ PDU.
pub fn encode_release_rq() -> Vec<u8> {
    raw_pdu(PDU_RELEASE_RQ, &[0u8; 4])
}

/// Encode an A-RELEASE-RP PDU.
pub fn encode_release_rp() -> Vec<u8> {
    raw_pdu(PDU_RELEASE_RP, &[0u8; 4])
}

/// Encode an A-ABORT PDU.
pub fn encode_a_abort(abort: &AAbort) -> Vec<u8> {
    raw_pdu(PDU_A_ABORT, &[0, 0, abort.source, abort.reason])
}

// ── Decoding helpers ──────────────────────────────────────────────────────────

fn decode_pc_rq(data: &[u8]) -> DcmResult<PresentationContextRqItem> {
    if data.len() < 4 {
        return Err(DcmError::Other("PC-RQ item too short".into()));
    }
    let id = data[0];
    // data[1..4] reserved
    let mut pos = 4;
    let mut abstract_syntax = String::new();
    let mut transfer_syntaxes = Vec::new();

    while pos + 4 <= data.len() {
        let sub_type = data[pos];
        let sub_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if pos + sub_len > data.len() {
            break;
        }
        let sub_data = &data[pos..pos + sub_len];
        pos += sub_len;
        match sub_type {
            ITEM_ABSTRACT_SYNTAX => abstract_syntax = decode_uid_bytes(sub_data),
            ITEM_TRANSFER_SYNTAX => transfer_syntaxes.push(decode_uid_bytes(sub_data)),
            _ => {}
        }
    }
    Ok(PresentationContextRqItem { id, abstract_syntax, transfer_syntaxes })
}

fn decode_pc_ac(data: &[u8]) -> DcmResult<PresentationContextAcItem> {
    if data.len() < 4 {
        return Err(DcmError::Other("PC-AC item too short".into()));
    }
    let id = data[0];
    // data[1] reserved
    let result = data[2];
    // data[3] reserved
    let mut pos = 4;
    let mut transfer_syntax = String::new();

    while pos + 4 <= data.len() {
        let sub_type = data[pos];
        let sub_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if pos + sub_len > data.len() {
            break;
        }
        let sub_data = &data[pos..pos + sub_len];
        pos += sub_len;
        if sub_type == ITEM_TRANSFER_SYNTAX {
            transfer_syntax = decode_uid_bytes(sub_data);
        }
    }
    Ok(PresentationContextAcItem { id, result, transfer_syntax })
}

fn decode_user_info(data: &[u8]) -> DcmResult<(u32, String, String)> {
    let mut max_pdu = 65_536u32;
    let mut impl_uid = String::new();
    let mut impl_version = String::new();

    let mut pos = 0;
    while pos + 4 <= data.len() {
        let item_type = data[pos];
        let item_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if pos + item_len > data.len() {
            break;
        }
        let item_data = &data[pos..pos + item_len];
        pos += item_len;
        match item_type {
            ITEM_MAX_PDU_LENGTH if item_data.len() >= 4 => {
                max_pdu = u32::from_be_bytes([
                    item_data[0],
                    item_data[1],
                    item_data[2],
                    item_data[3],
                ]);
            }
            ITEM_IMPLEMENTATION_CLASS_UID => {
                impl_uid = decode_uid_bytes(item_data);
            }
            ITEM_IMPLEMENTATION_VERSION_NAME => {
                impl_version = String::from_utf8_lossy(item_data).to_string();
            }
            _ => {}
        }
    }
    Ok((max_pdu, impl_uid, impl_version))
}

/// Decode the sub-items block common to both RQ and AC associate PDUs.
///
/// Returns `(rq_pcs, ac_pcs, app_context_uid, max_pdu, impl_class_uid, impl_version)`.
fn decode_sub_items(
    data: &[u8],
) -> DcmResult<(
    Vec<PresentationContextRqItem>,
    Vec<PresentationContextAcItem>,
    String,
    u32,
    String,
    String,
)> {
    let mut rq_pcs = Vec::new();
    let mut ac_pcs = Vec::new();
    let mut app_context = String::new();
    let mut max_pdu = 65_536u32;
    let mut impl_uid = String::new();
    let mut impl_version = String::new();

    let mut pos = 0;
    while pos + 4 <= data.len() {
        let item_type = data[pos];
        let item_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if pos + item_len > data.len() {
            return Err(DcmError::Other(format!(
                "sub-item 0x{:02X} truncated (need {}, have {})",
                item_type,
                item_len,
                data.len() - pos + item_len,
            )));
        }
        let item_data = &data[pos..pos + item_len];
        pos += item_len;

        match item_type {
            ITEM_APPLICATION_CONTEXT => {
                app_context = decode_uid_bytes(item_data);
            }
            ITEM_PRESENTATION_CONTEXT_RQ => {
                rq_pcs.push(decode_pc_rq(item_data)?);
            }
            ITEM_PRESENTATION_CONTEXT_AC => {
                ac_pcs.push(decode_pc_ac(item_data)?);
            }
            ITEM_USER_INFORMATION => {
                let (m, u, v) = decode_user_info(item_data)?;
                max_pdu = m;
                impl_uid = u;
                impl_version = v;
            }
            _ => {} // unknown sub-items are silently ignored per DICOM
        }
    }
    Ok((rq_pcs, ac_pcs, app_context, max_pdu, impl_uid, impl_version))
}

// ── Public decoding functions ─────────────────────────────────────────────────

/// Decode an A-ASSOCIATE-RQ body (everything after the 6-byte PDU header).
pub fn decode_associate_rq(body: &[u8]) -> DcmResult<AssociateRq> {
    // Fixed header: 2B ver + 2B reserved + 16B called + 16B calling + 32B reserved = 68 B
    if body.len() < 68 {
        return Err(DcmError::Other(format!(
            "A-ASSOCIATE-RQ body too short: {} bytes",
            body.len()
        )));
    }
    let called = read_ae_title(&body[4..20]);
    let calling = read_ae_title(&body[20..36]);
    let (rq_pcs, _, app_ctx, max_pdu, impl_uid, impl_version) =
        decode_sub_items(&body[68..])?;
    Ok(AssociateRq {
        called_ae_title: called,
        calling_ae_title: calling,
        application_context: app_ctx,
        presentation_contexts: rq_pcs,
        max_pdu_length: max_pdu,
        implementation_class_uid: impl_uid,
        implementation_version_name: impl_version,
    })
}

/// Decode an A-ASSOCIATE-AC body.
pub fn decode_associate_ac(body: &[u8]) -> DcmResult<AssociateAc> {
    if body.len() < 68 {
        return Err(DcmError::Other(format!(
            "A-ASSOCIATE-AC body too short: {} bytes",
            body.len()
        )));
    }
    let called = read_ae_title(&body[4..20]);
    let calling = read_ae_title(&body[20..36]);
    let (_, ac_pcs, app_ctx, max_pdu, impl_uid, impl_version) =
        decode_sub_items(&body[68..])?;
    Ok(AssociateAc {
        called_ae_title: called,
        calling_ae_title: calling,
        application_context: app_ctx,
        presentation_contexts: ac_pcs,
        max_pdu_length: max_pdu,
        implementation_class_uid: impl_uid,
        implementation_version_name: impl_version,
    })
}

/// Decode an A-ASSOCIATE-RJ body.
pub fn decode_associate_rj(body: &[u8]) -> DcmResult<AssociateRj> {
    if body.len() < 4 {
        return Err(DcmError::Other(format!(
            "A-ASSOCIATE-RJ body too short: {} bytes",
            body.len()
        )));
    }
    Ok(AssociateRj { result: body[1], source: body[2], reason: body[3] })
}

/// Decode a P-DATA-TF body (one or more PDV items).
pub fn decode_p_data_tf(body: &[u8]) -> DcmResult<PDataTf> {
    let mut pdvs = Vec::new();
    let mut pos = 0;
    while pos + 4 <= body.len() {
        let item_len =
            u32::from_be_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]])
                as usize;
        pos += 4;
        if item_len < 2 || pos + item_len > body.len() {
            return Err(DcmError::Other(format!(
                "invalid PDV item length: {}",
                item_len
            )));
        }
        let context_id = body[pos];
        let msg_control = body[pos + 1];
        let data = body[pos + 2..pos + item_len].to_vec();
        pos += item_len;
        pdvs.push(Pdv { context_id, msg_control, data });
    }
    Ok(PDataTf { pdvs })
}

/// Decode an A-ABORT body.
pub fn decode_a_abort(body: &[u8]) -> AAbort {
    let source = body.get(2).copied().unwrap_or(0);
    let reason = body.get(3).copied().unwrap_or(0);
    AAbort { source, reason }
}

// ── Async PDU I/O ─────────────────────────────────────────────────────────────

/// Read and decode a complete PDU from an async reader.
///
/// Reads the 6-byte header first, then the body.
pub async fn read_pdu<R: AsyncRead + Unpin>(reader: &mut R) -> DcmResult<Pdu> {
    let mut header = [0u8; 6];
    reader.read_exact(&mut header).await?;
    let pdu_type = header[0];
    let body_len =
        u32::from_be_bytes([header[2], header[3], header[4], header[5]]) as usize;
    let mut body = vec![0u8; body_len];
    reader.read_exact(&mut body).await?;

    match pdu_type {
        PDU_ASSOCIATE_RQ => Ok(Pdu::AssociateRq(decode_associate_rq(&body)?)),
        PDU_ASSOCIATE_AC => Ok(Pdu::AssociateAc(decode_associate_ac(&body)?)),
        PDU_ASSOCIATE_RJ => Ok(Pdu::AssociateRj(decode_associate_rj(&body)?)),
        PDU_P_DATA_TF => Ok(Pdu::PDataTf(decode_p_data_tf(&body)?)),
        PDU_RELEASE_RQ => Ok(Pdu::ReleaseRq),
        PDU_RELEASE_RP => Ok(Pdu::ReleaseRp),
        PDU_A_ABORT => Ok(Pdu::AAbort(decode_a_abort(&body))),
        other => Err(DcmError::Other(format!(
            "unknown PDU type: 0x{:02X}",
            other
        ))),
    }
}

/// Write pre-encoded PDU bytes to an async writer.
pub async fn write_pdu<W: AsyncWrite + Unpin>(writer: &mut W, data: &[u8]) -> DcmResult<()> {
    writer.write_all(data).await?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rq() -> AssociateRq {
        AssociateRq {
            called_ae_title: "SCP".to_string(),
            calling_ae_title: "SCU".to_string(),
            application_context: "1.2.840.10008.3.1.1.1".to_string(),
            presentation_contexts: vec![PresentationContextRqItem {
                id: 1,
                abstract_syntax: "1.2.840.10008.1.1".to_string(),
                transfer_syntaxes: vec![
                    "1.2.840.10008.1.2.1".to_string(),
                    "1.2.840.10008.1.2".to_string(),
                ],
            }],
            max_pdu_length: 65_536,
            implementation_class_uid: "1.3.6.1.4.1.30071.8.1".to_string(),
            implementation_version_name: "TEST_IMPL".to_string(),
        }
    }

    #[test]
    fn associate_rq_roundtrip() {
        let rq = sample_rq();
        let encoded = encode_associate_rq(&rq);

        // PDU type byte
        assert_eq!(encoded[0], PDU_ASSOCIATE_RQ);

        // Decode body (skip 6-byte header)
        let body_len = u32::from_be_bytes([encoded[2], encoded[3], encoded[4], encoded[5]]) as usize;
        assert_eq!(body_len, encoded.len() - 6);

        let decoded = decode_associate_rq(&encoded[6..]).unwrap();
        assert_eq!(decoded.called_ae_title, "SCP");
        assert_eq!(decoded.calling_ae_title, "SCU");
        assert_eq!(decoded.application_context, "1.2.840.10008.3.1.1.1");
        assert_eq!(decoded.presentation_contexts.len(), 1);
        assert_eq!(decoded.presentation_contexts[0].id, 1);
        assert_eq!(
            decoded.presentation_contexts[0].abstract_syntax,
            "1.2.840.10008.1.1"
        );
        assert_eq!(decoded.presentation_contexts[0].transfer_syntaxes.len(), 2);
        assert_eq!(decoded.max_pdu_length, 65_536);
        assert_eq!(decoded.implementation_class_uid, "1.3.6.1.4.1.30071.8.1");
        assert_eq!(decoded.implementation_version_name, "TEST_IMPL");
    }

    #[test]
    fn associate_ac_roundtrip() {
        let ac = AssociateAc {
            called_ae_title: "SCP".to_string(),
            calling_ae_title: "SCU".to_string(),
            application_context: "1.2.840.10008.3.1.1.1".to_string(),
            presentation_contexts: vec![PresentationContextAcItem {
                id: 1,
                result: 0, // acceptance
                transfer_syntax: "1.2.840.10008.1.2.1".to_string(),
            }],
            max_pdu_length: 32_768,
            implementation_class_uid: "1.3.6.1.4.1.30071.8.1".to_string(),
            implementation_version_name: "SCP_IMPL".to_string(),
        };

        let encoded = encode_associate_ac(&ac);
        assert_eq!(encoded[0], PDU_ASSOCIATE_AC);

        let decoded = decode_associate_ac(&encoded[6..]).unwrap();
        assert_eq!(decoded.called_ae_title, "SCP");
        assert_eq!(decoded.presentation_contexts.len(), 1);
        assert_eq!(decoded.presentation_contexts[0].result, 0);
        assert_eq!(
            decoded.presentation_contexts[0].transfer_syntax,
            "1.2.840.10008.1.2.1"
        );
        assert_eq!(decoded.max_pdu_length, 32_768);
    }

    #[test]
    fn associate_rj_roundtrip() {
        let rj = AssociateRj { result: 1, source: 1, reason: 1 };
        let encoded = encode_associate_rj(&rj);
        assert_eq!(encoded[0], PDU_ASSOCIATE_RJ);

        let decoded = decode_associate_rj(&encoded[6..]).unwrap();
        assert_eq!(decoded.result, 1);
        assert_eq!(decoded.source, 1);
        assert_eq!(decoded.reason, 1);
    }

    #[test]
    fn p_data_tf_roundtrip() {
        let data = b"Hello DICOM".to_vec();
        let pdv = Pdv { context_id: 1, msg_control: 0x03, data: data.clone() };
        let encoded = encode_p_data_tf(&[pdv]);
        assert_eq!(encoded[0], PDU_P_DATA_TF);

        let decoded = decode_p_data_tf(&encoded[6..]).unwrap();
        assert_eq!(decoded.pdvs.len(), 1);
        assert_eq!(decoded.pdvs[0].context_id, 1);
        assert!(decoded.pdvs[0].is_last());
        assert!(decoded.pdvs[0].is_command());
        assert_eq!(decoded.pdvs[0].data, data);
    }

    #[test]
    fn p_data_tf_data_pdv() {
        let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
        // msg_control: bit0=last, bit1=0 → data PDV
        let pdv = Pdv { context_id: 3, msg_control: 0x01, data: data.clone() };
        let encoded = encode_p_data_tf(&[pdv]);
        let decoded = decode_p_data_tf(&encoded[6..]).unwrap();
        assert!(decoded.pdvs[0].is_last());
        assert!(!decoded.pdvs[0].is_command());
        assert_eq!(decoded.pdvs[0].data, data);
    }

    #[test]
    fn release_rq_rp_encoding() {
        let rq = encode_release_rq();
        assert_eq!(rq[0], PDU_RELEASE_RQ);
        assert_eq!(u32::from_be_bytes([rq[2], rq[3], rq[4], rq[5]]), 4);

        let rp = encode_release_rp();
        assert_eq!(rp[0], PDU_RELEASE_RP);
    }

    #[test]
    fn a_abort_roundtrip() {
        let abort = AAbort { source: 2, reason: 0 };
        let encoded = encode_a_abort(&abort);
        assert_eq!(encoded[0], PDU_A_ABORT);

        let decoded = decode_a_abort(&encoded[6..]);
        assert_eq!(decoded.source, 2);
        assert_eq!(decoded.reason, 0);
    }

    #[test]
    fn ae_title_padding() {
        let rq = AssociateRq {
            called_ae_title: "A".to_string(),
            calling_ae_title: "BB".to_string(),
            application_context: "1.2.840.10008.3.1.1.1".to_string(),
            presentation_contexts: vec![],
            max_pdu_length: 65_536,
            implementation_class_uid: "1.2.3".to_string(),
            implementation_version_name: String::new(),
        };
        let encoded = encode_associate_rq(&rq);
        let decoded = decode_associate_rq(&encoded[6..]).unwrap();
        assert_eq!(decoded.called_ae_title, "A");
        assert_eq!(decoded.calling_ae_title, "BB");
    }

    /// Verify that reading a PDU works end-to-end through the async path
    /// (using an in-memory cursor as the reader).
    #[tokio::test]
    async fn read_pdu_associate_rq() {
        let rq = sample_rq();
        let bytes = encode_associate_rq(&rq);
        let mut cursor = std::io::Cursor::new(bytes);
        let pdu = read_pdu(&mut cursor).await.unwrap();
        assert!(matches!(pdu, Pdu::AssociateRq(_)));
    }

    #[tokio::test]
    async fn read_pdu_release_rq() {
        let bytes = encode_release_rq();
        let mut cursor = std::io::Cursor::new(bytes);
        let pdu = read_pdu(&mut cursor).await.unwrap();
        assert!(matches!(pdu, Pdu::ReleaseRq));
    }

    #[tokio::test]
    async fn read_pdu_p_data() {
        let pdv = Pdv { context_id: 1, msg_control: 0x03, data: vec![1, 2, 3] };
        let bytes = encode_p_data_tf(&[pdv]);
        let mut cursor = std::io::Cursor::new(bytes);
        let pdu = read_pdu(&mut cursor).await.unwrap();
        match pdu {
            Pdu::PDataTf(pd) => {
                assert_eq!(pd.pdvs.len(), 1);
                assert_eq!(pd.pdvs[0].data, vec![1, 2, 3]);
            }
            _ => panic!("expected PDataTf"),
        }
    }
}
