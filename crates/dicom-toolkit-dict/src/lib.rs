//! > ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for diagnostic or therapeutic purposes.
//! DICOM data dictionary: tags, value representations, UIDs, and transfer syntaxes.

pub mod tag;
pub mod ts;
pub mod uid_registry;
pub mod vr;

pub use tag::{Tag, tags};
pub use ts::{TransferSyntax, transfer_syntaxes};
pub use uid_registry::uid_name;
pub use vr::Vr;
