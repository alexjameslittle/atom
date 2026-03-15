use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use atom_ffi::{AtomLifecycleEvent, AtomResult};

use crate::state::{RuntimeState, validate_transition};

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
    StateChanged { key: String, value: String },
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

    pub(crate) fn handle_lifecycle_event(
        &self,
        event: AtomLifecycleEvent,
    ) -> AtomResult<RuntimeState> {
        let sequence = self.next_sequence();
        let mut snapshot = lock_snapshot(&self.snapshot);
        let state = validate_transition(snapshot.lifecycle, event)?;
        snapshot.lifecycle = state;
        snapshot.events.push(RuntimeEventRecord {
            sequence,
            event: RuntimeEvent::Lifecycle { event, state },
        });
        Ok(state)
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use atom_ffi::AtomLifecycleEvent;

    use super::RuntimeHost;
    use crate::state::RuntimeState;

    #[test]
    fn concurrent_lifecycle_updates_validate_against_latest_state() {
        let host = RuntimeHost::new();
        host.set_lifecycle(RuntimeState::Backgrounded);

        let barrier = Arc::new(Barrier::new(3));
        let foreground = {
            let barrier = Arc::clone(&barrier);
            let host = Arc::clone(&host);
            thread::spawn(move || {
                barrier.wait();
                host.handle_lifecycle_event(AtomLifecycleEvent::Foreground)
            })
        };
        let suspend = {
            let barrier = Arc::clone(&barrier);
            let host = Arc::clone(&host);
            thread::spawn(move || {
                barrier.wait();
                host.handle_lifecycle_event(AtomLifecycleEvent::Suspend)
            })
        };

        barrier.wait();

        let foreground = foreground.join().expect("foreground thread");
        let suspend = suspend.join().expect("suspend thread");
        assert_eq!(
            [foreground.is_ok(), suspend.is_ok()]
                .into_iter()
                .filter(|ok| *ok)
                .count(),
            1,
        );

        let snapshot = host.snapshot();
        assert_eq!(snapshot.events.len(), 1);
        assert!(matches!(
            snapshot.lifecycle,
            RuntimeState::Running | RuntimeState::Suspended
        ));
    }
}
