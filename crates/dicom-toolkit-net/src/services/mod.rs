//! DIMSE service implementations.

use dicom_toolkit_core::error::DcmResult;
use dicom_toolkit_data::DataSet;
use dicom_toolkit_dict::tags;

use crate::association::Association;

pub mod echo;
pub mod find;
pub mod get;
pub mod r#move;
pub mod provider;
pub mod store;

pub(crate) fn command_has_dataset(cmd: &DataSet) -> bool {
    cmd.get_u16(tags::COMMAND_DATA_SET_TYPE)
        .map(|v| v != 0x0101)
        .unwrap_or(false)
}

pub(crate) async fn recv_command_data_bytes(
    assoc: &mut Association,
    cmd: &DataSet,
) -> DcmResult<Vec<u8>> {
    if !command_has_dataset(cmd) {
        return Ok(Vec::new());
    }

    Ok(assoc.recv_optional_dimse_data().await?.unwrap_or_default())
}
