#[cfg(not(test))]
use std::sync::OnceLock;
#[cfg(test)]
use std::sync::{LazyLock, Mutex, MutexGuard};

use atom_ffi::{AtomError, AtomErrorCode, AtomLifecycleEvent, AtomResult};

use crate::config::RuntimeConfig;
use crate::kernel::Runtime;
use crate::logging;
use crate::state::RuntimeState;
use crate::store::{RuntimeEvent, RuntimeSnapshot};

#[cfg(not(test))]
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
#[cfg(test)]
static RUNTIME: LazyLock<Mutex<Option<Runtime>>> = LazyLock::new(|| Mutex::new(None));
#[cfg(test)]
static TEST_GUARD: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn runtime_not_initialized() -> AtomError {
    AtomError::new(
        AtomErrorCode::BridgeInitFailed,
        "runtime has not been initialized",
    )
}

fn runtime_already_initialized() -> AtomError {
    AtomError::new(
        AtomErrorCode::BridgeInitFailed,
        "runtime has already been initialized",
    )
}

#[cfg(test)]
fn lock_runtime_slot() -> MutexGuard<'static, Option<Runtime>> {
    match RUNTIME.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
fn lock_test_guard() -> MutexGuard<'static, ()> {
    match TEST_GUARD.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn with_runtime<T>(f: impl FnOnce(&Runtime) -> T) -> Option<T> {
    #[cfg(not(test))]
    {
        RUNTIME.get().map(f)
    }

    #[cfg(test)]
    {
        let slot = lock_runtime_slot();
        slot.as_ref().map(f)
    }
}

fn require_runtime<T>(f: impl FnOnce(&Runtime) -> AtomResult<T>) -> AtomResult<T> {
    with_runtime(f).unwrap_or_else(|| Err(runtime_not_initialized()))
}

fn install_runtime(runtime: Runtime) -> AtomResult<()> {
    #[cfg(not(test))]
    {
        RUNTIME
            .set(runtime)
            .map_err(|_| runtime_already_initialized())
    }

    #[cfg(test)]
    {
        let mut slot = lock_runtime_slot();
        if slot.is_some() {
            return Err(runtime_already_initialized());
        }
        *slot = Some(runtime);
        Ok(())
    }
}

#[doc(hidden)]
pub fn __init(config: RuntimeConfig) -> AtomResult<()> {
    logging::init_logging();
    install_runtime(Runtime::start(config)?)
}

#[doc(hidden)]
pub fn __handle_lifecycle(event: AtomLifecycleEvent) -> AtomResult<()> {
    require_runtime(|runtime| runtime.handle_event(event))
}

#[doc(hidden)]
pub fn __shutdown() {
    let _ = with_runtime(Runtime::shutdown);
}

pub fn set_state(key: &str, value: &str) {
    let _ = with_runtime(|runtime| runtime.set_state(key, value));
}

#[must_use]
pub fn state_value(key: &str) -> Option<String> {
    with_runtime(|runtime| runtime.state_value(key)).flatten()
}

pub fn dispatch_event(event: RuntimeEvent) {
    let _ = with_runtime(|runtime| runtime.dispatch_event(event));
}

/// Get the tokio runtime handle for async work.
///
/// # Panics
///
/// Panics if the runtime singleton has not been initialized. Unlike the other
/// free functions, there is no sensible fallback handle to return.
#[must_use]
pub fn tokio_handle() -> tokio::runtime::Handle {
    with_runtime(Runtime::tokio_handle)
        .expect("atom_runtime::tokio_handle() requires an initialized runtime")
}

#[must_use]
pub fn current_state() -> Option<RuntimeState> {
    with_runtime(Runtime::state)
}

#[must_use]
pub fn current_snapshot() -> Option<RuntimeSnapshot> {
    with_runtime(Runtime::snapshot)
}

/// Check that the runtime singleton is initialized and currently `Running`.
///
/// # Errors
///
/// Returns `BRIDGE_INIT_FAILED` when the runtime has not been initialized, or
/// `RUNTIME_TRANSITION_INVALID` when it is initialized but not `Running`.
pub fn ensure_running() -> AtomResult<()> {
    let state = with_runtime(Runtime::state).ok_or_else(runtime_not_initialized)?;
    if state == RuntimeState::Running {
        Ok(())
    } else {
        Err(AtomError::new(
            AtomErrorCode::RuntimeTransitionInvalid,
            format!("runtime is {state:?}, module calls require Running state"),
        ))
    }
}

#[cfg(test)]
pub(crate) fn __reset() {
    let mut slot = lock_runtime_slot();
    if let Some(runtime) = slot.take() {
        runtime.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use atom_ffi::{AtomErrorCode, AtomLifecycleEvent};

    use crate::config::RuntimeConfig;
    use crate::state::RuntimeState;
    use crate::store::RuntimeEvent;

    use super::{
        __handle_lifecycle, __init, __reset, __shutdown, current_snapshot, current_state,
        dispatch_event, ensure_running, lock_test_guard, set_state, state_value, tokio_handle,
    };

    fn run_isolated(test: impl FnOnce()) {
        let _guard = lock_test_guard();
        __reset();
        test();
        __shutdown();
        __reset();
    }

    #[test]
    fn ensure_running_ok_when_running() {
        run_isolated(|| {
            __init(RuntimeConfig).expect("init");
            ensure_running().expect("should be running");
        });
    }

    #[test]
    fn ensure_running_err_when_backgrounded() {
        run_isolated(|| {
            __init(RuntimeConfig).expect("init");
            __handle_lifecycle(AtomLifecycleEvent::Background).unwrap();
            ensure_running().expect_err("should not be running");
        });
    }

    #[test]
    fn ensure_running_err_when_no_runtime_exists() {
        run_isolated(|| {
            let error = ensure_running().expect_err("no runtime should fail");
            assert_eq!(error.code, AtomErrorCode::BridgeInitFailed);
        });
    }

    #[test]
    fn full_conformance_lifecycle() {
        run_isolated(|| {
            __init(RuntimeConfig).expect("init");
            assert_eq!(current_state(), Some(RuntimeState::Running));

            __handle_lifecycle(AtomLifecycleEvent::Background).unwrap();
            assert_eq!(current_state(), Some(RuntimeState::Backgrounded));

            __handle_lifecycle(AtomLifecycleEvent::Foreground).unwrap();
            assert_eq!(current_state(), Some(RuntimeState::Running));

            __handle_lifecycle(AtomLifecycleEvent::Terminate).unwrap();
            assert_eq!(current_state(), Some(RuntimeState::Terminated));
        });
    }

    #[test]
    fn snapshot_tracks_lifecycle() {
        run_isolated(|| {
            __init(RuntimeConfig).expect("init");
            assert_eq!(
                current_snapshot().map(|snapshot| snapshot.lifecycle),
                Some(RuntimeState::Running),
            );

            __handle_lifecycle(AtomLifecycleEvent::Background).unwrap();
            assert_eq!(
                current_snapshot().map(|snapshot| snapshot.lifecycle),
                Some(RuntimeState::Backgrounded),
            );
        });
    }

    #[test]
    fn reset_allows_reinitialization() {
        run_isolated(|| {
            __init(RuntimeConfig).expect("init");
            __shutdown();
            __reset();
            __init(RuntimeConfig).expect("reinit after reset");
        });
    }

    #[test]
    fn state_helpers_update_snapshot() {
        run_isolated(|| {
            __init(RuntimeConfig).expect("init");
            set_state("runtime.phase", "running");
            dispatch_event(RuntimeEvent::plugin("probe", "started", None));

            assert_eq!(state_value("runtime.phase").as_deref(), Some("running"));
            let snapshot = current_snapshot().expect("snapshot");
            assert_eq!(snapshot.events.len(), 1);
            assert_eq!(snapshot.effects.len(), 1);
        });
    }

    #[test]
    #[should_panic(expected = "atom_runtime::tokio_handle() requires an initialized runtime")]
    fn tokio_handle_requires_initialized_runtime() {
        let _guard = lock_test_guard();
        __reset();
        let _ = tokio_handle();
    }
}
