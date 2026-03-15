use std::fmt;
use std::mem;
use std::ptr;

use flatbuffers::{
    FlatBufferBuilder, ForwardsUOffset, InvalidFlatbuffer, Push, Table, TableFinishedWIPOffset,
    VOffsetT, Vector, Verifiable, Verifier, VerifierOptions, WIPOffset, root_unchecked,
};

pub type AtomResult<T> = Result<T, AtomError>;

pub trait AtomExportInput: Sized {
    /// Decode an Atom export request payload from `FlatBuffer` bytes.
    ///
    /// # Errors
    ///
    /// Returns an `AtomError` when the payload is missing or invalid for the
    /// implementing type.
    fn decode_atom_export(input: AtomSlice) -> AtomResult<Self>;
}

pub trait AtomExportOutput {
    /// Encode an Atom export response payload into `FlatBuffer` bytes.
    ///
    /// # Errors
    ///
    /// Returns an `AtomError` when the value cannot be encoded for the FFI
    /// boundary.
    fn encode_atom_export(self) -> AtomResult<Vec<u8>>;
}

trait AtomVectorCodec: Sized {
    fn decode_vector_root(bytes: &[u8]) -> AtomResult<Vec<Self>>;

    fn encode_vector_root(values: &[Self]) -> AtomResult<Vec<u8>>;
}

trait AtomOptionCodec: Sized {
    fn decode_option_root(bytes: &[u8]) -> AtomResult<Option<Self>>;

    fn encode_option_root(value: Option<Self>) -> AtomResult<Vec<u8>>;
}

impl<T: AtomVectorCodec> AtomExportInput for Vec<T> {
    fn decode_atom_export(input: AtomSlice) -> AtomResult<Self> {
        let bytes = unsafe { input.as_bytes() };
        T::decode_vector_root(bytes)
    }
}

impl<T: AtomVectorCodec> AtomExportOutput for Vec<T> {
    fn encode_atom_export(self) -> AtomResult<Vec<u8>> {
        T::encode_vector_root(&self)
    }
}

impl<T: AtomOptionCodec> AtomExportInput for Option<T> {
    fn decode_atom_export(input: AtomSlice) -> AtomResult<Self> {
        let bytes = unsafe { input.as_bytes() };
        T::decode_option_root(bytes)
    }
}

impl<T: AtomOptionCodec> AtomExportOutput for Option<T> {
    fn encode_atom_export(self) -> AtomResult<Vec<u8>> {
        T::encode_option_root(self)
    }
}

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

const VALUE_FIELD: VOffsetT = 4;
const OPTION_PRESENT_FIELD: VOffsetT = 4;
const OPTION_VALUE_FIELD: VOffsetT = 6;

fn invalid_export_payload(error: &InvalidFlatbuffer) -> AtomError {
    AtomError::new(
        AtomErrorCode::BridgeInvalidArgument,
        format!("invalid Atom export payload: {error}"),
    )
}

fn internal_codec_bug(message: &'static str) -> AtomError {
    AtomError::new(AtomErrorCode::InternalBug, message)
}

fn verify_required_root_field<T: Verifiable>(
    bytes: &[u8],
    field_name: &'static str,
) -> AtomResult<()> {
    let options = VerifierOptions::default();
    let mut verifier = Verifier::new(&options, bytes);
    let root = verifier
        .get_uoffset(0)
        .map_err(|error| invalid_export_payload(&error))? as usize;
    verifier
        .visit_table(root)
        .map_err(|error| invalid_export_payload(&error))?
        .visit_field::<T>(field_name, VALUE_FIELD, true)
        .map_err(|error| invalid_export_payload(&error))?
        .finish();
    Ok(())
}

fn verify_optional_root_field<T: Verifiable>(
    bytes: &[u8],
    field_name: &'static str,
) -> AtomResult<()> {
    let options = VerifierOptions::default();
    let mut verifier = Verifier::new(&options, bytes);
    let root = verifier
        .get_uoffset(0)
        .map_err(|error| invalid_export_payload(&error))? as usize;
    verifier
        .visit_table(root)
        .map_err(|error| invalid_export_payload(&error))?
        .visit_field::<bool>("present", OPTION_PRESENT_FIELD, true)
        .map_err(|error| invalid_export_payload(&error))?
        .visit_field::<T>(field_name, OPTION_VALUE_FIELD, false)
        .map_err(|error| invalid_export_payload(&error))?
        .finish();
    Ok(())
}

fn verified_root_table(bytes: &[u8]) -> Table<'_> {
    // SAFETY: callers only use this after `verify_required_root_field` or
    // `verify_optional_root_field` succeeds for the same buffer.
    unsafe { root_unchecked::<Table<'_>>(bytes) }
}

fn finish_table(
    builder: &mut FlatBufferBuilder<'_>,
    table: WIPOffset<TableFinishedWIPOffset>,
) -> Vec<u8> {
    builder.finish(table, None);
    builder.finished_data().to_vec()
}

fn encode_required_scalar_root<T: Push>(value: T) -> Vec<u8> {
    let mut builder = FlatBufferBuilder::new();
    let table = builder.start_table();
    builder.push_slot_always(VALUE_FIELD, value);
    let table = builder.end_table(table);
    finish_table(&mut builder, table)
}

fn encode_required_string_root(value: &str) -> Vec<u8> {
    let mut builder = FlatBufferBuilder::new();
    let value = builder.create_string(value);
    let table = builder.start_table();
    builder.push_slot_always(VALUE_FIELD, value);
    let table = builder.end_table(table);
    finish_table(&mut builder, table)
}

macro_rules! impl_scalar_export_codecs {
    ($($ty:ty),* $(,)?) => {
        $(
            impl AtomExportInput for $ty {
                fn decode_atom_export(input: AtomSlice) -> AtomResult<Self> {
                    let bytes = unsafe { input.as_bytes() };
                    verify_required_root_field::<$ty>(bytes, "value")?;
                    let table = verified_root_table(bytes);
                    unsafe { table.get::<$ty>(VALUE_FIELD, None) }.ok_or_else(|| {
                        internal_codec_bug(concat!(
                            "missing verified scalar Atom export value for ",
                            stringify!($ty),
                        ))
                    })
                }
            }

            impl AtomExportOutput for $ty {
                fn encode_atom_export(self) -> AtomResult<Vec<u8>> {
                    Ok(encode_required_scalar_root(self))
                }
            }

            impl AtomVectorCodec for $ty {
                fn decode_vector_root(bytes: &[u8]) -> AtomResult<Vec<Self>> {
                    verify_required_root_field::<ForwardsUOffset<Vector<'_, $ty>>>(bytes, "value")?;
                    let table = verified_root_table(bytes);
                    let values = unsafe {
                        table.get::<ForwardsUOffset<Vector<'_, $ty>>>(VALUE_FIELD, None)
                    }
                    .ok_or_else(|| {
                        internal_codec_bug(concat!(
                            "missing verified vector Atom export value for ",
                            stringify!($ty),
                        ))
                    })?;
                    Ok(values.iter().collect())
                }

                fn encode_vector_root(values: &[Self]) -> AtomResult<Vec<u8>> {
                    let mut builder = FlatBufferBuilder::new();
                    let values = builder.create_vector(values);
                    let table = builder.start_table();
                    builder.push_slot_always(VALUE_FIELD, values);
                    let table = builder.end_table(table);
                    Ok(finish_table(&mut builder, table))
                }
            }

            impl AtomOptionCodec for $ty {
                fn decode_option_root(bytes: &[u8]) -> AtomResult<Option<Self>> {
                    verify_optional_root_field::<$ty>(bytes, "value")?;
                    let table = verified_root_table(bytes);
                    let present = unsafe { table.get::<bool>(OPTION_PRESENT_FIELD, None) }
                        .ok_or_else(|| internal_codec_bug("missing verified Atom export option presence"))?;
                    if !present {
                        return Ok(None);
                    }

                    let value = unsafe {
                        table.get::<$ty>(OPTION_VALUE_FIELD, Some(<$ty>::default()))
                    }
                    .ok_or_else(|| {
                        internal_codec_bug(concat!(
                            "missing verified Atom export option value for ",
                            stringify!($ty),
                        ))
                    })?;
                    Ok(Some(value))
                }

                fn encode_option_root(value: Option<Self>) -> AtomResult<Vec<u8>> {
                    let mut builder = FlatBufferBuilder::new();
                    let table = builder.start_table();
                    builder.push_slot_always(OPTION_PRESENT_FIELD, value.is_some());
                    if let Some(value) = value {
                        builder.push_slot(OPTION_VALUE_FIELD, value, <$ty>::default());
                    }
                    let table = builder.end_table(table);
                    Ok(finish_table(&mut builder, table))
                }
            }
        )*
    };
}

impl_scalar_export_codecs!(i8, i16, i32, i64, u8, u16, u32, u64, f32, f64, bool);

impl AtomExportInput for String {
    fn decode_atom_export(input: AtomSlice) -> AtomResult<Self> {
        let bytes = unsafe { input.as_bytes() };
        verify_required_root_field::<ForwardsUOffset<&str>>(bytes, "value")?;
        let table = verified_root_table(bytes);
        let value = unsafe { table.get::<ForwardsUOffset<&str>>(VALUE_FIELD, None) }
            .ok_or_else(|| internal_codec_bug("missing verified string Atom export value"))?;
        Ok(value.to_owned())
    }
}

impl AtomExportOutput for String {
    fn encode_atom_export(self) -> AtomResult<Vec<u8>> {
        Ok(encode_required_string_root(&self))
    }
}

impl AtomExportOutput for &str {
    fn encode_atom_export(self) -> AtomResult<Vec<u8>> {
        Ok(encode_required_string_root(self))
    }
}

impl AtomVectorCodec for String {
    fn decode_vector_root(bytes: &[u8]) -> AtomResult<Vec<Self>> {
        verify_required_root_field::<ForwardsUOffset<Vector<'_, ForwardsUOffset<&str>>>>(
            bytes, "value",
        )?;
        let table = verified_root_table(bytes);
        let values = unsafe {
            table.get::<ForwardsUOffset<Vector<'_, ForwardsUOffset<&str>>>>(VALUE_FIELD, None)
        }
        .ok_or_else(|| internal_codec_bug("missing verified string vector Atom export value"))?;
        Ok(values.iter().map(str::to_owned).collect())
    }

    fn encode_vector_root(values: &[Self]) -> AtomResult<Vec<u8>> {
        let mut builder = FlatBufferBuilder::new();
        let values: Vec<_> = values
            .iter()
            .map(|value| builder.create_string(value))
            .collect();
        let values = builder.create_vector(&values);
        let table = builder.start_table();
        builder.push_slot_always(VALUE_FIELD, values);
        let table = builder.end_table(table);
        Ok(finish_table(&mut builder, table))
    }
}

impl AtomOptionCodec for String {
    fn decode_option_root(bytes: &[u8]) -> AtomResult<Option<Self>> {
        verify_optional_root_field::<ForwardsUOffset<&str>>(bytes, "value")?;
        let table = verified_root_table(bytes);
        let present = unsafe { table.get::<bool>(OPTION_PRESENT_FIELD, None) }
            .ok_or_else(|| internal_codec_bug("missing verified Atom export option presence"))?;
        if !present {
            return Ok(None);
        }

        let value = unsafe { table.get::<ForwardsUOffset<&str>>(OPTION_VALUE_FIELD, None) }
            .ok_or_else(|| {
                internal_codec_bug("missing verified Atom export string option value")
            })?;
        Ok(Some(value.to_owned()))
    }

    fn encode_option_root(value: Option<Self>) -> AtomResult<Vec<u8>> {
        let mut builder = FlatBufferBuilder::new();
        let value = value.map(|value| builder.create_string(&value));
        let table = builder.start_table();
        builder.push_slot_always(OPTION_PRESENT_FIELD, value.is_some());
        if let Some(value) = value {
            builder.push_slot_always(OPTION_VALUE_FIELD, value);
        }
        let table = builder.end_table(table);
        Ok(finish_table(&mut builder, table))
    }
}

impl AtomExportOutput for () {
    fn encode_atom_export(self) -> AtomResult<Vec<u8>> {
        Ok(Vec::new())
    }
}

pub type AtomRuntimeHandle = u64;

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
    unsafe { replace_buffer(slot, AtomOwnedBuffer::from_vec(error.encode())) };
}

/// Validate that a generated export received a writable response slot.
///
/// # Errors
///
/// Returns `BRIDGE_INVALID_ARGUMENT` when the caller passes a null response
/// buffer pointer.
pub fn require_owned_buffer_slot(
    slot: *mut AtomOwnedBuffer,
    slot_name: &'static str,
) -> AtomResult<()> {
    if slot.is_null() {
        Err(AtomError::new(
            AtomErrorCode::BridgeInvalidArgument,
            format!("{slot_name} must be a non-null AtomOwnedBuffer pointer"),
        ))
    } else {
        Ok(())
    }
}

/// # Safety
///
/// `slot` must be either null or a valid writable pointer to `AtomOwnedBuffer`.
pub unsafe fn clear_buffer(slot: *mut AtomOwnedBuffer) {
    if slot.is_null() {
        return;
    }

    // SAFETY: guarded by the caller contract.
    unsafe { replace_buffer(slot, AtomOwnedBuffer::empty()) };
}

/// # Safety
///
/// `slot` must be either null or a valid writable pointer to `AtomOwnedBuffer`.
pub unsafe fn write_response_buffer(slot: *mut AtomOwnedBuffer, data: Vec<u8>) {
    if slot.is_null() {
        return;
    }

    // SAFETY: guarded by the caller contract.
    unsafe { replace_buffer(slot, AtomOwnedBuffer::from_vec(data)) };
}

unsafe fn replace_buffer(slot: *mut AtomOwnedBuffer, buffer: AtomOwnedBuffer) {
    // SAFETY: guarded by the caller contract.
    let previous = unsafe { ptr::read(slot) };
    if !previous.ptr.is_null() {
        // SAFETY: `previous` came from the same buffer slot contract as `slot`.
        let _ = unsafe { previous.into_vec() };
    }
    // SAFETY: guarded by the caller contract.
    unsafe { ptr::write(slot, buffer) };
}

#[cfg(test)]
mod tests {
    use super::{
        AtomError, AtomErrorCode, AtomExportInput, AtomExportOutput, AtomOwnedBuffer, AtomSlice,
        clear_buffer, require_owned_buffer_slot, write_response_buffer,
    };

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

    #[test]
    fn unit_output_encodes_as_empty_payload() {
        assert_eq!(().encode_atom_export().unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn string_export_codec_round_trips() {
        let encoded = "hello atom".to_owned().encode_atom_export().unwrap();
        let decoded = String::decode_atom_export(AtomSlice::from_bytes(&encoded)).unwrap();
        assert_eq!(decoded, "hello atom");
    }

    #[test]
    fn option_scalar_codec_preserves_default_value() {
        let encoded = Some(0_i32).encode_atom_export().unwrap();
        let decoded = Option::<i32>::decode_atom_export(AtomSlice::from_bytes(&encoded)).unwrap();
        assert_eq!(decoded, Some(0));
    }

    #[test]
    fn string_vector_export_codec_round_trips() {
        let encoded = vec!["alpha".to_owned(), "beta".to_owned()]
            .encode_atom_export()
            .unwrap();
        let decoded = Vec::<String>::decode_atom_export(AtomSlice::from_bytes(&encoded)).unwrap();
        assert_eq!(decoded, vec!["alpha".to_owned(), "beta".to_owned()]);
    }

    #[test]
    fn string_option_export_codec_preserves_empty_string() {
        let encoded = Some(String::new()).encode_atom_export().unwrap();
        let decoded =
            Option::<String>::decode_atom_export(AtomSlice::from_bytes(&encoded)).unwrap();
        assert_eq!(decoded, Some(String::new()));
    }

    #[test]
    fn primitive_decode_rejects_invalid_flatbuffer_bytes() {
        let error = i32::decode_atom_export(AtomSlice::from_bytes(&[1, 2, 3]))
            .expect_err("invalid payload should fail");
        assert_eq!(error.code, AtomErrorCode::BridgeInvalidArgument);
    }

    #[test]
    fn require_owned_buffer_slot_rejects_null() {
        let error = require_owned_buffer_slot(std::ptr::null_mut(), "out_response_flatbuffer")
            .expect_err("null slot should fail");
        assert_eq!(error.code, AtomErrorCode::BridgeInvalidArgument);
    }

    #[test]
    fn write_response_buffer_round_trips() {
        let mut slot = AtomOwnedBuffer::empty();
        // SAFETY: `slot` is a valid writable pointer for the duration of the call.
        unsafe { write_response_buffer(&mut slot, vec![7, 8, 9]) };
        // SAFETY: the buffer was allocated by `write_response_buffer` immediately above.
        let round_trip = unsafe { slot.into_vec() };
        assert_eq!(round_trip, vec![7, 8, 9]);
    }

    #[test]
    fn clear_buffer_resets_existing_contents() {
        let mut slot = AtomOwnedBuffer::from_vec(vec![4, 5, 6]);
        // SAFETY: `slot` is a valid writable pointer for the duration of the call.
        unsafe { clear_buffer(&mut slot) };
        assert_eq!(slot.len, 0);
        assert!(slot.ptr.is_null());
    }
}
