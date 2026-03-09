use std::ptr;

use atom_ffi::{
    AtomError, AtomErrorCode, AtomLifecycleEvent, AtomOwnedBuffer, AtomRuntimeHandle, AtomSlice,
    write_error_buffer,
};

use crate::config::RuntimeConfig;
use crate::registry;

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

    // TODO: Parse config_flatbuffer into RuntimeConfig when app crates provide
    // module and plugin registrations via FlatBuffer or a Rust-side registration
    // mechanism. For now, construct a default config.
    let config = RuntimeConfig::default();

    match registry::init_runtime(config) {
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

    match registry::handle_lifecycle(handle, event) {
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
    registry::shutdown_runtime(handle);
}

/// # Safety
///
/// `buffer` must have been allocated by `AtomOwnedBuffer::from_vec`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atom_buffer_free(buffer: AtomOwnedBuffer) {
    // SAFETY: buffers returned through the FFI are created by `AtomOwnedBuffer::from_vec`.
    let _ = unsafe { buffer.into_vec() };
}
