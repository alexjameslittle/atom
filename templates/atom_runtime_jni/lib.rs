use std::ffi::c_void;

use atom_ffi::AtomLifecycleEvent;

type JniEnv = *mut c_void;
type JniClass = *mut c_void;
type JLong = i64;
type JInt = i32;

#[unsafe(export_name = env!("ATOM_JNI_INIT_EXPORT"))]
pub extern "system" fn atom_runtime_jni_init(_env: JniEnv, _class: JniClass) -> JLong {
    atom_runtime::init_runtime(atom_runtime::RuntimeConfig::default())
        .ok()
        .and_then(|handle| i64::try_from(handle).ok())
        .unwrap_or(0)
}

#[unsafe(export_name = env!("ATOM_JNI_LIFECYCLE_EXPORT"))]
pub extern "system" fn atom_runtime_jni_handle_lifecycle(
    _env: JniEnv,
    _class: JniClass,
    handle: JLong,
    event: JInt,
) -> JInt {
    let Ok(handle) = u64::try_from(handle) else {
        return atom_ffi::AtomErrorCode::BridgeInvalidArgument.exit_code();
    };
    let Ok(event) = u32::try_from(event) else {
        return atom_ffi::AtomErrorCode::BridgeInvalidArgument.exit_code();
    };
    let Ok(event) = AtomLifecycleEvent::try_from(event) else {
        return atom_ffi::AtomErrorCode::BridgeInvalidArgument.exit_code();
    };

    atom_runtime::handle_lifecycle(handle, event).map_or_else(|error| error.exit_code(), |()| 0)
}

#[unsafe(export_name = env!("ATOM_JNI_SHUTDOWN_EXPORT"))]
pub extern "system" fn atom_runtime_jni_shutdown(_env: JniEnv, _class: JniClass, handle: JLong) {
    if let Ok(handle) = u64::try_from(handle) {
        atom_runtime::shutdown_runtime(handle);
    }
}
