use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use atom_ffi::{AtomError, AtomErrorCode, AtomLifecycleEvent, AtomResult, AtomRuntimeHandle};

use crate::config::RuntimeConfig;
use crate::kernel::Runtime;
use crate::logging;
use crate::plugin::PluginContext;
use crate::state::RuntimeState;
use crate::store::RuntimeSnapshot;

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static RUNTIMES: LazyLock<Mutex<HashMap<AtomRuntimeHandle, Runtime>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn lock_runtimes() -> AtomResult<std::sync::MutexGuard<'static, HashMap<AtomRuntimeHandle, Runtime>>>
{
    RUNTIMES
        .lock()
        .map_err(|_| AtomError::new(AtomErrorCode::InternalBug, "runtime registry poisoned"))
}

fn get_runtime_mut(
    runtimes: &mut HashMap<AtomRuntimeHandle, Runtime>,
    handle: AtomRuntimeHandle,
) -> AtomResult<&mut Runtime> {
    runtimes.get_mut(&handle).ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::BridgeInvalidArgument,
            format!("unknown runtime handle: {handle}"),
        )
    })
}

/// Initialize a new runtime with the given config.
///
/// # Errors
///
/// Returns an error if the runtime registry mutex is poisoned, tokio fails to
/// initialize, or any plugin startup hook fails.
pub fn init_runtime(config: RuntimeConfig) -> AtomResult<AtomRuntimeHandle> {
    logging::init_logging();

    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    let runtime = Runtime::start(handle, config)?;

    let mut runtimes = lock_runtimes()?;
    runtimes.insert(handle, runtime);
    Ok(handle)
}

/// Dispatch a lifecycle event to the runtime identified by `handle`.
///
/// # Errors
///
/// Returns an error if the handle is unknown, the registry is poisoned, or the
/// state transition is invalid.
pub fn handle_lifecycle(handle: AtomRuntimeHandle, event: AtomLifecycleEvent) -> AtomResult<()> {
    let mut runtimes = lock_runtimes()?;
    let runtime = get_runtime_mut(&mut runtimes, handle)?;
    runtime.handle_event(event)
}

/// Shut down and remove the runtime identified by `handle`.
pub fn shutdown_runtime(handle: AtomRuntimeHandle) {
    if let Ok(mut runtimes) = RUNTIMES.lock() {
        if let Some(runtime) = runtimes.get_mut(&handle) {
            runtime.shutdown();
        }
        runtimes.remove(&handle);
    }
}

/// Query the current state of the runtime identified by `handle`.
pub fn current_state(handle: AtomRuntimeHandle) -> Option<RuntimeState> {
    RUNTIMES
        .lock()
        .ok()
        .and_then(|runtimes| runtimes.get(&handle).map(Runtime::state))
}

/// Query the current snapshot of the runtime identified by `handle`.
pub fn current_snapshot(handle: AtomRuntimeHandle) -> Option<RuntimeSnapshot> {
    RUNTIMES
        .lock()
        .ok()
        .and_then(|runtimes| runtimes.get(&handle).map(Runtime::snapshot))
}

/// Clone the live plugin context for a running runtime so generated/native glue
/// can invoke module crate APIs directly.
///
/// # Errors
///
/// Returns an error if the handle is unknown, the registry is poisoned, or the
/// runtime is not in the `Running` state.
pub fn running_plugin_context(handle: AtomRuntimeHandle) -> AtomResult<PluginContext<'static>> {
    let runtimes = lock_runtimes()?;
    let runtime = runtimes.get(&handle).ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::BridgeInvalidArgument,
            format!("unknown runtime handle: {handle}"),
        )
    })?;
    if runtime.state() != RuntimeState::Running {
        return Err(AtomError::new(
            AtomErrorCode::RuntimeTransitionInvalid,
            format!(
                "runtime is {:?}, public module APIs require Running state",
                runtime.state()
            ),
        ));
    }
    Ok(runtime.context())
}

/// Gate function for CNG-generated per-method exports. Returns `Ok(())` if the
/// runtime is in the `Running` state, or an error otherwise.
///
/// # Errors
///
/// Returns an error if the handle is unknown, the registry is poisoned, or the
/// runtime is not in the `Running` state.
pub fn ensure_running(handle: AtomRuntimeHandle) -> AtomResult<()> {
    let runtimes = lock_runtimes()?;
    let runtime = runtimes.get(&handle).ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::BridgeInvalidArgument,
            format!("unknown runtime handle: {handle}"),
        )
    })?;
    if runtime.state() == RuntimeState::Running {
        Ok(())
    } else {
        Err(AtomError::new(
            AtomErrorCode::RuntimeTransitionInvalid,
            format!(
                "runtime is {:?}, public module APIs require Running state",
                runtime.state()
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use atom_ffi::AtomLifecycleEvent;

    use crate::config::RuntimeConfig;
    use crate::state::RuntimeState;

    use super::{
        current_snapshot, current_state, ensure_running, handle_lifecycle, init_runtime,
        running_plugin_context, shutdown_runtime,
    };

    #[test]
    fn ensure_running_ok_when_running() {
        let handle = init_runtime(RuntimeConfig::default()).expect("init");
        ensure_running(handle).expect("should be running");
        shutdown_runtime(handle);
    }

    #[test]
    fn ensure_running_err_when_backgrounded() {
        let handle = init_runtime(RuntimeConfig::default()).expect("init");
        handle_lifecycle(handle, AtomLifecycleEvent::Background).unwrap();
        ensure_running(handle).expect_err("should not be running");
        shutdown_runtime(handle);
    }

    #[test]
    fn full_conformance_lifecycle() {
        let handle = init_runtime(RuntimeConfig::default()).expect("init");
        assert_eq!(current_state(handle), Some(RuntimeState::Running));

        handle_lifecycle(handle, AtomLifecycleEvent::Background).unwrap();
        assert_eq!(current_state(handle), Some(RuntimeState::Backgrounded));

        handle_lifecycle(handle, AtomLifecycleEvent::Foreground).unwrap();
        assert_eq!(current_state(handle), Some(RuntimeState::Running));

        handle_lifecycle(handle, AtomLifecycleEvent::Terminate).unwrap();
        assert_eq!(current_state(handle), Some(RuntimeState::Terminated));

        shutdown_runtime(handle);
    }

    #[test]
    fn snapshot_tracks_lifecycle() {
        let handle = init_runtime(RuntimeConfig::default()).expect("init");
        assert_eq!(
            current_snapshot(handle).map(|snapshot| snapshot.lifecycle),
            Some(RuntimeState::Running),
        );

        handle_lifecycle(handle, AtomLifecycleEvent::Background).unwrap();
        assert_eq!(
            current_snapshot(handle).map(|snapshot| snapshot.lifecycle),
            Some(RuntimeState::Backgrounded),
        );

        shutdown_runtime(handle);
    }

    #[test]
    fn running_plugin_context_clones_live_context() {
        let handle = init_runtime(RuntimeConfig::default()).expect("init");
        let ctx = running_plugin_context(handle).expect("context should be available");
        assert_eq!(ctx.handle, handle);
        shutdown_runtime(handle);
    }
}
