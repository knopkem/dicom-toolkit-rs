//! Runtime configuration for dcmtk-rs.
//!
//! Provides global and per-operation configuration that mirrors DCMTK's
//! runtime-configurable flags.

/// Global DICOM configuration.
#[derive(Debug, Clone)]
pub struct DcmConfig {
    /// Maximum size of a single value to load into memory (bytes).
    /// Larger values are read on demand. Default: 4 MiB.
    pub max_value_size: u64,

    /// Whether to accept unknown (private) tags during parsing.
    pub accept_unknown_tags: bool,

    /// Whether to use the data dictionary for VR lookup during implicit VR parsing.
    pub use_dictionary: bool,

    /// Default character set when Specific Character Set (0008,0005) is absent.
    pub default_charset: String,
}

impl Default for DcmConfig {
    fn default() -> Self {
        Self {
            max_value_size: 4 * 1024 * 1024,
            accept_unknown_tags: true,
            use_dictionary: true,
            default_charset: String::new(), // ASCII
        }
    }
}
