//! Presentation context negotiation types.
//!
//! Ports DCMTK's presentation context handling from `assoc.h`.

// ── PcResult ──────────────────────────────────────────────────────────────────

/// Result of a presentation context negotiation (A-ASSOCIATE-AC).
///
/// Maps to the result codes in PS3.8 §9.3.2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PcResult {
    /// Context accepted (result = 0).
    Acceptance,
    /// Rejected by user (result = 1).
    UserRejection,
    /// No reason given (result = 2).
    NoReason,
    /// Abstract syntax not supported (result = 3).
    AbstractSyntaxNotSupported,
    /// No matching transfer syntax (result = 4).
    TransferSyntaxesNotSupported,
}

impl PcResult {
    /// Convert a raw result byte from the PDU into a `PcResult`.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Acceptance,
            1 => Self::UserRejection,
            2 => Self::NoReason,
            3 => Self::AbstractSyntaxNotSupported,
            _ => Self::TransferSyntaxesNotSupported,
        }
    }

    /// Convert back to the raw byte for PDU encoding.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Acceptance => 0,
            Self::UserRejection => 1,
            Self::NoReason => 2,
            Self::AbstractSyntaxNotSupported => 3,
            Self::TransferSyntaxesNotSupported => 4,
        }
    }

    /// Returns `true` if this context was accepted.
    pub fn is_accepted(self) -> bool {
        self == Self::Acceptance
    }
}

// ── PresentationContextRq ────────────────────────────────────────────────────

/// A presentation context proposed by the SCU in an A-ASSOCIATE-RQ.
#[derive(Debug, Clone)]
pub struct PresentationContextRq {
    /// Context ID — must be an odd number 1, 3, 5, …, 255.
    pub id: u8,
    /// Abstract Syntax UID (SOP Class UID).
    pub abstract_syntax: String,
    /// Transfer Syntax UIDs offered (in order of preference).
    pub transfer_syntaxes: Vec<String>,
}

// ── PresentationContextAc ────────────────────────────────────────────────────

/// A presentation context response returned in an A-ASSOCIATE-AC.
#[derive(Debug, Clone)]
pub struct PresentationContextAc {
    /// Context ID that matches the corresponding RQ item.
    pub id: u8,
    /// Negotiation result.
    pub result: PcResult,
    /// The single accepted Transfer Syntax UID (meaningful only when `result == Acceptance`).
    pub transfer_syntax: String,
    /// Abstract Syntax UID, carried from the original proposal for convenient lookup.
    pub abstract_syntax: String,
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pc_result_roundtrip() {
        for v in 0u8..=4 {
            let r = PcResult::from_u8(v);
            assert_eq!(r.to_u8(), v);
        }
    }

    #[test]
    fn pc_result_is_accepted() {
        assert!(PcResult::Acceptance.is_accepted());
        assert!(!PcResult::UserRejection.is_accepted());
        assert!(!PcResult::TransferSyntaxesNotSupported.is_accepted());
    }
}
