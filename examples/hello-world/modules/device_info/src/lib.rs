use std::env;

use atom_runtime::{ModuleMethodRegistration, ModuleRegistration};

pub const METHOD_GET: &str = "get";
pub const MODULE_ID: &str = "device_info";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetDeviceInfoRequest {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetDeviceInfoResponse {
    pub model: String,
    pub os: String,
}

#[must_use]
pub fn module_id() -> &'static str {
    MODULE_ID
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
            |_ctx, _request: GetDeviceInfoRequest| {
                Ok(GetDeviceInfoResponse {
                    model: "atom-runtime".to_owned(),
                    os: format!("{}-{}", env::consts::OS, env::consts::ARCH),
                })
            },
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GetDeviceInfoRequest, GetDeviceInfoResponse, METHOD_GET, MODULE_ID, runtime_module,
    };

    #[test]
    fn request_type_is_plain_rust_struct() {
        assert_eq!(GetDeviceInfoRequest {}, GetDeviceInfoRequest {});
    }

    #[test]
    fn runtime_module_registers_get_method() {
        let module = runtime_module();
        assert_eq!(module.id, MODULE_ID);
        assert_eq!(module.methods.len(), 1);
        assert_eq!(module.methods[0].name, METHOD_GET);
    }

    #[test]
    fn response_type_exposes_model_and_os() {
        let response = GetDeviceInfoResponse {
            model: "atom-runtime".to_owned(),
            os: "ios-arm64".to_owned(),
        };
        assert_eq!(response.model, "atom-runtime");
        assert_eq!(response.os, "ios-arm64");
    }
}
