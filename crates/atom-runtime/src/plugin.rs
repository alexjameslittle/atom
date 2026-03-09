use atom_ffi::{AtomLifecycleEvent, AtomResult, AtomRuntimeHandle};

use crate::state::RuntimeState;

/// Context available to plugins during init and event handling.
pub struct PluginContext<'a> {
    pub handle: AtomRuntimeHandle,
    pub tokio_handle: &'a tokio::runtime::Handle,
}

/// A runtime plugin that observes lifecycle events and may own plugin-local state.
///
/// Plugins MUST use the kernel's dispatch, lifecycle, and task-execution
/// semantics. They MUST NOT introduce a second lifecycle model.
pub trait RuntimePlugin: Send + Sync {
    /// Unique identifier for this plugin.
    fn id(&self) -> &str;

    /// Called during runtime init, after all modules are initialized.
    /// Plugins init in registration order.
    ///
    /// # Errors
    ///
    /// Plugin implementations may return an error if initialization fails, which will cause
    /// the runtime to transition to the Failed state.
    fn on_init(&mut self, _ctx: &PluginContext) -> AtomResult<()> {
        Ok(())
    }

    /// Called when the runtime transitions states.
    fn on_lifecycle(&mut self, _event: AtomLifecycleEvent, _state: RuntimeState) {}

    /// Called during runtime shutdown, in reverse registration order.
    fn on_shutdown(&mut self) {}
}
