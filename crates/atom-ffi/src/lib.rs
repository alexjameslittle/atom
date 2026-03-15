use std::fmt;
use std::mem;
use std::ptr;

use flatbuffers::{FlatBufferBuilder, TableFinishedWIPOffset, WIPOffset};

pub type AtomResult<T> = Result<T, AtomError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomErrorCode {
    ManifestNotFound,
    ManifestParseError,
    ManifestMissingField,
    ManifestInvalidValue,
    ManifestUnknownKey,
    ModuleNotFound,
    ModuleDuplicateId,
    ModuleDependencyCycle,
    ModuleManifestInvalid,
    ExtensionIncompatible,
    CngConflict,
    CngTemplateError,
    CngWriteError,
    BridgeInvalidArgument,
    BridgeInitFailed,
    RuntimeTransitionInvalid,
    ModuleInitFailed,
    CliUsageError,
    AutomationUnavailable,
    AutomationTargetNotFound,
    AutomationLogCaptureFailed,
    ExternalToolFailed,
    InternalBug,
}

impl AtomErrorCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ManifestNotFound => "MANIFEST_NOT_FOUND",
            Self::ManifestParseError => "MANIFEST_PARSE_ERROR",
            Self::ManifestMissingField => "MANIFEST_MISSING_FIELD",
            Self::ManifestInvalidValue => "MANIFEST_INVALID_VALUE",
            Self::ManifestUnknownKey => "MANIFEST_UNKNOWN_KEY",
            Self::ModuleNotFound => "MODULE_NOT_FOUND",
            Self::ModuleDuplicateId => "MODULE_DUPLICATE_ID",
            Self::ModuleDependencyCycle => "MODULE_DEPENDENCY_CYCLE",
            Self::ModuleManifestInvalid => "MODULE_MANIFEST_INVALID",
            Self::ExtensionIncompatible => "EXTENSION_INCOMPATIBLE",
            Self::CngConflict => "CNG_CONFLICT",
            Self::CngTemplateError => "CNG_TEMPLATE_ERROR",
            Self::CngWriteError => "CNG_WRITE_ERROR",
            Self::BridgeInvalidArgument => "BRIDGE_INVALID_ARGUMENT",
            Self::BridgeInitFailed => "BRIDGE_INIT_FAILED",
            Self::RuntimeTransitionInvalid => "RUNTIME_TRANSITION_INVALID",
            Self::ModuleInitFailed => "MODULE_INIT_FAILED",
            Self::CliUsageError => "CLI_USAGE_ERROR",
            Self::AutomationUnavailable => "AUTOMATION_UNAVAILABLE",
            Self::AutomationTargetNotFound => "AUTOMATION_TARGET_NOT_FOUND",
            Self::AutomationLogCaptureFailed => "AUTOMATION_LOG_CAPTURE_FAILED",
            Self::ExternalToolFailed => "EXTERNAL_TOOL_FAILED",
            Self::InternalBug => "INTERNAL_BUG",
        }
    }

    #[must_use]
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::CliUsageError => 64,
            Self::ManifestNotFound
            | Self::ManifestParseError
            | Self::ManifestMissingField
            | Self::ManifestInvalidValue
            | Self::ManifestUnknownKey => 65,
            Self::ModuleNotFound
            | Self::ModuleDuplicateId
            | Self::ModuleDependencyCycle
            | Self::ModuleManifestInvalid
            | Self::ExtensionIncompatible => 66,
            Self::CngConflict | Self::CngTemplateError | Self::CngWriteError => 67,
            Self::BridgeInvalidArgument
            | Self::BridgeInitFailed
            | Self::RuntimeTransitionInvalid
            | Self::ModuleInitFailed => 68,
            Self::AutomationUnavailable
            | Self::AutomationTargetNotFound
            | Self::AutomationLogCaptureFailed
            | Self::ExternalToolFailed => 69,
            Self::InternalBug => 70,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomError {
    pub code: AtomErrorCode,
    pub message: String,
    pub path: Option<String>,
}

impl AtomError {
    #[must_use]
    pub fn new(code: AtomErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
        }
    }

    #[must_use]
    pub fn with_path(
        code: AtomErrorCode,
        message: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            path: Some(path.into()),
        }
    }

    #[must_use]
    pub fn exit_code(&self) -> i32 {
        self.code.exit_code()
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut builder = FlatBufferBuilder::new();
        let code = builder.create_string(self.code.as_str());
        let message = builder.create_string(&self.message);
        let path = self.path.as_ref().map(|path| builder.create_string(path));
        let root = create_atom_error(&mut builder, code, message, path);
        builder.finish(root, None);
        builder.finished_data().to_vec()
    }
}

impl fmt::Display for AtomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(path) => write!(
                formatter,
                "{}: {} ({})",
                self.code.as_str(),
                self.message,
                path
            ),
            None => write!(formatter, "{}: {}", self.code.as_str(), self.message),
        }
    }
}

fn create_atom_error<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    code: WIPOffset<&'a str>,
    message: WIPOffset<&'a str>,
    path: Option<WIPOffset<&'a str>>,
) -> WIPOffset<TableFinishedWIPOffset> {
    let table = builder.start_table();
    builder.push_slot_always::<WIPOffset<_>>(4, code);
    builder.push_slot_always::<WIPOffset<_>>(6, message);
    if let Some(path) = path {
        builder.push_slot_always::<WIPOffset<_>>(8, path);
    }
    builder.end_table(table)
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtomSlice {
    pub ptr: *const u8,
    pub len: usize,
}

impl AtomSlice {
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            ptr: bytes.as_ptr(),
            len: bytes.len(),
        }
    }

    /// # Safety
    ///
    /// The caller must guarantee that `ptr` and `len` describe a valid slice.
    #[must_use]
    pub unsafe fn as_bytes<'a>(self) -> &'a [u8] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            // SAFETY: guarded by the caller contract.
            unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct AtomOwnedBuffer {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl AtomOwnedBuffer {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            ptr: ptr::null_mut(),
            len: 0,
            cap: 0,
        }
    }

    #[must_use]
    pub fn from_vec(mut data: Vec<u8>) -> Self {
        let buffer = Self {
            ptr: data.as_mut_ptr(),
            len: data.len(),
            cap: data.capacity(),
        };
        mem::forget(data);
        buffer
    }

    /// # Safety
    ///
    /// The buffer must have been created by `AtomOwnedBuffer::from_vec`.
    #[must_use]
    pub unsafe fn into_vec(self) -> Vec<u8> {
        if self.ptr.is_null() {
            Vec::new()
        } else {
            // SAFETY: guarded by the caller contract.
            unsafe { Vec::from_raw_parts(self.ptr, self.len, self.cap) }
        }
    }
}

impl Default for AtomOwnedBuffer {
    fn default() -> Self {
        Self::empty()
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomLifecycleEvent {
    Foreground = 1,
    Background = 2,
    Suspend = 3,
    Resume = 4,
    Terminate = 5,
}

impl TryFrom<u32> for AtomLifecycleEvent {
    type Error = AtomError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Foreground),
            2 => Ok(Self::Background),
            3 => Ok(Self::Suspend),
            4 => Ok(Self::Resume),
            5 => Ok(Self::Terminate),
            _ => Err(AtomError::new(
                AtomErrorCode::BridgeInvalidArgument,
                format!("unknown lifecycle event: {value}"),
            )),
        }
    }
}

/// # Safety
///
/// `slot` must be either null or a valid writable pointer to `AtomOwnedBuffer`.
pub unsafe fn write_error_buffer(slot: *mut AtomOwnedBuffer, error: &AtomError) {
    if slot.is_null() {
        return;
    }

    // SAFETY: guarded by the caller contract.
    unsafe { ptr::write(slot, AtomOwnedBuffer::from_vec(error.encode())) };
}

#[cfg(test)]
mod tests {
    use super::{AtomError, AtomErrorCode, AtomOwnedBuffer};

    #[test]
    fn error_codes_map_to_spec_exit_codes() {
        assert_eq!(AtomErrorCode::CliUsageError.exit_code(), 64);
        assert_eq!(AtomErrorCode::ManifestInvalidValue.exit_code(), 65);
        assert_eq!(AtomErrorCode::ModuleDependencyCycle.exit_code(), 66);
        assert_eq!(AtomErrorCode::CngConflict.exit_code(), 67);
        assert_eq!(AtomErrorCode::RuntimeTransitionInvalid.exit_code(), 68);
        assert_eq!(AtomErrorCode::AutomationUnavailable.exit_code(), 69);
        assert_eq!(AtomErrorCode::AutomationTargetNotFound.exit_code(), 69);
        assert_eq!(AtomErrorCode::AutomationLogCaptureFailed.exit_code(), 69);
        assert_eq!(AtomErrorCode::ExternalToolFailed.exit_code(), 69);
        assert_eq!(AtomErrorCode::InternalBug.exit_code(), 70);
    }

    #[test]
    fn automation_error_codes_match_spec_strings() {
        assert_eq!(
            AtomErrorCode::AutomationUnavailable.as_str(),
            "AUTOMATION_UNAVAILABLE"
        );
        assert_eq!(
            AtomErrorCode::AutomationTargetNotFound.as_str(),
            "AUTOMATION_TARGET_NOT_FOUND"
        );
        assert_eq!(
            AtomErrorCode::AutomationLogCaptureFailed.as_str(),
            "AUTOMATION_LOG_CAPTURE_FAILED"
        );
    }

    #[test]
    fn atom_error_encodes_to_flatbuffer_bytes() {
        let error = AtomError::with_path(
            AtomErrorCode::ManifestNotFound,
            "manifest metadata missing",
            "//apps/hello_atom:hello_atom",
        );
        assert!(!error.encode().is_empty());
    }

    #[test]
    fn owned_buffer_round_trips() {
        let buffer = AtomOwnedBuffer::from_vec(vec![1, 2, 3]);
        // SAFETY: the buffer was allocated by `from_vec` immediately above.
        let round_trip = unsafe { buffer.into_vec() };
        assert_eq!(round_trip, vec![1, 2, 3]);
    }
}
