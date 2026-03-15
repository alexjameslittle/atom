use std::fmt;
use std::sync::Arc;

use atom_ffi::{AtomError, AtomErrorCode, AtomLifecycleEvent, AtomResult, AtomRuntimeHandle};

use crate::config::RuntimeConfig;
use crate::plugin::{PluginContext, RuntimePlugin};
use crate::state::{RuntimeState, validate_transition};
use crate::store::{RuntimeEvent, RuntimeHost, RuntimeSnapshot};

pub(crate) struct Runtime {
    state: RuntimeState,
    plugins: Vec<Box<dyn RuntimePlugin>>,
    ctx: PluginContext<'static>,
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
            .field("state", &self.state)
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
    /// sequence: Created → Initializing → (plugins) → Running.
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
        let host = RuntimeHost::new();
        let ctx = PluginContext::new(handle, tokio_handle, Arc::clone(&host));

        let mut runtime = Self {
            state: RuntimeState::Created,
            plugins: Vec::new(),
            ctx,
            host,
            rt: tokio_runtime,
        };

        runtime.state = RuntimeState::Initializing;
        runtime.host.set_lifecycle(RuntimeState::Initializing);
        tracing::info!("runtime initializing");

        // Init plugins in registration order.
        for mut plugin in config.plugins.drain(..) {
            tracing::info!(plugin_id = %plugin.id(), "initializing plugin");
            if let Err(err) = plugin.on_init(&runtime.ctx) {
                tracing::error!(plugin_id = %plugin.id(), error = %err, "plugin init failed");
                runtime.state = RuntimeState::Failed;
                runtime.host.set_lifecycle(RuntimeState::Failed);
                return Err(AtomError::new(
                    AtomErrorCode::ModuleInitFailed,
                    format!("plugin '{}' init failed: {err}", plugin.id()),
                ));
            }
            runtime.plugins.push(plugin);
        }

        runtime.state = RuntimeState::Running;
        runtime.host.set_lifecycle(RuntimeState::Running);
        tracing::info!("runtime running");

        for plugin in &mut runtime.plugins {
            tracing::info!(plugin_id = %plugin.id(), "running plugin startup hook");
            if let Err(err) = plugin.on_running(&runtime.ctx) {
                tracing::error!(plugin_id = %plugin.id(), error = %err, "plugin running hook failed");
                runtime.state = RuntimeState::Failed;
                runtime.host.set_lifecycle(RuntimeState::Failed);
                return Err(AtomError::new(
                    AtomErrorCode::ModuleInitFailed,
                    format!("plugin '{}' running hook failed: {err}", plugin.id()),
                ));
            }
        }

        Ok(runtime)
    }

    pub(crate) fn state(&self) -> RuntimeState {
        self.state
    }

    pub(crate) fn snapshot(&self) -> RuntimeSnapshot {
        self.host.snapshot()
    }

    pub(crate) fn context(&self) -> PluginContext<'static> {
        self.ctx.clone()
    }

    pub(crate) fn handle_event(&mut self, event: AtomLifecycleEvent) -> AtomResult<()> {
        let new_state = validate_transition(self.state, event)?;
        self.state = new_state;
        self.host.set_lifecycle(new_state);
        self.host.dispatch_event(RuntimeEvent::Lifecycle {
            event,
            state: new_state,
        });
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
        self.host.set_lifecycle(RuntimeState::Terminating);
        tracing::info!("runtime shutting down");
        self.complete_shutdown();
    }

    fn complete_shutdown(&mut self) {
        // Shutdown plugins in reverse registration order.
        for plugin in self.plugins.iter_mut().rev() {
            tracing::info!(plugin_id = %plugin.id(), "shutting down plugin");
            plugin.on_shutdown();
        }

        self.state = RuntimeState::Terminated;
        self.host.set_lifecycle(RuntimeState::Terminated);
        tracing::info!("runtime terminated");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use atom_ffi::{AtomError, AtomErrorCode, AtomLifecycleEvent, AtomResult};

    use crate::config::RuntimeConfig;
    use crate::plugin::{PluginContext, RuntimePlugin};
    use crate::state::RuntimeState;
    use crate::store::{RuntimeEffect, RuntimeEvent};

    use super::Runtime;

    fn empty_config() -> RuntimeConfig {
        RuntimeConfig::default()
    }

    #[test]
    fn init_with_no_plugins_succeeds() {
        let rt = Runtime::start(1, empty_config()).expect("should start");
        assert_eq!(rt.state(), RuntimeState::Running);
    }

    #[test]
    fn partial_plugin_init_failure_cleans_up_earlier_plugins() {
        struct TestPlugin {
            id: &'static str,
            fail_init: bool,
            events: Arc<Mutex<Vec<String>>>,
            shutdown_called: Option<Arc<Mutex<bool>>>,
        }

        impl RuntimePlugin for TestPlugin {
            fn id(&self) -> &str {
                self.id
            }

            fn on_init(&mut self, _ctx: &PluginContext) -> AtomResult<()> {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("init:{}", self.id));
                if self.fail_init {
                    Err(AtomError::new(AtomErrorCode::ModuleInitFailed, "boom"))
                } else {
                    Ok(())
                }
            }

            fn on_shutdown(&mut self) {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("shutdown:{}", self.id));
                if let Some(called) = &self.shutdown_called {
                    *called.lock().unwrap() = true;
                }
            }
        }

        let events = Arc::new(Mutex::new(Vec::new()));
        let shutdown_called = Arc::new(Mutex::new(false));

        let config = RuntimeConfig {
            plugins: vec![
                Box::new(TestPlugin {
                    id: "good",
                    fail_init: false,
                    events: Arc::clone(&events),
                    shutdown_called: Some(Arc::clone(&shutdown_called)),
                }),
                Box::new(TestPlugin {
                    id: "bad",
                    fail_init: true,
                    events: Arc::clone(&events),
                    shutdown_called: None,
                }),
            ],
        };

        let err = Runtime::start(1, config).unwrap_err();
        assert_eq!(err.code, AtomErrorCode::ModuleInitFailed);
        assert_eq!(
            *events.lock().unwrap(),
            vec!["init:good", "init:bad", "shutdown:good"]
        );
        assert!(*shutdown_called.lock().unwrap());
    }

    #[test]
    fn failing_plugin_transitions_to_failed() {
        struct FailingPlugin;

        impl RuntimePlugin for FailingPlugin {
            fn id(&self) -> &str {
                "bad"
            }

            fn on_init(&mut self, _ctx: &PluginContext) -> AtomResult<()> {
                Err(AtomError::new(AtomErrorCode::ModuleInitFailed, "boom"))
            }
        }

        let err = Runtime::start(
            1,
            RuntimeConfig {
                plugins: vec![Box::new(FailingPlugin)],
            },
        )
        .unwrap_err();
        assert_eq!(err.code, AtomErrorCode::ModuleInitFailed);
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
        let config = RuntimeConfig {
            plugins: vec![Box::new(ShutdownProbe {
                called: Arc::clone(&shutdown_called),
            })],
        };

        let mut rt = Runtime::start(1, config).expect("should start");
        rt.handle_event(AtomLifecycleEvent::Terminate).unwrap();
        assert_eq!(rt.state(), RuntimeState::Terminated);
        assert!(*shutdown_called.lock().unwrap());
    }

    #[test]
    fn plugins_follow_registration_and_shutdown_order() {
        struct TestPlugin {
            id: &'static str,
            events: Arc<Mutex<Vec<String>>>,
        }

        impl RuntimePlugin for TestPlugin {
            fn id(&self) -> &str {
                self.id
            }

            fn on_init(&mut self, _ctx: &PluginContext) -> AtomResult<()> {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("init:{}", self.id));
                Ok(())
            }

            fn on_running(&mut self, _ctx: &PluginContext) -> AtomResult<()> {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("running:{}", self.id));
                Ok(())
            }

            fn on_lifecycle(&mut self, event: AtomLifecycleEvent, state: RuntimeState) {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("lifecycle:{}:{event:?}:{state:?}", self.id));
            }

            fn on_shutdown(&mut self) {
                self.events
                    .lock()
                    .unwrap()
                    .push(format!("shutdown:{}", self.id));
            }
        }

        let events = Arc::new(Mutex::new(Vec::new()));
        let config = RuntimeConfig::builder()
            .plugin(TestPlugin {
                id: "first",
                events: Arc::clone(&events),
            })
            .plugin(TestPlugin {
                id: "second",
                events: Arc::clone(&events),
            })
            .build();

        let mut rt = Runtime::start(1, config).expect("should start");
        rt.handle_event(AtomLifecycleEvent::Background).unwrap();
        rt.handle_event(AtomLifecycleEvent::Terminate).unwrap();

        assert_eq!(
            *events.lock().unwrap(),
            vec![
                "init:first",
                "init:second",
                "running:first",
                "running:second",
                "lifecycle:first:Background:Backgrounded",
                "lifecycle:second:Background:Backgrounded",
                "lifecycle:first:Terminate:Terminating",
                "lifecycle:second:Terminate:Terminating",
                "shutdown:second",
                "shutdown:first",
            ]
        );
    }

    #[test]
    fn running_hook_can_update_state_and_run_tasks() {
        struct ProbePlugin;

        impl RuntimePlugin for ProbePlugin {
            fn id(&self) -> &str {
                "probe"
            }

            fn on_running(&mut self, ctx: &PluginContext) -> AtomResult<()> {
                ctx.set_state("app.phase", "running");
                ctx.dispatch_event(RuntimeEvent::plugin(self.id(), "boot", None));
                ctx.run_task(self.id(), "yield", async {
                    tokio::task::yield_now().await;
                    Ok(())
                })?;
                ctx.set_state("app.ready", "true");
                Ok(())
            }
        }

        let config = RuntimeConfig {
            plugins: vec![Box::new(ProbePlugin)],
        };

        let rt = Runtime::start(1, config).expect("should start");
        let snapshot = rt.snapshot();

        assert_eq!(snapshot.lifecycle, RuntimeState::Running);
        assert_eq!(
            snapshot.values.get("app.phase").map(String::as_str),
            Some("running"),
        );
        assert_eq!(
            snapshot.values.get("app.ready").map(String::as_str),
            Some("true"),
        );
        assert!(snapshot
            .events
            .iter()
            .any(|record| matches!(record.event, RuntimeEvent::Plugin { ref plugin_id, ref name, .. } if plugin_id == "probe" && name == "boot")));
        assert!(snapshot.events.iter().any(|record| matches!(
            record.event,
            RuntimeEvent::TaskCompleted { ref plugin_id, ref task_name, success: true }
                if plugin_id == "probe" && task_name == "yield"
        )));
        assert!(snapshot.effects.iter().any(|record| matches!(
            record.effect,
            RuntimeEffect::TaskStarted { ref plugin_id, ref task_name }
                if plugin_id == "probe" && task_name == "yield"
        )));
    }

    struct ShutdownProbe {
        called: Arc<Mutex<bool>>,
    }

    impl RuntimePlugin for ShutdownProbe {
        fn id(&self) -> &str {
            "shutdown_probe"
        }

        fn on_init(&mut self, _ctx: &PluginContext) -> AtomResult<()> {
            Ok(())
        }

        fn on_shutdown(&mut self) {
            *self.called.lock().unwrap() = true;
        }
    }
}
