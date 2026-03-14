use atom_ffi::{AtomLifecycleEvent, AtomResult};
use atom_runtime::{PluginContext, RuntimeEvent, RuntimePlugin, RuntimeState};
use device_info::{GetDeviceInfoRequest, GetDeviceInfoResponse, METHOD_GET, MODULE_ID};

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

    fn on_init(&mut self, ctx: &PluginContext) -> AtomResult<()> {
        ctx.set_state("plugins.lifecycle_logger", "initialized");
        ctx.dispatch_event(RuntimeEvent::plugin(self.id(), "initialized", None));
        tracing::info!(
            plugin_id = self.id(),
            "hello-world lifecycle logger initialized"
        );
        Ok(())
    }

    fn on_running(&mut self, ctx: &PluginContext) -> AtomResult<()> {
        ctx.run_task(self.id(), "warmup", async {
            tokio::task::yield_now().await;
            Ok(())
        })?;
        ctx.set_state("plugins.lifecycle_logger.async", "completed");

        let response: GetDeviceInfoResponse =
            ctx.call_module(MODULE_ID, METHOD_GET, GetDeviceInfoRequest {})?;
        ctx.set_state(
            "plugins.lifecycle_logger.device_info_model",
            response.model.clone(),
        );
        ctx.dispatch_event(RuntimeEvent::plugin(
            self.id(),
            "running",
            Some(format!("device_info_model={}", response.model)),
        ));

        let snapshot = ctx.snapshot();
        tracing::info!(
            plugin_id = self.id(),
            state_keys = snapshot.values.len(),
            event_count = snapshot.events.len(),
            effect_count = snapshot.effects.len(),
            module_calls = snapshot.module_calls.len(),
            "hello-world lifecycle logger completed runtime probe"
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
