use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use atom_backends::{DebugBacktrace, DebugBreakpoint, DebugStop, DebugThread};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

const REMOTE_LLDB_SERVER_PATH: &str = "/data/local/tmp/atom-lldb-server";
const SOCKET_POLL_ATTEMPTS: usize = 30;
const SOCKET_POLL_INTERVAL: Duration = Duration::from_millis(250);
const HELPER_SOURCE: &str = include_str!("native_lldb_helper.py");

pub struct AndroidNativeAttachOptions {
    pub serial: String,
    pub application_id: String,
    pub pid: u32,
    pub native_library_path: Utf8PathBuf,
    pub source_map_prefix: Option<String>,
}

pub struct AndroidNativeClient {
    repo_root: Utf8PathBuf,
    serial: String,
    application_id: String,
    local_port: u16,
    remote_socket: String,
    helper_script_dir: Utf8PathBuf,
    helper_child: Child,
    helper_stdin: ChildStdin,
    helper_stdout: BufReader<ChildStdout>,
    server_child: Child,
    breakpoints: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct HelperRequest<'a> {
    command: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    breakpoint_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct HelperEnvelope<T> {
    ok: bool,
    value: Option<T>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HelperReady {
    ready: bool,
}

impl AndroidNativeClient {
    /// # Errors
    ///
    /// Returns an error if lldb-server transport or the host helper cannot be started.
    pub fn attach(repo_root: &Utf8Path, options: &AndroidNativeAttachOptions) -> AtomResult<Self> {
        let abi = capture_command(
            repo_root,
            "adb",
            &[
                "-s".to_owned(),
                options.serial.clone(),
                "shell".to_owned(),
                "getprop".to_owned(),
                "ro.product.cpu.abi".to_owned(),
            ],
        )?;
        let lldb_server = resolve_android_lldb_server(abi.trim())?;
        run_command(
            repo_root,
            "adb",
            &[
                "-s".to_owned(),
                options.serial.clone(),
                "push".to_owned(),
                lldb_server.as_str().to_owned(),
                REMOTE_LLDB_SERVER_PATH.to_owned(),
            ],
        )?;
        run_command(
            repo_root,
            "adb",
            &[
                "-s".to_owned(),
                options.serial.clone(),
                "shell".to_owned(),
                "chmod".to_owned(),
                "755".to_owned(),
                REMOTE_LLDB_SERVER_PATH.to_owned(),
            ],
        )?;

        let app_data_dir = capture_command(
            repo_root,
            "adb",
            &[
                "-s".to_owned(),
                options.serial.clone(),
                "shell".to_owned(),
                "run-as".to_owned(),
                options.application_id.clone(),
                "pwd".to_owned(),
            ],
        )?;
        let app_data_dir = app_data_dir.trim();
        if app_data_dir.is_empty() {
            return Err(AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                "run-as did not return an app data directory for native debugging",
            ));
        }

        let timestamp = timestamp_nanos();
        let remote_socket = format!("{app_data_dir}/cache/atom-lldb-{timestamp}.sock");
        let local_port = free_tcp_port()?;
        run_command(
            repo_root,
            "adb",
            &[
                "-s".to_owned(),
                options.serial.clone(),
                "shell".to_owned(),
                "run-as".to_owned(),
                options.application_id.clone(),
                "rm".to_owned(),
                "-f".to_owned(),
                remote_socket.clone(),
            ],
        )?;
        run_command(
            repo_root,
            "adb",
            &[
                "-s".to_owned(),
                options.serial.clone(),
                "forward".to_owned(),
                format!("tcp:{local_port}"),
                format!("localfilesystem:{remote_socket}"),
            ],
        )?;

        let server_log_path =
            std::env::temp_dir().join(format!("atom-lldb-server-{timestamp}.log"));
        let server_log = fs::File::create(&server_log_path).map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to create lldb-server log file: {error}"),
            )
        })?;
        let server_log_err = server_log.try_clone().map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to clone lldb-server log file: {error}"),
            )
        })?;
        let server_child = Command::new("adb")
            .args([
                "-s",
                &options.serial,
                "shell",
                "run-as",
                &options.application_id,
                REMOTE_LLDB_SERVER_PATH,
                "gdbserver",
                &format!("unix://{remote_socket}"),
                "--attach",
                &options.pid.to_string(),
            ])
            .current_dir(repo_root)
            .stdout(Stdio::from(server_log))
            .stderr(Stdio::from(server_log_err))
            .spawn()
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to start Android lldb-server: {error}"),
                )
            })?;

        wait_for_socket(
            repo_root,
            &options.serial,
            &options.application_id,
            &remote_socket,
        )?;

        let helper_script_dir = write_helper_script()?;
        let ndk_python = resolve_ndk_python()?;
        let lldb_pythonpath = resolve_ndk_lldb_pythonpath()?;
        let helper_script = helper_script_dir.join("native_lldb_helper.py");
        let exec_search_path = options
            .native_library_path
            .parent()
            .ok_or_else(|| {
                AtomError::with_path(
                    AtomErrorCode::ExternalToolFailed,
                    "native library path has no parent directory",
                    options.native_library_path.as_str(),
                )
            })?
            .to_owned();
        let mut helper_child = Command::new(ndk_python)
            .arg(helper_script.as_str())
            .arg("--connect-port")
            .arg(local_port.to_string())
            .arg("--native-library")
            .arg(options.native_library_path.as_str())
            .arg("--exec-search-path")
            .arg(exec_search_path.as_str())
            .arg("--source-map-prefix")
            .arg(options.source_map_prefix.as_deref().unwrap_or(""))
            .arg("--source-map-root")
            .arg(repo_root.as_str())
            .current_dir(repo_root)
            .env("PYTHONPATH", lldb_pythonpath)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to start Android LLDB helper: {error}"),
                )
            })?;
        let helper_stdin = helper_child.stdin.take().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "Android LLDB helper stdin was not available",
            )
        })?;
        let helper_stdout = helper_child.stdout.take().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "Android LLDB helper stdout was not available",
            )
        })?;
        let mut client = Self {
            repo_root: repo_root.to_owned(),
            serial: options.serial.clone(),
            application_id: options.application_id.clone(),
            local_port,
            remote_socket,
            helper_script_dir,
            helper_child,
            helper_stdin,
            helper_stdout: BufReader::new(helper_stdout),
            server_child,
            breakpoints: BTreeMap::new(),
        };
        let ready: HelperReady = client.read_helper_line()?;
        if !ready.ready {
            return Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "Android LLDB helper did not report ready",
            ));
        }
        Ok(client)
    }

    /// # Errors
    ///
    /// Returns an error if the breakpoint could not be created.
    pub fn set_breakpoint(&mut self, file: &str, line: u32) -> AtomResult<DebugBreakpoint> {
        let breakpoint: DebugBreakpoint = self.send_request(&HelperRequest {
            command: "set_breakpoint",
            file: Some(file),
            line: Some(line),
            breakpoint_id: None,
            thread_id: None,
            timeout_ms: None,
        })?;
        self.breakpoints
            .insert(breakpoint_key(file, line), breakpoint.id.clone());
        Ok(breakpoint)
    }

    /// # Errors
    ///
    /// Returns an error if the breakpoint could not be deleted.
    pub fn clear_breakpoint(&mut self, file: &str, line: u32) -> AtomResult<()> {
        let key = breakpoint_key(file, line);
        let breakpoint_id = self.breakpoints.remove(&key).ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                format!("no active native breakpoint matches {key}"),
            )
        })?;
        let _: serde_json::Value = self.send_request(&HelperRequest {
            command: "clear_breakpoint",
            file: None,
            line: None,
            breakpoint_id: Some(&breakpoint_id),
            thread_id: None,
            timeout_ms: None,
        })?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the process does not stop before the timeout expires.
    pub fn wait_for_stop(&mut self, timeout_ms: Option<u64>) -> AtomResult<DebugStop> {
        self.send_request(&HelperRequest {
            command: "wait_for_stop",
            file: None,
            line: None,
            breakpoint_id: None,
            thread_id: None,
            timeout_ms,
        })
    }

    /// # Errors
    ///
    /// Returns an error if thread metadata could not be captured.
    pub fn threads(&mut self) -> AtomResult<Vec<DebugThread>> {
        self.send_request(&HelperRequest {
            command: "threads",
            file: None,
            line: None,
            breakpoint_id: None,
            thread_id: None,
            timeout_ms: None,
        })
    }

    /// # Errors
    ///
    /// Returns an error if backtrace metadata could not be captured.
    pub fn backtrace(&mut self, thread_id: Option<&str>) -> AtomResult<DebugBacktrace> {
        self.send_request(&HelperRequest {
            command: "backtrace",
            file: None,
            line: None,
            breakpoint_id: None,
            thread_id,
            timeout_ms: None,
        })
    }

    /// # Errors
    ///
    /// Returns an error if the process could not be interrupted.
    pub fn pause(&mut self) -> AtomResult<DebugStop> {
        self.send_request(&HelperRequest {
            command: "pause",
            file: None,
            line: None,
            breakpoint_id: None,
            thread_id: None,
            timeout_ms: None,
        })
    }

    /// # Errors
    ///
    /// Returns an error if the process could not be resumed.
    pub fn resume(&mut self) -> AtomResult<()> {
        let _: serde_json::Value = self.send_request(&HelperRequest {
            command: "resume",
            file: None,
            line: None,
            breakpoint_id: None,
            thread_id: None,
            timeout_ms: None,
        })?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the helper or lldb-server transport could not be shut down cleanly.
    pub fn shutdown(&mut self) -> AtomResult<()> {
        let _ = self.send_request::<serde_json::Value>(&HelperRequest {
            command: "shutdown",
            file: None,
            line: None,
            breakpoint_id: None,
            thread_id: None,
            timeout_ms: None,
        });
        let _ = self.helper_child.wait();
        let _ = self.server_child.kill();
        let _ = self.server_child.wait();
        let _ = run_command(
            &self.repo_root,
            "adb",
            &[
                "-s".to_owned(),
                self.serial.clone(),
                "forward".to_owned(),
                "--remove".to_owned(),
                format!("tcp:{}", self.local_port),
            ],
        );
        let _ = run_command(
            &self.repo_root,
            "adb",
            &[
                "-s".to_owned(),
                self.serial.clone(),
                "shell".to_owned(),
                "run-as".to_owned(),
                self.application_id.clone(),
                "rm".to_owned(),
                "-f".to_owned(),
                self.remote_socket.clone(),
            ],
        );
        let _ = fs::remove_dir_all(&self.helper_script_dir);
        Ok(())
    }

    fn send_request<T>(&mut self, request: &HelperRequest<'_>) -> AtomResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let payload = serde_json::to_string(request).map_err(|error| {
            AtomError::new(
                AtomErrorCode::InternalBug,
                format!("failed to encode helper request: {error}"),
            )
        })?;
        self.helper_stdin
            .write_all(payload.as_bytes())
            .and_then(|()| self.helper_stdin.write_all(b"\n"))
            .and_then(|()| self.helper_stdin.flush())
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to send helper request: {error}"),
                )
            })?;
        let envelope: HelperEnvelope<T> = self.read_helper_line()?;
        if envelope.ok {
            envelope.value.ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::InternalBug,
                    "helper response was missing a value",
                )
            })
        } else {
            Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                envelope
                    .message
                    .unwrap_or_else(|| "helper command failed".to_owned()),
            ))
        }
    }

    fn read_helper_line<T>(&mut self) -> AtomResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let mut line = String::new();
        let bytes = self.helper_stdout.read_line(&mut line).map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to read helper response: {error}"),
            )
        })?;
        if bytes == 0 {
            let stderr = self
                .helper_child
                .stderr
                .as_mut()
                .and_then(|stderr| {
                    let mut buffer = String::new();
                    BufReader::new(stderr).read_to_string(&mut buffer).ok()?;
                    Some(buffer)
                })
                .unwrap_or_default();
            return Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                if stderr.trim().is_empty() {
                    "Android LLDB helper exited unexpectedly".to_owned()
                } else {
                    format!("Android LLDB helper exited unexpectedly:\n{stderr}")
                },
            ));
        }
        serde_json::from_str(line.trim()).map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to decode helper response: {error}"),
            )
        })
    }
}

fn wait_for_socket(
    repo_root: &Utf8Path,
    serial: &str,
    application_id: &str,
    remote_socket: &str,
) -> AtomResult<()> {
    for _ in 0..SOCKET_POLL_ATTEMPTS {
        if run_command(
            repo_root,
            "adb",
            &[
                "-s".to_owned(),
                serial.to_owned(),
                "shell".to_owned(),
                "run-as".to_owned(),
                application_id.to_owned(),
                "sh".to_owned(),
                "-c".to_owned(),
                format!("test -S {remote_socket}"),
            ],
        )
        .is_ok()
        {
            return Ok(());
        }
        std::thread::sleep(SOCKET_POLL_INTERVAL);
    }
    Err(AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        "lldb-server did not create its Android debug socket in time",
    ))
}

fn write_helper_script() -> AtomResult<Utf8PathBuf> {
    let directory = Utf8PathBuf::from_path_buf(
        std::env::temp_dir().join(format!("atom-native-lldb-{}", timestamp_nanos())),
    )
    .map_err(|_| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            "temporary helper path was not valid UTF-8",
        )
    })?;
    fs::create_dir_all(&directory).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to create helper directory: {error}"),
            directory.as_str(),
        )
    })?;
    let helper_path = directory.join("native_lldb_helper.py");
    fs::write(&helper_path, HELPER_SOURCE).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to write helper script: {error}"),
            helper_path.as_str(),
        )
    })?;
    Ok(directory)
}

fn resolve_android_lldb_server(abi: &str) -> AtomResult<Utf8PathBuf> {
    let ndk_home = resolve_ndk_home()?;
    let arch = match abi {
        value if value.starts_with("arm64") || value.starts_with("aarch64") => "aarch64",
        value if value.starts_with("armeabi") || value == "arm" => "arm",
        value if value.starts_with("x86_64") => "x86_64",
        value if value.starts_with("x86") => "i386",
        value if value.starts_with("riscv64") => "riscv64",
        _ => {
            return Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("unsupported Android ABI for native debugging: {abi}"),
            ));
        }
    };
    let path = ndk_home
        .join("toolchains/llvm/prebuilt/darwin-x86_64/lib/clang/18/lib/linux")
        .join(arch)
        .join("lldb-server");
    if path.exists() {
        Ok(path)
    } else {
        Err(AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "could not find Android lldb-server in the configured NDK",
            path.as_str(),
        ))
    }
}

fn resolve_ndk_python() -> AtomResult<Utf8PathBuf> {
    let path =
        resolve_ndk_home()?.join("toolchains/llvm/prebuilt/darwin-x86_64/python3/bin/python3");
    if path.exists() {
        Ok(path)
    } else {
        Err(AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "could not find the Android NDK Python runtime",
            path.as_str(),
        ))
    }
}

fn resolve_ndk_lldb_pythonpath() -> AtomResult<String> {
    let ndk_home = resolve_ndk_home()?;
    let lldb_path =
        ndk_home.join("toolchains/llvm/prebuilt/darwin-x86_64/lib/python3.11/site-packages");
    let extra_path = ndk_home.join("python-packages");
    if !lldb_path.exists() {
        return Err(AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "could not find the Android NDK LLDB Python packages",
            lldb_path.as_str(),
        ));
    }
    Ok(format!("{}:{}", lldb_path, extra_path))
}

fn resolve_ndk_home() -> AtomResult<Utf8PathBuf> {
    if let Ok(path) = std::env::var("ANDROID_NDK_HOME") {
        return Ok(Utf8PathBuf::from(path));
    }
    if let Some(sdk_root) = std::env::var("ANDROID_SDK_ROOT")
        .ok()
        .or_else(|| std::env::var("ANDROID_HOME").ok())
        .map(Utf8PathBuf::from)
    {
        if let Some(ndk) = latest_android_ndk(&sdk_root) {
            return Ok(ndk);
        }
    }
    Err(AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        "could not resolve ANDROID_NDK_HOME for native Android debugging",
    ))
}

fn latest_android_ndk(sdk_root: &Utf8Path) -> Option<Utf8PathBuf> {
    let ndk_root = sdk_root.join("ndk");
    let mut entries = fs::read_dir(&ndk_root).ok()?.flatten().collect::<Vec<_>>();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    entries
        .pop()
        .and_then(|entry| Utf8PathBuf::from_path_buf(entry.path()).ok())
}

fn free_tcp_port() -> AtomResult<u16> {
    TcpListener::bind("127.0.0.1:0")
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to allocate a local TCP port: {error}"),
            )
        })?
        .local_addr()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to read allocated TCP port: {error}"),
            )
        })
        .map(|address| address.port())
}

fn run_command(repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()> {
    let output = Command::new(tool)
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to invoke {tool}: {error}"),
            )
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!(
                "{tool} {} exited with status {}:\n{}",
                args.join(" "),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ),
        ))
    }
}

fn capture_command(repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<String> {
    let output = Command::new(tool)
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to invoke {tool}: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!(
                "{tool} {} exited with status {}:\n{}",
                args.join(" "),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("{tool} returned non-UTF-8 output"),
        )
    })
}

fn breakpoint_key(file: &str, line: u32) -> String {
    format!("{file}:{line}")
}

fn timestamp_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::{breakpoint_key, latest_android_ndk};
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    #[test]
    fn latest_android_ndk_prefers_highest_directory_name() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8");
        std::fs::create_dir_all(root.join("ndk/26.1.1")).expect("mkdir");
        std::fs::create_dir_all(root.join("ndk/27.2.12479018")).expect("mkdir");
        let latest = latest_android_ndk(&root).expect("ndk");
        assert!(latest.ends_with("27.2.12479018"));
    }

    #[test]
    fn breakpoint_keys_are_stable() {
        assert_eq!(breakpoint_key("/tmp/lib.rs", 24), "/tmp/lib.rs:24");
    }
}
