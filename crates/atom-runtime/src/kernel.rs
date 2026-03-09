use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Debug;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use atom_ffi::{AtomError, AtomErrorCode, AtomLifecycleEvent, AtomResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeState {
    Created,
    Initializing,
    Running,
    Backgrounded,
    Suspended,
    Terminating,
    Terminated,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifecycleTransition {
    pub from: RuntimeState,
    pub event: AtomLifecycleEvent,
    pub to: RuntimeState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLogEntry {
    pub sequence: u64,
    pub level: RuntimeLogLevel,
    pub target: String,
    pub message: String,
    pub fields: BTreeMap<String, String>,
}

struct PendingLogEntry {
    level: RuntimeLogLevel,
    target: String,
    message: String,
    fields: BTreeMap<String, String>,
}

enum RuntimeCommand<E> {
    Dispatch(E),
    Log(PendingLogEntry),
    CallModule(ModuleCall<E>),
    SpawnTask(TaskSpec<E>),
}

pub struct RuntimeContext<E> {
    commands: Vec<RuntimeCommand<E>>,
}

impl<E> Default for RuntimeContext<E> {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
        }
    }
}

impl<E> RuntimeContext<E> {
    pub fn dispatch(&mut self, event: E) {
        self.commands.push(RuntimeCommand::Dispatch(event));
    }

    pub fn log(
        &mut self,
        level: RuntimeLogLevel,
        target: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.log_with_fields(
            level,
            target,
            message,
            std::iter::empty::<(String, String)>(),
        );
    }

    pub fn log_with_fields<I, K, V>(
        &mut self,
        level: RuntimeLogLevel,
        target: impl Into<String>,
        message: impl Into<String>,
        fields: I,
    ) where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let fields = fields
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        self.commands.push(RuntimeCommand::Log(PendingLogEntry {
            level,
            target: target.into(),
            message: message.into(),
            fields,
        }));
    }

    pub fn spawn_task<F>(&mut self, label: impl Into<String>, work: F)
    where
        F: FnOnce() -> AtomResult<E> + Send + 'static,
    {
        self.commands
            .push(RuntimeCommand::SpawnTask(TaskSpec::new(label, work)));
    }

    pub fn call_module<F>(
        &mut self,
        module_id: impl Into<String>,
        method: impl Into<String>,
        payload: impl Into<String>,
        on_response: F,
    ) where
        F: FnOnce(String) -> E + Send + 'static,
    {
        self.commands
            .push(RuntimeCommand::CallModule(ModuleCall::new(
                module_id,
                method,
                payload,
                on_response,
            )));
    }

    fn into_commands(self) -> Vec<RuntimeCommand<E>> {
        self.commands
    }
}

struct TaskSpec<E> {
    label: String,
    work: Box<dyn FnOnce() -> AtomResult<E> + Send + 'static>,
}

impl<E> TaskSpec<E> {
    fn new<F>(label: impl Into<String>, work: F) -> Self
    where
        F: FnOnce() -> AtomResult<E> + Send + 'static,
    {
        Self {
            label: label.into(),
            work: Box::new(work),
        }
    }
}

struct ModuleCall<E> {
    module_id: String,
    method: String,
    payload: String,
    on_response: Box<dyn FnOnce(String) -> E + Send + 'static>,
}

impl<E> ModuleCall<E> {
    fn new<F>(
        module_id: impl Into<String>,
        method: impl Into<String>,
        payload: impl Into<String>,
        on_response: F,
    ) -> Self
    where
        F: FnOnce(String) -> E + Send + 'static,
    {
        Self {
            module_id: module_id.into(),
            method: method.into(),
            payload: payload.into(),
            on_response: Box::new(on_response),
        }
    }
}

struct TaskEnvelope<E> {
    id: u64,
    label: String,
    result: AtomResult<E>,
}

pub trait RuntimeApp: Send + 'static {
    type State: Clone + Send + 'static;
    type Event: Clone + Debug + Send + 'static;

    fn name(&self) -> &'static str;
    fn initial_state(&self) -> Self::State;

    /// # Errors
    ///
    /// Returns an error if app startup work cannot complete successfully.
    fn on_start(
        &self,
        _state: &mut Self::State,
        _ctx: &mut RuntimeContext<Self::Event>,
    ) -> AtomResult<()> {
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if lifecycle-specific app work fails.
    fn on_lifecycle(
        &self,
        _state: &mut Self::State,
        _transition: LifecycleTransition,
        _ctx: &mut RuntimeContext<Self::Event>,
    ) -> AtomResult<()> {
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if reducing the event fails and the runtime should
    /// enter a failed state.
    fn on_event(
        &self,
        state: &mut Self::State,
        event: Self::Event,
        ctx: &mut RuntimeContext<Self::Event>,
    ) -> AtomResult<()>;
}

pub trait RuntimeModule: Send {
    fn id(&self) -> &'static str;

    /// # Errors
    ///
    /// Returns an error if the module cannot finish runtime startup.
    fn init(&mut self) -> AtomResult<()> {
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the module cannot finish runtime shutdown.
    fn shutdown(&mut self) -> AtomResult<()> {
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the requested method or payload is invalid or the
    /// module backend fails.
    fn call(&mut self, method: &str, payload: &str) -> AtomResult<String>;
}

pub trait RuntimePlugin<S, E>: Send
where
    S: Clone + Send + 'static,
    E: Clone + Debug + Send + 'static,
{
    fn id(&self) -> &'static str;

    /// # Errors
    ///
    /// Returns an error if plugin startup work fails.
    fn on_runtime_started(&mut self, _state: &S, _ctx: &mut RuntimeContext<E>) -> AtomResult<()> {
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if lifecycle observation fails.
    fn on_lifecycle(
        &mut self,
        _state: &S,
        _transition: LifecycleTransition,
        _ctx: &mut RuntimeContext<E>,
    ) -> AtomResult<()> {
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if pre-dispatch plugin work fails.
    fn before_event(
        &mut self,
        _state: &S,
        _event: &E,
        _ctx: &mut RuntimeContext<E>,
    ) -> AtomResult<()> {
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if post-dispatch plugin work fails.
    fn after_event(
        &mut self,
        _state: &S,
        _event: &E,
        _ctx: &mut RuntimeContext<E>,
    ) -> AtomResult<()> {
        Ok(())
    }
}

pub struct RuntimeBuilder<A>
where
    A: RuntimeApp,
{
    app: A,
    modules: Vec<Box<dyn RuntimeModule>>,
    plugins: Vec<Box<dyn RuntimePlugin<A::State, A::Event>>>,
}

impl<A> RuntimeBuilder<A>
where
    A: RuntimeApp,
{
    #[must_use]
    pub fn new(app: A) -> Self {
        Self {
            app,
            modules: Vec::new(),
            plugins: Vec::new(),
        }
    }

    #[must_use]
    pub fn module<M>(mut self, module: M) -> Self
    where
        M: RuntimeModule + 'static,
    {
        self.modules.push(Box::new(module));
        self
    }

    #[must_use]
    pub fn plugin<P>(mut self, plugin: P) -> Self
    where
        P: RuntimePlugin<A::State, A::Event> + 'static,
    {
        self.plugins.push(Box::new(plugin));
        self
    }

    /// # Errors
    ///
    /// Returns an error if module registration is invalid, startup hooks fail,
    /// or the app enters a failed state while starting.
    pub fn start(self) -> AtomResult<RuntimeKernel<A>> {
        let app = self.app;
        let state = app.initial_state();
        let (task_sender, task_receiver) = mpsc::channel();
        let mut kernel = RuntimeKernel {
            app,
            state,
            lifecycle_state: RuntimeState::Created,
            modules: BTreeMap::new(),
            module_order: Vec::new(),
            plugins: self.plugins,
            logs: Vec::new(),
            event_queue: VecDeque::new(),
            task_sender,
            task_receiver,
            next_log_sequence: 1,
            next_task_id: 1,
        };
        kernel.transition_to(
            RuntimeState::Initializing,
            Some(AtomLifecycleEvent::Foreground),
        );

        let mut seen_modules = BTreeSet::new();
        for mut module in self.modules {
            let module_id = module.id().to_owned();
            if !seen_modules.insert(module_id.clone()) {
                return Err(kernel.fail_runtime(AtomError::new(
                    AtomErrorCode::ModuleDuplicateId,
                    format!("duplicate runtime module id: {module_id}"),
                )));
            }
            if let Err(error) = module.init() {
                return Err(kernel.fail_runtime(module_hook_error("init", &module_id, &error)));
            }
            kernel.module_order.push(module_id.clone());
            kernel.modules.insert(module_id.clone(), module);
            kernel.push_log(
                RuntimeLogLevel::Info,
                "runtime.module",
                "module initialized",
                [("module", module_id)],
            );
        }

        kernel.transition_to(RuntimeState::Running, None);

        let mut start_ctx = RuntimeContext::default();
        if let Err(error) = kernel.app.on_start(&mut kernel.state, &mut start_ctx) {
            return Err(kernel.fail_runtime(error));
        }
        if let Err(error) = kernel.apply_commands(start_ctx.into_commands()) {
            return Err(kernel.fail_runtime(error));
        }

        let plugin_count = kernel.plugins.len().to_string();
        kernel.push_log(
            RuntimeLogLevel::Info,
            "runtime",
            "runtime started",
            [
                ("app", kernel.app.name().to_owned()),
                ("modules", kernel.module_order.len().to_string()),
                ("plugins", plugin_count),
            ],
        );

        let mut plugin_commands = Vec::new();
        for index in 0..kernel.plugins.len() {
            let plugin_result = {
                let plugin = &mut kernel.plugins[index];
                let plugin_id = plugin.id().to_owned();
                let mut ctx = RuntimeContext::default();
                match plugin.on_runtime_started(&kernel.state, &mut ctx) {
                    Ok(()) => Ok(ctx.into_commands()),
                    Err(error) => Err(plugin_error(&plugin_id, &error)),
                }
            };
            match plugin_result {
                Ok(commands) => plugin_commands.extend(commands),
                Err(error) => return Err(kernel.fail_runtime(error)),
            }
        }
        if let Err(error) = kernel.apply_commands(plugin_commands) {
            return Err(kernel.fail_runtime(error));
        }

        if let Err(error) = kernel.drain_event_queue() {
            return Err(kernel.fail_runtime(error));
        }

        Ok(kernel)
    }
}

pub struct RuntimeKernel<A>
where
    A: RuntimeApp,
{
    app: A,
    state: A::State,
    lifecycle_state: RuntimeState,
    modules: BTreeMap<String, Box<dyn RuntimeModule>>,
    module_order: Vec<String>,
    plugins: Vec<Box<dyn RuntimePlugin<A::State, A::Event>>>,
    logs: Vec<RuntimeLogEntry>,
    event_queue: VecDeque<A::Event>,
    task_sender: Sender<TaskEnvelope<A::Event>>,
    task_receiver: Receiver<TaskEnvelope<A::Event>>,
    next_log_sequence: u64,
    next_task_id: u64,
}

impl<A> RuntimeKernel<A>
where
    A: RuntimeApp,
{
    #[must_use]
    pub fn lifecycle_state(&self) -> RuntimeState {
        self.lifecycle_state
    }

    #[must_use]
    pub fn state(&self) -> &A::State {
        &self.state
    }

    #[must_use]
    pub fn logs(&self) -> &[RuntimeLogEntry] {
        &self.logs
    }

    #[must_use]
    pub fn module_ids(&self) -> &[String] {
        &self.module_order
    }

    /// # Errors
    ///
    /// Returns an error if the runtime is not dispatchable or the reducer,
    /// plugin hooks, task execution, or module calls fail.
    pub fn dispatch(&mut self, event: A::Event) -> AtomResult<()> {
        self.ensure_dispatchable()?;
        self.event_queue.push_back(event);
        self.drain_event_queue()
            .map_err(|error| self.fail_runtime(error))
    }

    /// # Errors
    ///
    /// Returns an error if the lifecycle transition is invalid or the runtime
    /// enters a failed state while handling transition side effects.
    pub fn handle_lifecycle(&mut self, event: AtomLifecycleEvent) -> AtomResult<()> {
        let transition = self
            .transition_for(event)
            .map_err(|error| self.fail_runtime(error))?;
        self.lifecycle_state = transition.to;
        self.push_log(
            RuntimeLogLevel::Info,
            "runtime.lifecycle",
            "lifecycle transition",
            [
                ("from", format!("{:?}", transition.from)),
                ("event", format!("{:?}", transition.event)),
                ("to", format!("{:?}", transition.to)),
            ],
        );

        let mut ctx = RuntimeContext::default();
        self.app
            .on_lifecycle(&mut self.state, transition, &mut ctx)
            .map_err(|error| self.fail_runtime(error))?;
        let mut commands = ctx.into_commands();

        for index in 0..self.plugins.len() {
            let plugin_result = {
                let plugin = &mut self.plugins[index];
                let plugin_id = plugin.id().to_owned();
                let mut plugin_ctx = RuntimeContext::default();
                match plugin.on_lifecycle(&self.state, transition, &mut plugin_ctx) {
                    Ok(()) => Ok(plugin_ctx.into_commands()),
                    Err(error) => Err(plugin_error(&plugin_id, &error)),
                }
            };
            match plugin_result {
                Ok(plugin_commands) => commands.extend(plugin_commands),
                Err(error) => return Err(self.fail_runtime(error)),
            }
        }

        self.apply_commands(commands)
            .map_err(|error| self.fail_runtime(error))?;
        self.drain_event_queue()
            .map_err(|error| self.fail_runtime(error))?;

        if transition.to == RuntimeState::Terminating {
            self.shutdown_modules()
                .map_err(|error| self.fail_runtime(error))?;
            self.transition_to(RuntimeState::Terminated, None);
        }

        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if a completed async task fails or a follow-up event
    /// enters the runtime failed state.
    pub fn pump(&mut self) -> AtomResult<usize> {
        let mut drained = 0;
        loop {
            match self.task_receiver.try_recv() {
                Ok(task) => {
                    drained += 1;
                    self.handle_task_completion(task)
                        .map_err(|error| self.fail_runtime(error))?;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(self.fail_runtime(AtomError::new(
                        AtomErrorCode::InternalBug,
                        "async task channel disconnected",
                    )));
                }
            }
        }

        if !self.event_queue.is_empty() {
            self.drain_event_queue()
                .map_err(|error| self.fail_runtime(error))?;
        }

        Ok(drained)
    }

    fn handle_event(&mut self, event: &A::Event) -> AtomResult<()> {
        let mut commands = Vec::new();
        for index in 0..self.plugins.len() {
            let plugin_result = {
                let plugin = &mut self.plugins[index];
                let plugin_id = plugin.id().to_owned();
                let mut ctx = RuntimeContext::default();
                match plugin.before_event(&self.state, event, &mut ctx) {
                    Ok(()) => Ok(ctx.into_commands()),
                    Err(error) => Err(plugin_error(&plugin_id, &error)),
                }
            };
            commands.extend(plugin_result?);
        }

        let mut app_ctx = RuntimeContext::default();
        self.app
            .on_event(&mut self.state, event.clone(), &mut app_ctx)?;
        commands.extend(app_ctx.into_commands());

        for index in 0..self.plugins.len() {
            let plugin_result = {
                let plugin = &mut self.plugins[index];
                let plugin_id = plugin.id().to_owned();
                let mut ctx = RuntimeContext::default();
                match plugin.after_event(&self.state, event, &mut ctx) {
                    Ok(()) => Ok(ctx.into_commands()),
                    Err(error) => Err(plugin_error(&plugin_id, &error)),
                }
            };
            commands.extend(plugin_result?);
        }

        self.apply_commands(commands)
    }

    fn apply_commands(&mut self, commands: Vec<RuntimeCommand<A::Event>>) -> AtomResult<()> {
        for command in commands {
            match command {
                RuntimeCommand::Dispatch(event) => self.event_queue.push_back(event),
                RuntimeCommand::Log(entry) => {
                    let sequence = self.take_log_sequence();
                    self.logs.push(RuntimeLogEntry {
                        sequence,
                        level: entry.level,
                        target: entry.target,
                        message: entry.message,
                        fields: entry.fields,
                    });
                }
                RuntimeCommand::CallModule(call) => {
                    let module = self.modules.get_mut(&call.module_id).ok_or_else(|| {
                        AtomError::new(
                            AtomErrorCode::BridgeInvalidArgument,
                            format!("unknown runtime module: {}", call.module_id),
                        )
                    })?;
                    let response = module.call(&call.method, &call.payload)?;
                    self.push_log(
                        RuntimeLogLevel::Info,
                        "runtime.module",
                        "module call completed",
                        [("module", call.module_id), ("method", call.method)],
                    );
                    self.event_queue.push_back((call.on_response)(response));
                }
                RuntimeCommand::SpawnTask(task) => {
                    let task_id = self.next_task_id;
                    self.next_task_id += 1;
                    let sender = self.task_sender.clone();
                    let label = task.label.clone();
                    thread::spawn(move || {
                        let result = (task.work)();
                        let _ = sender.send(TaskEnvelope {
                            id: task_id,
                            label,
                            result,
                        });
                    });
                    self.push_log(
                        RuntimeLogLevel::Info,
                        "runtime.task",
                        "spawned async task",
                        [("task_id", task_id.to_string()), ("label", task.label)],
                    );
                }
            }
        }
        Ok(())
    }

    fn drain_event_queue(&mut self) -> AtomResult<()> {
        while let Some(event) = self.event_queue.pop_front() {
            self.handle_event(&event)?;
        }
        Ok(())
    }

    fn handle_task_completion(&mut self, task: TaskEnvelope<A::Event>) -> AtomResult<()> {
        self.push_log(
            RuntimeLogLevel::Info,
            "runtime.task",
            "async task completed",
            [
                ("task_id", task.id.to_string()),
                ("label", task.label.clone()),
            ],
        );
        let event = task.result?;
        self.event_queue.push_back(event);
        Ok(())
    }

    fn shutdown_modules(&mut self) -> AtomResult<()> {
        for module_id in self.module_order.iter().rev().cloned().collect::<Vec<_>>() {
            let Some(module) = self.modules.get_mut(&module_id) else {
                continue;
            };
            if let Err(error) = module.shutdown() {
                return Err(module_hook_error("shutdown", &module_id, &error));
            }
            self.push_log(
                RuntimeLogLevel::Info,
                "runtime.module",
                "module shutdown completed",
                [("module", module_id)],
            );
        }
        Ok(())
    }

    fn ensure_dispatchable(&self) -> AtomResult<()> {
        match self.lifecycle_state {
            RuntimeState::Running | RuntimeState::Backgrounded | RuntimeState::Suspended => Ok(()),
            RuntimeState::Failed => Err(AtomError::new(
                AtomErrorCode::RuntimeTransitionInvalid,
                "runtime cannot dispatch events after failure",
            )),
            RuntimeState::Terminated | RuntimeState::Terminating => Err(AtomError::new(
                AtomErrorCode::RuntimeTransitionInvalid,
                "runtime cannot dispatch events after termination begins",
            )),
            RuntimeState::Created | RuntimeState::Initializing => Err(AtomError::new(
                AtomErrorCode::RuntimeTransitionInvalid,
                "runtime cannot dispatch events before startup completes",
            )),
        }
    }

    fn transition_for(&self, event: AtomLifecycleEvent) -> AtomResult<LifecycleTransition> {
        let to = match (self.lifecycle_state, event) {
            (RuntimeState::Running, AtomLifecycleEvent::Background) => RuntimeState::Backgrounded,
            (RuntimeState::Backgrounded, AtomLifecycleEvent::Foreground)
            | (RuntimeState::Suspended, AtomLifecycleEvent::Resume) => RuntimeState::Running,
            (RuntimeState::Backgrounded, AtomLifecycleEvent::Suspend) => RuntimeState::Suspended,
            (
                RuntimeState::Running | RuntimeState::Backgrounded | RuntimeState::Suspended,
                AtomLifecycleEvent::Terminate,
            ) => RuntimeState::Terminating,
            (RuntimeState::Terminated | RuntimeState::Failed, _) => {
                return Err(AtomError::new(
                    AtomErrorCode::RuntimeTransitionInvalid,
                    "runtime cannot transition from a terminal state",
                ));
            }
            _ => {
                return Err(AtomError::new(
                    AtomErrorCode::RuntimeTransitionInvalid,
                    format!(
                        "invalid transition from {:?} with {:?}",
                        self.lifecycle_state, event
                    ),
                ));
            }
        };

        Ok(LifecycleTransition {
            from: self.lifecycle_state,
            event,
            to,
        })
    }

    fn transition_to(&mut self, state: RuntimeState, event: Option<AtomLifecycleEvent>) {
        let from = self.lifecycle_state;
        self.lifecycle_state = state;
        self.push_log(
            RuntimeLogLevel::Info,
            "runtime.lifecycle",
            "state changed",
            [
                ("from", format!("{from:?}")),
                ("to", format!("{state:?}")),
                (
                    "event",
                    event.map_or_else(|| "startup".to_owned(), |value| format!("{value:?}")),
                ),
            ],
        );
    }

    fn push_log<I, K, V>(
        &mut self,
        level: RuntimeLogLevel,
        target: impl Into<String>,
        message: impl Into<String>,
        fields: I,
    ) where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let sequence = self.take_log_sequence();
        self.logs.push(RuntimeLogEntry {
            sequence,
            level,
            target: target.into(),
            message: message.into(),
            fields: fields
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        });
    }

    fn take_log_sequence(&mut self) -> u64 {
        let sequence = self.next_log_sequence;
        self.next_log_sequence += 1;
        sequence
    }

    fn fail_runtime(&mut self, error: AtomError) -> AtomError {
        if !matches!(
            self.lifecycle_state,
            RuntimeState::Failed | RuntimeState::Terminated
        ) {
            self.lifecycle_state = RuntimeState::Failed;
            self.push_log(
                RuntimeLogLevel::Error,
                "runtime",
                "runtime entered failed state",
                [
                    ("code", error.code.as_str().to_owned()),
                    ("message", error.message.clone()),
                ],
            );
        }
        error
    }
}

fn plugin_error(plugin_id: &str, error: &AtomError) -> AtomError {
    let message = &error.message;
    AtomError::new(error.code, format!("plugin {plugin_id} failed: {message}"))
}

fn module_hook_error(stage: &str, module_id: &str, error: &AtomError) -> AtomError {
    let message = &error.message;
    AtomError::new(
        AtomErrorCode::ModuleInitFailed,
        format!("module {stage} failed for {module_id}: {message}"),
    )
}
