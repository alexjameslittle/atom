use atom_backend_android_jvm_debug::{AndroidJvmAttachOptions, AndroidJvmClient};
use atom_backend_android_native_debug::{AndroidNativeAttachOptions, AndroidNativeClient};
use std::fs;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atom_backends::{
    BackendAutomationSession, BackendDebugSession, BackendDefinition, DebugBacktrace,
    DebugBreakpoint, DebugSourceLocation, DebugStop, DebugThread, DebuggerKind, DeployBackend,
    DeployBackendRegistry, DestinationCapability, DestinationDescriptor, InteractionRequest,
    InteractionResult, LaunchMode, SessionLaunchBehavior, SharedToolRunner, ToolRunner, UiSnapshot,
};
use atom_deploy::devices::{choose_from_menu, should_prompt_interactively};
use atom_deploy::progress::run_step;
use atom_deploy::{
    bazel_source_map_prefix, capture_tool, find_bazel_output_owned, generated_target,
    list_bazel_outputs_owned, run_bazel_owned, run_tool, stream_tool,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};

use crate::android_uiautomator::{
    inspect_ui_with_android_uiautomator, interact_with_android_uiautomator,
    interact_with_android_uiautomator_without_snapshot,
};

const BACKEND_ID: &str = "android";
const APP_PID_WAIT_ATTEMPTS: usize = 30;
const APP_PID_WAIT_INTERVAL: Duration = Duration::from_millis(500);
const BOOT_TIMEOUT_ATTEMPTS: usize = 60;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const APP_LAUNCH_READY_TIMEOUT: Duration = Duration::from_secs(15);
const APP_LAUNCH_READY_POLL_INTERVAL: Duration = Duration::from_millis(250);
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

struct AndroidAutomationSession<'a> {
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    runner: &'a SharedToolRunner<'a>,
    destination_id: String,
    launch_behavior: SessionLaunchBehavior,
    launch: Option<AndroidAppLaunch>,
    video_capture: Option<VideoCapture>,
}

struct AndroidDebugArtifacts {
    apk: Utf8PathBuf,
    application_id: String,
    generated_kotlin_jar: Utf8PathBuf,
    native_library_path: Utf8PathBuf,
    native_source_map_prefix: Option<String>,
}

struct AndroidJvmDebugSession<'a> {
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    runner: &'a SharedToolRunner<'a>,
    destination_id: String,
    client: Option<AndroidJvmClient>,
}

struct AndroidNativeDebugSession<'a> {
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    runner: &'a SharedToolRunner<'a>,
    destination_id: String,
    client: Option<AndroidNativeClient>,
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

    fn new_automation_session<'a>(
        &self,
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        destination_id: &'a str,
        runner: &'a SharedToolRunner<'a>,
        launch_behavior: SessionLaunchBehavior,
    ) -> AtomResult<Box<dyn BackendAutomationSession + 'a>> {
        Ok(Box::new(AndroidAutomationSession {
            repo_root,
            manifest,
            runner,
            destination_id: destination_id.to_owned(),
            launch_behavior,
            launch: None,
            video_capture: None,
        }))
    }

    fn new_debug_session<'a>(
        &self,
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        destination_id: &'a str,
        runner: &'a SharedToolRunner<'a>,
        _launch_behavior: SessionLaunchBehavior,
        debugger: DebuggerKind,
    ) -> AtomResult<Box<dyn BackendDebugSession + 'a>> {
        match debugger {
            DebuggerKind::Jvm => Ok(Box::new(AndroidJvmDebugSession {
                repo_root,
                manifest,
                runner,
                destination_id: destination_id.to_owned(),
                client: None,
            })),
            DebuggerKind::Native => Ok(Box::new(AndroidNativeDebugSession {
                repo_root,
                manifest,
                runner,
                destination_id: destination_id.to_owned(),
                client: None,
            })),
        }
    }
}

impl AndroidAutomationSession<'_> {
    fn active_launch(&self) -> AtomResult<AndroidAppLaunch> {
        self.launch.clone().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::InternalBug,
                "automation session expected a launch after ensure_launched",
            )
        })
    }
}

impl AndroidJvmDebugSession<'_> {
    fn resolve_destination(&self) -> AtomResult<AndroidDestination> {
        let mut runner = self.runner.borrow_mut();
        resolve_android_device(self.repo_root, &mut **runner, Some(&self.destination_id))
    }

    fn ensure_client(&mut self) -> AtomResult<&mut AndroidJvmClient> {
        self.client.as_mut().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                "Android JVM debugger session has not been attached or launched",
            )
        })
    }
}

impl AndroidNativeDebugSession<'_> {
    fn resolve_destination(&self) -> AtomResult<AndroidDestination> {
        let mut runner = self.runner.borrow_mut();
        resolve_android_device(self.repo_root, &mut **runner, Some(&self.destination_id))
    }

    fn ensure_client(&mut self) -> AtomResult<&mut AndroidNativeClient> {
        self.client.as_mut().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                "Android native debugger session has not been attached or launched",
            )
        })
    }
}

impl BackendAutomationSession for AndroidAutomationSession<'_> {
    fn video_extension(&self) -> &'static str {
        "mp4"
    }

    fn attach_existing(&mut self) -> AtomResult<bool> {
        if self.launch.is_some() {
            return Ok(true);
        }
        let mut runner = self.runner.borrow_mut();
        if let Some(launch) = attach_running_android_app(
            self.repo_root,
            self.manifest,
            &self.destination_id,
            &mut **runner,
        )? {
            self.launch = Some(launch);
            return Ok(true);
        }
        Ok(false)
    }

    fn ensure_launched(&mut self) -> AtomResult<()> {
        if self.launch.is_some() {
            return Ok(());
        }
        if self.launch_behavior == SessionLaunchBehavior::AttachOrLaunch
            && let Some(launch) = {
                let mut runner = self.runner.borrow_mut();
                attach_android_app(
                    self.repo_root,
                    self.manifest,
                    &self.destination_id,
                    &mut **runner,
                )?
            }
        {
            self.launch = Some(launch);
            return Ok(());
        }
        let destination = {
            let mut runner = self.runner.borrow_mut();
            find_android_destination(self.repo_root, &mut **runner, &self.destination_id)?
        };
        let Some(destination) = destination else {
            return Err(AtomError::with_path(
                AtomErrorCode::AutomationUnavailable,
                format!("unknown destination id: {}", self.destination_id),
                &self.destination_id,
            ));
        };
        let launch = {
            let mut runner = self.runner.borrow_mut();
            launch_android_app(self.repo_root, self.manifest, &destination, &mut **runner)?
        };
        {
            let mut runner = self.runner.borrow_mut();
            wait_for_android_launch_ready(
                self.repo_root,
                &launch.serial,
                &launch.application_id,
                &mut **runner,
            )?;
        }
        self.launch = Some(launch);
        Ok(())
    }

    fn interact(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult> {
        self.ensure_launched()?;
        let launch = self.active_launch()?;
        let mut runner = self.runner.borrow_mut();
        interact_with_android_uiautomator(self.repo_root, &launch.serial, &mut **runner, request)
    }

    fn interact_without_snapshot(&mut self, request: InteractionRequest) -> AtomResult<()> {
        self.ensure_launched()?;
        let launch = self.active_launch()?;
        let mut runner = self.runner.borrow_mut();
        interact_with_android_uiautomator_without_snapshot(
            self.repo_root,
            &launch.serial,
            &mut **runner,
            request,
        )
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
        let mut runner = self.runner.borrow_mut();
        capture_screenshot_for_launch(self.repo_root, &launch, output_path, &mut **runner)
    }

    fn capture_logs(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()> {
        self.ensure_launched()?;
        let launch = self.active_launch()?;
        let mut runner = self.runner.borrow_mut();
        capture_logs_for_launch(self.repo_root, &launch, output_path, seconds, &mut **runner)
    }

    fn capture_video(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()> {
        self.ensure_launched()?;
        let launch = self.active_launch()?;
        let mut runner = self.runner.borrow_mut();
        capture_video_for_launch(self.repo_root, &launch, output_path, seconds, &mut **runner)
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
        let mut runner = self.runner.borrow_mut();
        stop_video_capture(self.repo_root, video, &mut **runner)
    }

    fn shutdown_video(&mut self) -> AtomResult<()> {
        if self.video_capture.is_some() {
            let _ = self.stop_video()?;
        }
        Ok(())
    }
}

impl BackendDebugSession for AndroidJvmDebugSession<'_> {
    fn kind(&self) -> DebuggerKind {
        DebuggerKind::Jvm
    }

    fn launch(&mut self) -> AtomResult<()> {
        if self.client.is_some() {
            return Ok(());
        }
        let destination = self.resolve_destination()?;
        let build = {
            let mut runner = self.runner.borrow_mut();
            build_android_debug_artifacts(self.repo_root, self.manifest, &mut **runner)?
        };
        let launch = {
            let mut runner = self.runner.borrow_mut();
            launch_android_debug_app(self.repo_root, &destination, &build, &mut **runner)?
        };
        let pid = {
            let mut runner = self.runner.borrow_mut();
            read_android_debug_pid(
                self.repo_root,
                &launch.serial,
                &launch.application_id,
                &mut **runner,
            )?
        };
        let client = AndroidJvmClient::attach(
            self.repo_root,
            &AndroidJvmAttachOptions {
                serial: launch.serial,
                pid,
                source_root: self.repo_root.to_owned(),
                generated_kotlin_jar: build.generated_kotlin_jar,
            },
        )?;
        self.client = Some(client);
        Ok(())
    }

    fn attach(&mut self) -> AtomResult<()> {
        if self.client.is_some() {
            return Ok(());
        }
        let destination = self.resolve_destination()?;
        let build = {
            let mut runner = self.runner.borrow_mut();
            build_android_debug_artifacts(self.repo_root, self.manifest, &mut **runner)?
        };
        let launch = {
            let mut runner = self.runner.borrow_mut();
            attach_or_launch_android_debug_app(
                self.repo_root,
                self.manifest,
                &destination,
                &build,
                &mut **runner,
            )?
        };
        let pid = {
            let mut runner = self.runner.borrow_mut();
            read_android_debug_pid(
                self.repo_root,
                &launch.serial,
                &launch.application_id,
                &mut **runner,
            )?
        };
        let client = AndroidJvmClient::attach(
            self.repo_root,
            &AndroidJvmAttachOptions {
                serial: launch.serial,
                pid,
                source_root: self.repo_root.to_owned(),
                generated_kotlin_jar: build.generated_kotlin_jar,
            },
        )?;
        self.client = Some(client);
        Ok(())
    }

    fn set_breakpoint(&mut self, location: DebugSourceLocation) -> AtomResult<DebugBreakpoint> {
        self.ensure_client()?.set_breakpoint(&location)
    }

    fn clear_breakpoint(&mut self, location: DebugSourceLocation) -> AtomResult<()> {
        self.ensure_client()?.clear_breakpoint(&location)
    }

    fn wait_for_stop(&mut self, timeout_ms: Option<u64>) -> AtomResult<DebugStop> {
        self.ensure_client()?.wait_for_stop(timeout_ms)
    }

    fn threads(&mut self) -> AtomResult<Vec<DebugThread>> {
        self.ensure_client()?.threads()
    }

    fn backtrace(&mut self, thread_id: Option<&str>) -> AtomResult<DebugBacktrace> {
        self.ensure_client()?.backtrace(thread_id)
    }

    fn pause(&mut self) -> AtomResult<DebugStop> {
        self.ensure_client()?.pause()
    }

    fn resume(&mut self) -> AtomResult<()> {
        self.ensure_client()?.resume()
    }

    fn shutdown(&mut self) -> AtomResult<()> {
        if let Some(client) = self.client.as_mut() {
            client.shutdown()?;
        }
        Ok(())
    }
}

impl BackendDebugSession for AndroidNativeDebugSession<'_> {
    fn kind(&self) -> DebuggerKind {
        DebuggerKind::Native
    }

    fn launch(&mut self) -> AtomResult<()> {
        if self.client.is_some() {
            return Ok(());
        }
        let destination = self.resolve_destination()?;
        let build = {
            let mut runner = self.runner.borrow_mut();
            build_android_debug_artifacts(self.repo_root, self.manifest, &mut **runner)?
        };
        let launch = {
            let mut runner = self.runner.borrow_mut();
            launch_android_debug_app(self.repo_root, &destination, &build, &mut **runner)?
        };
        let pid = {
            let mut runner = self.runner.borrow_mut();
            read_android_debug_pid(
                self.repo_root,
                &launch.serial,
                &launch.application_id,
                &mut **runner,
            )?
        };
        let mut client = AndroidNativeClient::attach(
            self.repo_root,
            &AndroidNativeAttachOptions {
                serial: launch.serial,
                application_id: launch.application_id,
                pid,
                native_library_path: build.native_library_path,
                source_map_prefix: build.native_source_map_prefix,
            },
        )?;
        let _ = client.pause()?;
        self.client = Some(client);
        Ok(())
    }

    fn attach(&mut self) -> AtomResult<()> {
        if self.client.is_some() {
            return Ok(());
        }
        let destination = self.resolve_destination()?;
        let build = {
            let mut runner = self.runner.borrow_mut();
            build_android_debug_artifacts(self.repo_root, self.manifest, &mut **runner)?
        };
        let launch = {
            let mut runner = self.runner.borrow_mut();
            attach_or_launch_android_debug_app(
                self.repo_root,
                self.manifest,
                &destination,
                &build,
                &mut **runner,
            )?
        };
        let pid = {
            let mut runner = self.runner.borrow_mut();
            read_android_debug_pid(
                self.repo_root,
                &launch.serial,
                &launch.application_id,
                &mut **runner,
            )?
        };
        let mut client = AndroidNativeClient::attach(
            self.repo_root,
            &AndroidNativeAttachOptions {
                serial: launch.serial,
                application_id: launch.application_id,
                pid,
                native_library_path: build.native_library_path,
                source_map_prefix: build.native_source_map_prefix,
            },
        )?;
        let _ = client.pause()?;
        self.client = Some(client);
        Ok(())
    }

    fn set_breakpoint(&mut self, location: DebugSourceLocation) -> AtomResult<DebugBreakpoint> {
        self.ensure_client()?
            .set_breakpoint(&location.file, location.line)
    }

    fn clear_breakpoint(&mut self, location: DebugSourceLocation) -> AtomResult<()> {
        self.ensure_client()?
            .clear_breakpoint(&location.file, location.line)
    }

    fn wait_for_stop(&mut self, timeout_ms: Option<u64>) -> AtomResult<DebugStop> {
        self.ensure_client()?.wait_for_stop(timeout_ms)
    }

    fn threads(&mut self) -> AtomResult<Vec<DebugThread>> {
        self.ensure_client()?.threads()
    }

    fn backtrace(&mut self, thread_id: Option<&str>) -> AtomResult<DebugBacktrace> {
        self.ensure_client()?.backtrace(thread_id)
    }

    fn pause(&mut self) -> AtomResult<DebugStop> {
        self.ensure_client()?.pause()
    }

    fn resume(&mut self) -> AtomResult<()> {
        self.ensure_client()?.resume()
    }

    fn shutdown(&mut self) -> AtomResult<()> {
        if let Some(client) = self.client.as_mut() {
            client.shutdown()?;
        }
        Ok(())
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
        DestinationCapability::Debug,
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
        debuggers: vec![DebuggerKind::Native, DebuggerKind::Jvm],
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
    let target = generated_target(manifest, BACKEND_ID);
    let build_args = android_bazel_args(&target, false);

    run_step(
        "Building Android app...",
        "Built Android app",
        "Android build failed",
        || run_bazel_owned(runner, repo_root, &build_args),
    )?;

    let apk = find_bazel_output_owned(
        runner,
        repo_root,
        &build_args,
        &target,
        &["app.apk", ".apk"],
        "APK",
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
                &["-s", &serial, "install", "-r", apk.as_str()],
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
        .args(["-avd", avd])
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

fn read_android_app_pid(
    runner: &mut dyn ToolRunner,
    repo_root: &Utf8Path,
    serial: &str,
    application_id: &str,
) -> AtomResult<Option<u32>> {
    let output = runner.capture_output(
        repo_root,
        "adb",
        &[
            "-s".to_owned(),
            serial.to_owned(),
            "shell".to_owned(),
            "pidof".to_owned(),
            application_id.to_owned(),
        ],
    )?;
    if output.exit_code == 1 {
        return Ok(None);
    }
    if output.exit_code != 0 {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            format!(
                "adb -s {serial} shell pidof {application_id} exited with status {}",
                output.exit_code
            )
        } else {
            format!(
                "adb -s {serial} shell pidof {application_id} exited with status {}:\n{}",
                output.exit_code,
                stderr.trim()
            )
        };
        return Err(AtomError::new(AtomErrorCode::ExternalToolFailed, detail));
    }
    let output = String::from_utf8(output.stdout).map_err(|_| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("adb pidof returned non-UTF-8 output for {application_id}"),
        )
    })?;
    let Some(pid) = output.split_whitespace().next() else {
        return Ok(None);
    };
    let pid = pid.parse::<u32>().map_err(|error| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("adb pidof returned an invalid PID for {application_id}: {error}"),
        )
    })?;
    Ok(Some(pid))
}

fn read_android_debug_pid(
    repo_root: &Utf8Path,
    serial: &str,
    application_id: &str,
    runner: &mut dyn ToolRunner,
) -> AtomResult<u32> {
    for _ in 0..APP_PID_WAIT_ATTEMPTS {
        if let Some(pid) = read_android_app_pid(runner, repo_root, serial, application_id)? {
            return Ok(pid);
        }
        thread::sleep(APP_PID_WAIT_INTERVAL);
    }
    Err(AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        format!("could not resolve a debugger PID for {application_id}"),
    ))
}

fn android_bazel_args(target: &str, debug_build: bool) -> Vec<String> {
    android_bazel_args_for_targets(&[target.to_owned()], debug_build)
}

fn android_bazel_args_for_targets(targets: &[String], debug_build: bool) -> Vec<String> {
    let mut args = vec!["build".to_owned()];
    args.extend(targets.iter().cloned());
    args.push("--android_platforms=//platforms:arm64-v8a".to_owned());
    if debug_build {
        args.push("--compilation_mode=dbg".to_owned());
    }
    args
}

fn build_android_debug_artifacts(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    runner: &mut dyn ToolRunner,
) -> AtomResult<AndroidDebugArtifacts> {
    let target = generated_target(manifest, BACKEND_ID);
    let generated_kotlin_target = generated_sibling_target(&target, "generated_kotlin")?;
    let native_target = generated_sibling_target(&target, "atom_runtime_jni")?;
    let build_args = android_bazel_args_for_targets(
        &[
            target.clone(),
            generated_kotlin_target.clone(),
            native_target.clone(),
        ],
        true,
    );
    run_bazel_owned(runner, repo_root, &build_args)?;
    let query_args = android_bazel_args(&target, true);
    let apk = find_bazel_output_owned(
        runner,
        repo_root,
        &query_args,
        &target,
        &["app.apk", ".apk"],
        "APK",
    )?;
    let application_id = manifest.android.application_id.clone().ok_or_else(|| {
        AtomError::new(AtomErrorCode::InternalBug, "missing Android application id")
    })?;

    let generated_kotlin_jar = select_debug_artifact(
        list_bazel_outputs_owned(runner, repo_root, &query_args, &generated_kotlin_target)?,
        &generated_kotlin_target,
        "generated Kotlin jar",
        |path| {
            path.extension() == Some("jar")
                && path
                    .file_name()
                    .is_some_and(|file_name| file_name.contains("generated_kotlin"))
        },
    )?;

    let native_library_path = select_debug_artifact(
        list_bazel_outputs_owned(runner, repo_root, &query_args, &native_target)?,
        &native_target,
        "Android native library",
        |path| {
            path.extension() == Some("so")
                && path
                    .file_name()
                    .is_some_and(|file_name| file_name.contains("atom_runtime_jni"))
        },
    )?;

    Ok(AndroidDebugArtifacts {
        apk,
        application_id,
        generated_kotlin_jar,
        native_source_map_prefix: bazel_source_map_prefix(&native_library_path)?,
        native_library_path,
    })
}

fn generated_sibling_target(target: &str, sibling: &str) -> AtomResult<String> {
    let (package, _name) = target.rsplit_once(':').ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            format!("generated target {target} was not a Bazel label"),
        )
    })?;
    Ok(format!("{package}:{sibling}"))
}

fn select_debug_artifact(
    outputs: Vec<Utf8PathBuf>,
    target: &str,
    artifact_name: &str,
    predicate: impl Fn(&Utf8Path) -> bool,
) -> AtomResult<Utf8PathBuf> {
    let mut matches = outputs
        .into_iter()
        .filter(|path| predicate(path))
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    if matches.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("bazelisk cquery did not return a {artifact_name} for {target}"),
        ));
    }
    if matches.len() == 1 {
        return Ok(matches.remove(0));
    }

    let mut existing = matches
        .iter()
        .filter(|path| path.exists())
        .cloned()
        .collect::<Vec<_>>();
    if existing.len() == 1 {
        return Ok(existing.remove(0));
    }
    if let Some(preferred) = existing
        .iter()
        .find(|path| path.as_str().contains("arm64-v8a") || path.as_str().contains("/android/"))
    {
        return Ok(preferred.clone());
    }

    let candidates = matches
        .iter()
        .map(|path| path.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Err(AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        format!(
            "bazelisk cquery returned multiple {artifact_name} candidates for {target}: {candidates}"
        ),
    ))
}

fn launch_android_debug_app(
    repo_root: &Utf8Path,
    destination: &AndroidDestination,
    build: &AndroidDebugArtifacts,
    runner: &mut dyn ToolRunner,
) -> AtomResult<AndroidAppLaunch> {
    let serial = prepare_android_emulator(repo_root, runner, destination)?;
    run_tool(
        runner,
        repo_root,
        "adb",
        &["-s", &serial, "install", "-r", build.apk.as_str()],
    )?;
    let _ = run_tool(
        runner,
        repo_root,
        "adb",
        &[
            "-s",
            &serial,
            "shell",
            "am",
            "force-stop",
            &build.application_id,
        ],
    );
    let component = format!("{}/.MainActivity", build.application_id);
    run_tool(
        runner,
        repo_root,
        "adb",
        &[
            "-s", &serial, "shell", "am", "start", "-W", "-n", &component,
        ],
    )?;
    Ok(AndroidAppLaunch {
        serial,
        application_id: build.application_id.clone(),
    })
}

fn attach_or_launch_android_debug_app(
    repo_root: &Utf8Path,
    _manifest: &NormalizedManifest,
    destination: &AndroidDestination,
    build: &AndroidDebugArtifacts,
    runner: &mut dyn ToolRunner,
) -> AtomResult<AndroidAppLaunch> {
    let serial = prepare_android_emulator(repo_root, runner, destination)?;
    if read_android_app_pid(runner, repo_root, &serial, &build.application_id)?.is_some() {
        return Ok(AndroidAppLaunch {
            serial,
            application_id: build.application_id.clone(),
        });
    }
    launch_android_debug_app(repo_root, destination, build, runner)
}

fn launch_android_app(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination: &AndroidDestination,
    runner: &mut dyn ToolRunner,
) -> AtomResult<AndroidAppLaunch> {
    let serial = prepare_android_emulator(repo_root, runner, destination)?;
    let target = generated_target(manifest, BACKEND_ID);
    let build_args = android_bazel_args(&target, false);
    run_bazel_owned(runner, repo_root, &build_args)?;
    let apk = find_bazel_output_owned(
        runner,
        repo_root,
        &build_args,
        &target,
        &["app.apk", ".apk"],
        "APK",
    )?;
    let application_id = manifest.android.application_id.clone().ok_or_else(|| {
        AtomError::new(AtomErrorCode::InternalBug, "missing Android application id")
    })?;
    run_tool(
        runner,
        repo_root,
        "adb",
        &["-s", &serial, "install", "-r", apk.as_str()],
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

fn attach_running_android_app(
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
    if read_android_app_pid(runner, repo_root, &destination.serial, application_id)?.is_none() {
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

    use atom_backends::DestinationCapability;
    use atom_ffi::{AtomError, AtomErrorCode};
    use camino::{Utf8Path, Utf8PathBuf};

    use super::{
        AndroidDestination, BACKEND_ID, destination_descriptor_from_android,
        find_android_destination, list_android_destinations, list_android_devices,
        parse_adb_devices, select_debug_artifact,
    };

    #[derive(Default)]
    struct FakeToolRunner {
        captures: VecDeque<(&'static str, Vec<String>, Result<String, AtomError>)>,
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
    }

    impl atom_backends::ToolRunner for FakeToolRunner {
        fn run(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn capture_output(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<atom_backends::ToolCommandOutput> {
            Ok(atom_backends::ToolCommandOutput {
                stdout: Vec::new(),
                stderr: Vec::new(),
                exit_code: 0,
            })
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
    fn duplicate_debug_artifact_paths_do_not_fail_selection() {
        let selected = select_debug_artifact(
            vec![
                Utf8PathBuf::from("bazel-out/darwin_arm64-dbg/bin/cng-output/android/hello-atom/generated_kotlin.jar"),
                Utf8PathBuf::from("bazel-out/darwin_arm64-dbg/bin/cng-output/android/hello-atom/generated_kotlin.jar"),
            ],
            "//cng-output/android/hello-atom:generated_kotlin",
            "generated Kotlin jar",
            |path| path.extension() == Some("jar"),
        )
        .expect("duplicate paths should still resolve cleanly");

        assert_eq!(
            selected,
            Utf8PathBuf::from(
                "bazel-out/darwin_arm64-dbg/bin/cng-output/android/hello-atom/generated_kotlin.jar"
            )
        );
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
}
