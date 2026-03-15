use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use atom_ffi::{AtomError, AtomErrorCode, AtomLifecycleEvent, AtomResult};

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
pub struct RuntimeSnapshot {
    pub lifecycle: RuntimeState,
    pub values: BTreeMap<String, String>,
    pub events: Vec<RuntimeEventRecord>,
    pub effects: Vec<RuntimeEffectRecord>,
}

impl RuntimeSnapshot {
    pub(crate) fn new(lifecycle: RuntimeState) -> Self {
        Self {
            lifecycle,
            values: BTreeMap::new(),
            events: Vec::new(),
            effects: Vec::new(),
        }
    }
}

pub(crate) struct RuntimeHost {
    snapshot: Mutex<RuntimeSnapshot>,
    next_sequence: AtomicU64,
}

impl RuntimeHost {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            snapshot: Mutex::new(RuntimeSnapshot::new(RuntimeState::Created)),
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
