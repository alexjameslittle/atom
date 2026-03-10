use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use atom_ffi::{AtomError, AtomErrorCode, AtomLifecycleEvent, AtomResult};

use crate::config::ModuleMethodHandler;
use crate::state::RuntimeState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    Lifecycle {
        event: AtomLifecycleEvent,
        state: RuntimeState,
    },
    Plugin {
        plugin_id: String,
        name: String,
        detail: Option<String>,
    },
    TaskCompleted {
        plugin_id: String,
        task_name: String,
        success: bool,
    },
    ModuleCallCompleted {
        module_id: String,
        method: String,
        response_len: usize,
    },
}

impl RuntimeEvent {
    #[must_use]
    pub fn plugin(
        plugin_id: impl Into<String>,
        name: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self::Plugin {
            plugin_id: plugin_id.into(),
            name: name.into(),
            detail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEffect {
    StateChanged {
        key: String,
        value: String,
    },
    TaskStarted {
        plugin_id: String,
        task_name: String,
    },
    ModuleCall {
        module_id: String,
        method: String,
        request_len: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEventRecord {
    pub sequence: u64,
    pub event: RuntimeEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEffectRecord {
    pub sequence: u64,
    pub effect: RuntimeEffect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleCallRecord {
    pub sequence: u64,
    pub module_id: String,
    pub method: String,
    pub request_len: usize,
    pub response_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub lifecycle: RuntimeState,
    pub values: BTreeMap<String, String>,
    pub events: Vec<RuntimeEventRecord>,
    pub effects: Vec<RuntimeEffectRecord>,
    pub module_calls: Vec<ModuleCallRecord>,
}

impl RuntimeSnapshot {
    pub(crate) fn new(lifecycle: RuntimeState) -> Self {
        Self {
            lifecycle,
            values: BTreeMap::new(),
            events: Vec::new(),
            effects: Vec::new(),
            module_calls: Vec::new(),
        }
    }
}

pub(crate) struct RuntimeHost {
    snapshot: Mutex<RuntimeSnapshot>,
    module_methods: Mutex<HashMap<(String, String), ModuleMethodHandler>>,
    next_sequence: AtomicU64,
}

impl RuntimeHost {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            snapshot: Mutex::new(RuntimeSnapshot::new(RuntimeState::Created)),
            module_methods: Mutex::new(HashMap::new()),
            next_sequence: AtomicU64::new(1),
        })
    }

    pub(crate) fn set_lifecycle(&self, lifecycle: RuntimeState) {
        lock_snapshot(&self.snapshot).lifecycle = lifecycle;
    }

    pub(crate) fn snapshot(&self) -> RuntimeSnapshot {
        lock_snapshot(&self.snapshot).clone()
    }

    pub(crate) fn set_value(&self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        lock_snapshot(&self.snapshot)
            .values
            .insert(key.clone(), value.clone());
        self.emit_effect(RuntimeEffect::StateChanged { key, value });
    }

    #[must_use]
    pub(crate) fn value(&self, key: &str) -> Option<String> {
        lock_snapshot(&self.snapshot).values.get(key).cloned()
    }

    pub(crate) fn dispatch_event(&self, event: RuntimeEvent) {
        let sequence = self.next_sequence();
        lock_snapshot(&self.snapshot)
            .events
            .push(RuntimeEventRecord { sequence, event });
    }

    pub(crate) fn emit_effect(&self, effect: RuntimeEffect) {
        let sequence = self.next_sequence();
        lock_snapshot(&self.snapshot)
            .effects
            .push(RuntimeEffectRecord { sequence, effect });
    }

    pub(crate) fn register_module_method(
        &self,
        module_id: &str,
        method_name: &str,
        handler: ModuleMethodHandler,
    ) -> AtomResult<()> {
        let key = (module_id.to_owned(), method_name.to_owned());
        let previous = lock_module_methods(&self.module_methods).insert(key.clone(), handler);
        if previous.is_some() {
            return Err(AtomError::new(
                AtomErrorCode::ModuleManifestInvalid,
                format!(
                    "duplicate runtime module method registration: {}.{}",
                    key.0, key.1
                ),
            ));
        }
        Ok(())
    }

    pub(crate) fn call_module(
        &self,
        ctx: &crate::plugin::PluginContext,
        module_id: &str,
        method: &str,
        request: &[u8],
    ) -> AtomResult<Vec<u8>> {
        if self.snapshot().lifecycle != RuntimeState::Running {
            return Err(AtomError::new(
                AtomErrorCode::RuntimeTransitionInvalid,
                "runtime module calls require Running state",
            ));
        }

        self.emit_effect(RuntimeEffect::ModuleCall {
            module_id: module_id.to_owned(),
            method: method.to_owned(),
            request_len: request.len(),
        });

        let key = (module_id.to_owned(), method.to_owned());
        let handler = lock_module_methods(&self.module_methods)
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::ModuleNotFound,
                    format!("runtime module method not found: {}.{}", key.0, key.1),
                )
            })?;

        let response = handler(ctx, request)?;
        let sequence = self.next_sequence();
        let mut snapshot = lock_snapshot(&self.snapshot);
        snapshot.events.push(RuntimeEventRecord {
            sequence,
            event: RuntimeEvent::ModuleCallCompleted {
                module_id: module_id.to_owned(),
                method: method.to_owned(),
                response_len: response.len(),
            },
        });
        snapshot.module_calls.push(ModuleCallRecord {
            sequence,
            module_id: module_id.to_owned(),
            method: method.to_owned(),
            request_len: request.len(),
            response_len: response.len(),
        });
        drop(snapshot);

        Ok(response)
    }

    pub(crate) fn run_task<F, T>(
        &self,
        ctx: &crate::plugin::PluginContext,
        plugin_id: &str,
        task_name: &str,
        future: F,
    ) -> AtomResult<T>
    where
        F: Future<Output = AtomResult<T>>,
    {
        if self.snapshot().lifecycle != RuntimeState::Running {
            return Err(AtomError::new(
                AtomErrorCode::RuntimeTransitionInvalid,
                "runtime async tasks require Running state",
            ));
        }

        self.emit_effect(RuntimeEffect::TaskStarted {
            plugin_id: plugin_id.to_owned(),
            task_name: task_name.to_owned(),
        });

        match ctx.tokio_handle.block_on(future) {
            Ok(value) => {
                self.dispatch_event(RuntimeEvent::TaskCompleted {
                    plugin_id: plugin_id.to_owned(),
                    task_name: task_name.to_owned(),
                    success: true,
                });
                Ok(value)
            }
            Err(error) => {
                self.dispatch_event(RuntimeEvent::TaskCompleted {
                    plugin_id: plugin_id.to_owned(),
                    task_name: task_name.to_owned(),
                    success: false,
                });
                Err(error)
            }
        }
    }

    fn next_sequence(&self) -> u64 {
        self.next_sequence.fetch_add(1, Ordering::Relaxed)
    }
}

fn lock_snapshot(snapshot: &Mutex<RuntimeSnapshot>) -> MutexGuard<'_, RuntimeSnapshot> {
    match snapshot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn lock_module_methods(
    methods: &Mutex<HashMap<(String, String), ModuleMethodHandler>>,
) -> MutexGuard<'_, HashMap<(String, String), ModuleMethodHandler>> {
    match methods.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
