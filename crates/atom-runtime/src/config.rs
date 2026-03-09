use atom_ffi::AtomResult;

use crate::plugin::{PluginContext, RuntimePlugin};

/// Module init function type.
pub type ModuleInitFn = Box<dyn FnOnce(&PluginContext) -> AtomResult<()> + Send>;

/// Registration for a module's lifecycle (init/shutdown). Method dispatch is
/// handled by CNG-generated per-method FFI exports, not the runtime kernel.
pub struct ModuleRegistration {
    pub id: String,
    pub init_order: usize,
    pub init_fn: ModuleInitFn,
    pub shutdown_fn: Option<Box<dyn FnOnce() + Send>>,
}

#[derive(Default)]
pub struct RuntimeConfig {
    pub plugins: Vec<Box<dyn RuntimePlugin>>,
    pub modules: Vec<ModuleRegistration>,
}
