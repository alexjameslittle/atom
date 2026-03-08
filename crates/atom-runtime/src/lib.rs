use std::collections::HashMap;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

use atom_ffi::{
    AtomError, AtomErrorCode, AtomLifecycleEvent, AtomOwnedBuffer, AtomResult, AtomRuntimeHandle,
    AtomSlice, write_error_buffer,
};

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

#[derive(Debug, Clone, Copy)]
struct Runtime {
    state: RuntimeState,
}

impl Runtime {
    fn start() -> Self {
        let mut runtime = Self {
            state: RuntimeState::Created,
        };
        runtime.state = RuntimeState::Initializing;
        runtime.state = RuntimeState::Running;
        runtime
    }

    fn handle_event(&mut self, event: AtomLifecycleEvent) -> AtomResult<()> {
        match (self.state, event) {
            (RuntimeState::Running, AtomLifecycleEvent::Background) => {
                self.state = RuntimeState::Backgrounded;
                Ok(())
            }
            (RuntimeState::Backgrounded, AtomLifecycleEvent::Foreground) => {
                self.state = RuntimeState::Running;
                Ok(())
            }
            (RuntimeState::Backgrounded, AtomLifecycleEvent::Suspend) => {
                self.state = RuntimeState::Suspended;
                Ok(())
            }
            (RuntimeState::Suspended, AtomLifecycleEvent::Resume) => {
                self.state = RuntimeState::Running;
                Ok(())
            }
            (
                RuntimeState::Running | RuntimeState::Backgrounded | RuntimeState::Suspended,
                AtomLifecycleEvent::Terminate,
            ) => {
                self.state = RuntimeState::Terminating;
                self.state = RuntimeState::Terminated;
                Ok(())
            }
            (RuntimeState::Terminated, _) | (RuntimeState::Failed, _) => Err(AtomError::new(
                AtomErrorCode::RuntimeTransitionInvalid,
                "runtime cannot transition from a terminal state",
            )),
            _ => Err(AtomError::new(
                AtomErrorCode::RuntimeTransitionInvalid,
                format!("invalid transition from {:?} with {:?}", self.state, event),
            )),
        }
    }

    fn shutdown(&mut self) {
        if self.state == RuntimeState::Terminated {
            return;
        }

        self.state = RuntimeState::Terminating;
        self.state = RuntimeState::Terminated;
    }
}

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static RUNTIMES: LazyLock<Mutex<HashMap<AtomRuntimeHandle, Runtime>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn init_runtime() -> AtomResult<AtomRuntimeHandle> {
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    let runtime = Runtime::start();
    let mut runtimes = RUNTIMES
        .lock()
        .map_err(|_| AtomError::new(AtomErrorCode::InternalBug, "runtime registry poisoned"))?;
    runtimes.insert(handle, runtime);
    Ok(handle)
}

pub fn handle_lifecycle(handle: AtomRuntimeHandle, event: AtomLifecycleEvent) -> AtomResult<()> {
    let mut runtimes = RUNTIMES
        .lock()
        .map_err(|_| AtomError::new(AtomErrorCode::InternalBug, "runtime registry poisoned"))?;
    let runtime = runtimes.get_mut(&handle).ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::BridgeInvalidArgument,
            format!("unknown runtime handle: {handle}"),
        )
    })?;
    runtime.handle_event(event)
}

pub fn shutdown_runtime(handle: AtomRuntimeHandle) {
    if let Ok(mut runtimes) = RUNTIMES.lock() {
        if let Some(runtime) = runtimes.get_mut(&handle) {
            runtime.shutdown();
        }
        runtimes.remove(&handle);
    }
}

pub fn current_state(handle: AtomRuntimeHandle) -> Option<RuntimeState> {
    RUNTIMES
        .lock()
        .ok()
        .and_then(|runtimes| runtimes.get(&handle).map(|runtime| runtime.state))
}

/// # Safety
///
/// `out_handle` must be a valid writable pointer and `out_error_flatbuffer` must be null or a
/// valid writable pointer to `AtomOwnedBuffer`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_app_init(
    _config_flatbuffer: AtomSlice,
    out_handle: *mut AtomRuntimeHandle,
    out_error_flatbuffer: *mut AtomOwnedBuffer,
) -> i32 {
    if out_handle.is_null() {
        let error = AtomError::new(
            AtomErrorCode::BridgeInvalidArgument,
            "atom_app_init requires a non-null out_handle",
        );
        // SAFETY: the FFI caller owns `out_error_flatbuffer`.
        unsafe { write_error_buffer(out_error_flatbuffer, &error) };
        return error.exit_code();
    }

    match init_runtime() {
        Ok(handle) => {
            // SAFETY: `out_handle` is non-null and points to caller-provided writable memory.
            unsafe {
                ptr::write(out_handle, handle);
            }
            if !out_error_flatbuffer.is_null() {
                // SAFETY: `out_error_flatbuffer` points to writable caller memory.
                unsafe {
                    ptr::write(out_error_flatbuffer, AtomOwnedBuffer::empty());
                }
            }
            0
        }
        Err(error) => {
            // SAFETY: the FFI caller owns `out_error_flatbuffer`.
            unsafe { write_error_buffer(out_error_flatbuffer, &error) };
            error.exit_code()
        }
    }
}

/// # Safety
///
/// `out_error_flatbuffer` must be null or a valid writable pointer to `AtomOwnedBuffer`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_app_handle_lifecycle(
    handle: AtomRuntimeHandle,
    event: u32,
    out_error_flatbuffer: *mut AtomOwnedBuffer,
) -> i32 {
    let event = match AtomLifecycleEvent::try_from(event) {
        Ok(event) => event,
        Err(error) => {
            // SAFETY: the FFI caller owns `out_error_flatbuffer`.
            unsafe { write_error_buffer(out_error_flatbuffer, &error) };
            return error.exit_code();
        }
    };

    match handle_lifecycle(handle, event) {
        Ok(()) => {
            if !out_error_flatbuffer.is_null() {
                // SAFETY: `out_error_flatbuffer` points to writable caller memory.
                unsafe {
                    ptr::write(out_error_flatbuffer, AtomOwnedBuffer::empty());
                }
            }
            0
        }
        Err(error) => {
            // SAFETY: the FFI caller owns `out_error_flatbuffer`.
            unsafe { write_error_buffer(out_error_flatbuffer, &error) };
            error.exit_code()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn atom_app_shutdown(handle: AtomRuntimeHandle) {
    shutdown_runtime(handle);
}

/// # Safety
///
/// `buffer` must have been allocated by `AtomOwnedBuffer::from_vec`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_buffer_free(buffer: AtomOwnedBuffer) {
    // SAFETY: buffers returned through the FFI are created by `AtomOwnedBuffer::from_vec`.
    let _ = unsafe { buffer.into_vec() };
}

#[cfg(test)]
mod tests {
    use super::{RuntimeState, current_state, handle_lifecycle, init_runtime, shutdown_runtime};
    use atom_ffi::AtomLifecycleEvent;

    #[test]
    fn runtime_follows_valid_lifecycle() {
        let handle = init_runtime().expect("runtime should initialize");
        assert_eq!(current_state(handle), Some(RuntimeState::Running));

        handle_lifecycle(handle, AtomLifecycleEvent::Background).expect("background should work");
        assert_eq!(current_state(handle), Some(RuntimeState::Backgrounded));

        handle_lifecycle(handle, AtomLifecycleEvent::Foreground).expect("foreground should work");
        assert_eq!(current_state(handle), Some(RuntimeState::Running));

        handle_lifecycle(handle, AtomLifecycleEvent::Background).expect("background should work");
        handle_lifecycle(handle, AtomLifecycleEvent::Suspend).expect("suspend should work");
        assert_eq!(current_state(handle), Some(RuntimeState::Suspended));

        handle_lifecycle(handle, AtomLifecycleEvent::Resume).expect("resume should work");
        handle_lifecycle(handle, AtomLifecycleEvent::Terminate).expect("terminate should work");
        assert_eq!(current_state(handle), Some(RuntimeState::Terminated));
        shutdown_runtime(handle);
    }

    #[test]
    fn runtime_rejects_invalid_transition() {
        let handle = init_runtime().expect("runtime should initialize");
        let error = handle_lifecycle(handle, AtomLifecycleEvent::Suspend)
            .expect_err("running -> suspend should fail");
        assert_eq!(
            error.code,
            atom_ffi::AtomErrorCode::RuntimeTransitionInvalid
        );
        shutdown_runtime(handle);
    }
}
