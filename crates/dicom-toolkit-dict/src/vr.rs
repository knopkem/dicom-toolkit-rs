//! DICOM Value Representation (VR) definitions.
//!
//! Ports the VR enumeration and properties from DCMTK's `dcvr.h`.

use std::fmt;

/// DICOM Value Representation.
///
/// Each variant corresponds to a two-character VR code as defined in
/// PS3.5 §6.2 and PS3.5 §7.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Vr {
    /// Application Entity (max 16 chars)
    AE,
    /// Age String (4 chars, format nnnD/W/M/Y)
    AS,
    /// Attribute Tag (4 bytes: group + element)
    AT,
    /// Code String (max 16 chars)
    CS,
    /// Date (8 chars, YYYYMMDD)
    DA,
    /// Decimal String (max 16 chars per value)
    DS,
    /// Date Time (max 26 chars)
    DT,
    /// Floating Point Double (8 bytes)
    FD,
    /// Floating Point Single (4 bytes)
    FL,
    /// Integer String (max 12 chars)
    IS,
    /// Long String (max 64 chars)
    LO,
    /// Long Text (max 10240 chars)
    LT,
    /// Other Byte
    OB,
    /// Other Double
    OD,
    /// Other Float
    OF,
    /// Other Long
    OL,
    /// Other 64-bit Very Long
    OV,
    /// Other Word
    OW,
    /// Person Name (max 64 chars per component group)
    PN,
    /// Short String (max 16 chars)
    SH,
    /// Signed Long (4 bytes)
    SL,
    /// Sequence of Items
    SQ,
    /// Signed Short (2 bytes)
    SS,
    /// Short Text (max 1024 chars)
    ST,
    /// Signed 64-bit Very Long (8 bytes)
    SV,
    /// Time (max 14 chars)
    TM,
    /// Unlimited Characters (max 2^32-2 chars)
    UC,
    /// Unique Identifier (max 64 chars)
    UI,
    /// Unsigned Long (4 bytes)
    UL,
    /// Unknown
    UN,
    /// Universal Resource Identifier/Locator (max 2^32-2 chars)
    UR,
    /// Unsigned Short (2 bytes)
    US,
    /// Unlimited Text (max 2^32-2 chars)
    UT,
    /// Unsigned 64-bit Very Long (8 bytes)
    UV,
}

impl Vr {
    /// Parses a two-character VR code.
    pub fn from_bytes(bytes: [u8; 2]) -> Option<Self> {
        match &bytes {
            b"AE" => Some(Self::AE),
            b"AS" => Some(Self::AS),
            b"AT" => Some(Self::AT),
            b"CS" => Some(Self::CS),
            b"DA" => Some(Self::DA),
            b"DS" => Some(Self::DS),
            b"DT" => Some(Self::DT),
            b"FD" => Some(Self::FD),
            b"FL" => Some(Self::FL),
            b"IS" => Some(Self::IS),
            b"LO" => Some(Self::LO),
            b"LT" => Some(Self::LT),
            b"OB" => Some(Self::OB),
            b"OD" => Some(Self::OD),
            b"OF" => Some(Self::OF),
            b"OL" => Some(Self::OL),
            b"OV" => Some(Self::OV),
            b"OW" => Some(Self::OW),
            b"PN" => Some(Self::PN),
            b"SH" => Some(Self::SH),
            b"SL" => Some(Self::SL),
            b"SQ" => Some(Self::SQ),
            b"SS" => Some(Self::SS),
            b"ST" => Some(Self::ST),
            b"SV" => Some(Self::SV),
            b"TM" => Some(Self::TM),
            b"UC" => Some(Self::UC),
            b"UI" => Some(Self::UI),
            b"UL" => Some(Self::UL),
            b"UN" => Some(Self::UN),
            b"UR" => Some(Self::UR),
            b"US" => Some(Self::US),
            b"UT" => Some(Self::UT),
            b"UV" => Some(Self::UV),
            _ => None,
        }
    }

    /// Returns the two-character VR code as bytes.
    pub fn to_bytes(self) -> [u8; 2] {
        match self {
            Self::AE => *b"AE",
            Self::AS => *b"AS",
            Self::AT => *b"AT",
            Self::CS => *b"CS",
            Self::DA => *b"DA",
            Self::DS => *b"DS",
            Self::DT => *b"DT",
            Self::FD => *b"FD",
            Self::FL => *b"FL",
            Self::IS => *b"IS",
            Self::LO => *b"LO",
            Self::LT => *b"LT",
            Self::OB => *b"OB",
            Self::OD => *b"OD",
            Self::OF => *b"OF",
            Self::OL => *b"OL",
            Self::OV => *b"OV",
            Self::OW => *b"OW",
            Self::PN => *b"PN",
            Self::SH => *b"SH",
            Self::SL => *b"SL",
            Self::SQ => *b"SQ",
            Self::SS => *b"SS",
            Self::ST => *b"ST",
            Self::SV => *b"SV",
            Self::TM => *b"TM",
            Self::UC => *b"UC",
            Self::UI => *b"UI",
            Self::UL => *b"UL",
            Self::UN => *b"UN",
            Self::UR => *b"UR",
            Self::US => *b"US",
            Self::UT => *b"UT",
            Self::UV => *b"UV",
        }
    }

    /// Returns the two-character VR code as a string.
    pub fn code(self) -> &'static str {
        match self {
            Self::AE => "AE",
            Self::AS => "AS",
            Self::AT => "AT",
            Self::CS => "CS",
            Self::DA => "DA",
            Self::DS => "DS",
            Self::DT => "DT",
            Self::FD => "FD",
            Self::FL => "FL",
            Self::IS => "IS",
            Self::LO => "LO",
            Self::LT => "LT",
            Self::OB => "OB",
            Self::OD => "OD",
            Self::OF => "OF",
            Self::OL => "OL",
            Self::OV => "OV",
            Self::OW => "OW",
            Self::PN => "PN",
            Self::SH => "SH",
            Self::SL => "SL",
            Self::SQ => "SQ",
            Self::SS => "SS",
            Self::ST => "ST",
            Self::SV => "SV",
            Self::TM => "TM",
            Self::UC => "UC",
            Self::UI => "UI",
            Self::UL => "UL",
            Self::UN => "UN",
            Self::UR => "UR",
            Self::US => "US",
            Self::UT => "UT",
            Self::UV => "UV",
        }
    }

    /// Returns `true` if this VR is a string type (text-based).
    pub fn is_string(self) -> bool {
        matches!(
            self,
            Self::AE
                | Self::AS
                | Self::CS
                | Self::DA
                | Self::DS
                | Self::DT
                | Self::IS
                | Self::LO
                | Self::LT
                | Self::PN
                | Self::SH
                | Self::ST
                | Self::TM
                | Self::UC
                | Self::UI
                | Self::UR
                | Self::UT
        )
    }

    /// Returns `true` if this VR uses a 4-byte length field in explicit VR encoding
    /// (i.e., the "long" VR group that has 2 reserved bytes + 4-byte length).
    pub fn has_long_explicit_length(self) -> bool {
        matches!(
            self,
            Self::OB
                | Self::OD
                | Self::OF
                | Self::OL
                | Self::OV
                | Self::OW
                | Self::SQ
                | Self::UC
                | Self::UN
                | Self::UR
                | Self::UT
                | Self::SV
                | Self::UV
        )
    }

    /// Returns the padding character for this VR.
    /// String VRs pad with space (0x20), binary/UI VRs pad with null (0x00).
    pub fn padding_byte(self) -> u8 {
        if self == Self::UI || !self.is_string() {
            0x00
        } else {
            0x20
        }
    }

    /// Returns the fixed size of a single value in bytes, or `None` for variable-length VRs.
    pub fn fixed_value_size(self) -> Option<usize> {
        match self {
            Self::FL => Some(4),
            Self::FD => Some(8),
            Self::SL => Some(4),
            Self::SS => Some(2),
            Self::SV => Some(8),
            Self::UL => Some(4),
            Self::US => Some(2),
            Self::UV => Some(8),
            Self::AT => Some(4),
            _ => None,
        }
    }

    /// Returns a human-readable name for this VR.
    pub fn name(self) -> &'static str {
        match self {
            Self::AE => "Application Entity",
            Self::AS => "Age String",
            Self::AT => "Attribute Tag",
            Self::CS => "Code String",
            Self::DA => "Date",
            Self::DS => "Decimal String",
            Self::DT => "Date Time",
            Self::FD => "Floating Point Double",
            Self::FL => "Floating Point Single",
            Self::IS => "Integer String",
            Self::LO => "Long String",
            Self::LT => "Long Text",
            Self::OB => "Other Byte",
            Self::OD => "Other Double",
            Self::OF => "Other Float",
            Self::OL => "Other Long",
            Self::OV => "Other 64-bit Very Long",
            Self::OW => "Other Word",
            Self::PN => "Person Name",
            Self::SH => "Short String",
            Self::SL => "Signed Long",
            Self::SQ => "Sequence of Items",
            Self::SS => "Signed Short",
            Self::ST => "Short Text",
            Self::SV => "Signed 64-bit Very Long",
            Self::TM => "Time",
            Self::UC => "Unlimited Characters",
            Self::UI => "Unique Identifier",
            Self::UL => "Unsigned Long",
            Self::UN => "Unknown",
            Self::UR => "Universal Resource Identifier",
            Self::US => "Unsigned Short",
            Self::UT => "Unlimited Text",
            Self::UV => "Unsigned 64-bit Very Long",
        }
    }
}

impl fmt::Display for Vr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_vrs() {
        let all_vrs = [
            Vr::AE, Vr::AS, Vr::AT, Vr::CS, Vr::DA, Vr::DS, Vr::DT, Vr::FD, Vr::FL, Vr::IS,
            Vr::LO, Vr::LT, Vr::OB, Vr::OD, Vr::OF, Vr::OL, Vr::OV, Vr::OW, Vr::PN, Vr::SH,
            Vr::SL, Vr::SQ, Vr::SS, Vr::ST, Vr::SV, Vr::TM, Vr::UC, Vr::UI, Vr::UL, Vr::UN,
            Vr::UR, Vr::US, Vr::UT, Vr::UV,
        ];
        for vr in &all_vrs {
            let bytes = vr.to_bytes();
            let parsed = Vr::from_bytes(bytes).expect("should parse");
            assert_eq!(*vr, parsed, "roundtrip failed for {vr}");
        }
    }

    #[test]
    fn string_vrs() {
        assert!(Vr::LO.is_string());
        assert!(Vr::PN.is_string());
        assert!(Vr::DA.is_string());
        assert!(!Vr::OB.is_string());
        assert!(!Vr::SQ.is_string());
        assert!(!Vr::FL.is_string());
    }

    #[test]
    fn long_explicit_vrs() {
        assert!(Vr::OB.has_long_explicit_length());
        assert!(Vr::SQ.has_long_explicit_length());
        assert!(Vr::UN.has_long_explicit_length());
        assert!(!Vr::CS.has_long_explicit_length());
        assert!(!Vr::US.has_long_explicit_length());
    }

    #[test]
    fn padding_bytes() {
        assert_eq!(Vr::LO.padding_byte(), 0x20);
        assert_eq!(Vr::UI.padding_byte(), 0x00);
        assert_eq!(Vr::OB.padding_byte(), 0x00);
    }

    #[test]
    fn fixed_sizes() {
        assert_eq!(Vr::US.fixed_value_size(), Some(2));
        assert_eq!(Vr::UL.fixed_value_size(), Some(4));
        assert_eq!(Vr::FD.fixed_value_size(), Some(8));
        assert_eq!(Vr::LO.fixed_value_size(), None);
    }

    #[test]
    fn invalid_vr_bytes() {
        assert!(Vr::from_bytes(*b"XX").is_none());
        assert!(Vr::from_bytes(*b"00").is_none());
    }
}
