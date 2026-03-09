use atom_runtime::{
    AtomResult, LifecycleTransition, RuntimeApp, RuntimeBuilder, RuntimeContext, RuntimeLogLevel,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelloState {
    pub launch_count: u32,
    pub counter: u32,
    pub async_status: Option<String>,
    pub device_info: Option<String>,
    pub echoed_text: Option<String>,
    pub lifecycle: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelloEvent {
    Boot,
    Increment,
    DeviceInfoLoaded(String),
    EchoLoaded(String),
    AsyncReady(String),
}

#[derive(Debug, Clone, Copy)]
pub struct HelloApp;

impl RuntimeApp for HelloApp {
    type State = HelloState;
    type Event = HelloEvent;

    fn name(&self) -> &'static str {
        "hello_atom"
    }

    fn initial_state(&self) -> Self::State {
        HelloState::default()
    }

    fn on_start(
        &self,
        state: &mut Self::State,
        ctx: &mut RuntimeContext<Self::Event>,
    ) -> AtomResult<()> {
        state.launch_count += 1;
        state.lifecycle.push("runtime_started".to_owned());
        ctx.log(
            RuntimeLogLevel::Info,
            "hello_app",
            "bootstrapping hello atom app",
        );
        ctx.dispatch(HelloEvent::Boot);
        Ok(())
    }

    fn on_lifecycle(
        &self,
        state: &mut Self::State,
        transition: LifecycleTransition,
        ctx: &mut RuntimeContext<Self::Event>,
    ) -> AtomResult<()> {
        state
            .lifecycle
            .push(format!("{:?}->{:?}", transition.from, transition.to));
        ctx.log_with_fields(
            RuntimeLogLevel::Info,
            "hello_app.lifecycle",
            "observed lifecycle transition",
            [
                ("from", format!("{:?}", transition.from)),
                ("to", format!("{:?}", transition.to)),
            ],
        );
        Ok(())
    }

    fn on_event(
        &self,
        state: &mut Self::State,
        event: Self::Event,
        ctx: &mut RuntimeContext<Self::Event>,
    ) -> AtomResult<()> {
        match event {
            HelloEvent::Boot => {
                state.counter += 1;
                ctx.log(RuntimeLogLevel::Info, "hello_app", "boot event reduced");
                ctx.call_module("device_info", "get", "", HelloEvent::DeviceInfoLoaded);
                ctx.call_module("native_echo", "echo", "hello atom", HelloEvent::EchoLoaded);
                ctx.spawn_task("warmup", || {
                    Ok(HelloEvent::AsyncReady("warmup complete".to_owned()))
                });
            }
            HelloEvent::Increment => {
                state.counter += 1;
                ctx.log(
                    RuntimeLogLevel::Debug,
                    "hello_app",
                    "increment event reduced",
                );
            }
            HelloEvent::DeviceInfoLoaded(value) => {
                state.device_info = Some(value);
                ctx.log(
                    RuntimeLogLevel::Info,
                    "hello_app.module",
                    "captured device info",
                );
            }
            HelloEvent::EchoLoaded(value) => {
                state.echoed_text = Some(value);
                ctx.log(
                    RuntimeLogLevel::Info,
                    "hello_app.module",
                    "captured native echo response",
                );
            }
            HelloEvent::AsyncReady(value) => {
                state.async_status = Some(value);
                ctx.log(
                    RuntimeLogLevel::Info,
                    "hello_app.task",
                    "async warmup finished",
                );
            }
        }
        Ok(())
    }
}

#[must_use]
pub fn build_runtime() -> RuntimeBuilder<HelloApp> {
    RuntimeBuilder::new(HelloApp)
}

#[must_use]
pub fn bootstrap_message() -> &'static str {
    "hello atom"
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::thread;

    use atom_runtime::{
        AtomError, AtomErrorCode, AtomLifecycleEvent, RuntimeKernel, RuntimeModule, RuntimePlugin,
        RuntimeState,
    };

    use super::{
        HelloApp, HelloEvent, HelloState, RuntimeBuilder, RuntimeContext, RuntimeLogLevel,
        build_runtime,
    };

    struct DeviceInfoModule;

    impl RuntimeModule for DeviceInfoModule {
        fn id(&self) -> &'static str {
            "device_info"
        }

        fn call(&mut self, method: &str, _payload: &str) -> atom_runtime::AtomResult<String> {
            match method {
                "get" => Ok("platform=simulator".to_owned()),
                other => Err(AtomError::new(
                    AtomErrorCode::BridgeInvalidArgument,
                    format!("unsupported device_info method: {other}"),
                )),
            }
        }
    }

    struct NativeEchoModule;

    impl RuntimeModule for NativeEchoModule {
        fn id(&self) -> &'static str {
            "native_echo"
        }

        fn call(&mut self, method: &str, payload: &str) -> atom_runtime::AtomResult<String> {
            match method {
                "echo" => Ok(format!("echo:{payload}")),
                other => Err(AtomError::new(
                    AtomErrorCode::BridgeInvalidArgument,
                    format!("unsupported native_echo method: {other}"),
                )),
            }
        }
    }

    struct AuditPlugin {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl AuditPlugin {
        fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
            Self { events }
        }

        fn push(&self, value: String) {
            self.events
                .lock()
                .expect("audit plugin events lock should not poison")
                .push(value);
        }
    }

    impl RuntimePlugin<HelloState, HelloEvent> for AuditPlugin {
        fn id(&self) -> &'static str {
            "audit"
        }

        fn on_runtime_started(
            &mut self,
            _state: &HelloState,
            ctx: &mut RuntimeContext<HelloEvent>,
        ) -> atom_runtime::AtomResult<()> {
            self.push("runtime_started".to_owned());
            ctx.log(
                RuntimeLogLevel::Info,
                "audit",
                "runtime started hook observed",
            );
            Ok(())
        }

        fn on_lifecycle(
            &mut self,
            _state: &HelloState,
            transition: atom_runtime::LifecycleTransition,
            _ctx: &mut RuntimeContext<HelloEvent>,
        ) -> atom_runtime::AtomResult<()> {
            self.push(format!(
                "lifecycle:{:?}->{:?}",
                transition.from, transition.to
            ));
            Ok(())
        }

        fn before_event(
            &mut self,
            _state: &HelloState,
            event: &HelloEvent,
            _ctx: &mut RuntimeContext<HelloEvent>,
        ) -> atom_runtime::AtomResult<()> {
            self.push(format!("before:{event:?}"));
            Ok(())
        }

        fn after_event(
            &mut self,
            _state: &HelloState,
            event: &HelloEvent,
            _ctx: &mut RuntimeContext<HelloEvent>,
        ) -> atom_runtime::AtomResult<()> {
            self.push(format!("after:{event:?}"));
            Ok(())
        }
    }

    fn build_test_runtime(audit: Arc<Mutex<Vec<String>>>) -> RuntimeKernel<HelloApp> {
        RuntimeBuilder::new(HelloApp)
            .module(DeviceInfoModule)
            .module(NativeEchoModule)
            .plugin(AuditPlugin::new(audit))
            .start()
            .expect("hello app runtime should start")
    }

    fn pump_until_async_ready(runtime: &mut RuntimeKernel<HelloApp>) {
        for _ in 0..200 {
            if runtime.state().async_status.is_some() {
                return;
            }
            runtime.pump().expect("runtime pump should succeed");
            thread::yield_now();
        }
        panic!("async warmup never completed");
    }

    #[test]
    fn hello_app_boots_with_state_changes_async_work_and_module_calls() {
        let audit = Arc::new(Mutex::new(Vec::new()));
        let mut runtime = build_test_runtime(audit.clone());

        assert_eq!(runtime.lifecycle_state(), RuntimeState::Running);
        assert_eq!(
            runtime.module_ids(),
            &["device_info".to_owned(), "native_echo".to_owned()]
        );
        assert_eq!(runtime.state().launch_count, 1);
        assert_eq!(runtime.state().counter, 1);
        assert_eq!(
            runtime.state().device_info.as_deref(),
            Some("platform=simulator")
        );
        assert_eq!(
            runtime.state().echoed_text.as_deref(),
            Some("echo:hello atom")
        );

        pump_until_async_ready(&mut runtime);
        assert_eq!(
            runtime.state().async_status.as_deref(),
            Some("warmup complete")
        );

        runtime
            .dispatch(HelloEvent::Increment)
            .expect("increment should dispatch");
        assert_eq!(runtime.state().counter, 2);

        runtime
            .handle_lifecycle(AtomLifecycleEvent::Background)
            .expect("background should succeed");
        runtime
            .handle_lifecycle(AtomLifecycleEvent::Suspend)
            .expect("suspend should succeed");
        runtime
            .handle_lifecycle(AtomLifecycleEvent::Resume)
            .expect("resume should succeed");
        runtime
            .handle_lifecycle(AtomLifecycleEvent::Terminate)
            .expect("terminate should succeed");
        assert_eq!(runtime.lifecycle_state(), RuntimeState::Terminated);

        let events = audit
            .lock()
            .expect("audit plugin events lock should not poison")
            .clone();
        assert!(events.iter().any(|entry| entry == "runtime_started"));
        assert!(events.iter().any(|entry| entry == "before:Boot"));
        assert!(events.iter().any(|entry| entry == "after:Increment"));
        assert!(
            events
                .iter()
                .any(|entry| entry == "lifecycle:Running->Backgrounded")
        );

        let log_targets: Vec<_> = runtime
            .logs()
            .iter()
            .map(|entry| entry.target.as_str())
            .collect();
        assert!(log_targets.contains(&"runtime"));
        assert!(log_targets.contains(&"runtime.module"));
        assert!(log_targets.contains(&"runtime.task"));
        assert!(log_targets.contains(&"hello_app"));
        assert!(log_targets.contains(&"audit"));
    }

    #[test]
    fn builder_surface_stays_available_for_host_bootstrap() {
        let _builder = build_runtime();
    }
}
