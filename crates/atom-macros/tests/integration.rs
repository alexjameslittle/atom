use std::sync::{LazyLock, Mutex, OnceLock};

use atom_ffi::{
    AtomError, AtomErrorCode, AtomExportInput, AtomExportOutput, AtomOwnedBuffer, AtomResult,
    AtomSlice,
};
use atom_runtime::RuntimeConfig;
use flatbuffers::{FlatBufferBuilder, ForwardsUOffset, Table, root_unchecked};

static TEST_MUTEX: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static RUNTIME_INIT: OnceLock<()> = OnceLock::new();

#[atom_macros::atom_record]
#[derive(Debug, Clone, PartialEq, Eq)]
struct EchoRequest {
    message: String,
}

#[atom_macros::atom_record]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionStatus {
    Connected,
    Disconnected,
}

#[atom_macros::atom_record]
#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceInfo {
    model: String,
    os: String,
    status: ConnectionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EchoResponse(String);

impl From<ConnectionStatus> for u8 {
    fn from(value: ConnectionStatus) -> Self {
        match value {
            ConnectionStatus::Connected => 0,
            ConnectionStatus::Disconnected => 1,
        }
    }
}

impl TryFrom<u8> for ConnectionStatus {
    type Error = AtomError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Connected),
            1 => Ok(Self::Disconnected),
            _ => Err(AtomError::new(
                AtomErrorCode::BridgeInvalidArgument,
                format!("unknown connection status: {value}"),
            )),
        }
    }
}

impl AtomExportInput for EchoRequest {
    fn decode_atom_export(input: AtomSlice) -> AtomResult<Self> {
        let bytes = unsafe { input.as_bytes() };
        let table = unsafe { root_unchecked::<Table<'_>>(bytes) };
        let message = unsafe { table.get::<ForwardsUOffset<&str>>(4, None) }
            .unwrap_or("")
            .to_owned();
        Ok(Self { message })
    }
}

impl AtomExportOutput for DeviceInfo {
    fn encode_atom_export(self) -> AtomResult<Vec<u8>> {
        let mut builder = FlatBufferBuilder::new();
        let model = builder.create_string(&self.model);
        let os = builder.create_string(&self.os);
        let table = builder.start_table();
        builder.push_slot_always(4, model);
        builder.push_slot_always(6, os);
        builder.push_slot(8, u8::from(self.status), 0);
        let root = builder.end_table(table);
        builder.finish(root, None);
        Ok(builder.finished_data().to_vec())
    }
}

impl AtomExportOutput for EchoResponse {
    fn encode_atom_export(self) -> AtomResult<Vec<u8>> {
        let mut builder = FlatBufferBuilder::new();
        let value = builder.create_string(&self.0);
        let table = builder.start_table();
        builder.push_slot_always(4, value);
        let root = builder.end_table(table);
        builder.finish(root, None);
        Ok(builder.finished_data().to_vec())
    }
}

impl AtomExportOutput for ConnectionStatus {
    fn encode_atom_export(self) -> AtomResult<Vec<u8>> {
        let mut builder = FlatBufferBuilder::new();
        let table = builder.start_table();
        builder.push_slot(4, u8::from(self), 0);
        let root = builder.end_table(table);
        builder.finish(root, None);
        Ok(builder.finished_data().to_vec())
    }
}

#[atom_macros::atom_export]
fn get() -> DeviceInfo {
    DeviceInfo {
        model: "iPhone16,2".to_owned(),
        os: "ios-arm64".to_owned(),
        status: ConnectionStatus::Connected,
    }
}

#[atom_macros::atom_export]
fn echo(request: EchoRequest) -> Result<EchoResponse, AtomError> {
    let message = request.message;
    if message.is_empty() {
        Err(AtomError::new(
            AtomErrorCode::BridgeInvalidArgument,
            "echo request message must not be empty",
        ))
    } else {
        Ok(EchoResponse(format!("echo: {message}")))
    }
}

#[atom_macros::atom_export]
fn clear(request: EchoRequest) {
    std::mem::drop(request.message);
}

#[atom_macros::atom_export]
fn fail(_request: EchoRequest) -> Result<EchoResponse, AtomError> {
    Err(AtomError::new(
        AtomErrorCode::ModuleInitFailed,
        "intentional test failure",
    ))
}

#[atom_macros::atom_export]
fn status() -> ConnectionStatus {
    ConnectionStatus::Disconnected
}

fn encode_echo_request(message: &str) -> Vec<u8> {
    let mut builder = FlatBufferBuilder::new();
    let message = builder.create_string(message);
    let table = builder.start_table();
    builder.push_slot_always(4, message);
    let root = builder.end_table(table);
    builder.finish(root, None);
    builder.finished_data().to_vec()
}

fn decode_device_info(bytes: &[u8]) -> DeviceInfo {
    let table = unsafe { root_unchecked::<Table<'_>>(bytes) };
    let model = unsafe { table.get::<ForwardsUOffset<&str>>(4, None) }
        .unwrap_or("")
        .to_owned();
    let os = unsafe { table.get::<ForwardsUOffset<&str>>(6, None) }
        .unwrap_or("")
        .to_owned();
    let status = unsafe { table.get::<u8>(8, Some(0)) }
        .unwrap_or(0)
        .try_into()
        .expect("known status");
    DeviceInfo { model, os, status }
}

fn decode_string_response(bytes: &[u8]) -> String {
    let table = unsafe { root_unchecked::<Table<'_>>(bytes) };
    unsafe { table.get::<ForwardsUOffset<&str>>(4, None) }
        .unwrap_or("")
        .to_owned()
}

fn decode_status(bytes: &[u8]) -> ConnectionStatus {
    let table = unsafe { root_unchecked::<Table<'_>>(bytes) };
    unsafe { table.get::<u8>(4, Some(0)) }
        .unwrap_or(0)
        .try_into()
        .expect("known status")
}

fn take_buffer(buffer: AtomOwnedBuffer) -> Vec<u8> {
    unsafe { buffer.into_vec() }
}

fn ensure_runtime_initialized() {
    RUNTIME_INIT.get_or_init(|| {
        atom_runtime::__init(RuntimeConfig).expect("runtime init");
    });
}

#[test]
fn no_input_export_round_trips_device_info_flatbuffer() {
    let _guard = TEST_MUTEX.lock().unwrap();
    ensure_runtime_initialized();

    let mut response = AtomOwnedBuffer::empty();
    let mut error = AtomOwnedBuffer::empty();
    let status = unsafe { __atom_export_get(&raw mut response, &raw mut error) };

    assert_eq!(status, 0);
    assert!(take_buffer(error).is_empty());
    let decoded = decode_device_info(&take_buffer(response));
    assert_eq!(
        decoded,
        DeviceInfo {
            model: "iPhone16,2".to_owned(),
            os: "ios-arm64".to_owned(),
            status: ConnectionStatus::Connected,
        }
    );
}

#[test]
fn result_export_routes_ok_value_to_response_buffer() {
    let _guard = TEST_MUTEX.lock().unwrap();
    ensure_runtime_initialized();

    let input = encode_echo_request("hello");
    let mut response = AtomOwnedBuffer::empty();
    let mut error = AtomOwnedBuffer::empty();
    let status = unsafe {
        __atom_export_echo(
            AtomSlice::from_bytes(&input),
            &raw mut response,
            &raw mut error,
        )
    };

    assert_eq!(status, 0);
    assert!(take_buffer(error).is_empty());
    assert_eq!(
        decode_string_response(&take_buffer(response)),
        "echo: hello"
    );
}

#[test]
fn unit_return_export_clears_response_buffer() {
    let _guard = TEST_MUTEX.lock().unwrap();
    ensure_runtime_initialized();

    let input = encode_echo_request("clear");
    let mut response = AtomOwnedBuffer::from_vec(vec![1, 2, 3]);
    let mut error = AtomOwnedBuffer::from_vec(vec![4, 5, 6]);
    let status = unsafe {
        __atom_export_clear(
            AtomSlice::from_bytes(&input),
            &raw mut response,
            &raw mut error,
        )
    };

    assert_eq!(status, 0);
    assert!(take_buffer(response).is_empty());
    assert!(take_buffer(error).is_empty());
}

#[test]
fn result_export_routes_err_value_to_error_buffer() {
    let _guard = TEST_MUTEX.lock().unwrap();
    ensure_runtime_initialized();

    let input = encode_echo_request("boom");
    let mut response = AtomOwnedBuffer::empty();
    let mut error = AtomOwnedBuffer::empty();
    let status = unsafe {
        __atom_export_fail(
            AtomSlice::from_bytes(&input),
            &raw mut response,
            &raw mut error,
        )
    };

    assert_eq!(status, AtomErrorCode::ModuleInitFailed.exit_code());
    assert!(take_buffer(response).is_empty());
    assert!(!take_buffer(error).is_empty());
}

#[test]
fn enum_export_round_trips_flatbuffer_response() {
    let _guard = TEST_MUTEX.lock().unwrap();
    ensure_runtime_initialized();

    let mut response = AtomOwnedBuffer::empty();
    let mut error = AtomOwnedBuffer::empty();
    let status = unsafe { __atom_export_status(&raw mut response, &raw mut error) };

    assert_eq!(status, 0);
    assert!(take_buffer(error).is_empty());
    assert_eq!(
        decode_status(&take_buffer(response)),
        ConnectionStatus::Disconnected
    );
}
