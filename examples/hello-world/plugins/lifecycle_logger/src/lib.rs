use atom_ffi::{AtomLifecycleEvent, AtomResult};
use atom_runtime::{self, RuntimeEvent, RuntimeState};
use device_info::get as device_info;

pub struct LifecycleLogger;

impl LifecycleLogger {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for LifecycleLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl LifecycleLogger {
    #[must_use]
    pub fn id(&self) -> &'static str {
        "hello_world.lifecycle_logger"
    }

    pub fn record_initialized(&self) {
        atom_runtime::set_state("plugins.lifecycle_logger", "initialized");
        atom_runtime::dispatch_event(RuntimeEvent::plugin(self.id(), "initialized", None));
        tracing::info!(
            plugin_id = self.id(),
            "hello-world lifecycle logger initialized"
        );
    }

    /// Complete the example warmup probe against the runtime singleton.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime has not been initialized or is not in
    /// the `Running` state.
    pub fn record_running(&self) -> AtomResult<()> {
        atom_runtime::ensure_running()?;
        atom_runtime::tokio_handle().block_on(async {
            tokio::task::yield_now().await;
        });
        atom_runtime::dispatch_event(RuntimeEvent::plugin(self.id(), "warmup_completed", None));
        atom_runtime::set_state("plugins.lifecycle_logger.async", "completed");

        let info = device_info();
        atom_runtime::set_state("plugins.lifecycle_logger.device_info_model", &info.model);
        atom_runtime::set_state("plugins.lifecycle_logger.device_info_os", &info.os);
        atom_runtime::dispatch_event(RuntimeEvent::plugin(
            self.id(),
            "running",
            Some(format!("device_info={} {}", info.model, info.os)),
        ));

        if let Some(snapshot) = atom_runtime::current_snapshot() {
            tracing::info!(
                plugin_id = self.id(),
                state_keys = snapshot.values.len(),
                event_count = snapshot.events.len(),
                effect_count = snapshot.effects.len(),
                "hello-world lifecycle logger completed runtime probe"
            );
        }

        Ok(())
    }

    pub fn observe_lifecycle(&self, event: AtomLifecycleEvent, state: RuntimeState) {
        tracing::info!(
            plugin_id = self.id(),
            ?event,
            ?state,
            "hello-world lifecycle logger observed lifecycle change"
        );
    }

    pub fn record_shutdown(&self) {
        tracing::info!(
            plugin_id = self.id(),
            "hello-world lifecycle logger shutdown"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::LifecycleLogger;

    #[test]
    fn initialized_marker_does_not_require_runtime() {
        LifecycleLogger::new().record_initialized();
    }

    #[test]
    fn running_probe_requires_runtime() {
        let error = LifecycleLogger::new()
            .record_running()
            .expect_err("running probe should require runtime");
        assert_eq!(error.code, atom_ffi::AtomErrorCode::BridgeInitFailed);
    }
}
