use std::mem;
use std::sync::{Arc, Mutex, MutexGuard};

use atom_ffi::{AtomLifecycleEvent, AtomResult};
use atom_runtime::{PluginContext, RuntimePlugin, RuntimeState};

const DEFAULT_NAMESPACE: &str = "app";
const PLUGIN_ID: &str = "atom.analytics";

#[derive(Debug, Clone, PartialEq, Eq)]
struct AnalyticsState {
    namespace: String,
    pending_events: Vec<String>,
    flushed_batches: Vec<Vec<String>>,
}

/// Shared app-facing handle for analytics state owned by `AnalyticsPlugin`.
#[derive(Clone, Debug)]
pub struct AnalyticsHandle {
    state: Arc<Mutex<AnalyticsState>>,
}

impl AnalyticsHandle {
    pub fn track(&self, event: impl Into<String>) {
        lock_state(&self.state).pending_events.push(event.into());
    }

    #[must_use]
    pub fn namespace(&self) -> String {
        lock_state(&self.state).namespace.clone()
    }

    #[must_use]
    pub fn pending_events(&self) -> Vec<String> {
        lock_state(&self.state).pending_events.clone()
    }

    #[must_use]
    pub fn flushed_batches(&self) -> Vec<Vec<String>> {
        lock_state(&self.state).flushed_batches.clone()
    }
}

/// First-party runtime plugin that buffers analytics events and flushes on lifecycle boundaries.
pub struct AnalyticsPlugin {
    state: Arc<Mutex<AnalyticsState>>,
}

impl AnalyticsPlugin {
    #[must_use]
    pub fn new(namespace: impl Into<String>) -> Self {
        let namespace = namespace.into();
        let namespace = if namespace.is_empty() {
            DEFAULT_NAMESPACE.to_owned()
        } else {
            namespace
        };
        Self {
            state: Arc::new(Mutex::new(AnalyticsState {
                namespace,
                pending_events: Vec::new(),
                flushed_batches: Vec::new(),
            })),
        }
    }

    #[must_use]
    pub fn handle(&self) -> AnalyticsHandle {
        AnalyticsHandle {
            state: Arc::clone(&self.state),
        }
    }

    fn flush_pending(&self, reason: &'static str) {
        let mut state = lock_state(&self.state);
        if state.pending_events.is_empty() {
            return;
        }

        let namespace = state.namespace.clone();
        let batch = mem::take(&mut state.pending_events);
        let batch_size = batch.len();
        state.flushed_batches.push(batch);
        tracing::info!(
            plugin_id = PLUGIN_ID,
            namespace = %namespace,
            reason,
            batch_size,
            flush_count = state.flushed_batches.len(),
            "analytics plugin flushed events"
        );
    }
}

impl RuntimePlugin for AnalyticsPlugin {
    fn id(&self) -> &str {
        PLUGIN_ID
    }

    fn on_init(&mut self, _ctx: &PluginContext) -> AtomResult<()> {
        let state = lock_state(&self.state);
        tracing::info!(
            plugin_id = PLUGIN_ID,
            namespace = %state.namespace,
            pending_events = state.pending_events.len(),
            "analytics plugin initialized"
        );
        Ok(())
    }

    fn on_lifecycle(&mut self, event: AtomLifecycleEvent, state: RuntimeState) {
        if matches!(
            state,
            RuntimeState::Backgrounded | RuntimeState::Terminating
        ) {
            self.flush_pending(match state {
                RuntimeState::Backgrounded => "backgrounded",
                RuntimeState::Terminating => "terminating",
                _ => unreachable!(),
            });
        }

        let state_ref = lock_state(&self.state);
        tracing::info!(
            plugin_id = PLUGIN_ID,
            namespace = %state_ref.namespace,
            ?event,
            ?state,
            pending_events = state_ref.pending_events.len(),
            flush_count = state_ref.flushed_batches.len(),
            "analytics plugin observed lifecycle change"
        );
    }

    fn on_shutdown(&mut self) {
        self.flush_pending("shutdown");
        let state = lock_state(&self.state);
        tracing::info!(
            plugin_id = PLUGIN_ID,
            namespace = %state.namespace,
            flush_count = state.flushed_batches.len(),
            "analytics plugin shutdown"
        );
    }
}

fn lock_state(state: &Arc<Mutex<AnalyticsState>>) -> MutexGuard<'_, AnalyticsState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use atom_ffi::AtomLifecycleEvent;
    use atom_runtime::{RuntimePlugin, RuntimeState};

    use super::{AnalyticsPlugin, DEFAULT_NAMESPACE};

    #[test]
    fn empty_namespace_falls_back_to_app() {
        let plugin = AnalyticsPlugin::new("");
        assert_eq!(plugin.handle().namespace(), DEFAULT_NAMESPACE);
        assert_eq!(plugin.handle().pending_events(), Vec::<String>::new());
    }

    #[test]
    fn background_flushes_pending_events() {
        let mut plugin = AnalyticsPlugin::new("hello_atom");
        let handle = plugin.handle();
        handle.track("runtime_configured");
        handle.track("device_info_requested");

        plugin.on_lifecycle(AtomLifecycleEvent::Background, RuntimeState::Backgrounded);

        assert!(handle.pending_events().is_empty());
        assert_eq!(
            handle.flushed_batches(),
            vec![vec![
                "runtime_configured".to_owned(),
                "device_info_requested".to_owned(),
            ]]
        );
    }

    #[test]
    fn shutdown_flushes_remaining_events() {
        let mut plugin = AnalyticsPlugin::new("hello_atom");
        let handle = plugin.handle();
        handle.track("runtime_started");

        plugin.on_shutdown();

        assert!(handle.pending_events().is_empty());
        assert_eq!(
            handle.flushed_batches(),
            vec![vec!["runtime_started".to_owned()]]
        );
    }
}
