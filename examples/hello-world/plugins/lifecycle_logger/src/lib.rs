use atom_ffi::{AtomLifecycleEvent, AtomResult};
use atom_runtime::{PluginContext, RuntimePlugin, RuntimeState};

pub struct LifecycleLoggerPlugin;

impl LifecycleLoggerPlugin {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for LifecycleLoggerPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimePlugin for LifecycleLoggerPlugin {
    fn id(&self) -> &'static str {
        "hello_world.lifecycle_logger"
    }

    fn on_init(&mut self, _ctx: &PluginContext) -> AtomResult<()> {
        tracing::info!(
            plugin_id = self.id(),
            "hello-world lifecycle logger initialized"
        );
        Ok(())
    }

    fn on_lifecycle(&mut self, event: AtomLifecycleEvent, state: RuntimeState) {
        tracing::info!(
            plugin_id = self.id(),
            ?event,
            ?state,
            "hello-world lifecycle logger observed lifecycle change"
        );
    }

    fn on_shutdown(&mut self) {
        tracing::info!(
            plugin_id = self.id(),
            "hello-world lifecycle logger shutdown"
        );
    }
}
