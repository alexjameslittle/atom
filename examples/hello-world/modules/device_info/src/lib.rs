use std::env;

use atom_ffi::{AtomExportOutput, AtomResult};
use atom_runtime::{ModuleMethodRegistration, ModuleRegistration};
use flatbuffers::{FlatBufferBuilder, TableFinishedWIPOffset, WIPOffset};

pub const METHOD_GET: &str = "get";
pub const MODULE_ID: &str = "device_info";

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
pub fn module_id() -> &'static str {
    MODULE_ID
}

#[must_use]
#[atom_macros::atom_export]
pub fn get() -> DeviceInfo {
    current_device_info()
}

#[must_use]
pub fn encode_get_device_info_request() -> Vec<u8> {
    let mut builder = FlatBufferBuilder::new();
    let root = create_get_device_info_request(&mut builder);
    builder.finish(root, None);
    builder.finished_data().to_vec()
}

impl AtomExportOutput for DeviceInfo {
    fn encode_atom_export(self) -> AtomResult<Vec<u8>> {
        Ok(encode_device_info(&self))
    }
}

#[must_use]
pub fn runtime_module() -> ModuleRegistration {
    ModuleRegistration {
        id: MODULE_ID.to_owned(),
        init_order: 0,
        init_fn: Box::new(|ctx| {
            ctx.set_state("modules.device_info", "ready");
            Ok(())
        }),
        shutdown_fn: None,
        methods: vec![ModuleMethodRegistration::new(
            METHOD_GET,
            |_ctx, _request| get().encode_atom_export(),
        )],
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

fn create_get_device_info_request(
    builder: &mut FlatBufferBuilder<'_>,
) -> WIPOffset<TableFinishedWIPOffset> {
    let table = builder.start_table();
    builder.end_table(table)
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
    use super::{
        ConnectionStatus, METHOD_GET, MODULE_ID, encode_get_device_info_request, get,
        runtime_module,
    };

    #[test]
    fn request_encoding_produces_flatbuffer_bytes() {
        assert!(encode_get_device_info_request().len() > 4);
    }

    #[test]
    fn runtime_module_registers_get_method() {
        let module = runtime_module();
        assert_eq!(module.id, MODULE_ID);
        assert_eq!(module.methods.len(), 1);
        assert_eq!(module.methods[0].name, METHOD_GET);
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
