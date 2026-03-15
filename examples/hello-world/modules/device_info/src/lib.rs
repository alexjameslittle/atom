use std::env;

use atom_ffi::AtomResult;
use atom_runtime::PluginContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub model: String,
    pub os: String,
}

pub type GetDeviceInfoResponse = DeviceInfo;

/// Public Rust API for the module. Generated bridge code and runtime plugins
/// call this directly instead of routing through the runtime kernel.
///
/// # Errors
///
/// This example implementation is infallible, but real modules may return
/// `AtomError` values through `AtomResult`.
pub fn get(_ctx: &PluginContext) -> AtomResult<DeviceInfo> {
    Ok(DeviceInfo {
        model: "atom-runtime".to_owned(),
        os: format!("{}-{}", env::consts::OS, env::consts::ARCH),
    })
}

#[cfg(test)]
mod tests {
    use atom_runtime::RuntimeConfig;

    use super::get;

    #[test]
    fn direct_rust_api_returns_device_info() {
        let handle = atom_runtime::init_runtime(RuntimeConfig::default()).expect("runtime init");
        let ctx = atom_runtime::running_plugin_context(handle).expect("running context");
        let response = get(&ctx).expect("response");
        assert_eq!(response.model, "atom-runtime");
        assert!(!response.os.is_empty());
        atom_runtime::shutdown_runtime(handle);
    }
}
