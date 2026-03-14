use std::env;

use atom_ffi::AtomResult;
use atom_runtime::{ModuleRegistration, PluginContext};

pub const MODULE_ID: &str = "device_info";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetDeviceInfoRequest {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetDeviceInfoResponse {
    pub model: String,
    pub os: String,
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
    }
}

/// Public Rust API for the module. Generated bridge code and runtime plugins
/// call this directly instead of routing through the runtime kernel.
///
/// # Errors
///
/// This example implementation is infallible, but real modules may return
/// `AtomError` values through `AtomResult`.
pub fn get(
    _ctx: &PluginContext,
    _request: GetDeviceInfoRequest,
) -> AtomResult<GetDeviceInfoResponse> {
    Ok(GetDeviceInfoResponse {
        model: "atom-runtime".to_owned(),
        os: format!("{}-{}", env::consts::OS, env::consts::ARCH),
    })
}

#[cfg(test)]
mod tests {
    use atom_runtime::RuntimeConfig;

    use super::{GetDeviceInfoRequest, MODULE_ID, get, runtime_module};

    #[test]
    fn direct_rust_api_returns_device_info() {
        let handle = atom_runtime::init_runtime(RuntimeConfig::default()).expect("runtime init");
        let ctx = atom_runtime::running_plugin_context(handle).expect("running context");
        let response = get(&ctx, GetDeviceInfoRequest {}).expect("response");
        assert_eq!(response.model, "atom-runtime");
        assert!(!response.os.is_empty());
        atom_runtime::shutdown_runtime(handle);
    }

    #[test]
    fn runtime_module_sets_up_lifecycle_registration() {
        let module = runtime_module();
        assert_eq!(module.id, MODULE_ID);
    }
}
