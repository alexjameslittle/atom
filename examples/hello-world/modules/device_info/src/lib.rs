use std::env;

use atom_ffi::{AtomExportOutput, AtomResult};
use flatbuffers::{FlatBufferBuilder, TableFinishedWIPOffset, WIPOffset};

#[atom_macros::atom_record]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub model: String,
    pub os: String,
}

#[atom_macros::atom_record]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    Connecting,
}

#[atom_macros::atom_record]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    Loaded { info: DeviceInfo },
    Unavailable { reason: String },
}

#[must_use]
#[atom_macros::atom_export]
pub fn get() -> DeviceInfo {
    current_device_info()
}

impl AtomExportOutput for DeviceInfo {
    fn encode_atom_export(self) -> AtomResult<Vec<u8>> {
        Ok(encode_device_info(&self))
    }
}

fn current_device_info() -> DeviceInfo {
    DeviceInfo {
        model: "atom-runtime".to_owned(),
        os: format!("{}-{}", env::consts::OS, env::consts::ARCH),
    }
}

fn encode_device_info(value: &DeviceInfo) -> Vec<u8> {
    let mut builder = FlatBufferBuilder::new();
    let model = builder.create_string(&value.model);
    let os = builder.create_string(&value.os);
    let root = create_device_info(&mut builder, model, os);
    builder.finish(root, None);
    builder.finished_data().to_vec()
}

fn create_device_info<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    model: WIPOffset<&'a str>,
    os: WIPOffset<&'a str>,
) -> WIPOffset<TableFinishedWIPOffset> {
    let table = builder.start_table();
    builder.push_slot_always::<WIPOffset<_>>(4, model);
    builder.push_slot_always::<WIPOffset<_>>(6, os);
    builder.end_table(table)
}

#[cfg(test)]
mod tests {
    use super::{ConnectionStatus, get};

    #[test]
    fn get_returns_model_and_os() {
        let info = get();
        assert_eq!(info.model, "atom-runtime");
        assert!(info.os.contains('-'));
    }

    #[test]
    fn annotated_export_returns_runtime_device_info() {
        let info = get();
        assert_eq!(info.model, "atom-runtime");
        assert!(info.os.contains('-'));
        assert!(matches!(
            ConnectionStatus::Connected,
            ConnectionStatus::Connected
        ));
    }
}
