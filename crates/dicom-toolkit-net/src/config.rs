//! Association configuration.
//!
//! Mirrors DCMTK's `DcmAssociationConfiguration` / `T_ASC_Parameters`.

// ── AssociationConfig ─────────────────────────────────────────────────────────

/// Configuration for both SCU (outbound) and SCP (inbound) associations.
#[derive(Debug, Clone)]
pub struct AssociationConfig {
    /// AE title advertised as the local application entity.
    pub local_ae_title: String,

    /// Maximum PDU length we are willing to receive (bytes).
    ///
    /// A value of `0` means unlimited.  Defaults to 65 536.
    pub max_pdu_length: u32,

    /// Seconds to wait for a response during association negotiation and
    /// DIMSE operations before returning a timeout error.
    pub dimse_timeout_secs: u64,

    /// If `true`, the SCP accepts any transfer syntax offered by the SCU
    /// without checking against a preferred list.
    pub accept_all_transfer_syntaxes: bool,

    /// Implementation Class UID advertised in the User Information sub-item.
    pub implementation_class_uid: String,

    /// Implementation Version Name advertised in the User Information sub-item.
    pub implementation_version_name: String,

    /// Abstract Syntax UIDs this SCP is willing to accept.
    ///
    /// An empty list means **accept all** (useful for testing / generic SCPs).
    pub accepted_abstract_syntaxes: Vec<String>,
}

impl Default for AssociationConfig {
    fn default() -> Self {
        Self {
            local_ae_title: "DCMTKRS".to_string(),
            max_pdu_length: 65_536,
            dimse_timeout_secs: 30,
            accept_all_transfer_syntaxes: false,
            implementation_class_uid: "1.3.6.1.4.1.30071.8.1".to_string(),
            implementation_version_name: "DCMTK_RS_010".to_string(),
            accepted_abstract_syntaxes: Vec::new(),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_sensible() {
        let cfg = AssociationConfig::default();
        assert_eq!(cfg.max_pdu_length, 65_536);
        assert_eq!(cfg.dimse_timeout_secs, 30);
        assert!(!cfg.accept_all_transfer_syntaxes);
        assert!(!cfg.implementation_class_uid.is_empty());
    }
}
