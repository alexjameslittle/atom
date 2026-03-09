use std::fmt;

use atom_ffi::{AtomError, AtomErrorCode, AtomLifecycleEvent, AtomResult, AtomRuntimeHandle};

use crate::config::RuntimeConfig;
use crate::plugin::{PluginContext, RuntimePlugin};
use crate::state::{RuntimeState, validate_transition};

pub(crate) struct Runtime {
    state: RuntimeState,
    module_ids: Vec<String>,
    module_shutdown_fns: Vec<Option<Box<dyn FnOnce() + Send>>>,
    plugins: Vec<Box<dyn RuntimePlugin>>,
    tokio_handle: tokio::runtime::Handle,
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
            .field("state", &self.state)
            .field("module_ids", &self.module_ids)
            .finish_non_exhaustive()
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if self.state != RuntimeState::Terminated {
            self.shutdown();
        }
    }
}

impl Runtime {
    /// Create and start a runtime with the given config. Runs the full init
    /// sequence: Created → Initializing → (modules) → (plugins) → Running.
    pub(crate) fn start(handle: AtomRuntimeHandle, mut config: RuntimeConfig) -> AtomResult<Self> {
        let tokio_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| {
                AtomError::new(
                    AtomErrorCode::BridgeInitFailed,
                    format!("tokio init: {err}"),
                )
            })?;
        let tokio_handle = tokio_runtime.handle().clone();

        let mut runtime = Self {
            state: RuntimeState::Created,
            module_ids: Vec::new(),
            module_shutdown_fns: Vec::new(),
            plugins: Vec::new(),
            tokio_handle,
            rt: tokio_runtime,
        };

        runtime.state = RuntimeState::Initializing;
        tracing::info!("runtime initializing");

        // Sort modules by init_order, then init in order.
        config.modules.sort_by_key(|m| m.init_order);

        let ctx = PluginContext {
            handle,
            tokio_handle: &runtime.tokio_handle,
        };

        for module in config.modules {
            tracing::info!(module_id = %module.id, "initializing module");
            if let Err(err) = (module.init_fn)(&ctx) {
                tracing::error!(module_id = %module.id, error = %err, "module init failed");
                runtime.state = RuntimeState::Failed;
                return Err(AtomError::new(
                    AtomErrorCode::ModuleInitFailed,
                    format!("module '{}' init failed: {err}", module.id),
                ));
            }
            runtime.module_ids.push(module.id);
            runtime.module_shutdown_fns.push(module.shutdown_fn);
        }

        // Init plugins in registration order.
        for plugin in &mut config.plugins {
            tracing::info!(plugin_id = %plugin.id(), "initializing plugin");
            if let Err(err) = plugin.on_init(&ctx) {
                tracing::error!(plugin_id = %plugin.id(), error = %err, "plugin init failed");
                runtime.state = RuntimeState::Failed;
                return Err(AtomError::new(
                    AtomErrorCode::ModuleInitFailed,
                    format!("plugin '{}' init failed: {err}", plugin.id()),
                ));
            }
        }
        runtime.plugins = config.plugins;

        runtime.state = RuntimeState::Running;
        tracing::info!("runtime running");

        Ok(runtime)
    }

    pub(crate) fn state(&self) -> RuntimeState {
        self.state
    }

    pub(crate) fn handle_event(&mut self, event: AtomLifecycleEvent) -> AtomResult<()> {
        let new_state = validate_transition(self.state, event)?;
        self.state = new_state;
        tracing::info!(?event, ?new_state, "lifecycle transition");

        for plugin in &mut self.plugins {
            plugin.on_lifecycle(event, new_state);
        }

        // If we transitioned to Terminating, complete the shutdown.
        if new_state == RuntimeState::Terminating {
            self.complete_shutdown();
        }

        Ok(())
    }

    pub(crate) fn shutdown(&mut self) {
        if self.state == RuntimeState::Terminated {
            return;
        }

        self.state = RuntimeState::Terminating;
        tracing::info!("runtime shutting down");
        self.complete_shutdown();
    }

    fn complete_shutdown(&mut self) {
        // Shutdown plugins in reverse registration order.
        for plugin in self.plugins.iter_mut().rev() {
            tracing::info!(plugin_id = %plugin.id(), "shutting down plugin");
            plugin.on_shutdown();
        }

        // Shutdown modules in reverse init order.
        while let Some(shutdown_fn) = self.module_shutdown_fns.pop() {
            let module_id = self.module_ids.pop().unwrap_or_default();
            if let Some(f) = shutdown_fn {
                tracing::info!(module_id = %module_id, "shutting down module");
                f();
            }
        }

        self.state = RuntimeState::Terminated;
        tracing::info!("runtime terminated");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use atom_ffi::{AtomError, AtomErrorCode, AtomLifecycleEvent};

    use crate::config::{ModuleRegistration, RuntimeConfig};
    use crate::state::RuntimeState;

    use super::Runtime;

    fn empty_config() -> RuntimeConfig {
        RuntimeConfig::default()
    }

    #[test]
    fn init_with_no_modules_succeeds() {
        let rt = Runtime::start(1, empty_config()).expect("should start");
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn init_calls_modules_in_order() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let o1 = Arc::clone(&order);
        let o2 = Arc::clone(&order);

        let config = RuntimeConfig {
            plugins: Vec::new(),
            modules: vec![
                ModuleRegistration {
                    id: "second".to_owned(),
                    init_order: 2,
                    init_fn: Box::new(move |_| {
                        o2.lock().unwrap().push("second");
                        Ok(())
                    }),
                    shutdown_fn: None,
                },
                ModuleRegistration {
                    id: "first".to_owned(),
                    init_order: 1,
                    init_fn: Box::new(move |_| {
                        o1.lock().unwrap().push("first");
                        Ok(())
                    }),
                    shutdown_fn: None,
                },
            ],
        };

        let _rt = Runtime::start(1, config).expect("should start");
        assert_eq!(*order.lock().unwrap(), vec!["first", "second"]);
    }

    #[test]
    fn failing_module_transitions_to_failed() {
        let config = RuntimeConfig {
            plugins: Vec::new(),
            modules: vec![ModuleRegistration {
                id: "bad".to_owned(),
                init_order: 0,
                init_fn: Box::new(|_| Err(AtomError::new(AtomErrorCode::ModuleInitFailed, "boom"))),
                shutdown_fn: None,
            }],
        };

        let err = Runtime::start(1, config).unwrap_err();
        assert_eq!(err.code, AtomErrorCode::ModuleInitFailed);
    }

    #[test]
    fn partial_init_failure_cleans_up_earlier_modules() {
        let shutdown_called = Arc::new(Mutex::new(false));
        let sc = Arc::clone(&shutdown_called);

        let config = RuntimeConfig {
            plugins: Vec::new(),
            modules: vec![
                ModuleRegistration {
                    id: "good".to_owned(),
                    init_order: 1,
                    init_fn: Box::new(|_| Ok(())),
                    shutdown_fn: Some(Box::new(move || {
                        *sc.lock().unwrap() = true;
                    })),
                },
                ModuleRegistration {
                    id: "bad".to_owned(),
                    init_order: 2,
                    init_fn: Box::new(|_| {
                        Err(AtomError::new(AtomErrorCode::ModuleInitFailed, "boom"))
                    }),
                    shutdown_fn: None,
                },
            ],
        };

        let err = Runtime::start(1, config).unwrap_err();
        assert_eq!(err.code, AtomErrorCode::ModuleInitFailed);
        // Drop impl should have called shutdown on the "good" module.
        assert!(*shutdown_called.lock().unwrap());
    }

    #[test]
    fn shutdown_calls_modules_in_reverse_order() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let o1 = Arc::clone(&order);
        let o2 = Arc::clone(&order);

        let config = RuntimeConfig {
            plugins: Vec::new(),
            modules: vec![
                ModuleRegistration {
                    id: "first".to_owned(),
                    init_order: 1,
                    init_fn: Box::new(|_| Ok(())),
                    shutdown_fn: Some(Box::new(move || {
                        o1.lock().unwrap().push("first");
                    })),
                },
                ModuleRegistration {
                    id: "second".to_owned(),
                    init_order: 2,
                    init_fn: Box::new(|_| Ok(())),
                    shutdown_fn: Some(Box::new(move || {
                        o2.lock().unwrap().push("second");
                    })),
                },
            ],
        };

        let mut rt = Runtime::start(1, config).expect("should start");
        rt.shutdown();
        assert_eq!(rt.state(), RuntimeState::Terminated);
        assert_eq!(*order.lock().unwrap(), vec!["second", "first"]);
    }

    #[test]
    fn lifecycle_transitions_correctly() {
        let mut rt = Runtime::start(1, empty_config()).expect("should start");
        assert_eq!(rt.state(), RuntimeState::Running);

        rt.handle_event(AtomLifecycleEvent::Background).unwrap();
        assert_eq!(rt.state(), RuntimeState::Backgrounded);

        rt.handle_event(AtomLifecycleEvent::Foreground).unwrap();
        assert_eq!(rt.state(), RuntimeState::Running);

        rt.handle_event(AtomLifecycleEvent::Background).unwrap();
        rt.handle_event(AtomLifecycleEvent::Suspend).unwrap();
        assert_eq!(rt.state(), RuntimeState::Suspended);

        rt.handle_event(AtomLifecycleEvent::Resume).unwrap();
        assert_eq!(rt.state(), RuntimeState::Running);

        rt.handle_event(AtomLifecycleEvent::Terminate).unwrap();
        assert_eq!(rt.state(), RuntimeState::Terminated);
    }

    #[test]
    fn terminate_triggers_shutdown() {
        let shutdown_called = Arc::new(Mutex::new(false));
        let sc = Arc::clone(&shutdown_called);

        let config = RuntimeConfig {
            plugins: Vec::new(),
            modules: vec![ModuleRegistration {
                id: "m".to_owned(),
                init_order: 0,
                init_fn: Box::new(|_| Ok(())),
                shutdown_fn: Some(Box::new(move || {
                    *sc.lock().unwrap() = true;
                })),
            }],
        };

        let mut rt = Runtime::start(1, config).expect("should start");
        rt.handle_event(AtomLifecycleEvent::Terminate).unwrap();
        assert_eq!(rt.state(), RuntimeState::Terminated);
        assert!(*shutdown_called.lock().unwrap());
    }
}
