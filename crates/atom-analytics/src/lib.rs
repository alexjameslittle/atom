use std::mem;
use std::sync::{Arc, Mutex, MutexGuard};

use atom_runtime::{self, RuntimeEvent};

const DEFAULT_NAMESPACE: &str = "app";
const PLUGIN_ID: &str = "atom.analytics";
const NAMESPACE_KEY: &str = "atom.analytics.namespace";
const PENDING_COUNT_KEY: &str = "atom.analytics.pending_count";
const FLUSH_COUNT_KEY: &str = "atom.analytics.flush_count";

#[derive(Debug, Clone, PartialEq, Eq)]
struct AnalyticsState {
    namespace: String,
    pending_events: Vec<String>,
    flushed_batches: Vec<Vec<String>>,
}

/// Shared app-facing handle for analytics state owned by `AnalyticsRuntime`.
#[derive(Clone, Debug)]
pub struct AnalyticsHandle {
    state: Arc<Mutex<AnalyticsState>>,
}

impl AnalyticsHandle {
    pub fn track(&self, event: impl Into<String>) {
        let event = event.into();
        lock_state(&self.state).pending_events.push(event.clone());
        sync_runtime(&self.state, "track", Some(event));
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

/// Shared analytics state that can mirror updates into the runtime store.
pub struct AnalyticsRuntime {
    state: Arc<Mutex<AnalyticsState>>,
}

impl AnalyticsRuntime {
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

    pub fn flush(&self, reason: &'static str) {
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
            "analytics flushed events"
        );
        drop(state);
        sync_runtime(&self.state, "flush", Some(reason.to_owned()));
    }
}

fn lock_state(state: &Arc<Mutex<AnalyticsState>>) -> MutexGuard<'_, AnalyticsState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn sync_runtime(state: &Arc<Mutex<AnalyticsState>>, action: &'static str, detail: Option<String>) {
    let state = lock_state(state);
    let namespace = state.namespace.clone();
    let pending_count = state.pending_events.len().to_string();
    let flush_count = state.flushed_batches.len().to_string();
    drop(state);

    atom_runtime::set_state(NAMESPACE_KEY, &namespace);
    atom_runtime::set_state(PENDING_COUNT_KEY, &pending_count);
    atom_runtime::set_state(FLUSH_COUNT_KEY, &flush_count);
    atom_runtime::dispatch_event(RuntimeEvent::plugin(PLUGIN_ID, action, detail));
}

#[cfg(test)]
mod tests {
    use super::{AnalyticsRuntime, DEFAULT_NAMESPACE};

    #[test]
    fn empty_namespace_falls_back_to_app() {
        let runtime = AnalyticsRuntime::new("");
        assert_eq!(runtime.handle().namespace(), DEFAULT_NAMESPACE);
        assert_eq!(runtime.handle().pending_events(), Vec::<String>::new());
    }

    #[test]
    fn background_flushes_pending_events() {
        let runtime = AnalyticsRuntime::new("hello_atom");
        let handle = runtime.handle();
        handle.track("runtime_configured");
        handle.track("device_info_requested");

        runtime.flush("backgrounded");

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
        let runtime = AnalyticsRuntime::new("hello_atom");
        let handle = runtime.handle();
        handle.track("runtime_started");

        runtime.flush("shutdown");

        assert!(handle.pending_events().is_empty());
        assert_eq!(
            handle.flushed_batches(),
            vec![vec!["runtime_started".to_owned()]]
        );
    }
}
