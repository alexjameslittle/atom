use std::future::Future;
use std::sync::Arc;

use atom_ffi::{AtomLifecycleEvent, AtomResult, AtomRuntimeHandle};

use crate::state::RuntimeState;
use crate::store::{RuntimeEffect, RuntimeEvent, RuntimeHost, RuntimeSnapshot};

/// Context available to plugins during init and event handling.
#[derive(Clone)]
pub struct PluginContext<'a> {
    pub handle: AtomRuntimeHandle,
    pub tokio_handle: tokio::runtime::Handle,
    pub(crate) host: Arc<RuntimeHost>,
    pub(crate) _marker: std::marker::PhantomData<&'a ()>,
}

impl PluginContext<'_> {
    pub(crate) fn new(
        handle: AtomRuntimeHandle,
        tokio_handle: tokio::runtime::Handle,
        host: Arc<RuntimeHost>,
    ) -> Self {
        Self {
            handle,
            tokio_handle,
            host,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn set_state(&self, key: impl Into<String>, value: impl Into<String>) {
        self.host.set_value(key, value);
    }

    #[must_use]
    pub fn state_value(&self, key: &str) -> Option<String> {
        self.host.value(key)
    }

    pub fn dispatch_event(&self, event: RuntimeEvent) {
        self.host.dispatch_event(event);
    }

    pub fn emit_effect(&self, effect: RuntimeEffect) {
        self.host.emit_effect(effect);
    }

    /// # Errors
    ///
    /// Returns an error if the runtime is not yet `Running`, or if the task
    /// future resolves to an `AtomError`.
    pub fn run_task<F, T>(&self, plugin_id: &str, task_name: &str, future: F) -> AtomResult<T>
    where
        F: Future<Output = AtomResult<T>>,
    {
        self.host.run_task(self, plugin_id, task_name, future)
    }

    #[must_use]
    pub fn snapshot(&self) -> RuntimeSnapshot {
        self.host.snapshot()
    }
}

/// A runtime plugin that observes lifecycle events and may own plugin-local state.
///
/// Plugins MUST use the kernel's dispatch, lifecycle, and task-execution
/// semantics. They MUST NOT introduce a second lifecycle model.
pub trait RuntimePlugin: Send + Sync {
    /// Unique identifier for this plugin.
    fn id(&self) -> &str;

    /// Called during runtime init before the runtime reaches `Running`.
    /// Plugins init in registration order.
    ///
    /// # Errors
    ///
    /// Plugin implementations may return an error if initialization fails, which will cause
    /// the runtime to transition to the Failed state.
    fn on_init(&mut self, _ctx: &PluginContext) -> AtomResult<()> {
        Ok(())
    }

    /// Called once the runtime reaches the `Running` state and public runtime APIs are available.
    ///
    /// # Errors
    ///
    /// Plugin implementations may return an error if their runtime-startup
    /// work fails, which will fail runtime startup.
    fn on_running(&mut self, _ctx: &PluginContext) -> AtomResult<()> {
        Ok(())
    }

    /// Called when the runtime transitions states.
    fn on_lifecycle(&mut self, _event: AtomLifecycleEvent, _state: RuntimeState) {}

    /// Called during runtime shutdown, in reverse registration order.
    fn on_shutdown(&mut self) {}
}
