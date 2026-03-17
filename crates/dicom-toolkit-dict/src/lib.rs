//! ⚠️ **NOT FOR CLINICAL USE** — This software has not been validated for diagnostic or therapeutic purposes.
//!
//! DICOM data dictionary: tags, value representations, UIDs, and transfer syntaxes.

pub mod tag;
pub mod ts;
pub mod uid_registry;
pub mod vr;

pub use tag::{
    lookup_entry, lookup_entry_with_private_creator, raw_vr_for_tag, tags, vr_for_tag, DictEntry,
    RangeRestriction, Tag, VARIABLE_VM,
};
pub use ts::{transfer_syntaxes, TransferSyntax};
pub use uid_registry::uid_name;
pub use vr::Vr;
