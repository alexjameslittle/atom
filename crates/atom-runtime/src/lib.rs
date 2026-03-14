mod config;
mod kernel;
mod logging;
mod plugin;
mod registry;
mod state;
mod store;

pub use config::{ModuleRegistration, RuntimeConfig, RuntimeConfigBuilder};
pub use plugin::{PluginContext, RuntimePlugin};
pub use registry::{
    current_snapshot, current_state, ensure_running, handle_lifecycle, init_runtime,
    running_plugin_context, shutdown_runtime,
};
pub use state::RuntimeState;
pub use store::{RuntimeEffect, RuntimeEvent, RuntimeSnapshot};
