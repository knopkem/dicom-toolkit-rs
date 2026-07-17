//! DICOM Upper Layer Protocol PDU types, encoding, and decoding.
//!
//! Implements PS3.8 §9 — Upper Layer Service and Protocol.
//!
//! Every PDU is preceded by a 6-byte header:
//! ```text
//! [1B type][1B reserved=0][4B body-length (u32 BE)]
//! ```

use dicom_toolkit_core::error::{DcmError, DcmResult};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

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
const ITEM_ASYNC_OPS_WINDOW: u8 = 0x53;
const ITEM_SCP_SCU_ROLE_SELECTION: u8 = 0x54;
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

/// Maximum outstanding DIMSE operations in each direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsynchronousOperationsWindow {
    /// Maximum number of operations this association participant may invoke.
    pub invoked: u16,
    /// Maximum number of operations this association participant may perform.
    pub performed: u16,
}

impl Default for AsynchronousOperationsWindow {
    fn default() -> Self {
        Self {
            invoked: 1,
            performed: 1,
        }
    }
}

/// Proposed or accepted roles for one SOP Class.
///
/// The flags always describe the association requestor's roles, including
/// when this value is carried in A-ASSOCIATE-AC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScpScuRoleSelection {
    /// SOP Class UID to which the role selection applies.
    pub sop_class_uid: String,
    /// Whether the association requestor may act as SCU.
    pub scu_role: bool,
    /// Whether the association requestor may act as SCP.
    pub scp_role: bool,
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

/// Extended association negotiation carried in User Information sub-items.
///
/// This sidecar keeps [`AssociateRq`] and [`AssociateAc`] source-compatible
/// with earlier releases while exposing asynchronous-operation and role
/// negotiation to callers that opt in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssociationUserInformation {
    /// Optional asynchronous operations window.
    pub asynchronous_operations_window: Option<AsynchronousOperationsWindow>,
    /// Optional SOP Class role selections expressed as requestor roles.
    pub role_selections: Vec<ScpScuRoleSelection>,
}

/// A decoded PDU plus extended association User Information when applicable.
#[derive(Debug, Clone)]
pub struct DecodedPdu {
    /// Decoded public PDU value.
    pub pdu: Pdu,
    /// Extended User Information for A-ASSOCIATE-RQ/AC PDUs.
    pub association_user_information: Option<AssociationUserInformation>,
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
    /// PDV message control header (DICOM PS3.8 §9.3.1):
    /// - Bit 0: 1 = command information, 0 = data set information
    /// - Bit 1: 1 = last fragment, 0 = not the last fragment
    pub msg_control: u8,
    pub data: Vec<u8>,
}

impl Pdv {
    /// Returns `true` if this is the last fragment of the message.
    pub fn is_last(&self) -> bool {
        self.msg_control & 0x02 != 0
    }

    /// Returns `true` if this PDV carries a DIMSE command dataset.
    pub fn is_command(&self) -> bool {
        self.msg_control & 0x01 != 0
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
    let mut body = vec![pc.id, 0, 0, 0];
    body.extend_from_slice(&encode_uid_sub_item(
        ITEM_ABSTRACT_SYNTAX,
        &pc.abstract_syntax,
    ));
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
    let mut body = vec![pc.id, 0, pc.result, 0];
    body.extend_from_slice(&encode_uid_sub_item(
        ITEM_TRANSFER_SYNTAX,
        &pc.transfer_syntax,
    ));
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.push(ITEM_PRESENTATION_CONTEXT_AC);
    buf.push(0);
    buf.extend_from_slice(&(body.len() as u16).to_be_bytes());
    buf.extend_from_slice(&body);
    buf
}

fn encode_user_information(
    max_pdu: u32,
    impl_uid: &str,
    impl_version: &str,
    asynchronous_operations_window: Option<AsynchronousOperationsWindow>,
    role_selections: &[ScpScuRoleSelection],
) -> Vec<u8> {
    let mut user_data = Vec::new();

    // Max PDU Length sub-item (0x51)
    user_data.push(ITEM_MAX_PDU_LENGTH);
    user_data.push(0);
    user_data.extend_from_slice(&4u16.to_be_bytes());
    user_data.extend_from_slice(&max_pdu.to_be_bytes());

    // Implementation Class UID (0x52)
    user_data.extend_from_slice(&encode_uid_sub_item(
        ITEM_IMPLEMENTATION_CLASS_UID,
        impl_uid,
    ));

    if let Some(window) = asynchronous_operations_window {
        user_data.push(ITEM_ASYNC_OPS_WINDOW);
        user_data.push(0);
        user_data.extend_from_slice(&4u16.to_be_bytes());
        user_data.extend_from_slice(&window.invoked.to_be_bytes());
        user_data.extend_from_slice(&window.performed.to_be_bytes());
    }

    for role in role_selections {
        let uid_bytes = role.sop_class_uid.as_bytes();
        let item_length = u16::try_from(uid_bytes.len().saturating_add(4)).unwrap_or(u16::MAX);
        let uid_length = u16::try_from(uid_bytes.len()).unwrap_or(u16::MAX);
        user_data.push(ITEM_SCP_SCU_ROLE_SELECTION);
        user_data.push(0);
        user_data.extend_from_slice(&item_length.to_be_bytes());
        user_data.extend_from_slice(&uid_length.to_be_bytes());
        user_data.extend_from_slice(uid_bytes);
        user_data.push(u8::from(role.scu_role));
        user_data.push(u8::from(role.scp_role));
    }

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

fn encode_associate_header(buf: &mut Vec<u8>, called: &str, calling: &str, app_ctx: &str) {
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
    encode_associate_rq_with_user_information(rq, &AssociationUserInformation::default())
}

/// Encode an A-ASSOCIATE-RQ with extended User Information.
pub fn encode_associate_rq_with_user_information(
    rq: &AssociateRq,
    user_information: &AssociationUserInformation,
) -> Vec<u8> {
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
        user_information.asynchronous_operations_window,
        &user_information.role_selections,
    ));
    raw_pdu(PDU_ASSOCIATE_RQ, &body)
}

/// Encode an A-ASSOCIATE-AC PDU.
pub fn encode_associate_ac(ac: &AssociateAc) -> Vec<u8> {
    encode_associate_ac_with_user_information(ac, &AssociationUserInformation::default())
}

/// Encode an A-ASSOCIATE-AC with extended User Information.
pub fn encode_associate_ac_with_user_information(
    ac: &AssociateAc,
    user_information: &AssociationUserInformation,
) -> Vec<u8> {
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
        user_information.asynchronous_operations_window,
        &user_information.role_selections,
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
    Ok(PresentationContextRqItem {
        id,
        abstract_syntax,
        transfer_syntaxes,
    })
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
    Ok(PresentationContextAcItem {
        id,
        result,
        transfer_syntax,
    })
}

#[derive(Debug)]
struct DecodedUserInformation {
    max_pdu_length: u32,
    implementation_class_uid: String,
    implementation_version_name: String,
    asynchronous_operations_window: Option<AsynchronousOperationsWindow>,
    role_selections: Vec<ScpScuRoleSelection>,
}

impl Default for DecodedUserInformation {
    fn default() -> Self {
        Self {
            max_pdu_length: 65_536,
            implementation_class_uid: String::new(),
            implementation_version_name: String::new(),
            asynchronous_operations_window: None,
            role_selections: Vec::new(),
        }
    }
}

fn decode_legacy_user_info(data: &[u8]) -> (u32, String, String) {
    let mut maximum_pdu_length = 65_536;
    let mut implementation_class_uid = String::new();
    let mut implementation_version_name = String::new();

    let mut position = 0;
    while position + 4 <= data.len() {
        let item_type = data[position];
        let item_length = u16::from_be_bytes([data[position + 2], data[position + 3]]) as usize;
        position += 4;
        if position + item_length > data.len() {
            break;
        }
        let item_data = &data[position..position + item_length];
        position += item_length;
        match item_type {
            ITEM_MAX_PDU_LENGTH if item_data.len() >= 4 => {
                maximum_pdu_length =
                    u32::from_be_bytes([item_data[0], item_data[1], item_data[2], item_data[3]]);
            }
            ITEM_IMPLEMENTATION_CLASS_UID => {
                implementation_class_uid = decode_uid_bytes(item_data);
            }
            ITEM_IMPLEMENTATION_VERSION_NAME => {
                implementation_version_name = String::from_utf8_lossy(item_data).to_string();
            }
            _ => {}
        }
    }

    (
        maximum_pdu_length,
        implementation_class_uid,
        implementation_version_name,
    )
}

fn decode_extended_user_info(data: &[u8]) -> DcmResult<DecodedUserInformation> {
    let mut user_information = DecodedUserInformation::default();

    let mut pos = 0;
    while pos + 4 <= data.len() {
        let item_type = data[pos];
        let item_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;
        if pos + item_len > data.len() {
            return Err(DcmError::Other(format!(
                "user-information item 0x{item_type:02X} is truncated"
            )));
        }
        let item_data = &data[pos..pos + item_len];
        pos += item_len;
        match item_type {
            ITEM_MAX_PDU_LENGTH if item_data.len() == 4 => {
                user_information.max_pdu_length =
                    u32::from_be_bytes([item_data[0], item_data[1], item_data[2], item_data[3]]);
            }
            ITEM_MAX_PDU_LENGTH => {
                return Err(DcmError::Other(format!(
                    "maximum PDU length item has invalid length {}",
                    item_data.len()
                )));
            }
            ITEM_IMPLEMENTATION_CLASS_UID => {
                user_information.implementation_class_uid = decode_uid_bytes(item_data);
            }
            ITEM_ASYNC_OPS_WINDOW if item_data.len() == 4 => {
                user_information.asynchronous_operations_window =
                    Some(AsynchronousOperationsWindow {
                        invoked: u16::from_be_bytes([item_data[0], item_data[1]]),
                        performed: u16::from_be_bytes([item_data[2], item_data[3]]),
                    });
            }
            ITEM_ASYNC_OPS_WINDOW => {
                return Err(DcmError::Other(format!(
                    "asynchronous operations window has invalid length {}",
                    item_data.len()
                )));
            }
            ITEM_SCP_SCU_ROLE_SELECTION => {
                if item_data.len() < 4 {
                    return Err(DcmError::Other(
                        "SCP/SCU role selection item is too short".into(),
                    ));
                }
                let uid_length = u16::from_be_bytes([item_data[0], item_data[1]]) as usize;
                let expected_length = uid_length.saturating_add(4);
                if item_data.len() != expected_length {
                    return Err(DcmError::Other(format!(
                        "SCP/SCU role selection length mismatch: expected {expected_length}, got {}",
                        item_data.len()
                    )));
                }
                let scu_role = item_data[uid_length + 2];
                let scp_role = item_data[uid_length + 3];
                if scu_role > 1 || scp_role > 1 {
                    return Err(DcmError::Other(
                        "SCP/SCU role values must be zero or one".into(),
                    ));
                }
                user_information.role_selections.push(ScpScuRoleSelection {
                    sop_class_uid: decode_uid_bytes(&item_data[2..uid_length + 2]),
                    scu_role: scu_role == 1,
                    scp_role: scp_role == 1,
                });
            }
            ITEM_IMPLEMENTATION_VERSION_NAME => {
                user_information.implementation_version_name =
                    String::from_utf8_lossy(item_data).to_string();
            }
            _ => {}
        }
    }
    Ok(user_information)
}

type LegacySubItemsResult = (
    Vec<PresentationContextRqItem>,
    Vec<PresentationContextAcItem>,
    String,
    u32,
    String,
    String,
);

fn decode_legacy_sub_items(data: &[u8]) -> DcmResult<LegacySubItemsResult> {
    let mut request_contexts = Vec::new();
    let mut accept_contexts = Vec::new();
    let mut application_context = String::new();
    let mut maximum_pdu_length = 65_536;
    let mut implementation_class_uid = String::new();
    let mut implementation_version_name = String::new();

    let mut position = 0;
    while position + 4 <= data.len() {
        let item_type = data[position];
        let item_length = u16::from_be_bytes([data[position + 2], data[position + 3]]) as usize;
        position += 4;
        if position + item_length > data.len() {
            return Err(DcmError::Other(format!(
                "sub-item 0x{:02X} truncated (need {}, have {})",
                item_type,
                item_length,
                data.len() - position + item_length,
            )));
        }
        let item_data = &data[position..position + item_length];
        position += item_length;

        match item_type {
            ITEM_APPLICATION_CONTEXT => application_context = decode_uid_bytes(item_data),
            ITEM_PRESENTATION_CONTEXT_RQ => request_contexts.push(decode_pc_rq(item_data)?),
            ITEM_PRESENTATION_CONTEXT_AC => accept_contexts.push(decode_pc_ac(item_data)?),
            ITEM_USER_INFORMATION => {
                let decoded = decode_legacy_user_info(item_data);
                maximum_pdu_length = decoded.0;
                implementation_class_uid = decoded.1;
                implementation_version_name = decoded.2;
            }
            _ => {}
        }
    }

    Ok((
        request_contexts,
        accept_contexts,
        application_context,
        maximum_pdu_length,
        implementation_class_uid,
        implementation_version_name,
    ))
}

type ExtendedSubItemsResult = (
    Vec<PresentationContextRqItem>,
    Vec<PresentationContextAcItem>,
    String,
    DecodedUserInformation,
);

/// Decode the sub-items block common to both RQ and AC associate PDUs.
///
/// Returns request contexts, accept contexts, application context, and user information.
fn decode_extended_sub_items(data: &[u8]) -> DcmResult<ExtendedSubItemsResult> {
    let mut rq_pcs = Vec::new();
    let mut ac_pcs = Vec::new();
    let mut app_context = String::new();
    let mut user_information = DecodedUserInformation::default();

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
                user_information = decode_extended_user_info(item_data)?;
            }
            _ => {} // unknown sub-items are silently ignored per DICOM
        }
    }
    Ok((rq_pcs, ac_pcs, app_context, user_information))
}

// ── Public decoding functions ─────────────────────────────────────────────────

/// Decode an A-ASSOCIATE-RQ body (everything after the 6-byte PDU header).
pub fn decode_associate_rq(body: &[u8]) -> DcmResult<AssociateRq> {
    if body.len() < 68 {
        return Err(DcmError::Other(format!(
            "A-ASSOCIATE-RQ body too short: {} bytes",
            body.len()
        )));
    }
    let called_ae_title = read_ae_title(&body[4..20]);
    let calling_ae_title = read_ae_title(&body[20..36]);
    let (
        presentation_contexts,
        _,
        application_context,
        max_pdu_length,
        implementation_class_uid,
        implementation_version_name,
    ) = decode_legacy_sub_items(&body[68..])?;
    Ok(AssociateRq {
        called_ae_title,
        calling_ae_title,
        application_context,
        presentation_contexts,
        max_pdu_length,
        implementation_class_uid,
        implementation_version_name,
    })
}

/// Decode an A-ASSOCIATE-RQ and its extended User Information.
pub fn decode_associate_rq_with_user_information(
    body: &[u8],
) -> DcmResult<(AssociateRq, AssociationUserInformation)> {
    // Fixed header: 2B ver + 2B reserved + 16B called + 16B calling + 32B reserved = 68 B
    if body.len() < 68 {
        return Err(DcmError::Other(format!(
            "A-ASSOCIATE-RQ body too short: {} bytes",
            body.len()
        )));
    }
    let called = read_ae_title(&body[4..20]);
    let calling = read_ae_title(&body[20..36]);
    let (rq_pcs, _, app_ctx, user_information) = decode_extended_sub_items(&body[68..])?;
    let request = AssociateRq {
        called_ae_title: called,
        calling_ae_title: calling,
        application_context: app_ctx,
        presentation_contexts: rq_pcs,
        max_pdu_length: user_information.max_pdu_length,
        implementation_class_uid: user_information.implementation_class_uid,
        implementation_version_name: user_information.implementation_version_name,
    };
    let extended = AssociationUserInformation {
        asynchronous_operations_window: user_information.asynchronous_operations_window,
        role_selections: user_information.role_selections,
    };
    Ok((request, extended))
}

/// Decode an A-ASSOCIATE-AC body.
pub fn decode_associate_ac(body: &[u8]) -> DcmResult<AssociateAc> {
    if body.len() < 68 {
        return Err(DcmError::Other(format!(
            "A-ASSOCIATE-AC body too short: {} bytes",
            body.len()
        )));
    }
    let called_ae_title = read_ae_title(&body[4..20]);
    let calling_ae_title = read_ae_title(&body[20..36]);
    let (
        _,
        presentation_contexts,
        application_context,
        max_pdu_length,
        implementation_class_uid,
        implementation_version_name,
    ) = decode_legacy_sub_items(&body[68..])?;
    Ok(AssociateAc {
        called_ae_title,
        calling_ae_title,
        application_context,
        presentation_contexts,
        max_pdu_length,
        implementation_class_uid,
        implementation_version_name,
    })
}

/// Decode an A-ASSOCIATE-AC and its extended User Information.
pub fn decode_associate_ac_with_user_information(
    body: &[u8],
) -> DcmResult<(AssociateAc, AssociationUserInformation)> {
    if body.len() < 68 {
        return Err(DcmError::Other(format!(
            "A-ASSOCIATE-AC body too short: {} bytes",
            body.len()
        )));
    }
    let called = read_ae_title(&body[4..20]);
    let calling = read_ae_title(&body[20..36]);
    let (_, ac_pcs, app_ctx, user_information) = decode_extended_sub_items(&body[68..])?;
    let accept = AssociateAc {
        called_ae_title: called,
        calling_ae_title: calling,
        application_context: app_ctx,
        presentation_contexts: ac_pcs,
        max_pdu_length: user_information.max_pdu_length,
        implementation_class_uid: user_information.implementation_class_uid,
        implementation_version_name: user_information.implementation_version_name,
    };
    let extended = AssociationUserInformation {
        asynchronous_operations_window: user_information.asynchronous_operations_window,
        role_selections: user_information.role_selections,
    };
    Ok((accept, extended))
}

/// Decode an A-ASSOCIATE-RJ body.
pub fn decode_associate_rj(body: &[u8]) -> DcmResult<AssociateRj> {
    if body.len() < 4 {
        return Err(DcmError::Other(format!(
            "A-ASSOCIATE-RJ body too short: {} bytes",
            body.len()
        )));
    }
    Ok(AssociateRj {
        result: body[1],
        source: body[2],
        reason: body[3],
    })
}

/// Decode a P-DATA-TF body (one or more PDV items).
pub fn decode_p_data_tf(body: &[u8]) -> DcmResult<PDataTf> {
    let mut pdvs = Vec::new();
    let mut pos = 0;
    while pos + 4 <= body.len() {
        let item_len =
            u32::from_be_bytes([body[pos], body[pos + 1], body[pos + 2], body[pos + 3]]) as usize;
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
        pdvs.push(Pdv {
            context_id,
            msg_control,
            data,
        });
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
    let (pdu_type, body) = read_pdu_bytes(reader, None).await?;
    decode_pdu_body(pdu_type, &body)
}

/// Read and decode a complete PDU while enforcing a variable-field limit.
///
/// The limit is checked before allocating the body buffer. It refers to the
/// four-byte PDU-length field and therefore excludes the six-byte PDU header.
pub async fn read_pdu_with_limit<R: AsyncRead + Unpin>(
    reader: &mut R,
    maximum_pdu_length: u32,
) -> DcmResult<Pdu> {
    let maximum = (maximum_pdu_length != 0).then_some(maximum_pdu_length);
    let (pdu_type, body) = read_pdu_bytes(reader, maximum).await?;
    decode_pdu_body(pdu_type, &body)
}

/// Read a bounded PDU and retain extended association User Information.
pub async fn read_pdu_with_limit_and_user_information<R: AsyncRead + Unpin>(
    reader: &mut R,
    maximum_pdu_length: u32,
) -> DcmResult<DecodedPdu> {
    let maximum = (maximum_pdu_length != 0).then_some(maximum_pdu_length);
    let (pdu_type, body) = read_pdu_bytes(reader, maximum).await?;
    decode_pdu_body_with_user_information(pdu_type, &body)
}

async fn read_pdu_bytes<R: AsyncRead + Unpin>(
    reader: &mut R,
    maximum_pdu_length: Option<u32>,
) -> DcmResult<(u8, Vec<u8>)> {
    let mut header = [0u8; 6];
    reader.read_exact(&mut header).await?;
    let pdu_type = header[0];
    let body_length = u32::from_be_bytes([header[2], header[3], header[4], header[5]]);
    if maximum_pdu_length.is_some_and(|maximum| body_length > maximum) {
        return Err(DcmError::Other(format!(
            "PDU length {body_length} exceeds the configured maximum of {} bytes",
            maximum_pdu_length.unwrap_or_default()
        )));
    }
    let body_len = body_length as usize;
    let mut body = vec![0u8; body_len];
    reader.read_exact(&mut body).await?;

    Ok((pdu_type, body))
}

pub(crate) fn decode_pdu_body(pdu_type: u8, body: &[u8]) -> DcmResult<Pdu> {
    match pdu_type {
        PDU_ASSOCIATE_RQ => Ok(Pdu::AssociateRq(decode_associate_rq(body)?)),
        PDU_ASSOCIATE_AC => Ok(Pdu::AssociateAc(decode_associate_ac(body)?)),
        PDU_ASSOCIATE_RJ => Ok(Pdu::AssociateRj(decode_associate_rj(body)?)),
        PDU_P_DATA_TF => Ok(Pdu::PDataTf(decode_p_data_tf(body)?)),
        PDU_RELEASE_RQ => Ok(Pdu::ReleaseRq),
        PDU_RELEASE_RP => Ok(Pdu::ReleaseRp),
        PDU_A_ABORT => Ok(Pdu::AAbort(decode_a_abort(body))),
        other => Err(DcmError::Other(format!("unknown PDU type: 0x{other:02X}"))),
    }
}

fn decode_pdu_body_with_user_information(pdu_type: u8, body: &[u8]) -> DcmResult<DecodedPdu> {
    let (pdu, association_user_information) = match pdu_type {
        PDU_ASSOCIATE_RQ => {
            let (request, user_information) = decode_associate_rq_with_user_information(body)?;
            (Pdu::AssociateRq(request), Some(user_information))
        }
        PDU_ASSOCIATE_AC => {
            let (accept, user_information) = decode_associate_ac_with_user_information(body)?;
            (Pdu::AssociateAc(accept), Some(user_information))
        }
        PDU_ASSOCIATE_RJ => (Pdu::AssociateRj(decode_associate_rj(body)?), None),
        PDU_P_DATA_TF => (Pdu::PDataTf(decode_p_data_tf(body)?), None),
        PDU_RELEASE_RQ => (Pdu::ReleaseRq, None),
        PDU_RELEASE_RP => (Pdu::ReleaseRp, None),
        PDU_A_ABORT => (Pdu::AAbort(decode_a_abort(body)), None),
        other => Err(DcmError::Other(format!(
            "unknown PDU type: 0x{:02X}",
            other
        )))?,
    };
    Ok(DecodedPdu {
        pdu,
        association_user_information,
    })
}

/// Write pre-encoded PDU bytes to an async writer.
pub async fn write_pdu<W: AsyncWrite + Unpin>(writer: &mut W, data: &[u8]) -> DcmResult<()> {
    writer.write_all(data).await?;
    Ok(())
}

/// Write one P-DATA-TF PDU containing a single PDV without copying its payload.
///
/// This is the bounded streaming primitive used for large C-STORE datasets.
pub async fn write_pdata_fragment<W: AsyncWrite + Unpin>(
    writer: &mut W,
    context_id: u8,
    message_control: u8,
    data: &[u8],
) -> DcmResult<()> {
    let header = pdata_fragment_header(context_id, message_control, data.len())?;
    writer.write_all(&header).await?;
    writer.write_all(data).await?;
    Ok(())
}

/// Encode one P-DATA-TF PDU containing a single PDV.
pub fn encode_pdata_fragment(
    context_id: u8,
    message_control: u8,
    data: &[u8],
) -> DcmResult<Vec<u8>> {
    let header = pdata_fragment_header(context_id, message_control, data.len())?;
    let mut encoded = Vec::with_capacity(header.len() + data.len());
    encoded.extend_from_slice(&header);
    encoded.extend_from_slice(data);
    Ok(encoded)
}

fn pdata_fragment_header(
    context_id: u8,
    message_control: u8,
    data_length: usize,
) -> DcmResult<[u8; 12]> {
    let item_length = u32::try_from(data_length.saturating_add(2))
        .map_err(|_| DcmError::Other("PDV payload exceeds the DICOM u32 length field".into()))?;
    let body_length = item_length
        .checked_add(4)
        .ok_or_else(|| DcmError::Other("P-DATA-TF body length overflow".into()))?;

    let mut header = [0u8; 12];
    header[0] = PDU_P_DATA_TF;
    header[2..6].copy_from_slice(&body_length.to_be_bytes());
    header[6..10].copy_from_slice(&item_length.to_be_bytes());
    header[10] = context_id;
    header[11] = message_control;

    Ok(header)
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

    fn sample_user_information() -> AssociationUserInformation {
        AssociationUserInformation {
            asynchronous_operations_window: Some(AsynchronousOperationsWindow {
                invoked: 4,
                performed: 3,
            }),
            role_selections: vec![ScpScuRoleSelection {
                sop_class_uid: "1.2.840.10008.1.1".to_string(),
                scu_role: true,
                scp_role: true,
            }],
        }
    }

    #[test]
    fn associate_rq_roundtrip() {
        let rq = sample_rq();
        let user_information = sample_user_information();
        let encoded = encode_associate_rq_with_user_information(&rq, &user_information);

        // PDU type byte
        assert_eq!(encoded[0], PDU_ASSOCIATE_RQ);

        // Decode body (skip 6-byte header)
        let body_len =
            u32::from_be_bytes([encoded[2], encoded[3], encoded[4], encoded[5]]) as usize;
        assert_eq!(body_len, encoded.len() - 6);

        let (decoded, decoded_user_information) =
            decode_associate_rq_with_user_information(&encoded[6..]).unwrap();
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
        assert_eq!(
            decoded_user_information.asynchronous_operations_window,
            Some(AsynchronousOperationsWindow {
                invoked: 4,
                performed: 3,
            })
        );
        assert_eq!(decoded_user_information, user_information);
    }

    #[test]
    fn legacy_user_information_decoder_ignores_malformed_extended_items() {
        let malformed_role_selection = [ITEM_SCP_SCU_ROLE_SELECTION, 0, 0, 3, 0, 1, b'1'];

        let legacy = decode_legacy_user_info(&malformed_role_selection);
        assert_eq!(legacy.0, 65_536);
        assert!(legacy.1.is_empty());
        assert!(legacy.2.is_empty());
        assert!(decode_extended_user_info(&malformed_role_selection).is_err());
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
        let user_information = AssociationUserInformation {
            asynchronous_operations_window: Some(AsynchronousOperationsWindow {
                invoked: 2,
                performed: 1,
            }),
            role_selections: vec![ScpScuRoleSelection {
                sop_class_uid: "1.2.840.10008.1.1".to_string(),
                scu_role: true,
                scp_role: false,
            }],
        };

        let encoded = encode_associate_ac_with_user_information(&ac, &user_information);
        assert_eq!(encoded[0], PDU_ASSOCIATE_AC);

        let (decoded, decoded_user_information) =
            decode_associate_ac_with_user_information(&encoded[6..]).unwrap();
        assert_eq!(decoded.called_ae_title, "SCP");
        assert_eq!(decoded.presentation_contexts.len(), 1);
        assert_eq!(decoded.presentation_contexts[0].result, 0);
        assert_eq!(
            decoded.presentation_contexts[0].transfer_syntax,
            "1.2.840.10008.1.2.1"
        );
        assert_eq!(decoded.max_pdu_length, 32_768);
        assert_eq!(
            decoded_user_information.asynchronous_operations_window,
            user_information.asynchronous_operations_window
        );
        assert_eq!(decoded_user_information, user_information);
    }

    #[test]
    fn associate_rj_roundtrip() {
        let rj = AssociateRj {
            result: 1,
            source: 1,
            reason: 1,
        };
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
        let pdv = Pdv {
            context_id: 1,
            msg_control: 0x03,
            data: data.clone(),
        };
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
        // DICOM PS3.8 §9.3.1: bit0=0 (data), bit1=1 (last) → 0x02
        let pdv = Pdv {
            context_id: 3,
            msg_control: 0x02,
            data: data.clone(),
        };
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
        let abort = AAbort {
            source: 2,
            reason: 0,
        };
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
        let pdv = Pdv {
            context_id: 1,
            msg_control: 0x03,
            data: vec![1, 2, 3],
        };
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

    #[tokio::test]
    async fn read_pdu_rejects_body_before_allocating_above_limit() {
        let bytes = vec![PDU_P_DATA_TF, 0, 0, 0, 16, 0];
        let mut cursor = std::io::Cursor::new(bytes);

        let result = read_pdu_with_limit(&mut cursor, 1_024).await;

        assert!(
            matches!(result, Err(DcmError::Other(message)) if message.contains("4096") && message.contains("1024"))
        );
    }
}
