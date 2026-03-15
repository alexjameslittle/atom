use std::sync::{Arc, Mutex, MutexGuard};

use atom_runtime::{self, RuntimeEvent};

const DEFAULT_NAMESPACE: &str = "app";
const EVENT_SOURCE_ID: &str = "atom.analytics";
const NAMESPACE_KEY: &str = "atom.analytics.namespace";
const PENDING_COUNT_KEY: &str = "atom.analytics.pending_count";

#[derive(Debug, Clone, PartialEq, Eq)]
struct AnalyticsState {
    namespace: String,
    pending_events: Vec<String>,
}

/// Shared app-facing handle for analytics state owned by `Analytics`.
#[derive(Clone, Debug)]
pub struct AnalyticsHandle {
    state: Arc<Mutex<AnalyticsState>>,
}

impl AnalyticsHandle {
    pub fn track(&self, event: impl Into<String>) {
        let event = event.into();
        let (namespace, pending_count) = {
            let mut state = lock_state(&self.state);
            state.pending_events.push(event.clone());
            (state.namespace.clone(), state.pending_events.len())
        };
        record_tracking_event(&namespace, pending_count, &event);
    }

    #[must_use]
    pub fn namespace(&self) -> String {
        lock_state(&self.state).namespace.clone()
    }

    #[must_use]
    pub fn pending_events(&self) -> Vec<String> {
        lock_state(&self.state).pending_events.clone()
    }
}

/// Plain analytics state that can publish tracking activity through `atom_runtime::*`.
pub struct Analytics {
    state: Arc<Mutex<AnalyticsState>>,
}

impl Analytics {
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
            })),
        }
    }

    #[must_use]
    pub fn handle(&self) -> AnalyticsHandle {
        AnalyticsHandle {
            state: Arc::clone(&self.state),
        }
    }
}

fn lock_state(state: &Arc<Mutex<AnalyticsState>>) -> MutexGuard<'_, AnalyticsState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn record_tracking_event(namespace: &str, pending_count: usize, event: &str) {
    atom_runtime::set_state(NAMESPACE_KEY, namespace);
    atom_runtime::set_state(PENDING_COUNT_KEY, &pending_count.to_string());
    atom_runtime::dispatch_event(RuntimeEvent::plugin(
        EVENT_SOURCE_ID,
        "track",
        Some(event.to_owned()),
    ));
}

#[cfg(test)]
mod tests {
    use super::{Analytics, DEFAULT_NAMESPACE};

    #[test]
    fn empty_namespace_falls_back_to_app() {
        let analytics = Analytics::new("");
        assert_eq!(analytics.handle().namespace(), DEFAULT_NAMESPACE);
        assert_eq!(analytics.handle().pending_events(), Vec::<String>::new());
    }

    #[test]
    fn track_buffers_pending_events_in_order() {
        let analytics = Analytics::new("hello_atom");
        let handle = analytics.handle();
        handle.track("runtime_configured");
        handle.track("device_info_requested");

        assert_eq!(
            handle.pending_events(),
            vec![
                "runtime_configured".to_owned(),
                "device_info_requested".to_owned(),
            ]
        );
    }
}
