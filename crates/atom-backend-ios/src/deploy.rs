use std::fs;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atom_backends::{
    AppSessionBuildProfile, AppSessionOptions, BackendAppSession, BackendDefinition, DeployBackend,
    DeployBackendRegistry, DestinationCapability, DestinationDescriptor, InteractionRequest,
    InteractionResult, LaunchMode, ScreenInfo, SessionLaunchBehavior, ToolRunner, UiBounds, UiNode,
    UiSnapshot,
};
use atom_deploy::devices::{choose_from_menu, should_prompt_interactively};
use atom_deploy::progress::run_step;
use atom_deploy::{
    capture_bazel_cquery_starlark_owned, capture_tool, generated_target, parse_bazel_output_paths,
    run_bazel_owned, run_tool, stream_tool,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

use crate::agent_device;

const BACKEND_ID: &str = "ios";
const APP_LAUNCH_READY_TIMEOUT: Duration = Duration::from_secs(15);
const APP_LAUNCH_READY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const SCREENSHOT_READY_TIMEOUT: Duration = Duration::from_secs(5);
const SCREENSHOT_READY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const VIDEO_STOP_TIMEOUT: Duration = Duration::from_secs(5);

struct IosDeployBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum IosDestinationKind {
    Simulator,
    Device,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IosDestination {
    kind: IosDestinationKind,
    id: String,
    alternate_id: Option<String>,
    name: String,
    state: String,
    runtime: Option<String>,
    architecture: Option<String>,
    is_available: bool,
}

#[derive(Clone)]
struct IosAppLaunch {
    destination_id: String,
    bundle_id: String,
    app_name: String,
    app_slug: String,
}

enum VideoCapture {
    AgentDevice {
        output_path: Utf8PathBuf,
        destination_id: String,
        session_name: String,
    },
    Idb {
        output_path: Utf8PathBuf,
        child: Child,
    },
}

struct IosAppSession<'a> {
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    runner: &'a mut dyn ToolRunner,
    destination_id: String,
    session_options: AppSessionOptions,
    launch: Option<IosAppLaunch>,
    agent_device_session: Option<String>,
    video_capture: Option<VideoCapture>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IosBuildArtifacts {
    installable_app: Utf8PathBuf,
    dsym_bundle: Option<Utf8PathBuf>,
}

impl IosDestination {
    fn matches_identifier(&self, value: &str) -> bool {
        self.id == value || self.alternate_id.as_deref() == Some(value) || self.name == value
    }

    fn is_booted_simulator(&self) -> bool {
        self.kind == IosDestinationKind::Simulator && self.state == "Booted"
    }

    fn display_label(&self) -> String {
        match self.kind {
            IosDestinationKind::Simulator => match &self.runtime {
                Some(runtime) => format!("Simulator: {} [{}; {}]", self.name, runtime, self.state),
                None => format!("Simulator: {} [{}]", self.name, self.state),
            },
            IosDestinationKind::Device => format!("Device: {} [{}]", self.name, self.state),
        }
    }
}

pub fn register(registry: &mut DeployBackendRegistry) -> AtomResult<()> {
    registry.register(Box::new(IosDeployBackend))
}

impl BackendDefinition for IosDeployBackend {
    fn id(&self) -> &'static str {
        BACKEND_ID
    }

    fn platform(&self) -> &'static str {
        "ios"
    }
}

impl DeployBackend for IosDeployBackend {
    fn is_enabled(&self, manifest: &NormalizedManifest) -> bool {
        manifest.ios.enabled
    }

    fn list_destinations(
        &self,
        repo_root: &Utf8Path,
        runner: &mut dyn ToolRunner,
    ) -> AtomResult<Vec<DestinationDescriptor>> {
        let agent_device_available = agent_device::is_available(repo_root, runner);
        Ok(list_ios_destinations(repo_root, runner)?
            .into_iter()
            .map(|destination| destination_descriptor_from_ios(destination, agent_device_available))
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
        deploy_ios(
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
        stop_ios(repo_root, manifest, requested_destination, runner)
    }

    fn new_app_session<'a>(
        &self,
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        destination_id: &'a str,
        runner: &'a mut dyn ToolRunner,
        options: AppSessionOptions,
    ) -> AtomResult<Box<dyn BackendAppSession + 'a>> {
        Ok(Box::new(IosAppSession {
            repo_root,
            manifest,
            runner,
            destination_id: destination_id.to_owned(),
            session_options: options,
            launch: None,
            agent_device_session: None,
            video_capture: None,
        }))
    }
}

impl IosAppSession<'_> {
    fn active_launch(&self) -> AtomResult<IosAppLaunch> {
        self.launch.clone().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::InternalBug,
                "app session expected a launch after ensure_launched",
            )
        })
    }

    fn agent_device_session_name(&mut self) -> AtomResult<String> {
        if let Some(session_name) = &self.agent_device_session {
            return Ok(session_name.clone());
        }
        let bundle_id =
            self.manifest.ios.bundle_id.as_deref().ok_or_else(|| {
                AtomError::new(AtomErrorCode::InternalBug, "missing iOS bundle id")
            })?;
        let session_name = format!(
            "atom-{}-{}-{}",
            sanitize_session_segment(bundle_id),
            sanitize_session_segment(&self.destination_id),
            timestamp_suffix(),
        );
        self.agent_device_session = Some(session_name.clone());
        Ok(session_name)
    }

    fn cleanup_agent_device_session(&mut self) -> AtomResult<()> {
        if let Some(video) = self.video_capture.take() {
            let _ = stop_video_capture(self.repo_root, video)?;
        }
        if let Some(session_name) = self.agent_device_session.take() {
            agent_device::close_session(
                self.repo_root,
                &self.destination_id,
                &session_name,
                self.runner,
            )?;
        }
        Ok(())
    }

    fn cleanup_agent_device_session_best_effort(&mut self) {
        let _ = self.cleanup_agent_device_session();
    }
}

impl BackendAppSession for IosAppSession<'_> {
    fn video_extension(&self) -> &'static str {
        "mov"
    }

    fn ensure_launched(&mut self) -> AtomResult<()> {
        if self.launch.is_some() {
            return Ok(());
        }
        let Some(destination) = list_ios_destinations(self.repo_root, self.runner)?
            .into_iter()
            .find(|destination| destination.id == self.destination_id)
        else {
            return Err(AtomError::with_path(
                AtomErrorCode::AutomationUnavailable,
                format!("unknown destination id: {}", self.destination_id),
                &self.destination_id,
            ));
        };
        if agent_device::is_available(self.repo_root, self.runner) {
            let session_name = self.agent_device_session_name()?;
            if self.session_options.launch_behavior == SessionLaunchBehavior::AttachOrLaunch
                && let Some(launch) = attach_ios_app_with_agent_device(
                    self.repo_root,
                    self.manifest,
                    &destination,
                    &session_name,
                    self.runner,
                )?
            {
                self.launch = Some(launch);
                return Ok(());
            }
            let launch = launch_ios_app_with_agent_device(
                self.repo_root,
                self.manifest,
                &destination,
                &session_name,
                self.session_options.build_profile,
                self.runner,
            )?;
            self.launch = Some(launch);
            return Ok(());
        }
        if self.session_options.launch_behavior == SessionLaunchBehavior::AttachOrLaunch
            && let Some(launch) = attach_ios_app(
                self.repo_root,
                self.manifest,
                &self.destination_id,
                self.runner,
            )?
        {
            self.launch = Some(launch);
            return Ok(());
        }
        let launch = launch_ios_app(
            self.repo_root,
            self.manifest,
            &destination,
            self.session_options.build_profile,
            self.runner,
        )?;
        wait_for_launch_ready(
            self.repo_root,
            &launch.destination_id,
            &launch.app_name,
            &launch.app_slug,
            self.runner,
        )?;
        self.launch = Some(launch);
        Ok(())
    }

    fn interact(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult> {
        self.ensure_launched()?;
        if let Some(session_name) = self.agent_device_session.as_deref() {
            return agent_device::interact_with_agent_device(
                self.repo_root,
                &self.destination_id,
                session_name,
                self.runner,
                request,
            );
        }
        interact_with_idb(self.repo_root, &self.destination_id, self.runner, request)
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
        if let Some(session_name) = self.agent_device_session.as_deref() {
            return agent_device::capture_screenshot(
                self.repo_root,
                &self.destination_id,
                session_name,
                output_path,
                self.runner,
            );
        }
        let launch = self.active_launch()?;
        capture_screenshot_for_launch(self.repo_root, &launch, output_path, self.runner)
    }

    fn capture_logs(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()> {
        self.ensure_launched()?;
        if let Some(session_name) = self.agent_device_session.as_deref() {
            return capture_logs_with_agent_device(
                self.repo_root,
                &self.destination_id,
                session_name,
                output_path,
                seconds,
                self.runner,
            );
        }
        let launch = self.active_launch()?;
        capture_logs_for_launch(self.repo_root, &launch, output_path, seconds, self.runner)
    }

    fn capture_video(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()> {
        self.ensure_launched()?;
        if let Some(session_name) = self.agent_device_session.as_deref() {
            agent_device::start_video_capture(
                self.repo_root,
                &self.destination_id,
                session_name,
                output_path,
                self.runner,
            )?;
            thread::sleep(Duration::from_secs(seconds));
            agent_device::stop_video_capture(
                self.repo_root,
                &self.destination_id,
                session_name,
                self.runner,
            )?;
            ensure_video_artifact(output_path)?;
            return Ok(());
        }
        let launch = self.active_launch()?;
        capture_video_for_launch(self.repo_root, &launch, output_path, seconds)
    }

    fn start_video(&mut self, output_path: &Utf8Path) -> AtomResult<()> {
        self.ensure_launched()?;
        if let Some(session_name) = self.agent_device_session.as_deref() {
            agent_device::start_video_capture(
                self.repo_root,
                &self.destination_id,
                session_name,
                output_path,
                self.runner,
            )?;
            self.video_capture = Some(VideoCapture::AgentDevice {
                output_path: output_path.to_owned(),
                destination_id: self.destination_id.clone(),
                session_name: session_name.to_owned(),
            });
            return Ok(());
        }
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
        stop_video_capture(self.repo_root, video)
    }

    fn shutdown_video(&mut self) -> AtomResult<()> {
        self.cleanup_agent_device_session()
    }
}

impl Drop for IosAppSession<'_> {
    fn drop(&mut self) {
        self.cleanup_agent_device_session_best_effort();
    }
}

fn destination_descriptor_from_ios(
    destination: IosDestination,
    agent_device_available: bool,
) -> DestinationDescriptor {
    let display_name = destination.display_label();
    let id = destination.id.clone();
    let kind = match destination.kind {
        IosDestinationKind::Simulator => "simulator",
        IosDestinationKind::Device => "device",
    };
    let capabilities = match destination.kind {
        IosDestinationKind::Simulator => vec![
            DestinationCapability::Launch,
            DestinationCapability::Logs,
            DestinationCapability::Screenshot,
            DestinationCapability::Video,
            DestinationCapability::InspectUi,
            DestinationCapability::Interact,
            DestinationCapability::Evaluate,
        ],
        IosDestinationKind::Device if agent_device_available => vec![
            DestinationCapability::Launch,
            DestinationCapability::Logs,
            DestinationCapability::Screenshot,
            DestinationCapability::Video,
            DestinationCapability::InspectUi,
            DestinationCapability::Interact,
            DestinationCapability::Evaluate,
        ],
        IosDestinationKind::Device => vec![DestinationCapability::Launch],
    };

    DestinationDescriptor {
        platform: "ios".to_owned(),
        backend_id: BACKEND_ID.to_owned(),
        id,
        kind: kind.to_owned(),
        display_name,
        available: destination.is_available,
        debug_state: destination.state,
        capabilities,
    }
}

fn deploy_ios(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_destination: Option<&str>,
    launch_mode: LaunchMode,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    let destination = resolve_ios_destination(repo_root, runner, requested_destination)?;
    if agent_device::is_available(repo_root, runner) {
        return deploy_ios_with_agent_device(
            repo_root,
            manifest,
            &destination,
            launch_mode,
            runner,
        );
    }
    let artifacts = run_step(
        "Building iOS app...",
        "Built iOS app",
        "iOS build failed",
        || {
            build_ios_artifacts(
                repo_root,
                manifest,
                &destination,
                AppSessionBuildProfile::Standard,
                runner,
            )
        },
    )?;
    let bundle_id = manifest.ios.bundle_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated iOS manifest is missing bundle_id",
        )
    })?;

    match destination.kind {
        IosDestinationKind::Simulator => install_and_launch_simulator(
            repo_root,
            manifest,
            runner,
            &destination,
            &artifacts.installable_app,
            bundle_id,
            launch_mode,
        ),
        IosDestinationKind::Device => install_and_launch_device(
            repo_root,
            manifest,
            runner,
            &destination.id,
            &artifacts.installable_app,
            bundle_id,
            launch_mode,
        ),
    }
}

fn stop_ios(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_destination: Option<&str>,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    let destination = resolve_ios_destination(repo_root, runner, requested_destination)?;
    if agent_device::is_available(repo_root, runner) {
        return stop_ios_with_agent_device(repo_root, manifest, &destination, runner);
    }
    let bundle_id = manifest.ios.bundle_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated iOS manifest is missing bundle_id",
        )
    })?;

    if destination.kind == IosDestinationKind::Simulator && !destination.is_booted_simulator() {
        return Ok(());
    }

    if !ios_app_is_running(repo_root, runner, &destination.id, bundle_id)? {
        return Ok(());
    }

    run_step("Stopping app...", "App stopped", "Stop failed", || {
        run_tool(
            runner,
            repo_root,
            "idb",
            &["terminate", "--udid", &destination.id, bundle_id],
        )
    })
}

fn deploy_ios_with_agent_device(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination: &IosDestination,
    launch_mode: LaunchMode,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    let artifacts = run_step(
        "Building iOS app...",
        "Built iOS app",
        "iOS build failed",
        || {
            build_ios_artifacts(
                repo_root,
                manifest,
                destination,
                AppSessionBuildProfile::Standard,
                runner,
            )
        },
    )?;
    let bundle_id = manifest.ios.bundle_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated iOS manifest is missing bundle_id",
        )
    })?;
    let destination_id = run_step(
        "Preparing destination...",
        "Destination ready",
        "Destination preparation failed",
        || {
            agent_device::boot_destination(repo_root, &destination.id, runner)?;
            Ok(destination.id.clone())
        },
    )?;
    let session_name = format!(
        "atom-run-{}-{}-{}",
        sanitize_session_segment(bundle_id),
        sanitize_session_segment(&destination_id),
        timestamp_suffix(),
    );
    let mut logs_started = false;
    let result = (|| -> AtomResult<()> {
        run_step(
            "Installing app...",
            "App installed",
            "Installation failed",
            || {
                agent_device::reinstall_app(
                    repo_root,
                    &destination_id,
                    &session_name,
                    bundle_id,
                    &artifacts.installable_app,
                    runner,
                )
            },
        )?;
        run_step("Launching app...", "App launched", "Launch failed", || {
            agent_device::open_app(
                repo_root,
                &destination_id,
                &session_name,
                bundle_id,
                false,
                runner,
            )?;
            wait_for_launch_ready_with_agent_device(
                repo_root,
                &destination_id,
                &session_name,
                &manifest.app.name,
                &manifest.app.slug,
                runner,
            )
        })?;
        if launch_mode == LaunchMode::Attached {
            agent_device::start_logs(repo_root, &destination_id, &session_name, runner)?;
            logs_started = true;
            let log_path =
                agent_device::logs_path(repo_root, &destination_id, &session_name, runner)?;
            eprintln!("→ Streaming logs... (Ctrl+C to stop)");
            stream_tool(
                runner,
                repo_root,
                "tail",
                &["-n", "+1", "-f", log_path.as_str()],
            )?;
        }
        Ok(())
    })();
    let cleanup_result = cleanup_ios_agent_device_command(
        repo_root,
        &destination_id,
        &session_name,
        logs_started,
        runner,
    );
    if let Err(error) = result {
        let _ = cleanup_result;
        return Err(error);
    }
    cleanup_result
}

fn stop_ios_with_agent_device(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination: &IosDestination,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    let bundle_id = manifest.ios.bundle_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated iOS manifest is missing bundle_id",
        )
    })?;
    if destination.kind == IosDestinationKind::Simulator && !destination.is_booted_simulator() {
        return Ok(());
    }
    let session_name = format!(
        "atom-stop-{}-{}-{}",
        sanitize_session_segment(bundle_id),
        sanitize_session_segment(&destination.id),
        timestamp_suffix(),
    );
    let result = run_step("Stopping app...", "App stopped", "Stop failed", || {
        agent_device::open_app(
            repo_root,
            &destination.id,
            &session_name,
            bundle_id,
            false,
            runner,
        )?;
        agent_device::close_app(repo_root, &destination.id, &session_name, bundle_id, runner)
    });
    if let Err(error) = result {
        let _ = agent_device::close_session(repo_root, &destination.id, &session_name, runner);
        return Err(error);
    }
    Ok(())
}

fn install_and_launch_simulator(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    runner: &mut dyn ToolRunner,
    destination: &IosDestination,
    installable_app: &Utf8Path,
    bundle_id: &str,
    launch_mode: LaunchMode,
) -> AtomResult<()> {
    let target_id = run_step(
        "Preparing simulator...",
        "Simulator ready",
        "Simulator preparation failed",
        || prepare_ios_simulator(repo_root, runner, destination),
    )?;
    install_and_launch_with_idb(
        repo_root,
        manifest,
        runner,
        &target_id,
        installable_app,
        bundle_id,
        launch_mode,
    )
}

fn install_and_launch_device(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    runner: &mut dyn ToolRunner,
    device_id: &str,
    installable_app: &Utf8Path,
    bundle_id: &str,
    launch_mode: LaunchMode,
) -> AtomResult<()> {
    install_and_launch_with_idb(
        repo_root,
        manifest,
        runner,
        device_id,
        installable_app,
        bundle_id,
        launch_mode,
    )
}

fn install_and_launch_with_idb(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    runner: &mut dyn ToolRunner,
    destination_id: &str,
    installable_app: &Utf8Path,
    bundle_id: &str,
    launch_mode: LaunchMode,
) -> AtomResult<()> {
    run_step(
        "Installing app...",
        "App installed",
        "Installation failed",
        || {
            run_tool(
                runner,
                repo_root,
                "idb",
                &[
                    "install",
                    "--udid",
                    destination_id,
                    installable_app.as_str(),
                ],
            )
        },
    )?;
    let _ = run_tool(
        runner,
        repo_root,
        "idb",
        &["terminate", "--udid", destination_id, bundle_id],
    );
    match launch_mode {
        LaunchMode::Attached => {
            eprintln!("→ Launching app and streaming logs... (Ctrl+C to stop)");
            stream_tool(
                runner,
                repo_root,
                "idb",
                &["launch", "-f", "-w", "--udid", destination_id, bundle_id],
            )
        }
        LaunchMode::Detached => {
            run_step("Launching app...", "App launched", "Launch failed", || {
                run_tool(
                    runner,
                    repo_root,
                    "idb",
                    &["launch", "-f", "--udid", destination_id, bundle_id],
                )?;
                wait_for_launch_ready(
                    repo_root,
                    destination_id,
                    &manifest.app.name,
                    &manifest.app.slug,
                    runner,
                )
            })
        }
    }
}

fn cleanup_ios_agent_device_command(
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    logs_started: bool,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    if logs_started {
        agent_device::stop_logs(repo_root, destination_id, session_name, runner)?;
    }
    agent_device::close_session(repo_root, destination_id, session_name, runner)
}

fn resolve_ios_destination(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
    requested_destination: Option<&str>,
) -> AtomResult<IosDestination> {
    if let Some(requested_destination) = requested_destination {
        return resolve_requested_ios_destination(repo_root, runner, requested_destination);
    }

    let destinations = list_ios_destinations(repo_root, runner)?;
    if should_prompt_interactively() {
        return choose_from_menu(
            "Select iOS destination",
            &destinations,
            IosDestination::display_label,
        );
    }

    select_default_ios_destination(&destinations).ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            "idb list-targets did not report an available iOS target",
        )
    })
}

fn resolve_requested_ios_destination(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
    requested_destination: &str,
) -> AtomResult<IosDestination> {
    let destinations = list_ios_destinations(repo_root, runner)?;
    if requested_destination == "booted" {
        return select_booted_ios_destination(&destinations)
            .or_else(|| select_default_ios_destination(&destinations))
            .ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    "idb list-targets did not report a booted iOS target",
                )
            });
    }

    if let Some(destination) = destinations
        .into_iter()
        .find(|destination| destination.matches_identifier(requested_destination))
    {
        return Ok(destination);
    }

    Ok(IosDestination {
        kind: IosDestinationKind::Device,
        id: requested_destination.to_owned(),
        alternate_id: None,
        name: requested_destination.to_owned(),
        state: "requested".to_owned(),
        runtime: None,
        architecture: None,
        is_available: true,
    })
}

fn list_ios_destinations(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
) -> AtomResult<Vec<IosDestination>> {
    let mut destinations = if agent_device::is_available(repo_root, runner) {
        agent_device::list_destinations(repo_root, runner)?
            .into_iter()
            .map(|destination| match destination.kind {
                agent_device::AgentDeviceDestinationKind::Simulator => IosDestination {
                    kind: IosDestinationKind::Simulator,
                    id: destination.id,
                    alternate_id: None,
                    name: destination.name,
                    state: if destination.booted {
                        "Booted".to_owned()
                    } else {
                        "Shutdown".to_owned()
                    },
                    runtime: None,
                    architecture: None,
                    is_available: true,
                },
                agent_device::AgentDeviceDestinationKind::Device => IosDestination {
                    kind: IosDestinationKind::Device,
                    id: destination.id,
                    alternate_id: None,
                    name: destination.name,
                    state: if destination.booted {
                        "Ready".to_owned()
                    } else {
                        "Disconnected".to_owned()
                    },
                    runtime: None,
                    architecture: None,
                    is_available: true,
                },
            })
            .collect()
    } else {
        parse_idb_targets(&capture_tool(
            runner,
            repo_root,
            "idb",
            &["list-targets", "--json"],
        )?)?
    };
    sort_ios_destinations(&mut destinations);
    Ok(destinations)
}

fn select_booted_ios_destination(destinations: &[IosDestination]) -> Option<IosDestination> {
    destinations
        .iter()
        .find(|destination| destination.is_booted_simulator())
        .cloned()
}

fn select_default_ios_destination(destinations: &[IosDestination]) -> Option<IosDestination> {
    let mut simulators = destinations
        .iter()
        .filter(|destination| destination.kind == IosDestinationKind::Simulator)
        .cloned()
        .collect::<Vec<_>>();
    simulators.sort_by(|left, right| {
        right
            .runtime
            .cmp(&left.runtime)
            .then_with(|| right.is_booted_simulator().cmp(&left.is_booted_simulator()))
            .then_with(|| left.name.cmp(&right.name))
    });

    select_booted_ios_destination(&simulators)
        .or_else(|| {
            simulators
                .iter()
                .find(|simulator| simulator.name.contains("iPhone"))
                .cloned()
        })
        .or_else(|| simulators.first().cloned())
}

fn sort_ios_destinations(destinations: &mut [IosDestination]) {
    destinations.sort_by(|left, right| {
        right
            .is_available
            .cmp(&left.is_available)
            .then_with(|| right.is_booted_simulator().cmp(&left.is_booted_simulator()))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| right.runtime.cmp(&left.runtime))
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn prepare_ios_simulator(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
    destination: &IosDestination,
) -> AtomResult<String> {
    let simulator = destination.id.clone();
    if !destination.is_booted_simulator() {
        run_tool(runner, repo_root, "idb", &["boot", &simulator])?;
    }
    Ok(simulator)
}

fn parse_idb_targets(json: &str) -> AtomResult<Vec<IosDestination>> {
    let targets = json
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Value>(line).map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to parse idb target JSON: {error}"),
                )
            })
        })
        .collect::<AtomResult<Vec<_>>>()?;

    let mut destinations = Vec::new();
    for target in targets {
        let Some(id) = target.get("udid").and_then(Value::as_str) else {
            continue;
        };
        let name = target
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_owned();
        let state = target
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_owned();
        let target_type = target
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("simulator");
        let kind = if target_type == "device" {
            IosDestinationKind::Device
        } else {
            IosDestinationKind::Simulator
        };
        let runtime = target
            .get("os_version")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let architecture = target
            .get("architecture")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let is_available = target
            .get("state")
            .and_then(Value::as_str)
            .is_some_and(|value| value != "Unavailable");

        destinations.push(IosDestination {
            kind,
            id: id.to_owned(),
            alternate_id: target
                .get("device")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            name,
            state,
            runtime,
            architecture,
            is_available,
        });
    }
    Ok(destinations)
}

fn ios_bazel_args(
    target: &str,
    destination: &IosDestination,
    build_profile: AppSessionBuildProfile,
) -> Vec<String> {
    let cpu = match destination.kind {
        IosDestinationKind::Simulator => "sim_arm64,x86_64",
        IosDestinationKind::Device => "arm64",
    };
    let mut args = vec![
        "build".to_owned(),
        target.to_owned(),
        format!("--ios_multi_cpus={cpu}"),
    ];
    if build_profile == AppSessionBuildProfile::Debugger {
        args.push("--compilation_mode=dbg".to_owned());
        args.push("--apple_generate_dsym".to_owned());
        args.push("--output_groups=+dsyms".to_owned());
    }
    args
}

fn build_ios_artifacts(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination: &IosDestination,
    build_profile: AppSessionBuildProfile,
    runner: &mut dyn ToolRunner,
) -> AtomResult<IosBuildArtifacts> {
    let target = generated_target(manifest, BACKEND_ID);
    let build_args = ios_bazel_args(&target, destination, build_profile);
    run_bazel_owned(runner, repo_root, &build_args)?;
    resolve_ios_build_artifacts(repo_root, &build_args, build_profile, runner)
}

fn resolve_ios_build_artifacts(
    repo_root: &Utf8Path,
    build_args: &[String],
    build_profile: AppSessionBuildProfile,
    runner: &mut dyn ToolRunner,
) -> AtomResult<IosBuildArtifacts> {
    let expression = if build_profile == AppSessionBuildProfile::Debugger {
        r#"providers(target)["@@rules_apple+//apple/internal:providers.bzl%AppleBundleInfo"].archive.path + "\n" + providers(target)["@@rules_apple+//apple/internal:providers.bzl%AppleDsymBundleInfo"].direct_dsyms[0].path"#
    } else {
        r#"providers(target)["@@rules_apple+//apple/internal:providers.bzl%AppleBundleInfo"].archive.path"#
    };
    let output = capture_bazel_cquery_starlark_owned(runner, repo_root, build_args, expression)?;
    let mut paths = parse_bazel_output_paths(repo_root, &output, "iOS debugger artifact")?;
    let archive = paths.first().cloned().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            "bazelisk cquery did not return an iOS archive",
        )
    })?;
    let dsym_bundle = if build_profile == AppSessionBuildProfile::Debugger {
        if paths.len() != 2 {
            return Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "bazelisk cquery did not return exactly one iOS dSYM bundle",
            ));
        }
        Some(paths.remove(1))
    } else {
        None
    };

    Ok(IosBuildArtifacts {
        installable_app: resolve_ios_installable_artifact(&archive)?,
        dsym_bundle,
    })
}

fn resolve_ios_installable_artifact(path: &Utf8Path) -> AtomResult<Utf8PathBuf> {
    if path.extension() == Some("app") {
        return Ok(path.to_owned());
    }
    if path.extension() != Some("ipa") {
        return Err(AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "bazelisk did not produce an installable iOS artifact",
            path.as_str(),
        ));
    }

    let parent = path.parent().ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "bazelisk returned an invalid iOS artifact path",
            path.as_str(),
        )
    })?;

    if let Some(app) = find_descendant_with_suffix(parent, ".app")? {
        return Ok(app);
    }

    let extract_dir = parent.join("_ipa_extract");
    let _ = fs::remove_dir_all(&extract_dir);
    let status = Command::new("unzip")
        .args(["-q", "-o", path.as_str(), "-d", extract_dir.as_str()])
        .status()
        .map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to unzip .ipa: {error}"),
                path.as_str(),
            )
        })?;
    if !status.success() {
        return Err(AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "failed to unzip .ipa archive",
            path.as_str(),
        ));
    }

    find_descendant_with_suffix(&extract_dir, ".app")?.ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "unzipped .ipa did not contain a .app bundle",
            path.as_str(),
        )
    })
}

fn find_descendant_with_suffix(root: &Utf8Path, suffix: &str) -> AtomResult<Option<Utf8PathBuf>> {
    for entry in fs::read_dir(root).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to inspect generated iOS outputs: {error}"),
            root.as_str(),
        )
    })? {
        let entry = entry.map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to inspect generated iOS outputs: {error}"),
                root.as_str(),
            )
        })?;
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|_| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                "generated iOS output path was not valid UTF-8",
                root.as_str(),
            )
        })?;
        if path.as_str().ends_with(suffix) {
            return Ok(Some(path));
        }
        if path.is_dir()
            && let Some(found) = find_descendant_with_suffix(&path, suffix)?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn ios_app_is_running(
    repo_root: &Utf8Path,
    runner: &mut dyn ToolRunner,
    destination_id: &str,
    bundle_id: &str,
) -> AtomResult<bool> {
    let output = capture_tool(
        runner,
        repo_root,
        "idb",
        &["list-apps", "--udid", destination_id],
    )?;
    Ok(output.lines().any(|line| {
        let mut fields = line.split('|').map(str::trim);
        let identifier = fields.next();
        let _name = fields.next();
        let _install_type = fields.next();
        let _architectures = fields.next();
        let debug_state = fields.next();
        identifier == Some(bundle_id) && debug_state == Some("Running")
    }))
}

fn launch_ios_app(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination: &IosDestination,
    build_profile: AppSessionBuildProfile,
    runner: &mut dyn ToolRunner,
) -> AtomResult<IosAppLaunch> {
    let artifacts = build_ios_artifacts(repo_root, manifest, destination, build_profile, runner)?;
    let bundle_id = manifest
        .ios
        .bundle_id
        .clone()
        .ok_or_else(|| AtomError::new(AtomErrorCode::InternalBug, "missing iOS bundle id"))?;

    let destination_id = match destination.kind {
        IosDestinationKind::Simulator => prepare_ios_simulator(repo_root, runner, destination)?,
        IosDestinationKind::Device => destination.id.clone(),
    };
    run_idb(
        runner,
        repo_root,
        &destination_id,
        &[
            "install".to_owned(),
            artifacts.installable_app.as_str().to_owned(),
        ],
    )?;
    let _ = run_idb(
        runner,
        repo_root,
        &destination_id,
        &["terminate".to_owned(), bundle_id.clone()],
    );
    run_idb(
        runner,
        repo_root,
        &destination_id,
        &["launch".to_owned(), "-f".to_owned(), bundle_id.clone()],
    )?;

    Ok(IosAppLaunch {
        destination_id,
        bundle_id,
        app_name: manifest.app.name.clone(),
        app_slug: manifest.app.slug.clone(),
    })
}

fn attach_ios_app(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination_id: &str,
    runner: &mut dyn ToolRunner,
) -> AtomResult<Option<IosAppLaunch>> {
    let Some(bundle_id) = manifest.ios.bundle_id.clone() else {
        return Ok(None);
    };
    let snapshot = inspect_ui_with_idb(repo_root, destination_id, runner)?;
    if !snapshot_matches_ios_app(&snapshot, &manifest.app.name, &manifest.app.slug)
        || !snapshot_is_launch_ready(&snapshot)
    {
        return Ok(None);
    }
    Ok(Some(IosAppLaunch {
        destination_id: destination_id.to_owned(),
        bundle_id,
        app_name: manifest.app.name.clone(),
        app_slug: manifest.app.slug.clone(),
    }))
}

fn wait_for_launch_ready(
    repo_root: &Utf8Path,
    destination_id: &str,
    app_name: &str,
    app_slug: &str,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    let deadline = Instant::now() + APP_LAUNCH_READY_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(snapshot) = inspect_ui_with_idb(repo_root, destination_id, runner)
            && snapshot_matches_ios_app(&snapshot, app_name, app_slug)
            && snapshot_is_launch_ready(&snapshot)
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

fn launch_ios_app_with_agent_device(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination: &IosDestination,
    session_name: &str,
    build_profile: AppSessionBuildProfile,
    runner: &mut dyn ToolRunner,
) -> AtomResult<IosAppLaunch> {
    let artifacts = build_ios_artifacts(repo_root, manifest, destination, build_profile, runner)?;
    let bundle_id = manifest
        .ios
        .bundle_id
        .clone()
        .ok_or_else(|| AtomError::new(AtomErrorCode::InternalBug, "missing iOS bundle id"))?;
    let destination_id = match destination.kind {
        IosDestinationKind::Simulator => prepare_ios_simulator(repo_root, runner, destination)?,
        IosDestinationKind::Device => destination.id.clone(),
    };
    agent_device::reinstall_app(
        repo_root,
        &destination_id,
        session_name,
        &bundle_id,
        &artifacts.installable_app,
        runner,
    )?;
    let _ = agent_device::close_session(repo_root, &destination_id, session_name, runner);
    agent_device::open_app(
        repo_root,
        &destination_id,
        session_name,
        &bundle_id,
        false,
        runner,
    )?;
    wait_for_launch_ready_with_agent_device(
        repo_root,
        &destination_id,
        session_name,
        &manifest.app.name,
        &manifest.app.slug,
        runner,
    )?;
    Ok(IosAppLaunch {
        destination_id,
        bundle_id,
        app_name: manifest.app.name.clone(),
        app_slug: manifest.app.slug.clone(),
    })
}

fn attach_ios_app_with_agent_device(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination: &IosDestination,
    session_name: &str,
    runner: &mut dyn ToolRunner,
) -> AtomResult<Option<IosAppLaunch>> {
    let Some(bundle_id) = manifest.ios.bundle_id.clone() else {
        return Ok(None);
    };
    let destination_id = match destination.kind {
        IosDestinationKind::Simulator => prepare_ios_simulator(repo_root, runner, destination)?,
        IosDestinationKind::Device => destination.id.clone(),
    };
    if agent_device::open_app(
        repo_root,
        &destination_id,
        session_name,
        &bundle_id,
        false,
        runner,
    )
    .is_err()
    {
        let _ = agent_device::close_session(repo_root, &destination_id, session_name, runner);
        return Ok(None);
    }
    wait_for_launch_ready_with_agent_device(
        repo_root,
        &destination_id,
        session_name,
        &manifest.app.name,
        &manifest.app.slug,
        runner,
    )?;
    Ok(Some(IosAppLaunch {
        destination_id,
        bundle_id,
        app_name: manifest.app.name.clone(),
        app_slug: manifest.app.slug.clone(),
    }))
}

fn wait_for_launch_ready_with_agent_device(
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    app_name: &str,
    app_slug: &str,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    let deadline = Instant::now() + APP_LAUNCH_READY_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(snapshot) = agent_device::inspect_ui_with_agent_device(
            repo_root,
            destination_id,
            session_name,
            runner,
        ) && snapshot_matches_ios_app(&snapshot, app_name, app_slug)
            && snapshot_is_launch_ready(&snapshot)
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

#[expect(
    clippy::too_many_lines,
    reason = "The idb adapter keeps per-command translation in one place for the iOS backend."
)]
fn interact_with_idb(
    repo_root: &Utf8Path,
    destination_id: &str,
    runner: &mut dyn ToolRunner,
    request: InteractionRequest,
) -> AtomResult<InteractionResult> {
    match request {
        InteractionRequest::InspectUi => Ok(InteractionResult {
            ok: true,
            snapshot: inspect_ui_with_idb(repo_root, destination_id, runner)?,
            message: None,
        }),
        InteractionRequest::Tap { target_id, x, y } => {
            let snapshot = inspect_ui_with_idb(repo_root, destination_id, runner)?;
            let (tap_x, tap_y) = resolve_interaction_point(&snapshot, target_id.as_deref(), x, y)?;
            run_idb(
                runner,
                repo_root,
                destination_id,
                &[
                    "ui".to_owned(),
                    "tap".to_owned(),
                    format_coordinate(tap_x),
                    format_coordinate(tap_y),
                ],
            )?;
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_idb(repo_root, destination_id, runner)?,
                message: None,
            })
        }
        InteractionRequest::LongPress { target_id, x, y } => {
            let snapshot = inspect_ui_with_idb(repo_root, destination_id, runner)?;
            let (tap_x, tap_y) = resolve_interaction_point(&snapshot, target_id.as_deref(), x, y)?;
            run_idb(
                runner,
                repo_root,
                destination_id,
                &[
                    "ui".to_owned(),
                    "tap".to_owned(),
                    "--duration".to_owned(),
                    "1.0".to_owned(),
                    format_coordinate(tap_x),
                    format_coordinate(tap_y),
                ],
            )?;
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_idb(repo_root, destination_id, runner)?,
                message: None,
            })
        }
        InteractionRequest::TypeText { target_id, text } => {
            if let Some(target_id) = target_id.as_deref() {
                let snapshot = inspect_ui_with_idb(repo_root, destination_id, runner)?;
                let (tap_x, tap_y) =
                    resolve_interaction_point(&snapshot, Some(target_id), None, None)?;
                run_idb(
                    runner,
                    repo_root,
                    destination_id,
                    &[
                        "ui".to_owned(),
                        "tap".to_owned(),
                        format_coordinate(tap_x),
                        format_coordinate(tap_y),
                    ],
                )?;
            }
            run_idb(
                runner,
                repo_root,
                destination_id,
                &["ui".to_owned(), "text".to_owned(), text],
            )?;
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_idb(repo_root, destination_id, runner)?,
                message: None,
            })
        }
        InteractionRequest::Swipe { x, y } | InteractionRequest::Drag { x, y } => {
            let snapshot = inspect_ui_with_idb(repo_root, destination_id, runner)?;
            let start_x = snapshot.screen.width / 2.0;
            let start_y = snapshot.screen.height * 0.75;
            let end_x = x.unwrap_or(start_x);
            let end_y = y.unwrap_or(snapshot.screen.height * 0.25);
            run_idb(
                runner,
                repo_root,
                destination_id,
                &[
                    "ui".to_owned(),
                    "swipe".to_owned(),
                    format_coordinate(start_x),
                    format_coordinate(start_y),
                    format_coordinate(end_x),
                    format_coordinate(end_y),
                ],
            )?;
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_idb(repo_root, destination_id, runner)?,
                message: None,
            })
        }
    }
}

fn inspect_ui_with_idb(
    repo_root: &Utf8Path,
    destination_id: &str,
    runner: &mut dyn ToolRunner,
) -> AtomResult<UiSnapshot> {
    let raw = capture_idb(
        runner,
        repo_root,
        destination_id,
        &["ui".to_owned(), "describe-all".to_owned()],
    )?;
    let parsed: Value = serde_json::from_str(&raw).map_err(|error| {
        AtomError::new(
            AtomErrorCode::AutomationUnavailable,
            format!("failed to parse idb accessibility JSON: {error}"),
        )
    })?;
    let nodes = idb_elements(&parsed)
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| idb_node_from_value(entry, index))
        .collect::<Vec<_>>();

    let mut width = 0.0_f64;
    let mut height = 0.0_f64;
    for node in &nodes {
        width = width.max(node.bounds.x + node.bounds.width);
        height = height.max(node.bounds.y + node.bounds.height);
    }

    Ok(UiSnapshot {
        screen: ScreenInfo {
            width: width.max(1.0),
            height: height.max(1.0),
        },
        nodes,
        screenshot_path: None,
    })
}

fn idb_elements(parsed: &Value) -> &[Value] {
    parsed
        .get("elements")
        .and_then(Value::as_array)
        .or_else(|| parsed.as_array())
        .map_or(&[], Vec::as_slice)
}

fn idb_node_from_value(entry: &Value, index: usize) -> Option<UiNode> {
    let bounds = entry.get("frame").and_then(Value::as_object)?;
    let x = json_f64(bounds.get("x"))?;
    let y = json_f64(bounds.get("y"))?;
    let width = json_f64(bounds.get("width"))?;
    let height = json_f64(bounds.get("height"))?;
    let label = json_string(entry.get("AXLabel"))
        .or_else(|| json_string(entry.get("label")))
        .unwrap_or_default();
    let text = json_string(entry.get("AXValue"))
        .or_else(|| json_string(entry.get("value")))
        .unwrap_or_else(|| label.clone());
    Some(UiNode {
        id: json_string(entry.get("AXUniqueId"))
            .or_else(|| json_string(entry.get("identifier")))
            .unwrap_or_else(|| format!("idb-node-{index}")),
        role: json_string(entry.get("type"))
            .or_else(|| json_string(entry.get("role_description")))
            .or_else(|| json_string(entry.get("AXRoleDescription")))
            .unwrap_or_else(|| "unknown".to_owned()),
        label,
        text,
        visible: entry
            .get("visible")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        enabled: entry
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        bounds: UiBounds {
            x,
            y,
            width,
            height,
        },
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

fn json_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        Some(Value::Bool(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn json_f64(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(value)) => value.as_f64(),
        Some(Value::String(value)) => value.parse::<f64>().ok(),
        _ => None,
    }
}

fn format_coordinate(value: f64) -> String {
    value.round().to_string()
}

fn run_idb(
    runner: &mut dyn ToolRunner,
    repo_root: &Utf8Path,
    destination_id: &str,
    subcommand: &[String],
) -> AtomResult<()> {
    let args = idb_args(destination_id, subcommand);
    runner.run(repo_root, "idb", &args)
}

fn capture_idb(
    runner: &mut dyn ToolRunner,
    repo_root: &Utf8Path,
    destination_id: &str,
    subcommand: &[String],
) -> AtomResult<String> {
    let args = idb_args(destination_id, subcommand);
    runner.capture(repo_root, "idb", &args)
}

fn idb_args(destination_id: &str, subcommand: &[String]) -> Vec<String> {
    let insert_at = if matches!(subcommand.first().map(String::as_str), Some("ui")) {
        2
    } else {
        1
    };
    let mut args = Vec::with_capacity(subcommand.len() + 2);
    let split = insert_at.min(subcommand.len());
    args.extend(subcommand[..split].iter().cloned());
    args.push("--udid".to_owned());
    args.push(destination_id.to_owned());
    args.extend(subcommand[split..].iter().cloned());
    args
}

fn snapshot_matches_ios_app(snapshot: &UiSnapshot, app_name: &str, app_slug: &str) -> bool {
    snapshot.nodes.iter().any(|node| {
        node.role.eq_ignore_ascii_case("application")
            && [node.label.as_str(), node.text.as_str()]
                .into_iter()
                .any(|value| {
                    let value = value.trim();
                    !value.is_empty()
                        && (value.eq_ignore_ascii_case(app_name)
                            || value.eq_ignore_ascii_case(app_slug))
                })
    })
}

fn snapshot_is_launch_ready(snapshot: &UiSnapshot) -> bool {
    snapshot.nodes.iter().any(|node| {
        !node.role.eq_ignore_ascii_case("application")
            && (node.bounds.width > 1.0 || node.bounds.height > 1.0)
            && (!node.label.is_empty() || !node.text.is_empty())
    })
}

fn capture_screenshot_for_launch(
    repo_root: &Utf8Path,
    launch: &IosAppLaunch,
    output_path: &Utf8Path,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    capture_ios_screenshot(repo_root, &launch.destination_id, output_path, runner)
}

fn capture_ios_screenshot(
    repo_root: &Utf8Path,
    destination_id: &str,
    output_path: &Utf8Path,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    match run_idb(
        runner,
        repo_root,
        destination_id,
        &["screenshot".to_owned(), output_path.as_str().to_owned()],
    ) {
        Ok(()) => Ok(()),
        Err(idb_error) => run_simctl_screenshot_with_retry(
            runner,
            repo_root,
            destination_id,
            output_path,
            SCREENSHOT_READY_TIMEOUT,
        )
        .map_err(|simctl_error| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                format!(
                    "failed to capture iOS simulator screenshot via idb ({}) or simctl ({})",
                    idb_error.message, simctl_error.message
                ),
                output_path.as_str(),
            )
        }),
    }
}

fn run_simctl_screenshot_with_retry(
    runner: &mut dyn ToolRunner,
    repo_root: &Utf8Path,
    destination_id: &str,
    output_path: &Utf8Path,
    timeout: Duration,
) -> AtomResult<()> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    while Instant::now() < deadline {
        let args = vec![
            "simctl".to_owned(),
            "io".to_owned(),
            destination_id.to_owned(),
            "screenshot".to_owned(),
            output_path.as_str().to_owned(),
        ];
        match runner.run(repo_root, "xcrun", &args) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(SCREENSHOT_READY_POLL_INTERVAL);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "failed to capture iOS simulator screenshot via simctl",
            output_path.as_str(),
        )
    }))
}

fn capture_logs_for_launch(
    repo_root: &Utf8Path,
    launch: &IosAppLaunch,
    output_path: &Utf8Path,
    seconds: u64,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    let contents = capture_ios_logs_for_launch(
        runner,
        repo_root,
        &launch.destination_id,
        &launch.bundle_id,
        &launch.app_name,
        &launch.app_slug,
        seconds,
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

fn capture_logs_with_agent_device(
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    output_path: &Utf8Path,
    seconds: u64,
    runner: &mut dyn ToolRunner,
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    agent_device::start_logs(repo_root, destination_id, session_name, runner).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::AutomationLogCaptureFailed,
            format!("failed to start log capture: {}", error.message),
            output_path.as_str(),
        )
    })?;
    thread::sleep(Duration::from_secs(seconds));
    agent_device::stop_logs(repo_root, destination_id, session_name, runner).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::AutomationLogCaptureFailed,
            format!("failed to stop log capture: {}", error.message),
            output_path.as_str(),
        )
    })?;
    let log_path = agent_device::logs_path(repo_root, destination_id, session_name, runner)
        .map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::AutomationLogCaptureFailed,
                format!("failed to resolve log path: {}", error.message),
                output_path.as_str(),
            )
        })?;
    fs::copy(&log_path, output_path).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::AutomationLogCaptureFailed,
            format!("failed to copy captured logs: {error}"),
            output_path.as_str(),
        )
    })?;
    Ok(())
}

fn capture_ios_logs_for_launch(
    runner: &mut dyn ToolRunner,
    repo_root: &Utf8Path,
    destination_id: &str,
    bundle_id: &str,
    app_name: &str,
    app_slug: &str,
    seconds: u64,
) -> AtomResult<String> {
    let timeout = format!("{seconds}s");
    let process_scoped = capture_idb(
        runner,
        repo_root,
        destination_id,
        &[
            "log".to_owned(),
            "--".to_owned(),
            "--style".to_owned(),
            "syslog".to_owned(),
            "--process".to_owned(),
            app_slug.to_owned(),
            "--timeout".to_owned(),
            timeout.clone(),
        ],
    );

    let contents = match process_scoped {
        Ok(contents) => contents,
        Err(_) => capture_idb(
            runner,
            repo_root,
            destination_id,
            &[
                "log".to_owned(),
                "--".to_owned(),
                "--style".to_owned(),
                "syslog".to_owned(),
                "--timeout".to_owned(),
                timeout,
            ],
        )?,
    };

    let filtered = filter_ios_log_lines(
        &contents,
        &[bundle_id, app_name, app_slug, "AtomRuntime", "atom_runtime"],
    );
    Ok(if filtered.is_empty() {
        contents
    } else {
        filtered
    })
}

fn filter_ios_log_lines(contents: &str, tokens: &[&str]) -> String {
    let tokens = tokens
        .iter()
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let filtered = contents
        .lines()
        .filter(|line| {
            let lowered = line.to_ascii_lowercase();
            tokens.iter().any(|token| lowered.contains(token))
        })
        .collect::<Vec<_>>();
    if filtered.is_empty() {
        String::new()
    } else {
        let mut joined = filtered.join("\n");
        joined.push('\n');
        joined
    }
}

fn capture_video_for_launch(
    repo_root: &Utf8Path,
    launch: &IosAppLaunch,
    output_path: &Utf8Path,
    seconds: u64,
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    let mut child = spawn_idb_video(repo_root, &launch.destination_id, output_path)?;
    thread::sleep(Duration::from_secs(seconds));
    stop_recording_process(repo_root, &mut child)?;
    ensure_video_artifact(output_path)?;
    Ok(())
}

fn start_video_capture(
    repo_root: &Utf8Path,
    launch: &IosAppLaunch,
    output_path: &Utf8Path,
) -> AtomResult<VideoCapture> {
    write_parent_dir(output_path)?;
    let child = spawn_idb_video(repo_root, &launch.destination_id, output_path)?;
    Ok(VideoCapture::Idb {
        output_path: output_path.to_owned(),
        child,
    })
}

fn spawn_idb_video(
    repo_root: &Utf8Path,
    destination_id: &str,
    output_path: &Utf8Path,
) -> AtomResult<Child> {
    Command::new("idb")
        .args(["video", "--udid", destination_id, output_path.as_str()])
        .current_dir(repo_root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to start iOS video capture: {error}"),
            )
        })
}

fn stop_video_capture(repo_root: &Utf8Path, video: VideoCapture) -> AtomResult<Utf8PathBuf> {
    match video {
        VideoCapture::AgentDevice {
            output_path,
            destination_id,
            session_name,
        } => {
            let mut runner = atom_deploy::ProcessRunner;
            agent_device::stop_video_capture(
                repo_root,
                &destination_id,
                &session_name,
                &mut runner,
            )?;
            ensure_video_artifact(&output_path)?;
            Ok(output_path)
        }
        VideoCapture::Idb {
            output_path,
            mut child,
        } => {
            stop_recording_process(repo_root, &mut child)?;
            ensure_video_artifact(&output_path)?;
            Ok(output_path)
        }
    }
}

fn stop_recording_process(repo_root: &Utf8Path, child: &mut Child) -> AtomResult<()> {
    if wait_for_child_exit(child, Duration::from_millis(100))? {
        return Ok(());
    }

    let _ = signal_child(repo_root, child, "INT");
    if wait_for_child_exit(child, VIDEO_STOP_TIMEOUT)? {
        return Ok(());
    }

    let _ = signal_child(repo_root, child, "TERM");
    if wait_for_child_exit(child, VIDEO_STOP_TIMEOUT)? {
        return Ok(());
    }

    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

fn signal_child(repo_root: &Utf8Path, child: &Child, signal: &str) -> AtomResult<()> {
    let status = Command::new("/bin/kill")
        .args([format!("-{signal}"), child.id().to_string()])
        .current_dir(repo_root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to signal recorder process: {error}"),
            )
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to signal recorder process with SIG{signal}"),
        ))
    }
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

fn sanitize_session_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn timestamp_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

#[cfg(test)]
mod tests {
    use atom_backends::{AppSessionBuildProfile, DestinationCapability};

    use super::{
        BACKEND_ID, IosDestination, IosDestinationKind, destination_descriptor_from_ios,
        ios_bazel_args, parse_idb_targets,
    };

    #[test]
    fn parses_idb_targets_into_backend_destinations() {
        let json = r#"{"udid":"SIM-1","name":"Fixture Sim","state":"Booted","type":"simulator","os_version":"17.0","architecture":"arm64"}
{"udid":"DEV-1","name":"Fixture Phone","state":"Shutdown","type":"device","device":"physical-1"}"#;

        let destinations = parse_idb_targets(json).expect("targets should parse");

        assert_eq!(destinations.len(), 2);
        assert_eq!(destinations[0].id, "SIM-1");
        assert_eq!(destinations[0].kind, IosDestinationKind::Simulator);
        assert_eq!(destinations[1].alternate_id.as_deref(), Some("physical-1"));
        assert_eq!(destinations[1].kind, IosDestinationKind::Device);
    }

    #[test]
    fn simulator_descriptors_expose_automation_capabilities() {
        let descriptor = destination_descriptor_from_ios(
            IosDestination {
                kind: IosDestinationKind::Simulator,
                id: "SIM-1".to_owned(),
                alternate_id: None,
                name: "Fixture Sim".to_owned(),
                state: "Booted".to_owned(),
                runtime: Some("17.0".to_owned()),
                architecture: Some("arm64".to_owned()),
                is_available: true,
            },
            false,
        );

        assert_eq!(descriptor.platform, "ios");
        assert_eq!(descriptor.backend_id, BACKEND_ID);
        assert_eq!(descriptor.kind, "simulator");
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
    fn device_descriptors_limit_capabilities_to_launch() {
        let descriptor = destination_descriptor_from_ios(
            IosDestination {
                kind: IosDestinationKind::Device,
                id: "DEV-1".to_owned(),
                alternate_id: None,
                name: "Fixture Phone".to_owned(),
                state: "Ready".to_owned(),
                runtime: None,
                architecture: None,
                is_available: true,
            },
            false,
        );

        assert_eq!(descriptor.platform, "ios");
        assert_eq!(descriptor.kind, "device");
        assert_eq!(descriptor.capabilities, vec![DestinationCapability::Launch]);
    }

    #[test]
    fn device_descriptors_expose_agent_device_capabilities_when_available() {
        let descriptor = destination_descriptor_from_ios(
            IosDestination {
                kind: IosDestinationKind::Device,
                id: "DEV-1".to_owned(),
                alternate_id: None,
                name: "Fixture Phone".to_owned(),
                state: "Ready".to_owned(),
                runtime: None,
                architecture: None,
                is_available: true,
            },
            true,
        );

        assert!(
            descriptor
                .capabilities
                .contains(&DestinationCapability::Logs)
        );
        assert!(
            descriptor
                .capabilities
                .contains(&DestinationCapability::InspectUi)
        );
        assert!(
            descriptor
                .capabilities
                .contains(&DestinationCapability::Evaluate)
        );
    }

    #[test]
    fn debugger_ios_builds_enable_dsym_generation() {
        let args = ios_bazel_args(
            "//apps/demo:demo",
            &IosDestination {
                kind: IosDestinationKind::Simulator,
                id: "SIM-1".to_owned(),
                alternate_id: None,
                name: "Fixture Sim".to_owned(),
                state: "Booted".to_owned(),
                runtime: Some("17.0".to_owned()),
                architecture: Some("arm64".to_owned()),
                is_available: true,
            },
            AppSessionBuildProfile::Debugger,
        );

        assert_eq!(
            args,
            vec![
                "build".to_owned(),
                "//apps/demo:demo".to_owned(),
                "--ios_multi_cpus=sim_arm64,x86_64".to_owned(),
                "--compilation_mode=dbg".to_owned(),
                "--apple_generate_dsym".to_owned(),
                "--output_groups=+dsyms".to_owned(),
            ]
        );
    }
}
