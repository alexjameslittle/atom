mod config;
mod ffi;
mod kernel;
mod logging;
mod plugin;
mod registry;
mod state;

pub use config::{ModuleRegistration, RuntimeConfig};
pub use plugin::{PluginContext, RuntimePlugin};
pub use registry::{
    current_state, ensure_running, handle_lifecycle, init_runtime, shutdown_runtime,
};
pub use state::RuntimeState;
