use std::fmt;
use std::sync::Arc;

use atom_ffi::{AtomError, AtomErrorCode, AtomLifecycleEvent, AtomResult};

use crate::config::RuntimeConfig;
use crate::state::{RuntimeState, validate_transition};
use crate::store::{RuntimeEvent, RuntimeHost, RuntimeSnapshot};

pub(crate) struct Runtime {
    host: Arc<RuntimeHost>,
    #[allow(
        clippy::allow_attributes,
        dead_code,
        reason = "tokio_runtime must be kept alive to prevent runtime shutdown"
    )]
    rt: tokio::runtime::Runtime,
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if self.state() != RuntimeState::Terminated {
            self.shutdown();
        }
    }
}

impl Runtime {
    /// Create and start a runtime with the given config. Runs the init
    /// sequence: Created -> Initializing -> Running.
    pub(crate) fn start(_config: RuntimeConfig) -> AtomResult<Self> {
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| {
                AtomError::new(
                    AtomErrorCode::BridgeInitFailed,
                    format!("tokio init: {err}"),
                )
            })?;
        let host = RuntimeHost::new();

        let runtime = Self {
            host,
            rt: tokio_runtime,
        };

        runtime.host.set_lifecycle(RuntimeState::Initializing);
        tracing::info!("runtime initializing");
        runtime.host.set_lifecycle(RuntimeState::Running);
        tracing::info!("runtime running");

        Ok(runtime)
    }

    pub(crate) fn state(&self) -> RuntimeState {
        self.host.snapshot().lifecycle
    }

    pub(crate) fn snapshot(&self) -> RuntimeSnapshot {
        self.host.snapshot()
    }

    pub(crate) fn tokio_handle(&self) -> tokio::runtime::Handle {
        self.rt.handle().clone()
    }

    pub(crate) fn set_state(&self, key: &str, value: &str) {
        self.host.set_value(key, value);
    }

    pub(crate) fn state_value(&self, key: &str) -> Option<String> {
        self.host.value(key)
    }

    pub(crate) fn dispatch_event(&self, event: RuntimeEvent) {
        self.host.dispatch_event(event);
    }

    pub(crate) fn handle_event(&self, event: AtomLifecycleEvent) -> AtomResult<()> {
        let new_state = validate_transition(self.state(), event)?;
        self.host.set_lifecycle(new_state);
        self.host.dispatch_event(RuntimeEvent::Lifecycle {
            event,
            state: new_state,
        });
        tracing::info!(?event, ?new_state, "lifecycle transition");

        if new_state == RuntimeState::Terminating {
            self.complete_shutdown();
        }

        Ok(())
    }

    pub(crate) fn shutdown(&self) {
        if self.state() == RuntimeState::Terminated {
            return;
        }

        self.host.set_lifecycle(RuntimeState::Terminating);
        tracing::info!("runtime shutting down");
        self.complete_shutdown();
    }

    fn complete_shutdown(&self) {
        self.host.set_lifecycle(RuntimeState::Terminated);
        tracing::info!("runtime terminated");
    }
}

#[cfg(test)]
mod tests {
    use atom_ffi::AtomLifecycleEvent;

    use crate::config::RuntimeConfig;
    use crate::state::RuntimeState;
    use crate::store::RuntimeEvent;

    use super::Runtime;

    fn empty_config() -> RuntimeConfig {
        RuntimeConfig::default()
    }

    #[test]
    fn init_with_empty_config_succeeds() {
        let rt = Runtime::start(empty_config()).expect("should start");
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn tokio_handle_can_drive_async_work() {
        let rt = Runtime::start(empty_config()).expect("should start");
        let value = rt.tokio_handle().block_on(async {
            tokio::task::yield_now().await;
            7
        });

        assert_eq!(value, 7);
    }

    #[test]
    fn state_and_events_update_snapshot() {
        let rt = Runtime::start(empty_config()).expect("should start");
        rt.set_state("app.phase", "running");
        rt.dispatch_event(RuntimeEvent::plugin(
            "hello_world.lifecycle_logger",
            "initialized",
            None,
        ));

        let snapshot = rt.snapshot();
        assert_eq!(
            snapshot.values.get("app.phase").map(String::as_str),
            Some("running"),
        );
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.effects.len(), 1);
    }

    #[test]
    fn lifecycle_events_update_state() {
        let rt = Runtime::start(empty_config()).expect("should start");
        rt.handle_event(AtomLifecycleEvent::Background)
            .expect("background should succeed");
        assert_eq!(rt.state(), RuntimeState::Backgrounded);

        rt.handle_event(AtomLifecycleEvent::Foreground)
            .expect("foreground should succeed");
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn terminate_moves_runtime_to_terminated() {
        let rt = Runtime::start(empty_config()).expect("should start");
        rt.handle_event(AtomLifecycleEvent::Terminate)
            .expect("terminate should succeed");

        assert_eq!(rt.state(), RuntimeState::Terminated);
    }

    #[test]
    fn shutdown_is_idempotent() {
        let rt = Runtime::start(empty_config()).expect("should start");
        rt.shutdown();
        rt.shutdown();

        assert_eq!(rt.state(), RuntimeState::Terminated);
    }
}
