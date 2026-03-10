use std::env;

use atom_runtime::{ModuleMethodRegistration, ModuleRegistration};
use flatbuffers::{FlatBufferBuilder, TableFinishedWIPOffset, WIPOffset};

pub const METHOD_GET: &str = "get";
pub const MODULE_ID: &str = "device_info";

#[must_use]
pub fn module_id() -> &'static str {
    MODULE_ID
}

#[must_use]
pub fn encode_get_device_info_request() -> Vec<u8> {
    let mut builder = FlatBufferBuilder::new();
    let root = create_get_device_info_request(&mut builder);
    builder.finish(root, None);
    builder.finished_data().to_vec()
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
            |_ctx, _request| {
                Ok(encode_get_device_info_response(
                    "atom-runtime",
                    &format!("{}-{}", env::consts::OS, env::consts::ARCH),
                ))
            },
        )],
    }
}

fn encode_get_device_info_response(model: &str, os: &str) -> Vec<u8> {
    let mut builder = FlatBufferBuilder::new();
    let model = builder.create_string(model);
    let os = builder.create_string(os);
    let root = create_get_device_info_response(&mut builder, model, os);
    builder.finish(root, None);
    builder.finished_data().to_vec()
}

fn create_get_device_info_request(
    builder: &mut FlatBufferBuilder<'_>,
) -> WIPOffset<TableFinishedWIPOffset> {
    let table = builder.start_table();
    builder.end_table(table)
}

fn create_get_device_info_response<'a>(
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
    use super::{METHOD_GET, MODULE_ID, encode_get_device_info_request, runtime_module};

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
}
