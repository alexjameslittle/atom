use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atom_backends::{
    AppSessionBuildProfile, AppSessionOptions, BackendAppSession, BackendDebugSession,
    BackendDefinition, DebugFrame, DebugSessionRequest, DebugSessionResponse, DebugSessionState,
    DebugThread, DeployBackend, DeployBackendRegistry, DestinationCapability,
    DestinationDescriptor, InteractionRequest, InteractionResult, LaunchMode,
    SessionLaunchBehavior, ToolRunner, UiSnapshot,
};
use atom_deploy::devices::{choose_from_menu, should_prompt_interactively};
use atom_deploy::progress::run_step;
use atom_deploy::{
    ProcessRunner, capture_bazel_cquery_starlark_owned, capture_tool, generated_target,
    parse_bazel_output_paths, run_bazel_owned, run_tool, stream_tool,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};

use crate::android_uiautomator::{
    inspect_ui_with_android_uiautomator, interact_with_android_uiautomator,
};

const BACKEND_ID: &str = "android";
const APP_PID_WAIT_ATTEMPTS: usize = 30;
const APP_PID_WAIT_INTERVAL: Duration = Duration::from_millis(500);
const BOOT_TIMEOUT_ATTEMPTS: usize = 60;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const APP_LAUNCH_READY_TIMEOUT: Duration = Duration::from_secs(15);
const APP_LAUNCH_READY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const DEBUGGER_ATTACH_READY_TIMEOUT: Duration = Duration::from_secs(10);
const DEBUGGER_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const DEBUGGER_STOP_POLL_INTERVAL: Duration = Duration::from_millis(200);
const VIDEO_STOP_TIMEOUT: Duration = Duration::from_secs(5);

struct AndroidDeployBackend;

#[derive(Debug, Clone, PartialEq, Eq)]
struct AndroidDestination {
    serial: String,
    state: String,
    model: Option<String>,
    device_name: Option<String>,
    is_emulator: bool,
    avd_name: Option<String>,
}

#[derive(Clone)]
struct AndroidAppLaunch {
    serial: String,
    application_id: String,
}

struct VideoCapture {
    output_path: Utf8PathBuf,
    child: Child,
    remote_path: String,
    serial: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AndroidBuildArtifacts {
    signed_apk: Utf8PathBuf,
    unsigned_apk: Utf8PathBuf,
    deploy_jar: Utf8PathBuf,
}

struct JdbProcess {
    child: Child,
    stdin: ChildStdin,
    output_rx: Receiver<Vec<u8>>,
}

struct AndroidJvmDebugSession {
    repo_root: Utf8PathBuf,
    serial: String,
    application_id: String,
    local_port: Option<u16>,
    jdb: Option<JdbProcess>,
    state: DebugSessionState,
    selected_thread_id: Option<String>,
}

struct AndroidAppSession<'a> {
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    runner: &'a mut dyn ToolRunner,
    destination_id: String,
    session_options: AppSessionOptions,
    launch: Option<AndroidAppLaunch>,
    video_capture: Option<VideoCapture>,
}

impl AndroidDestination {
    fn destination_id(&self) -> String {
        self.avd_name
            .as_deref()
            .map_or_else(|| self.serial.clone(), |avd| format!("avd:{avd}"))
    }

    fn display_label(&self) -> String {
        if self.state == "avd" {
            let avd = self.avd_name.as_deref().unwrap_or("unknown");
            return format!("AVD: {avd} (not running)");
        }
        let kind = if self.is_emulator {
            "Emulator"
        } else {
            "Device"
        };
        let model = self.model.as_deref().or(self.device_name.as_deref());
        if self.is_emulator
            && let Some(avd) = self.avd_name.as_deref()
        {
            return match model {
                Some(model) => format!("Emulator: {model} [AVD: {avd}; {}]", self.serial),
                None => format!("Emulator: {avd} [{}]", self.serial),
            };
        }
        match model {
            Some(model) => format!("{kind}: {model} [{kind}; {}]", self.serial),
            None => format!("{kind}: {} [{}]", self.serial, self.state),
        }
    }
}

pub fn register(registry: &mut DeployBackendRegistry) -> AtomResult<()> {
    registry.register(Box::new(AndroidDeployBackend))
}

impl BackendDefinition for AndroidDeployBackend {
    fn id(&self) -> &'static str {
        BACKEND_ID
    }

    fn platform(&self) -> &'static str {
        "android"
    }
}

impl DeployBackend for AndroidDeployBackend {
    fn is_enabled(&self, manifest: &NormalizedManifest) -> bool {
        manifest.android.enabled
    }

    fn list_destinations(
        &self,
        repo_root: &Utf8Path,
        runner: &mut dyn ToolRunner,
    ) -> AtomResult<Vec<DestinationDescriptor>> {
        Ok(list_android_destinations(repo_root, runner)?
            .into_iter()
            .map(destination_descriptor_from_android)
            .collect())
    }

    fn deploy(
        &self,
        repo_root: &Utf8Path,
        manifest: &NormalizedManifest,
        requested_destination: Option<&str>,
        launch_mode: LaunchMode,
        runner: &mut dyn ToolRunner,
    ) -> AtomResult<()> {
        deploy_android(
            repo_root,
            manifest,
            requested_destination,
            launch_mode,
            runner,
        )
    }

    fn stop(
        &self,
        repo_root: &Utf8Path,
        manifest: &NormalizedManifest,
        requested_destination: Option<&str>,
        runner: &mut dyn ToolRunner,
    ) -> AtomResult<()> {
        stop_android(repo_root, manifest, requested_destination, runner)
    }

    fn new_app_session<'a>(
        &self,
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        destination_id: &'a str,
        runner: &'a mut dyn ToolRunner,
        options: AppSessionOptions,
    ) -> AtomResult<Box<dyn BackendAppSession + 'a>> {
        Ok(Box::new(AndroidAppSession {
            repo_root,
            manifest,
            runner,
            destination_id: destination_id.to_owned(),
            session_options: options,
            launch: None,
            video_capture: None,
        }))
    }
}

impl AndroidAppSession<'_> {
    fn active_launch(&self) -> AtomResult<AndroidAppLaunch> {
        self.launch.clone().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::InternalBug,
                "app session expected a launch after ensure_launched",
            )
        })
    }
}

impl BackendAppSession for AndroidAppSession<'_> {
    fn video_extension(&self) -> &'static str {
        "mp4"
    }

    fn ensure_launched(&mut self) -> AtomResult<()> {
        if self.launch.is_some() {
            return Ok(());
        }
        if self.session_options.launch_behavior == SessionLaunchBehavior::AttachOrLaunch
            && let Some(launch) = attach_android_app(
                self.repo_root,
                self.manifest,
                &self.destination_id,
                self.runner,
            )?
        {
            self.launch = Some(launch);
            return Ok(());
        }
        let Some(destination) =
            find_android_destination(self.repo_root, self.runner, &self.destination_id)?
        else {
            return Err(AtomError::with_path(
                AtomErrorCode::AutomationUnavailable,
                format!("unknown destination id: {}", self.destination_id),
                &self.destination_id,
            ));
        };
        let launch = launch_android_app(
            self.repo_root,
            self.manifest,
            &destination,
            self.session_options.build_profile,
            self.runner,
        )?;
        wait_for_android_launch_ready(
            self.repo_root,
            &launch.serial,
            &launch.application_id,
            self.runner,
        )?;
        self.launch = Some(launch);
        Ok(())
    }

    fn interact(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult> {
        self.ensure_launched()?;
        let launch = self.active_launch()?;
        interact_with_android_uiautomator(self.repo_root, &launch.serial, self.runner, request)
    }

    fn capture_auto_screenshot(&mut self) -> AtomResult<Utf8PathBuf> {
        let root = self.repo_root.join("cng-output").join("artifacts");
        write_parent_dir(&root)?;
        let path = root.join(format!("inspect-{}.png", timestamp_suffix()));
        self.capture_screenshot(&path)?;
        Ok(path)
    }

    fn capture_screenshot(&mut self, output_path: &Utf8Path) -> AtomResult<()> {
        self.ensure_launched()?;
        let launch = self.active_launch()?;
        capture_screenshot_for_launch(self.repo_root, &launch, output_path, self.runner)
    }

    fn capture_logs(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()> {
        self.ensure_launched()?;
        let launch = self.active_launch()?;
        capture_logs_for_launch(self.repo_root, &launch, output_path, seconds, self.runner)
    }

    fn capture_video(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()> {
        self.ensure_launched()?;
        let launch = self.active_launch()?;
        capture_video_for_launch(self.repo_root, &launch, output_path, seconds, self.runner)
    }

    fn start_video(&mut self, output_path: &Utf8Path) -> AtomResult<()> {
        self.ensure_launched()?;
        let launch = self.active_launch()?;
        self.video_capture = Some(start_video_capture(self.repo_root, &launch, output_path)?);
        Ok(())
    }

    fn stop_video(&mut self) -> AtomResult<Utf8PathBuf> {
        let video = self.video_capture.take().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                "video recording has not been started",
            )
        })?;
        stop_video_capture(self.repo_root, video, self.runner)
    }

    fn shutdown_video(&mut self) -> AtomResult<()> {
        if self.video_capture.is_some() {
            let _ = self.stop_video()?;
        }
        Ok(())
    }

    fn debug_session(&mut self) -> AtomResult<Option<Box<dyn BackendDebugSession>>> {
        self.ensure_launched()?;
        let launch = self.active_launch()?;
        Ok(Some(Box::new(AndroidJvmDebugSession::new(
            self.repo_root.to_owned(),
            launch.serial,
            launch.application_id,
        ))))
    }
}

impl JdbProcess {
    fn attach(repo_root: &Utf8Path, local_port: u16) -> AtomResult<Self> {
        let mut child = Command::new("jdb")
            .args(["-attach", &format!("127.0.0.1:{local_port}")])
            .current_dir(repo_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to start jdb: {error}"),
                )
            })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "failed to open jdb stdin pipe",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "failed to open jdb stdout pipe",
            )
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "failed to open jdb stderr pipe",
            )
        })?;
        let (tx, rx) = mpsc::channel();
        spawn_reader_thread(stdout, tx.clone());
        spawn_reader_thread(stderr, tx);
        let mut process = Self {
            child,
            stdin,
            output_rx: rx,
        };
        let _ = process.read_until_prompt(None, DEBUGGER_ATTACH_READY_TIMEOUT)?;
        Ok(process)
    }

    fn send_command(&mut self, command: &str) -> AtomResult<String> {
        self.stdin
            .write_all(command.as_bytes())
            .and_then(|()| self.stdin.write_all(b"\n"))
            .and_then(|()| self.stdin.flush())
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to write jdb command `{command}`: {error}"),
                )
            })?;
        self.read_until_prompt(Some(command), DEBUGGER_COMMAND_TIMEOUT)
    }

    fn read_until_prompt(
        &mut self,
        echoed_command: Option<&str>,
        timeout: Duration,
    ) -> AtomResult<String> {
        let deadline = Instant::now() + timeout;
        let mut buffer = Vec::new();
        loop {
            if let Some(prompt_len) = prompt_suffix_len(&buffer) {
                buffer.truncate(buffer.len().saturating_sub(prompt_len));
                let output = String::from_utf8(buffer).map_err(|error| {
                    AtomError::new(
                        AtomErrorCode::ExternalToolFailed,
                        format!("jdb returned non-UTF-8 output: {error}"),
                    )
                })?;
                return Ok(strip_echoed_command(&output, echoed_command));
            }

            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!(
                        "timed out waiting for jdb output after `{}`",
                        echoed_command.unwrap_or("attach")
                    ),
                ));
            };
            match self
                .output_rx
                .recv_timeout(remaining.min(Duration::from_millis(100)))
            {
                Ok(chunk) => buffer.extend_from_slice(&chunk),
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(status) = self.child.try_wait().map_err(|error| {
                        AtomError::new(
                            AtomErrorCode::ExternalToolFailed,
                            format!("failed to poll jdb process: {error}"),
                        )
                    })? {
                        let output = String::from_utf8_lossy(&buffer);
                        return Err(AtomError::new(
                            AtomErrorCode::ExternalToolFailed,
                            format!("jdb exited with {status} before reaching a prompt: {output}"),
                        ));
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let output = String::from_utf8_lossy(&buffer);
                    return Err(AtomError::new(
                        AtomErrorCode::ExternalToolFailed,
                        format!("jdb output closed unexpectedly: {output}"),
                    ));
                }
            }
        }
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl AndroidJvmDebugSession {
    fn new(repo_root: Utf8PathBuf, serial: String, application_id: String) -> Self {
        Self {
            repo_root,
            serial,
            application_id,
            local_port: None,
            jdb: None,
            state: DebugSessionState::Unknown,
            selected_thread_id: None,
        }
    }

    fn ensure_attached(&mut self) -> AtomResult<()> {
        if self.jdb.is_some() {
            return Ok(());
        }
        let mut runner = ProcessRunner;
        let pid = wait_for_app_pid(
            &mut runner,
            &self.repo_root,
            &self.serial,
            &self.application_id,
        )?;
        let local_port = reserve_tcp_port()?;
        forward_jdwp_port(&mut runner, &self.repo_root, &self.serial, local_port, &pid)?;

        match JdbProcess::attach(&self.repo_root, local_port) {
            Ok(process) => {
                self.local_port = Some(local_port);
                self.jdb = Some(process);
                Ok(())
            }
            Err(error) => {
                cleanup_jdwp_forward(&self.repo_root, &self.serial, local_port);
                Err(error)
            }
        }
    }

    fn run_jdb_command(&mut self, command: &str) -> AtomResult<String> {
        self.ensure_attached()?;
        self.jdb
            .as_mut()
            .expect("jdb should exist after attach")
            .send_command(command)
    }

    fn inspect_state(&mut self) -> AtomResult<DebugSessionState> {
        let output = self.run_jdb_command("where all")?;
        let state = debug_state_from_where_all_output(&output);
        self.state = state;
        Ok(state)
    }

    fn list_threads(&mut self) -> AtomResult<Vec<DebugThread>> {
        let output = self.run_jdb_command("threads")?;
        Ok(parse_jdb_threads(
            &output,
            self.selected_thread_id.as_deref(),
        ))
    }

    fn resolve_thread_id(&mut self, requested: Option<String>) -> AtomResult<String> {
        if let Some(thread_id) = requested {
            return Ok(thread_id);
        }
        if let Some(thread_id) = self.selected_thread_id.clone() {
            return Ok(thread_id);
        }
        let threads = self.list_threads()?;
        threads
            .iter()
            .find(|thread| thread.name.as_deref() == Some("main"))
            .or_else(|| threads.first())
            .map(|thread| thread.id.clone())
            .ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::AutomationUnavailable,
                    "jdb did not report any runnable threads",
                )
            })
    }
}

impl BackendDebugSession for AndroidJvmDebugSession {
    fn execute(&mut self, request: DebugSessionRequest) -> AtomResult<DebugSessionResponse> {
        match request {
            DebugSessionRequest::Attach => Ok(DebugSessionResponse::Attached {
                state: self.inspect_state()?,
            }),
            DebugSessionRequest::InspectState => Ok(DebugSessionResponse::State {
                state: self.inspect_state()?,
            }),
            DebugSessionRequest::WaitForStop { timeout_ms } => {
                self.ensure_attached()?;
                let deadline = Instant::now() + Duration::from_millis(timeout_ms);
                loop {
                    let state = self.inspect_state()?;
                    if state == DebugSessionState::Stopped {
                        return Ok(DebugSessionResponse::Stopped { state });
                    }
                    if Instant::now() >= deadline {
                        return Err(AtomError::new(
                            AtomErrorCode::AutomationUnavailable,
                            format!(
                                "debugger target did not stop within {timeout_ms}ms after attach"
                            ),
                        ));
                    }
                    thread::sleep(DEBUGGER_STOP_POLL_INTERVAL);
                }
            }
            DebugSessionRequest::Pause => {
                let _ = self.run_jdb_command("suspend")?;
                self.state = DebugSessionState::Stopped;
                Ok(DebugSessionResponse::Paused)
            }
            DebugSessionRequest::Resume => {
                let _ = self.run_jdb_command("resume")?;
                self.state = DebugSessionState::Running;
                Ok(DebugSessionResponse::Resumed)
            }
            DebugSessionRequest::ListThreads => Ok(DebugSessionResponse::Threads {
                threads: self.list_threads()?,
            }),
            DebugSessionRequest::ListFrames { thread_id } => {
                let thread_id = self.resolve_thread_id(thread_id)?;
                let output = self.run_jdb_command(&format!("where {thread_id}"))?;
                if output.contains("Current thread isn't suspended.") {
                    self.state = DebugSessionState::Running;
                    return Err(AtomError::new(
                        AtomErrorCode::AutomationUnavailable,
                        "debugger target is running; pause or wait_for_stop before requesting stack frames",
                    ));
                }
                self.state = DebugSessionState::Stopped;
                self.selected_thread_id = Some(thread_id.clone());
                Ok(DebugSessionResponse::Frames {
                    thread_id,
                    frames: parse_jdb_frames(&output),
                })
            }
        }
    }
}

impl Drop for AndroidJvmDebugSession {
    fn drop(&mut self) {
        if let Some(jdb) = self.jdb.as_mut() {
            jdb.kill();
        }
        if let Some(local_port) = self.local_port {
            cleanup_jdwp_forward(&self.repo_root, &self.serial, local_port);
        }
    }
}

fn destination_descriptor_from_android(destination: AndroidDestination) -> DestinationDescriptor {
    let display_name = destination.display_label();
    let id = destination.destination_id();
    let kind = if destination.state == "avd" {
        "avd"
    } else if destination.is_emulator {
        "emulator"
    } else {
        "device"
    };
    let capabilities = vec![
        DestinationCapability::Launch,
        DestinationCapability::Logs,
        DestinationCapability::Screenshot,
        DestinationCapability::Video,
        DestinationCapability::InspectUi,
        DestinationCapability::Interact,
        DestinationCapability::Evaluate,
        DestinationCapability::DebugSession,
    ];

    DestinationDescriptor {
        platform: "android".to_owned(),
        backend_id: BACKEND_ID.to_owned(),
        id,
        kind: kind.to_owned(),
        display_name,
        available: destination.state == "device" || destination.state == "avd",
        debug_state: destination.state,
        capabilities,
    }
}

fn deploy_android(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_destination: Option<&str>,
    launch_mode: LaunchMode,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    let destination = resolve_android_device(repo_root, runner, requested_destination)?;
    let artifacts = run_step(
        "Building Android app...",
        "Built Android app",
        "Android build failed",
        || {
            build_android_artifacts(
                repo_root,
                manifest,
                AppSessionBuildProfile::Standard,
                runner,
            )
        },
    )?;
    let application_id = manifest.android.application_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated Android manifest is missing application_id",
        )
    })?;

    let serial = run_step(
        "Preparing emulator...",
        "Emulator ready",
        "Emulator preparation failed",
        || prepare_android_emulator(repo_root, runner, &destination),
    )?;

    let component = format!("{application_id}/.MainActivity");
    run_step(
        "Installing app...",
        "App installed",
        "Installation failed",
        || {
            run_tool(
                runner,
                repo_root,
                "adb",
                &[
                    "-s",
                    &serial,
                    "install",
                    "-r",
                    artifacts.signed_apk.as_str(),
                ],
            )
        },
    )?;
    match launch_mode {
        LaunchMode::Attached => {
            run_tool(runner, repo_root, "adb", &["-s", &serial, "logcat", "-c"])?;
            run_step("Launching app...", "App launched", "Launch failed", || {
                run_tool(
                    runner,
                    repo_root,
                    "adb",
                    &[
                        "-s", &serial, "shell", "am", "start", "-W", "-n", &component,
                    ],
                )
            })?;
            let pid = wait_for_app_pid(runner, repo_root, &serial, application_id)?;
            eprintln!("→ Streaming logs for {application_id} (pid {pid})... (Ctrl+C to stop)");
            stream_tool(
                runner,
                repo_root,
                "adb",
                &[
                    "-s",
                    &serial,
                    "logcat",
                    "--pid",
                    &pid,
                    "-s",
                    "AtomRuntime:*",
                ],
            )
        }
        LaunchMode::Detached => {
            run_step("Launching app...", "App launched", "Launch failed", || {
                run_tool(
                    runner,
                    repo_root,
                    "adb",
                    &[
                        "-s", &serial, "shell", "am", "start", "-W", "-n", &component,
                    ],
                )
            })
        }
    }
}

fn stop_android(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_destination: Option<&str>,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    let destination = resolve_android_device(repo_root, runner, requested_destination)?;
    let application_id = manifest.android.application_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated Android manifest is missing application_id",
        )
    })?;

    if destination.state == "avd" {
        return Ok(());
    }

    run_step("Stopping app...", "App stopped", "Stop failed", || {
        run_tool(
            runner,
            repo_root,
            "adb",
            &[
                "-s",
                &destination.serial,
                "shell",
                "am",
                "force-stop",
                application_id,
            ],
        )
    })
}

fn resolve_android_device(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
    requested_destination: Option<&str>,
) -> AtomResult<AndroidDestination> {
    if let Some(requested) = requested_destination {
        if requested.strip_prefix("avd:").is_some() {
            if let Some(destination) = find_android_destination(repo_root, runner, requested)? {
                return Ok(destination);
            }
            return Err(AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                format!("unknown Android destination id: {requested}"),
                requested,
            ));
        }
        return Ok(AndroidDestination {
            serial: requested.to_owned(),
            state: "device".to_owned(),
            model: None,
            device_name: None,
            is_emulator: requested.starts_with("emulator-"),
            avd_name: None,
        });
    }

    let mut destinations = list_android_devices(repo_root, runner)?;
    let running_avds = destinations
        .iter()
        .filter_map(|destination| destination.avd_name.as_deref())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if let Ok(avds) = list_avds(repo_root, runner) {
        for avd in avds {
            if !running_avds.contains(&avd) {
                destinations.push(AndroidDestination {
                    serial: String::new(),
                    state: "avd".to_owned(),
                    model: None,
                    device_name: None,
                    is_emulator: true,
                    avd_name: Some(avd),
                });
            }
        }
    }

    if destinations.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            "no Android devices or AVDs found — run scripts/setup-android-sdk.sh first",
        ));
    }

    if should_prompt_interactively() {
        return choose_from_menu(
            "Select Android destination",
            &destinations,
            AndroidDestination::display_label,
        );
    }

    destinations
        .iter()
        .find(|destination| destination.state == "device")
        .or_else(|| {
            destinations
                .iter()
                .find(|destination| destination.state == "avd")
        })
        .cloned()
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "no Android devices or AVDs available",
            )
        })
}

fn list_android_destinations(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
) -> AtomResult<Vec<AndroidDestination>> {
    let mut destinations = list_android_devices(repo_root, runner)?;
    let running_avds = destinations
        .iter()
        .filter_map(|destination| destination.avd_name.as_deref())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if let Ok(avds) = list_avds(repo_root, runner) {
        for avd in avds {
            if !running_avds.contains(&avd) {
                destinations.push(AndroidDestination {
                    serial: format!("avd:{avd}"),
                    state: "avd".to_owned(),
                    model: None,
                    device_name: None,
                    is_emulator: true,
                    avd_name: Some(avd),
                });
            }
        }
    }
    Ok(destinations)
}

fn list_android_devices(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
) -> AtomResult<Vec<AndroidDestination>> {
    let output = capture_tool(runner, repo_root, "adb", &["devices", "-l"])?;
    let mut destinations = parse_adb_devices(&output);
    for destination in &mut destinations {
        if destination.is_emulator {
            destination.avd_name = emulator_avd_name(repo_root, runner, &destination.serial)?;
        }
    }
    Ok(destinations)
}

fn parse_adb_devices(output: &str) -> Vec<AndroidDestination> {
    let mut destinations = Vec::new();
    for line in output.lines().skip(1) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(serial) = parts.next() else {
            continue;
        };
        let Some(state) = parts.next() else {
            continue;
        };
        if state != "device" {
            continue;
        }
        let mut model = None;
        let mut device_name = None;
        for part in parts {
            if let Some(value) = part.strip_prefix("model:") {
                model = Some(value.replace('_', " "));
            } else if let Some(value) = part.strip_prefix("device:") {
                device_name = Some(value.replace('_', " "));
            }
        }
        let is_emulator = serial.starts_with("emulator-");
        destinations.push(AndroidDestination {
            serial: serial.to_owned(),
            state: "device".to_owned(),
            model,
            device_name,
            is_emulator,
            avd_name: None,
        });
    }
    destinations
}

fn list_avds(repo_root: &Utf8Path, runner: &mut dyn ToolRunner) -> AtomResult<Vec<String>> {
    Ok(
        capture_tool(runner, repo_root, "emulator", &["-list-avds"])?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    )
}

fn emulator_avd_name(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
    serial: &str,
) -> AtomResult<Option<String>> {
    let output = capture_tool(
        runner,
        repo_root,
        "adb",
        &["-s", serial, "shell", "getprop", "ro.boot.qemu.avd_name"],
    )?;
    let avd_name = output.trim();
    if avd_name.is_empty() {
        Ok(None)
    } else {
        Ok(Some(avd_name.to_owned()))
    }
}

fn find_android_destination(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
    requested: &str,
) -> AtomResult<Option<AndroidDestination>> {
    let running_devices = list_android_devices(repo_root, runner)?;
    if let Some(avd_name) = requested.strip_prefix("avd:") {
        if let Some(destination) = running_devices
            .iter()
            .find(|destination| destination.avd_name.as_deref() == Some(avd_name))
            .cloned()
        {
            return Ok(Some(destination));
        }
        let avds = list_avds(repo_root, runner)?;
        return Ok(avds
            .into_iter()
            .find(|candidate| candidate == avd_name)
            .map(|avd_name| AndroidDestination {
                serial: format!("avd:{avd_name}"),
                state: "avd".to_owned(),
                model: None,
                device_name: None,
                is_emulator: true,
                avd_name: Some(avd_name),
            }));
    }
    Ok(running_devices
        .into_iter()
        .find(|destination| destination.serial == requested))
}

fn prepare_android_emulator(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
    destination: &AndroidDestination,
) -> AtomResult<String> {
    if destination.state == "device" {
        if destination.is_emulator {
            wait_for_android_boot(repo_root, runner, &destination.serial)?;
        }
        return Ok(destination.serial.clone());
    }

    let avd = destination.avd_name.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "Android destination has no AVD name and is not running",
        )
    })?;

    if let Some(serial) = running_emulator_serial_for_avd(repo_root, runner, avd)? {
        wait_for_android_boot(repo_root, runner, &serial)?;
        return Ok(serial);
    }

    Command::new("emulator")
        .args(["-avd", avd, "-no-snapshot-load"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to launch Android emulator: {error}"),
            )
        })?;

    let serial = wait_for_android_emulator_serial(repo_root, runner, avd)?;
    wait_for_android_boot(repo_root, runner, &serial)?;
    Ok(serial)
}

fn running_emulator_serial_for_avd(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
    avd_name: &str,
) -> AtomResult<Option<String>> {
    for destination in list_android_devices(repo_root, runner)? {
        if !destination.is_emulator {
            continue;
        }
        if emulator_avd_name(repo_root, runner, &destination.serial)?.as_deref() == Some(avd_name) {
            return Ok(Some(destination.serial));
        }
    }
    Ok(None)
}

fn wait_for_android_emulator_serial(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
    avd_name: &str,
) -> AtomResult<String> {
    for _ in 0..BOOT_TIMEOUT_ATTEMPTS {
        if let Some(serial) = running_emulator_serial_for_avd(repo_root, runner, avd_name)? {
            return Ok(serial);
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        format!("timed out waiting for Android emulator {avd_name} to appear"),
    ))
}

fn wait_for_android_boot(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
    serial: &str,
) -> AtomResult<()> {
    for _ in 0..BOOT_TIMEOUT_ATTEMPTS {
        let output = capture_tool(
            runner,
            repo_root,
            "adb",
            &["-s", serial, "shell", "getprop", "sys.boot_completed"],
        )?;
        if output.trim() == "1" {
            return Ok(());
        }
        thread::sleep(POLL_INTERVAL);
    }
    Err(AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        format!("timed out waiting for Android destination {serial} to boot"),
    ))
}

fn wait_for_app_pid(
    runner: &mut dyn ToolRunner,
    repo_root: &Utf8Path,
    serial: &str,
    application_id: &str,
) -> AtomResult<String> {
    for _ in 0..APP_PID_WAIT_ATTEMPTS {
        if let Ok(output) = capture_tool(
            runner,
            repo_root,
            "adb",
            &["-s", serial, "shell", "pidof", application_id],
        ) {
            let pid = output.trim();
            if !pid.is_empty() {
                return Ok(pid.to_owned());
            }
        }
        thread::sleep(APP_PID_WAIT_INTERVAL);
    }
    Err(AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        format!(
            "could not find running process for {application_id} — the app may have crashed on launch"
        ),
    ))
}

fn forward_jdwp_port(
    runner: &mut dyn ToolRunner,
    repo_root: &Utf8Path,
    serial: &str,
    local_port: u16,
    pid: &str,
) -> AtomResult<()> {
    let local_binding = format!("tcp:{local_port}");
    let remote_binding = format!("jdwp:{pid}");
    let mut last_error = None;
    for _ in 0..APP_PID_WAIT_ATTEMPTS {
        match run_tool(
            runner,
            repo_root,
            "adb",
            &["-s", serial, "forward", &local_binding, &remote_binding],
        ) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error.message);
            }
        }
        thread::sleep(APP_PID_WAIT_INTERVAL);
    }
    Err(AtomError::new(
        AtomErrorCode::AutomationUnavailable,
        format!(
            "process {pid} never became attachable as a JDWP target on {serial}{}",
            last_error
                .as_deref()
                .map_or(String::new(), |detail| format!(": {detail}"))
        ),
    ))
}

fn reserve_tcp_port() -> AtomResult<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|error| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to reserve a local TCP port for jdb attach: {error}"),
        )
    })?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to read reserved local TCP port: {error}"),
            )
        })
}

fn cleanup_jdwp_forward(repo_root: &Utf8Path, serial: &str, local_port: u16) {
    let mut runner = ProcessRunner;
    let local_binding = format!("tcp:{local_port}");
    let _ = run_tool(
        &mut runner,
        repo_root,
        "adb",
        &["-s", serial, "forward", "--remove", &local_binding],
    );
}

fn spawn_reader_thread<R>(mut source: R, tx: Sender<Vec<u8>>)
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if tx.send(buffer[..read].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

fn prompt_suffix_len(buffer: &[u8]) -> Option<usize> {
    if buffer.ends_with(b"> ") {
        return Some(2);
    }
    let line_start = buffer
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |idx| idx + 1);
    let tail = &buffer[line_start..];
    if tail.len() < 4 || tail[0].is_ascii_whitespace() || !tail.ends_with(b"] ") {
        return None;
    }
    let open_bracket = tail.iter().rposition(|byte| *byte == b'[')?;
    if open_bracket == 0 {
        return None;
    }
    let frame_index = &tail[open_bracket + 1..tail.len() - 2];
    if frame_index.is_empty() || !frame_index.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(tail.len())
}

fn strip_echoed_command(output: &str, echoed_command: Option<&str>) -> String {
    let normalized = output.replace('\r', "");
    let Some(command) = echoed_command else {
        return normalized;
    };
    if let Some(stripped) = normalized
        .strip_prefix(command)
        .and_then(|rest| rest.strip_prefix('\n'))
    {
        stripped.to_owned()
    } else {
        normalized
    }
}

fn debug_state_from_where_all_output(output: &str) -> DebugSessionState {
    let normalized = output.replace('\r', "");
    if normalized
        .lines()
        .any(|line| line.trim_start().starts_with('['))
    {
        DebugSessionState::Stopped
    } else if normalized.contains("Current thread isn't suspended.") {
        DebugSessionState::Running
    } else {
        DebugSessionState::Unknown
    }
}

fn parse_jdb_threads(output: &str, selected_thread_id: Option<&str>) -> Vec<DebugThread> {
    output
        .lines()
        .filter_map(|line| parse_jdb_thread_line(line, selected_thread_id))
        .collect()
}

fn parse_jdb_thread_line(line: &str, selected_thread_id: Option<&str>) -> Option<DebugThread> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('(') {
        return None;
    }
    let close_paren = trimmed.find(')')?;
    let remainder = trimmed[close_paren + 1..].trim_start();
    let mut parts = remainder.splitn(2, char::is_whitespace);
    let id = parts.next()?.trim();
    let name_and_state = parts.next()?.trim();
    let name = trim_known_thread_state(name_and_state)
        .unwrap_or(name_and_state)
        .trim();
    Some(DebugThread {
        id: id.to_owned(),
        name: (!name.is_empty()).then(|| name.to_owned()),
        selected: selected_thread_id == Some(id),
    })
}

fn trim_known_thread_state(value: &str) -> Option<&str> {
    [
        "waiting in native",
        "cond. waiting",
        "monitor wait",
        "not started",
        "sleeping",
        "running",
        "waiting",
    ]
    .into_iter()
    .find_map(|suffix| value.strip_suffix(suffix).map(str::trim_end))
}

fn parse_jdb_frames(output: &str) -> Vec<DebugFrame> {
    output.lines().filter_map(parse_jdb_frame_line).collect()
}

fn parse_jdb_frame_line(line: &str) -> Option<DebugFrame> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('[') {
        return None;
    }
    let close_bracket = trimmed.find(']')?;
    let raw_index = trimmed[1..close_bracket].parse::<usize>().ok()?;
    let remainder = trimmed[close_bracket + 1..].trim();
    let open_paren = remainder.rfind(" (")?;
    let function = remainder[..open_paren].to_owned();
    let location = remainder[open_paren + 2..].strip_suffix(')')?;
    let (source_path, line) = parse_jdb_frame_location(location);
    Some(DebugFrame {
        index: raw_index.saturating_sub(1),
        function,
        source_path,
        line,
        column: None,
    })
}

fn parse_jdb_frame_location(location: &str) -> (Option<String>, Option<u32>) {
    if matches!(location, "native method" | "null") {
        return (None, None);
    }
    let Some((path, line)) = location.rsplit_once(':') else {
        return (Some(location.to_owned()), None);
    };
    let normalized_line = line.replace(',', "");
    let line_number = normalized_line.parse::<u32>().ok();
    (Some(path.to_owned()), line_number)
}

fn android_bazel_args(target: &str, build_profile: AppSessionBuildProfile) -> Vec<String> {
    let mut args = vec![
        "build".to_owned(),
        target.to_owned(),
        "--android_platforms=//platforms:arm64-v8a".to_owned(),
    ];
    if build_profile == AppSessionBuildProfile::Debugger {
        args.push("--compilation_mode=dbg".to_owned());
    }
    args
}

fn build_android_artifacts(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    build_profile: AppSessionBuildProfile,
    runner: &mut dyn ToolRunner,
) -> AtomResult<AndroidBuildArtifacts> {
    let target = generated_target(manifest, BACKEND_ID);
    let build_args = android_bazel_args(&target, build_profile);
    run_bazel_owned(runner, repo_root, &build_args)?;
    resolve_android_build_artifacts(repo_root, &build_args, runner)
}

fn resolve_android_build_artifacts(
    repo_root: &Utf8Path,
    build_args: &[String],
    runner: &mut dyn ToolRunner,
) -> AtomResult<AndroidBuildArtifacts> {
    let output = capture_bazel_cquery_starlark_owned(
        runner,
        repo_root,
        build_args,
        r#"providers(target)["@@rules_android+//providers:providers.bzl%ApkInfo"].signed_apk.path + "\n" + providers(target)["@@rules_android+//providers:providers.bzl%ApkInfo"].deploy_jar.path + "\n" + providers(target)["@@rules_android+//providers:providers.bzl%ApkInfo"].unsigned_apk.path"#,
    )?;
    let paths = parse_bazel_output_paths(repo_root, &output, "Android debugger artifact")?;
    if paths.len() != 3 {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            "bazelisk cquery did not return signed APK, deploy JAR, and unsigned APK paths",
        ));
    }
    Ok(AndroidBuildArtifacts {
        signed_apk: paths[0].clone(),
        deploy_jar: paths[1].clone(),
        unsigned_apk: paths[2].clone(),
    })
}

fn launch_android_app(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination: &AndroidDestination,
    build_profile: AppSessionBuildProfile,
    runner: &mut dyn ToolRunner,
) -> AtomResult<AndroidAppLaunch> {
    let serial = prepare_android_emulator(repo_root, runner, destination)?;
    let artifacts = build_android_artifacts(repo_root, manifest, build_profile, runner)?;
    let application_id = manifest.android.application_id.clone().ok_or_else(|| {
        AtomError::new(AtomErrorCode::InternalBug, "missing Android application id")
    })?;
    run_tool(
        runner,
        repo_root,
        "adb",
        &[
            "-s",
            &serial,
            "install",
            "-r",
            artifacts.signed_apk.as_str(),
        ],
    )?;
    let component = format!("{application_id}/.MainActivity");
    run_tool(
        runner,
        repo_root,
        "adb",
        &[
            "-s", &serial, "shell", "am", "start", "-W", "-n", &component,
        ],
    )?;
    wait_for_app_pid(runner, repo_root, &serial, &application_id)?;
    Ok(AndroidAppLaunch {
        serial,
        application_id,
    })
}

fn attach_android_app(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination_id: &str,
    runner: &mut dyn ToolRunner,
) -> AtomResult<Option<AndroidAppLaunch>> {
    let Some(application_id) = manifest.android.application_id.as_deref() else {
        return Ok(None);
    };
    let Some(destination) = find_android_destination(repo_root, runner, destination_id)? else {
        return Ok(None);
    };
    if destination.state != "device" {
        return Ok(None);
    }
    let snapshot = inspect_ui_with_android_uiautomator(repo_root, &destination.serial, runner)?;
    if !snapshot_is_launch_ready(&snapshot.snapshot)
        || !snapshot
            .packages
            .iter()
            .any(|package| package == application_id)
    {
        return Ok(None);
    }
    Ok(Some(AndroidAppLaunch {
        serial: destination.serial,
        application_id: application_id.to_owned(),
    }))
}

fn wait_for_android_launch_ready(
    repo_root: &Utf8Path,
    serial: &str,
    application_id: &str,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    let deadline = Instant::now() + APP_LAUNCH_READY_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(snapshot) = inspect_ui_with_android_uiautomator(repo_root, serial, runner)
            && snapshot_is_launch_ready(&snapshot.snapshot)
            && snapshot
                .packages
                .iter()
                .any(|package| package == application_id)
        {
            return Ok(());
        }
        thread::sleep(APP_LAUNCH_READY_POLL_INTERVAL);
    }
    Err(AtomError::new(
        AtomErrorCode::AutomationUnavailable,
        "app did not become responsive after launch",
    ))
}

fn snapshot_is_launch_ready(snapshot: &UiSnapshot) -> bool {
    snapshot.nodes.iter().any(|node| {
        !node.role.eq_ignore_ascii_case("application")
            && (node.bounds.width > 1.0 || node.bounds.height > 1.0)
            && (!node.label.is_empty() || !node.text.is_empty())
    })
}

pub(crate) fn resolve_interaction_point(
    snapshot: &UiSnapshot,
    target_id: Option<&str>,
    x: Option<f64>,
    y: Option<f64>,
) -> AtomResult<(f64, f64)> {
    if let Some(target_id) = target_id {
        let node = snapshot
            .nodes
            .iter()
            .find(|node| node.id == target_id)
            .ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::AutomationTargetNotFound,
                    format!("target {target_id} was not found in the UI snapshot"),
                )
            })?;
        return Ok((
            node.bounds.x + (node.bounds.width / 2.0),
            node.bounds.y + (node.bounds.height / 2.0),
        ));
    }
    match (x, y) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(AtomError::new(
            AtomErrorCode::AutomationTargetNotFound,
            "interaction requires either a semantic target id or explicit x/y coordinates",
        )),
    }
}

fn capture_screenshot_for_launch(
    repo_root: &Utf8Path,
    launch: &AndroidAppLaunch,
    output_path: &Utf8Path,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    let remote = format!("/sdcard/atom-screenshot-{}.png", timestamp_suffix());
    run_tool(
        runner,
        repo_root,
        "adb",
        &["-s", &launch.serial, "shell", "screencap", "-p", &remote],
    )?;
    run_tool(
        runner,
        repo_root,
        "adb",
        &["-s", &launch.serial, "pull", &remote, output_path.as_str()],
    )?;
    run_tool(
        runner,
        repo_root,
        "adb",
        &["-s", &launch.serial, "shell", "rm", "-f", &remote],
    )?;
    Ok(())
}

fn capture_logs_for_launch(
    repo_root: &Utf8Path,
    launch: &AndroidAppLaunch,
    output_path: &Utf8Path,
    _seconds: u64,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    let pid = wait_for_app_pid(runner, repo_root, &launch.serial, &launch.application_id)?;
    let contents = capture_tool(
        runner,
        repo_root,
        "adb",
        &["-s", &launch.serial, "logcat", "--pid", &pid, "-d"],
    )
    .map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::AutomationLogCaptureFailed,
            format!("failed to collect logs: {}", error.message),
            output_path.as_str(),
        )
    })?;
    fs::write(output_path, contents).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::AutomationLogCaptureFailed,
            format!("failed to write log output: {error}"),
            output_path.as_str(),
        )
    })
}

fn capture_video_for_launch(
    repo_root: &Utf8Path,
    launch: &AndroidAppLaunch,
    output_path: &Utf8Path,
    seconds: u64,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    let remote = format!("/sdcard/atom-video-{}.mp4", timestamp_suffix());
    run_tool(
        runner,
        repo_root,
        "adb",
        &[
            "-s",
            &launch.serial,
            "shell",
            "screenrecord",
            "--time-limit",
            &seconds.to_string(),
            &remote,
        ],
    )?;
    run_tool(
        runner,
        repo_root,
        "adb",
        &["-s", &launch.serial, "pull", &remote, output_path.as_str()],
    )?;
    run_tool(
        runner,
        repo_root,
        "adb",
        &["-s", &launch.serial, "shell", "rm", "-f", &remote],
    )?;
    Ok(())
}

fn start_video_capture(
    repo_root: &Utf8Path,
    launch: &AndroidAppLaunch,
    output_path: &Utf8Path,
) -> AtomResult<VideoCapture> {
    write_parent_dir(output_path)?;
    let remote_path = format!("/sdcard/atom-video-{}.mp4", timestamp_suffix());
    let child = Command::new("adb")
        .args(["-s", &launch.serial, "shell", "screenrecord", &remote_path])
        .current_dir(repo_root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to start Android video capture: {error}"),
            )
        })?;
    Ok(VideoCapture {
        output_path: output_path.to_owned(),
        child,
        remote_path,
        serial: launch.serial.clone(),
    })
}

fn stop_video_capture(
    repo_root: &Utf8Path,
    video: VideoCapture,
    runner: &mut dyn ToolRunner,
) -> AtomResult<Utf8PathBuf> {
    let mut child = video.child;
    stop_android_screenrecord(repo_root, &video.serial, &mut child, runner)?;
    run_tool(
        runner,
        repo_root,
        "adb",
        &[
            "-s",
            &video.serial,
            "pull",
            &video.remote_path,
            video.output_path.as_str(),
        ],
    )?;
    run_tool(
        runner,
        repo_root,
        "adb",
        &["-s", &video.serial, "shell", "rm", "-f", &video.remote_path],
    )?;
    ensure_video_artifact(&video.output_path)?;
    Ok(video.output_path)
}

fn stop_android_screenrecord(
    repo_root: &Utf8Path,
    serial: &str,
    child: &mut Child,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    if wait_for_child_exit(child, Duration::from_millis(100))? {
        return Ok(());
    }

    if let Ok(pids) = capture_tool(
        runner,
        repo_root,
        "adb",
        &["-s", serial, "shell", "pidof", "screenrecord"],
    ) {
        for pid in pids.split_whitespace() {
            let _ = run_tool(
                runner,
                repo_root,
                "adb",
                &["-s", serial, "shell", "kill", "-2", pid],
            );
        }
    }

    if wait_for_child_exit(child, VIDEO_STOP_TIMEOUT)? {
        return Ok(());
    }

    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> AtomResult<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to poll recorder process: {error}"),
            )
        })? {
            Some(_) => return Ok(true),
            None if Instant::now() >= deadline => return Ok(false),
            None => thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn ensure_video_artifact(path: &Utf8Path) -> AtomResult<()> {
    let metadata = fs::metadata(path).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("video recording did not produce an output file: {error}"),
            path.as_str(),
        )
    })?;
    if metadata.len() == 0 {
        return Err(AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "video recording produced an empty output file",
            path.as_str(),
        ));
    }
    Ok(())
}

fn write_parent_dir(path: &Utf8Path) -> AtomResult<()> {
    let directory = if path.extension().is_some() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    fs::create_dir_all(directory).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to create output directory: {error}"),
            directory.as_str(),
        )
    })
}

pub(crate) fn timestamp_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use atom_backends::{AppSessionBuildProfile, DebugSessionState, DestinationCapability};
    use atom_ffi::{AtomError, AtomErrorCode};
    use camino::{Utf8Path, Utf8PathBuf};

    use super::{
        AndroidDestination, BACKEND_ID, android_bazel_args, debug_state_from_where_all_output,
        destination_descriptor_from_android, find_android_destination, forward_jdwp_port,
        list_android_destinations, list_android_devices, parse_adb_devices, parse_jdb_frames,
        parse_jdb_threads, prompt_suffix_len,
    };

    #[derive(Default)]
    struct FakeToolRunner {
        captures: VecDeque<(&'static str, Vec<String>, Result<String, AtomError>)>,
        runs: VecDeque<(&'static str, Vec<String>, Result<(), AtomError>)>,
    }

    impl FakeToolRunner {
        fn push_capture(
            &mut self,
            tool: &'static str,
            args: &[&str],
            output: Result<&str, AtomError>,
        ) {
            self.captures.push_back((
                tool,
                args.iter().map(|arg| (*arg).to_owned()).collect(),
                output.map(str::to_owned),
            ));
        }

        fn push_run(&mut self, tool: &'static str, args: &[&str], result: Result<(), AtomError>) {
            self.runs.push_back((
                tool,
                args.iter().map(|arg| (*arg).to_owned()).collect(),
                result,
            ));
        }
    }

    impl atom_backends::ToolRunner for FakeToolRunner {
        fn run(
            &mut self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            if let Some((expected_tool, expected_args, result)) = self.runs.pop_front() {
                assert_eq!(tool, expected_tool);
                assert_eq!(args, expected_args.as_slice());
                return result;
            }
            Ok(())
        }

        fn capture(
            &mut self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            let (expected_tool, expected_args, output) = self
                .captures
                .pop_front()
                .expect("expected capture invocation");
            assert_eq!(tool, expected_tool);
            assert_eq!(args, expected_args.as_slice());
            output
        }

        fn capture_json_file(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            Ok(String::new())
        }

        fn stream(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
        }
    }

    #[test]
    fn parses_adb_devices_into_backend_destinations() {
        let output = r#"List of devices attached
emulator-5554 device product:sdk_gphone64_arm64 model:Pixel_9 device:emu64a transport_id:1
ABC123 device model:Pixel_8_Pro device:husky transport_id:2
"#;

        let destinations = parse_adb_devices(output);

        assert_eq!(destinations.len(), 2);
        assert!(destinations[0].is_emulator);
        assert_eq!(destinations[0].model.as_deref(), Some("Pixel 9"));
        assert_eq!(destinations[1].serial, "ABC123");
        assert_eq!(destinations[1].device_name.as_deref(), Some("husky"));
    }

    #[test]
    fn emulator_descriptors_expose_automation_capabilities() {
        let descriptor = destination_descriptor_from_android(AndroidDestination {
            serial: "emulator-5554".to_owned(),
            state: "device".to_owned(),
            model: Some("Pixel 9".to_owned()),
            device_name: Some("emu64a".to_owned()),
            is_emulator: true,
            avd_name: Some("FixtureApi35".to_owned()),
        });

        assert_eq!(descriptor.platform, "android");
        assert_eq!(descriptor.backend_id, BACKEND_ID);
        assert_eq!(descriptor.id, "avd:FixtureApi35");
        assert_eq!(descriptor.kind, "emulator");
        assert!(
            descriptor
                .capabilities
                .contains(&DestinationCapability::InspectUi)
        );
        assert!(
            descriptor
                .capabilities
                .contains(&DestinationCapability::Interact)
        );
        assert!(
            descriptor
                .capabilities
                .contains(&DestinationCapability::Evaluate)
        );
        assert!(
            descriptor
                .capabilities
                .contains(&DestinationCapability::DebugSession)
        );
    }

    #[test]
    fn avd_descriptors_preserve_backend_specific_kind() {
        let descriptor = destination_descriptor_from_android(AndroidDestination {
            serial: "avd:FixtureApi35".to_owned(),
            state: "avd".to_owned(),
            model: None,
            device_name: None,
            is_emulator: false,
            avd_name: Some("FixtureApi35".to_owned()),
        });

        assert_eq!(descriptor.kind, "avd");
        assert_eq!(descriptor.id, "avd:FixtureApi35");
        assert!(descriptor.available);
    }

    #[test]
    fn running_emulators_recover_their_avd_names() {
        let root = Utf8PathBuf::from(".");
        let mut runner = FakeToolRunner::default();
        runner.push_capture(
            "adb",
            &["devices", "-l"],
            Ok(
                "List of devices attached\nemulator-5554 device product:sdk_gphone64_arm64 model:Pixel_9 device:emu64a transport_id:1\nABC123 device model:Pixel_8_Pro device:husky transport_id:2\n",
            ),
        );
        runner.push_capture(
            "adb",
            &[
                "-s",
                "emulator-5554",
                "shell",
                "getprop",
                "ro.boot.qemu.avd_name",
            ],
            Ok("FixtureApi35\n"),
        );

        let destinations = list_android_devices(&root, &mut runner).expect("devices should load");

        assert_eq!(destinations[0].avd_name.as_deref(), Some("FixtureApi35"));
        assert_eq!(destinations[0].destination_id(), "avd:FixtureApi35");
        assert_eq!(destinations[1].avd_name, None);
        assert!(runner.captures.is_empty());
    }

    #[test]
    fn running_emulators_do_not_duplicate_offline_avds() {
        let root = Utf8PathBuf::from(".");
        let mut runner = FakeToolRunner::default();
        runner.push_capture(
            "adb",
            &["devices", "-l"],
            Ok(
                "List of devices attached\nemulator-5554 device product:sdk_gphone64_arm64 model:Pixel_9 device:emu64a transport_id:1\n",
            ),
        );
        runner.push_capture(
            "adb",
            &[
                "-s",
                "emulator-5554",
                "shell",
                "getprop",
                "ro.boot.qemu.avd_name",
            ],
            Ok("FixtureApi35\n"),
        );
        runner.push_capture("emulator", &["-list-avds"], Ok("FixtureApi35\n"));

        let destinations =
            list_android_destinations(&root, &mut runner).expect("destinations should load");

        assert_eq!(destinations.len(), 1);
        assert_eq!(destinations[0].destination_id(), "avd:FixtureApi35");
        assert_eq!(destinations[0].state, "device");
        assert!(runner.captures.is_empty());
    }

    #[test]
    fn avd_identifier_resolves_to_running_emulator() {
        let root = Utf8PathBuf::from(".");
        let mut runner = FakeToolRunner::default();
        runner.push_capture(
            "adb",
            &["devices", "-l"],
            Ok(
                "List of devices attached\nemulator-5554 device product:sdk_gphone64_arm64 model:Pixel_9 device:emu64a transport_id:1\n",
            ),
        );
        runner.push_capture(
            "adb",
            &[
                "-s",
                "emulator-5554",
                "shell",
                "getprop",
                "ro.boot.qemu.avd_name",
            ],
            Ok("FixtureApi35\n"),
        );

        let destination = find_android_destination(&root, &mut runner, "avd:FixtureApi35")
            .expect("lookup should succeed")
            .expect("destination should resolve");

        assert_eq!(destination.serial, "emulator-5554");
        assert_eq!(destination.state, "device");
        assert_eq!(destination.avd_name.as_deref(), Some("FixtureApi35"));
        assert!(runner.captures.is_empty());
    }

    #[test]
    fn emulator_avd_lookup_failures_surface_tool_error() {
        let root = Utf8PathBuf::from(".");
        let mut runner = FakeToolRunner::default();
        runner.push_capture(
            "adb",
            &["devices", "-l"],
            Ok(
                "List of devices attached\nemulator-5554 device product:sdk_gphone64_arm64 model:Pixel_9 device:emu64a transport_id:1\n",
            ),
        );
        runner.push_capture(
            "adb",
            &[
                "-s",
                "emulator-5554",
                "shell",
                "getprop",
                "ro.boot.qemu.avd_name",
            ],
            Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "adb getprop failed",
            )),
        );

        let error = list_android_devices(&root, &mut runner).expect_err("lookup should fail");
        assert_eq!(error.code, AtomErrorCode::ExternalToolFailed);
    }

    #[test]
    fn debugger_android_builds_enable_dbg_mode() {
        let args = android_bazel_args("//apps/demo:demo", AppSessionBuildProfile::Debugger);

        assert_eq!(
            args,
            vec![
                "build".to_owned(),
                "//apps/demo:demo".to_owned(),
                "--android_platforms=//platforms:arm64-v8a".to_owned(),
                "--compilation_mode=dbg".to_owned(),
            ]
        );
    }

    #[test]
    fn jdwp_forward_retries_until_the_process_becomes_attachable() {
        let root = Utf8PathBuf::from(".");
        let mut runner = FakeToolRunner::default();
        runner.push_run(
            "adb",
            &["-s", "emulator-5554", "forward", "tcp:5005", "jdwp:4242"],
            Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "target not ready",
            )),
        );
        runner.push_run(
            "adb",
            &["-s", "emulator-5554", "forward", "tcp:5005", "jdwp:4242"],
            Ok(()),
        );

        forward_jdwp_port(&mut runner, &root, "emulator-5554", 5005, "4242")
            .expect("forward should retry until attachable");

        assert!(runner.runs.is_empty());
    }

    #[test]
    fn prompt_parser_recognizes_plain_and_thread_prompts() {
        assert_eq!(prompt_suffix_len(b"Initializing jdb ...\n> "), Some(2));
        assert_eq!(prompt_suffix_len(b"worker-thread[1] "), Some(17));
        assert_eq!(prompt_suffix_len(b"  [1] JdwpFixture.main "), None);
    }

    #[test]
    fn where_all_output_distinguishes_running_and_stopped_sessions() {
        assert_eq!(
            debug_state_from_where_all_output(
                "main:\nCurrent thread isn't suspended.\nworker-thread:\nCurrent thread isn't suspended.\n"
            ),
            DebugSessionState::Running
        );
        assert_eq!(
            debug_state_from_where_all_output(
                "main:\n  [1] JdwpFixture.main (JdwpFixture.java:9)\nworker-thread:\n  [1] JdwpFixture.workLoop (JdwpFixture.java:17)\n"
            ),
            DebugSessionState::Stopped
        );
    }

    #[test]
    fn jdb_thread_output_preserves_ids_names_and_selection() {
        let threads = parse_jdb_threads(
            "Group main:\n  (java.lang.Thread)1                           main                sleeping\n  (java.lang.Thread)564                         worker-thread       running\n",
            Some("564"),
        );

        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, "1");
        assert_eq!(threads[0].name.as_deref(), Some("main"));
        assert!(!threads[0].selected);
        assert_eq!(threads[1].id, "564");
        assert_eq!(threads[1].name.as_deref(), Some("worker-thread"));
        assert!(threads[1].selected);
    }

    #[test]
    fn jdb_where_output_parses_stack_frames() {
        let frames = parse_jdb_frames(
            "  [1] JdwpFixture.workLoop (JdwpFixture.java:17)\n  [2] java.lang.Thread.runWith (Thread.java:1,596)\n  [3] java.lang.Thread.run (native method)\n",
        );

        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].index, 0);
        assert_eq!(frames[0].function, "JdwpFixture.workLoop");
        assert_eq!(frames[0].source_path.as_deref(), Some("JdwpFixture.java"));
        assert_eq!(frames[0].line, Some(17));
        assert_eq!(frames[1].line, Some(1596));
        assert_eq!(frames[2].source_path, None);
        assert_eq!(frames[2].line, None);
    }
}
