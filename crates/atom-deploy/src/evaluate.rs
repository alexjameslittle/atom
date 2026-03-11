use std::fs;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atom_backends::{
    BackendAutomationSession, DeployBackendRegistry, DestinationCapability, DestinationDescriptor,
    DestinationPlatform, ToolRunner,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;
use serde_json::Value;

mod android_uiautomator;

use self::android_uiautomator::{
    inspect_ui_with_android_uiautomator, interact_with_android_uiautomator,
};
use crate::deploy::{
    generated_target, ios_bazel_args, resolve_ios_installable_artifact, wait_for_app_pid,
};
use crate::destinations::list_backend_destinations;
use crate::devices::android::{find_android_destination, prepare_android_emulator};
use crate::devices::ios::{
    IosDestination, IosDestinationKind, list_ios_destinations, prepare_ios_simulator,
};
use crate::tools::{capture_tool, find_bazel_output_owned, run_bazel_owned, run_tool};

const APP_LAUNCH_READY_TIMEOUT: Duration = Duration::from_secs(15);
const APP_LAUNCH_READY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const IOS_SCREENSHOT_READY_TIMEOUT: Duration = Duration::from_secs(5);
const IOS_SCREENSHOT_READY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const VIDEO_STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub use atom_backends::{
    ArtifactRecord, EvaluationBundleManifest, EvaluationPlan, EvaluationStep, InteractionRequest,
    InteractionResult, ScreenInfo, SessionLaunchBehavior, StepRecord, UiBounds, UiNode, UiSnapshot,
};

pub struct EvaluateCommandOutput {
    pub manifest: EvaluationBundleManifest,
    pub manifest_path: Utf8PathBuf,
}

/// # Errors
///
/// Returns an error if destination resolution, app launch, or UI inspection fails.
pub fn inspect_ui(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    destination_id: &str,
    runner: &mut impl ToolRunner,
) -> AtomResult<UiSnapshot> {
    let descriptor =
        resolve_destination_descriptor(repo_root, registry, backend_id, destination_id, runner)?;
    require_capability(&descriptor, DestinationCapability::InspectUi)?;
    let mut session = AutomationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        SessionLaunchBehavior::AttachOrLaunch,
    )?;
    session.ensure_launched()?;
    let mut snapshot = session.interact(InteractionRequest::InspectUi)?;
    session.shutdown_video()?;
    if snapshot.snapshot.screenshot_path.is_none() {
        let screenshot = session.capture_auto_screenshot()?;
        snapshot.snapshot.screenshot_path = Some(screenshot.as_str().to_owned());
    }
    Ok(snapshot.snapshot)
}

/// # Errors
///
/// Returns an error if destination resolution, app launch, or the requested interaction fails.
pub fn interact(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    destination_id: &str,
    request: InteractionRequest,
    runner: &mut impl ToolRunner,
) -> AtomResult<InteractionResult> {
    let descriptor =
        resolve_destination_descriptor(repo_root, registry, backend_id, destination_id, runner)?;
    require_capability(&descriptor, DestinationCapability::Interact)?;
    let mut session = AutomationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        SessionLaunchBehavior::AttachOrLaunch,
    )?;
    session.ensure_launched()?;
    let result = session.interact(request)?;
    session.shutdown_video()?;
    Ok(result)
}

/// # Errors
///
/// Returns an error if destination resolution, app launch, or screenshot capture fails.
pub fn capture_screenshot(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    destination_id: &str,
    output_path: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let descriptor =
        resolve_destination_descriptor(repo_root, registry, backend_id, destination_id, runner)?;
    require_capability(&descriptor, DestinationCapability::Screenshot)?;
    let mut session = AutomationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        SessionLaunchBehavior::AttachOrLaunch,
    )?;
    session.ensure_launched()?;
    session.capture_screenshot(output_path)
}

/// # Errors
///
/// Returns an error if destination resolution, app launch, or log capture fails.
#[expect(
    clippy::too_many_arguments,
    reason = "The public evidence API keeps repo, manifest, registry, destination, and capture options explicit."
)]
pub fn capture_logs(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    destination_id: &str,
    output_path: &Utf8Path,
    seconds: u64,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let descriptor =
        resolve_destination_descriptor(repo_root, registry, backend_id, destination_id, runner)?;
    require_capability(&descriptor, DestinationCapability::Logs)?;
    let mut session = AutomationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        SessionLaunchBehavior::AttachOrLaunch,
    )?;
    session.ensure_launched()?;
    session.capture_logs(output_path, seconds)
}

/// # Errors
///
/// Returns an error if destination resolution, app launch, or video capture fails.
#[expect(
    clippy::too_many_arguments,
    reason = "The public evidence API keeps repo, manifest, registry, destination, and capture options explicit."
)]
pub fn capture_video(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    destination_id: &str,
    output_path: &Utf8Path,
    seconds: u64,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let descriptor =
        resolve_destination_descriptor(repo_root, registry, backend_id, destination_id, runner)?;
    require_capability(&descriptor, DestinationCapability::Video)?;
    let mut session = AutomationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        SessionLaunchBehavior::AttachOrLaunch,
    )?;
    session.ensure_launched()?;
    session.capture_video(output_path, seconds)
}

/// # Errors
///
/// Returns an error if the evaluation plan cannot be loaded, a required capability is unavailable,
/// or any step fails while executing.
#[expect(
    clippy::too_many_arguments,
    reason = "The public evaluation API keeps repo, manifest, registry, destination, plan, and artifact paths explicit."
)]
pub fn evaluate_run(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    destination_id: &str,
    plan_path: &Utf8Path,
    artifacts_dir: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<EvaluateCommandOutput> {
    let plan = load_evaluation_plan(plan_path)?;
    write_parent_dir(artifacts_dir)?;

    let descriptor =
        resolve_destination_descriptor(repo_root, registry, backend_id, destination_id, runner)?;
    require_plan_capabilities(&descriptor, &plan)?;

    let started_at_ms = timestamp_millis();
    let mut session = AutomationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        SessionLaunchBehavior::LaunchOnly,
    )?;
    let mut steps = Vec::new();
    let mut artifacts = Vec::new();

    for (index, step) in plan.steps.into_iter().enumerate() {
        let step_kind = step_kind(&step);
        let step_record = execute_step(index, step, artifacts_dir, &mut session, &mut artifacts)
            .map_err(|error| {
                AtomError::new(
                    error.code,
                    format!(
                        "evaluation step {index} ({step_kind}) failed: {}",
                        error.message
                    ),
                )
            })?;
        steps.push(step_record);
    }

    session.shutdown_video()?;
    let transcript_path = artifacts_dir.join("steps.json");
    write_json(&transcript_path, &steps)?;
    artifacts.push(ArtifactRecord {
        name: "steps.json".to_owned(),
        kind: "step_transcript".to_owned(),
        path: transcript_path.as_str().to_owned(),
    });
    let finished_at_ms = timestamp_millis();

    let bundle = EvaluationBundleManifest {
        target_label: manifest.target_label.clone(),
        destination: session.descriptor.clone(),
        started_at_ms,
        finished_at_ms,
        transcript_path: transcript_path.as_str().to_owned(),
        steps,
        artifacts,
    };
    let manifest_path = artifacts_dir.join("manifest.json");
    write_json(&manifest_path, &bundle)?;

    Ok(EvaluateCommandOutput {
        manifest: bundle,
        manifest_path,
    })
}

fn execute_step(
    index: usize,
    step: EvaluationStep,
    artifacts_dir: &Utf8Path,
    session: &mut AutomationSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
) -> AtomResult<StepRecord> {
    let started_at_ms = timestamp_millis();
    match step {
        EvaluationStep::Launch => execute_launch_step(index, started_at_ms, session),
        EvaluationStep::WaitForUi {
            target_id,
            text,
            timeout_ms,
        } => execute_wait_for_ui_step(
            index,
            started_at_ms,
            session,
            target_id.as_deref(),
            text.as_deref(),
            timeout_ms.unwrap_or(5_000),
        ),
        EvaluationStep::Tap { target_id, x, y } => execute_interaction_step(
            index,
            "tap",
            started_at_ms,
            session,
            InteractionRequest::Tap { target_id, x, y },
        ),
        EvaluationStep::LongPress { target_id, x, y } => execute_interaction_step(
            index,
            "long_press",
            started_at_ms,
            session,
            InteractionRequest::LongPress { target_id, x, y },
        ),
        EvaluationStep::Swipe { x, y } => execute_interaction_step(
            index,
            "swipe",
            started_at_ms,
            session,
            InteractionRequest::Swipe { x, y },
        ),
        EvaluationStep::Drag { x, y } => execute_interaction_step(
            index,
            "drag",
            started_at_ms,
            session,
            InteractionRequest::Drag { x, y },
        ),
        EvaluationStep::TypeText { target_id, text } => execute_interaction_step(
            index,
            "type_text",
            started_at_ms,
            session,
            InteractionRequest::TypeText { target_id, text },
        ),
        EvaluationStep::Screenshot { name } => execute_screenshot_step(
            index,
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            name,
        ),
        EvaluationStep::InspectUi { name } => execute_inspect_ui_step(
            index,
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            name,
        ),
        EvaluationStep::StartVideo { name } => {
            execute_start_video_step(index, started_at_ms, artifacts_dir, session, name)
        }
        EvaluationStep::StopVideo => {
            execute_stop_video_step(index, started_at_ms, session, artifacts)
        }
        EvaluationStep::CollectLogs { name, seconds } => execute_collect_logs_step(
            index,
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            name,
            seconds.unwrap_or(60),
        ),
    }
}

fn execute_launch_step(
    index: usize,
    started_at_ms: u128,
    session: &mut AutomationSession<'_>,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    Ok(simple_step(index, "launch", started_at_ms))
}

fn execute_wait_for_ui_step(
    index: usize,
    started_at_ms: u128,
    session: &mut AutomationSession<'_>,
    target_id: Option<&str>,
    text: Option<&str>,
    timeout_ms: u64,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    wait_for_ui(session, target_id, text, timeout_ms)?;
    Ok(simple_step(index, "wait_for_ui", started_at_ms))
}

fn execute_interaction_step(
    index: usize,
    kind: &str,
    started_at_ms: u128,
    session: &mut AutomationSession<'_>,
    request: InteractionRequest,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    session.interact(request)?;
    Ok(simple_step(index, kind, started_at_ms))
}

fn execute_screenshot_step(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut AutomationSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
    name: Option<String>,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    let artifact_name = artifact_name(name, index, "screenshot", "png");
    let output_path = artifacts_dir.join(&artifact_name);
    session.capture_screenshot(&output_path)?;
    artifacts.push(ArtifactRecord {
        name: artifact_name.clone(),
        kind: "screenshot".to_owned(),
        path: output_path.as_str().to_owned(),
    });
    Ok(step_with_artifacts(
        index,
        "screenshot",
        started_at_ms,
        vec![artifact_name],
    ))
}

fn execute_inspect_ui_step(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut AutomationSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
    name: Option<String>,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    let artifact_file_name = artifact_name(name, index, "inspect", "json");
    let output_path = artifacts_dir.join(&artifact_file_name);
    let mut snapshot = session.interact(InteractionRequest::InspectUi)?.snapshot;
    let screenshot_name = artifact_name(None, index, "inspect", "png");
    let screenshot_path = artifacts_dir.join(&screenshot_name);
    session.capture_screenshot(&screenshot_path)?;
    snapshot.screenshot_path = Some(screenshot_path.as_str().to_owned());
    write_json(&output_path, &snapshot)?;
    artifacts.push(ArtifactRecord {
        name: artifact_file_name.clone(),
        kind: "ui_snapshot".to_owned(),
        path: output_path.as_str().to_owned(),
    });
    artifacts.push(ArtifactRecord {
        name: screenshot_name.clone(),
        kind: "screenshot".to_owned(),
        path: screenshot_path.as_str().to_owned(),
    });
    Ok(step_with_artifacts(
        index,
        "inspect_ui",
        started_at_ms,
        vec![artifact_file_name, screenshot_name],
    ))
}

fn execute_start_video_step(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut AutomationSession<'_>,
    name: Option<String>,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    let artifact_name = video_artifact_name(name, index, session.video_extension());
    let output_path = artifacts_dir.join(&artifact_name);
    session.start_video(&output_path)?;
    Ok(step_with_artifacts(
        index,
        "start_video",
        started_at_ms,
        vec![artifact_name],
    ))
}

fn execute_stop_video_step(
    index: usize,
    started_at_ms: u128,
    session: &mut AutomationSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
) -> AtomResult<StepRecord> {
    let output_path = session.stop_video()?;
    let artifact_name = output_path
        .file_name()
        .map_or_else(|| output_path.as_str().to_owned(), ToOwned::to_owned);
    artifacts.push(ArtifactRecord {
        name: artifact_name.clone(),
        kind: "video".to_owned(),
        path: output_path.as_str().to_owned(),
    });
    Ok(step_with_artifacts(
        index,
        "stop_video",
        started_at_ms,
        vec![artifact_name],
    ))
}

fn execute_collect_logs_step(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut AutomationSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
    name: Option<String>,
    seconds: u64,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    let artifact_name = artifact_name(name, index, "logs", "txt");
    let output_path = artifacts_dir.join(&artifact_name);
    session.capture_logs(&output_path, seconds)?;
    artifacts.push(ArtifactRecord {
        name: artifact_name.clone(),
        kind: "logs".to_owned(),
        path: output_path.as_str().to_owned(),
    });
    Ok(step_with_artifacts(
        index,
        "collect_logs",
        started_at_ms,
        vec![artifact_name],
    ))
}

fn wait_for_ui(
    session: &mut AutomationSession<'_>,
    target_id: Option<&str>,
    text: Option<&str>,
    timeout_ms: u64,
) -> AtomResult<()> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        let snapshot = session.interact(InteractionRequest::InspectUi)?.snapshot;
        if snapshot_matches(&snapshot, target_id, text) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(150));
    }
    Err(AtomError::new(
        AtomErrorCode::AutomationTargetNotFound,
        "wait_for_ui timed out before the requested target appeared",
    ))
}

fn snapshot_matches(snapshot: &UiSnapshot, target_id: Option<&str>, text: Option<&str>) -> bool {
    snapshot.nodes.iter().any(|node| {
        let id_matches = target_id.is_none_or(|target_id| node.id == target_id);
        let text_matches = text.is_none_or(|text| node.text == text || node.label == text);
        id_matches && text_matches
    })
}

fn snapshot_is_launch_ready(snapshot: &UiSnapshot) -> bool {
    snapshot.nodes.iter().any(|node| {
        !node.role.eq_ignore_ascii_case("application")
            && (node.bounds.width > 1.0 || node.bounds.height > 1.0)
            && (!node.label.is_empty() || !node.text.is_empty())
    })
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

fn wait_for_idb_launch_ready(
    repo_root: &Utf8Path,
    destination_id: &str,
    app_name: &str,
    app_slug: &str,
    runner: &mut (impl ToolRunner + ?Sized),
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

pub(crate) fn wait_for_ios_launch_ready(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination_id: &str,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    wait_for_idb_launch_ready(
        repo_root,
        destination_id,
        &manifest.app.name,
        &manifest.app.slug,
        runner,
    )
}

fn wait_for_android_launch_ready(
    repo_root: &Utf8Path,
    serial: &str,
    application_id: &str,
    runner: &mut (impl ToolRunner + ?Sized),
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

fn simple_step(index: usize, kind: &str, started_at_ms: u128) -> StepRecord {
    StepRecord {
        index,
        kind: kind.to_owned(),
        ok: true,
        started_at_ms,
        finished_at_ms: timestamp_millis(),
        message: None,
        artifacts: Vec::new(),
    }
}

fn step_with_artifacts(
    index: usize,
    kind: &str,
    started_at_ms: u128,
    artifacts: Vec<String>,
) -> StepRecord {
    StepRecord {
        index,
        kind: kind.to_owned(),
        ok: true,
        started_at_ms,
        finished_at_ms: timestamp_millis(),
        message: None,
        artifacts,
    }
}

fn resolve_destination_descriptor(
    repo_root: &Utf8Path,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    destination_id: &str,
    runner: &mut impl ToolRunner,
) -> AtomResult<DestinationDescriptor> {
    if let Some(destination) = list_backend_destinations(repo_root, registry, backend_id, runner)?
        .into_iter()
        .find(|destination| destination.id == destination_id)
    {
        return Ok(destination);
    }
    Err(AtomError::with_path(
        AtomErrorCode::AutomationUnavailable,
        format!("unknown destination id: {destination_id}"),
        destination_id,
    ))
}

fn require_capability(
    descriptor: &DestinationDescriptor,
    capability: DestinationCapability,
) -> AtomResult<()> {
    if descriptor.capabilities.contains(&capability) {
        Ok(())
    } else {
        Err(AtomError::new(
            AtomErrorCode::AutomationUnavailable,
            format!(
                "destination {} does not support {:?}",
                descriptor.id, capability
            ),
        ))
    }
}

fn require_plan_capabilities(
    descriptor: &DestinationDescriptor,
    plan: &EvaluationPlan,
) -> AtomResult<()> {
    for step in &plan.steps {
        let capability = match step {
            EvaluationStep::Launch => DestinationCapability::Launch,
            EvaluationStep::WaitForUi { .. } | EvaluationStep::InspectUi { .. } => {
                DestinationCapability::InspectUi
            }
            EvaluationStep::Tap { .. }
            | EvaluationStep::LongPress { .. }
            | EvaluationStep::Swipe { .. }
            | EvaluationStep::Drag { .. }
            | EvaluationStep::TypeText { .. } => DestinationCapability::Interact,
            EvaluationStep::Screenshot { .. } => DestinationCapability::Screenshot,
            EvaluationStep::StartVideo { .. } | EvaluationStep::StopVideo => {
                DestinationCapability::Video
            }
            EvaluationStep::CollectLogs { .. } => DestinationCapability::Logs,
        };
        require_capability(descriptor, capability)?;
    }
    Ok(())
}

fn step_kind(step: &EvaluationStep) -> &'static str {
    match step {
        EvaluationStep::Launch => "launch",
        EvaluationStep::WaitForUi { .. } => "wait_for_ui",
        EvaluationStep::Tap { .. } => "tap",
        EvaluationStep::LongPress { .. } => "long_press",
        EvaluationStep::Swipe { .. } => "swipe",
        EvaluationStep::Drag { .. } => "drag",
        EvaluationStep::TypeText { .. } => "type_text",
        EvaluationStep::Screenshot { .. } => "screenshot",
        EvaluationStep::InspectUi { .. } => "inspect_ui",
        EvaluationStep::StartVideo { .. } => "start_video",
        EvaluationStep::StopVideo => "stop_video",
        EvaluationStep::CollectLogs { .. } => "collect_logs",
    }
}

fn artifact_name(requested: Option<String>, index: usize, prefix: &str, extension: &str) -> String {
    requested.unwrap_or_else(|| format!("{index:02}-{prefix}.{extension}"))
}

fn video_artifact_name(requested: Option<String>, index: usize, extension: &str) -> String {
    match requested {
        Some(name) => {
            let path = Utf8PathBuf::from(name);
            if path
                .extension()
                .is_some_and(|existing| existing.eq_ignore_ascii_case(extension))
            {
                path.into_string()
            } else if path.extension().is_some() {
                path.with_extension(extension).into_string()
            } else {
                format!("{path}.{extension}")
            }
        }
        None => format!("{index:02}-video.{extension}"),
    }
}

fn load_evaluation_plan(path: &Utf8Path) -> AtomResult<EvaluationPlan> {
    let contents = fs::read_to_string(path).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CliUsageError,
            format!("failed to read evaluation plan: {error}"),
            path.as_str(),
        )
    })?;
    serde_json::from_str(&contents).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CliUsageError,
            format!("failed to parse evaluation plan JSON: {error}"),
            path.as_str(),
        )
    })
}

struct AutomationSession<'a> {
    descriptor: DestinationDescriptor,
    backend: Box<dyn BackendAutomationSession + 'a>,
}

impl<'a> AutomationSession<'a> {
    #[expect(
        clippy::too_many_arguments,
        reason = "Automation sessions are assembled from explicit repo, manifest, registry, destination, and launch inputs."
    )]
    fn new(
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        registry: &DeployBackendRegistry,
        backend_id: &'a str,
        destination_id: &'a str,
        runner: &'a mut dyn ToolRunner,
        descriptor: DestinationDescriptor,
        launch_behavior: SessionLaunchBehavior,
    ) -> AtomResult<Self> {
        debug_assert_eq!(descriptor.id, destination_id);
        let backend = automation_session_with_registry(
            registry,
            repo_root,
            manifest,
            backend_id,
            destination_id,
            runner,
            launch_behavior,
        )?;
        Ok(Self {
            descriptor,
            backend,
        })
    }

    fn ensure_launched(&mut self) -> AtomResult<()> {
        self.backend.ensure_launched()
    }

    fn interact(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult> {
        self.backend.interact(request)
    }

    fn video_extension(&self) -> &'static str {
        self.backend.video_extension()
    }

    fn capture_auto_screenshot(&mut self) -> AtomResult<Utf8PathBuf> {
        self.backend.capture_auto_screenshot()
    }

    fn capture_screenshot(&mut self, output_path: &Utf8Path) -> AtomResult<()> {
        self.backend.capture_screenshot(output_path)
    }

    fn capture_logs(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()> {
        self.backend.capture_logs(output_path, seconds)
    }

    fn capture_video(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()> {
        self.backend.capture_video(output_path, seconds)
    }

    fn start_video(&mut self, output_path: &Utf8Path) -> AtomResult<()> {
        self.backend.start_video(output_path)
    }

    fn stop_video(&mut self) -> AtomResult<Utf8PathBuf> {
        self.backend.stop_video()
    }

    fn shutdown_video(&mut self) -> AtomResult<()> {
        self.backend.shutdown_video()
    }
}

fn automation_session_with_registry<'a>(
    registry: &DeployBackendRegistry,
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    backend_id: &'a str,
    destination_id: &'a str,
    runner: &'a mut dyn ToolRunner,
    launch_behavior: SessionLaunchBehavior,
) -> AtomResult<Box<dyn BackendAutomationSession + 'a>> {
    let backend = registry.get(backend_id).map(Box::as_ref).ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::CliUsageError,
            format!("unknown backend id: {backend_id}"),
            backend_id,
        )
    })?;
    if !backend.is_enabled(manifest) {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            format!("{backend_id} platform is not enabled"),
            backend_id,
        ));
    }
    backend.new_automation_session(repo_root, manifest, destination_id, runner, launch_behavior)
}

pub fn new_ios_automation_session<'a>(
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    destination_id: &'a str,
    runner: &'a mut dyn ToolRunner,
    launch_behavior: SessionLaunchBehavior,
) -> Box<dyn BackendAutomationSession + 'a> {
    Box::new(IosAutomationSession {
        repo_root,
        manifest,
        runner,
        destination_id: destination_id.to_owned(),
        launch_behavior,
        launch: None,
        video_capture: None,
    })
}

pub fn new_android_automation_session<'a>(
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    destination_id: &'a str,
    runner: &'a mut dyn ToolRunner,
    launch_behavior: SessionLaunchBehavior,
) -> Box<dyn BackendAutomationSession + 'a> {
    Box::new(AndroidAutomationSession {
        repo_root,
        manifest,
        runner,
        destination_id: destination_id.to_owned(),
        launch_behavior,
        launch: None,
        video_capture: None,
    })
}

struct IosAutomationSession<'a> {
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    runner: &'a mut dyn ToolRunner,
    destination_id: String,
    launch_behavior: SessionLaunchBehavior,
    launch: Option<AppLaunch>,
    video_capture: Option<VideoCapture>,
}

impl IosAutomationSession<'_> {
    fn active_launch(&self) -> AtomResult<AppLaunch> {
        self.launch.clone().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::InternalBug,
                "automation session expected a launch after ensure_launched",
            )
        })
    }
}

impl BackendAutomationSession for IosAutomationSession<'_> {
    fn video_extension(&self) -> &'static str {
        "mov"
    }

    fn ensure_launched(&mut self) -> AtomResult<()> {
        if self.launch.is_some() {
            return Ok(());
        }
        if self.launch_behavior == SessionLaunchBehavior::AttachOrLaunch
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
        let launch = launch_ios_app(self.repo_root, self.manifest, destination, self.runner)?;
        let launch_destination_id = match &launch {
            AppLaunch::IosSimulator { destination_id, .. }
            | AppLaunch::IosDevice { destination_id, .. } => destination_id.as_str(),
            AppLaunch::Android { .. } => {
                return Err(AtomError::new(
                    AtomErrorCode::AutomationUnavailable,
                    "iOS automation session launched an Android app",
                ));
            }
        };
        wait_for_idb_launch_ready(
            self.repo_root,
            launch_destination_id,
            &self.manifest.app.name,
            &self.manifest.app.slug,
            self.runner,
        )?;
        self.launch = Some(launch);
        Ok(())
    }

    fn interact(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult> {
        self.ensure_launched()?;
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
}

struct AndroidAutomationSession<'a> {
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    runner: &'a mut dyn ToolRunner,
    destination_id: String,
    launch_behavior: SessionLaunchBehavior,
    launch: Option<AppLaunch>,
    video_capture: Option<VideoCapture>,
}

impl AndroidAutomationSession<'_> {
    fn active_launch(&self) -> AtomResult<AppLaunch> {
        self.launch.clone().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::InternalBug,
                "automation session expected a launch after ensure_launched",
            )
        })
    }
}

impl BackendAutomationSession for AndroidAutomationSession<'_> {
    fn video_extension(&self) -> &'static str {
        "mp4"
    }

    fn ensure_launched(&mut self) -> AtomResult<()> {
        if self.launch.is_some() {
            return Ok(());
        }
        if self.launch_behavior == SessionLaunchBehavior::AttachOrLaunch
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
        let launch = launch_android_app(self.repo_root, self.manifest, &destination, self.runner)?;
        let (serial, application_id) = match &launch {
            AppLaunch::Android {
                serial,
                application_id,
            } => (serial.as_str(), application_id.as_str()),
            AppLaunch::IosSimulator { .. } | AppLaunch::IosDevice { .. } => {
                return Err(AtomError::new(
                    AtomErrorCode::AutomationUnavailable,
                    "Android automation session launched an iOS app",
                ));
            }
        };
        wait_for_android_launch_ready(self.repo_root, serial, application_id, self.runner)?;
        self.launch = Some(launch);
        Ok(())
    }

    fn interact(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult> {
        self.ensure_launched()?;
        let launch = self.active_launch()?;
        match launch {
            AppLaunch::Android { serial, .. } => {
                interact_with_android_uiautomator(self.repo_root, &serial, self.runner, request)
            }
            AppLaunch::IosSimulator { .. } | AppLaunch::IosDevice { .. } => Err(AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                "Android automation session expected an Android launch",
            )),
        }
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
}

#[expect(
    clippy::too_many_lines,
    reason = "The idb adapter keeps per-command translation in one place for the iOS backend."
)]
fn interact_with_idb(
    repo_root: &Utf8Path,
    destination_id: &str,
    runner: &mut (impl ToolRunner + ?Sized),
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
                    format_idb_coordinate(tap_x),
                    format_idb_coordinate(tap_y),
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
                    format_idb_coordinate(tap_x),
                    format_idb_coordinate(tap_y),
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
                        format_idb_coordinate(tap_x),
                        format_idb_coordinate(tap_y),
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
                    format_idb_coordinate(start_x),
                    format_idb_coordinate(start_y),
                    format_idb_coordinate(end_x),
                    format_idb_coordinate(end_y),
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
    runner: &mut (impl ToolRunner + ?Sized),
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

fn resolve_interaction_point(
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

fn format_idb_coordinate(value: f64) -> String {
    value.round().to_string()
}

fn run_idb(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    destination_id: &str,
    subcommand: &[String],
) -> AtomResult<()> {
    let args = idb_args(destination_id, subcommand);
    runner.run(repo_root, "idb", &args)
}

fn capture_idb(
    runner: &mut (impl ToolRunner + ?Sized),
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

#[derive(Clone)]
enum AppLaunch {
    IosSimulator {
        destination_id: String,
        bundle_id: String,
        app_name: String,
        app_slug: String,
    },
    IosDevice {
        destination_id: String,
        bundle_id: String,
        app_name: String,
        app_slug: String,
    },
    Android {
        serial: String,
        application_id: String,
    },
}

fn launch_ios_app(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination: IosDestination,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<AppLaunch> {
    if !manifest.ios.enabled {
        return Err(AtomError::new(
            AtomErrorCode::ManifestInvalidValue,
            "iOS is not enabled for this target",
        ));
    }
    let target = generated_target(manifest, "ios");
    let build_args = ios_bazel_args(&target, &destination);
    run_bazel_owned(runner, repo_root, &build_args)?;
    let app_bundle = find_bazel_output_owned(
        runner,
        repo_root,
        &build_args,
        &target,
        &[".app", ".ipa"],
        "iOS app artifact",
    )?;
    let installable_app = resolve_ios_installable_artifact(&app_bundle)?;
    let bundle_id = manifest
        .ios
        .bundle_id
        .clone()
        .ok_or_else(|| AtomError::new(AtomErrorCode::InternalBug, "missing iOS bundle id"))?;

    match destination.kind {
        IosDestinationKind::Simulator => {
            let simulator = prepare_ios_simulator(repo_root, runner, &destination)?;
            run_idb(
                runner,
                repo_root,
                &simulator,
                &["install".to_owned(), installable_app.as_str().to_owned()],
            )?;
            let _ = run_idb(
                runner,
                repo_root,
                &simulator,
                &["terminate".to_owned(), bundle_id.clone()],
            );
            run_idb(
                runner,
                repo_root,
                &simulator,
                &["launch".to_owned(), "-f".to_owned(), bundle_id.clone()],
            )?;
            Ok(AppLaunch::IosSimulator {
                destination_id: destination.id,
                bundle_id,
                app_name: manifest.app.name.clone(),
                app_slug: manifest.app.slug.clone(),
            })
        }
        IosDestinationKind::Device => {
            run_idb(
                runner,
                repo_root,
                &destination.id,
                &["install".to_owned(), installable_app.as_str().to_owned()],
            )?;
            let _ = run_idb(
                runner,
                repo_root,
                &destination.id,
                &["terminate".to_owned(), bundle_id.clone()],
            );
            run_idb(
                runner,
                repo_root,
                &destination.id,
                &["launch".to_owned(), "-f".to_owned(), bundle_id.clone()],
            )?;
            Ok(AppLaunch::IosDevice {
                destination_id: destination.id,
                bundle_id,
                app_name: manifest.app.name.clone(),
                app_slug: manifest.app.slug.clone(),
            })
        }
    }
}

fn attach_ios_app(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination_id: &str,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<Option<AppLaunch>> {
    if !manifest.ios.enabled {
        return Ok(None);
    }
    let Some(bundle_id) = manifest.ios.bundle_id.clone() else {
        return Ok(None);
    };
    let snapshot = inspect_ui_with_idb(repo_root, destination_id, runner)?;
    if !snapshot_matches_ios_app(&snapshot, &manifest.app.name, &manifest.app.slug)
        || !snapshot_is_launch_ready(&snapshot)
    {
        return Ok(None);
    }
    Ok(Some(AppLaunch::IosSimulator {
        destination_id: destination_id.to_owned(),
        bundle_id,
        app_name: manifest.app.name.clone(),
        app_slug: manifest.app.slug.clone(),
    }))
}

fn launch_android_app(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination: &crate::devices::android::AndroidDestination,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<AppLaunch> {
    if !manifest.android.enabled {
        return Err(AtomError::new(
            AtomErrorCode::ManifestInvalidValue,
            "Android is not enabled for this target",
        ));
    }

    let serial = prepare_android_emulator(repo_root, runner, destination)?;
    let target = generated_target(manifest, "android");
    let build_args = vec![
        "build".to_owned(),
        target.clone(),
        "--android_platforms=//platforms:arm64-v8a".to_owned(),
    ];
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
    let args = vec![
        "-s".to_owned(),
        serial.clone(),
        "shell".to_owned(),
        "am".to_owned(),
        "start".to_owned(),
        "-W".to_owned(),
        "-n".to_owned(),
        component,
    ];
    runner.run(repo_root, "adb", &args)?;
    wait_for_app_pid(runner, repo_root, &serial, &application_id)?;

    Ok(AppLaunch::Android {
        serial,
        application_id,
    })
}

fn attach_android_app(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination_id: &str,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<Option<AppLaunch>> {
    if !manifest.android.enabled {
        return Ok(None);
    }
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
    Ok(Some(AppLaunch::Android {
        serial: destination.serial,
        application_id: application_id.to_owned(),
    }))
}

fn capture_screenshot_for_launch(
    repo_root: &Utf8Path,
    launch: &AppLaunch,
    output_path: &Utf8Path,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    match launch {
        AppLaunch::IosSimulator { destination_id, .. } => {
            capture_ios_simulator_screenshot(repo_root, destination_id, output_path, runner)
        }
        AppLaunch::IosDevice { destination_id, .. } => run_idb_screenshot_with_retry(
            runner,
            repo_root,
            destination_id,
            output_path,
            IOS_SCREENSHOT_READY_TIMEOUT,
        ),
        AppLaunch::Android { serial, .. } => {
            let remote = format!("/sdcard/atom-screenshot-{}.png", timestamp_suffix());
            run_tool(
                runner,
                repo_root,
                "adb",
                &["-s", serial, "shell", "screencap", "-p", &remote],
            )?;
            run_tool(
                runner,
                repo_root,
                "adb",
                &["-s", serial, "pull", &remote, output_path.as_str()],
            )?;
            run_tool(
                runner,
                repo_root,
                "adb",
                &["-s", serial, "shell", "rm", "-f", &remote],
            )?;
            Ok(())
        }
    }
}

fn capture_ios_simulator_screenshot(
    repo_root: &Utf8Path,
    destination_id: &str,
    output_path: &Utf8Path,
    runner: &mut (impl ToolRunner + ?Sized),
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
            IOS_SCREENSHOT_READY_TIMEOUT,
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

fn run_idb_screenshot_with_retry(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    destination_id: &str,
    output_path: &Utf8Path,
    timeout: Duration,
) -> AtomResult<()> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    while Instant::now() < deadline {
        match run_idb(
            runner,
            repo_root,
            destination_id,
            &["screenshot".to_owned(), output_path.as_str().to_owned()],
        ) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                thread::sleep(IOS_SCREENSHOT_READY_POLL_INTERVAL);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "failed to capture iOS screenshot after launch readiness wait",
            output_path.as_str(),
        )
    }))
}

fn run_simctl_screenshot_with_retry(
    runner: &mut (impl ToolRunner + ?Sized),
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
                thread::sleep(IOS_SCREENSHOT_READY_POLL_INTERVAL);
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
    launch: &AppLaunch,
    output_path: &Utf8Path,
    seconds: u64,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    let contents = match launch {
        AppLaunch::IosSimulator {
            destination_id,
            bundle_id,
            app_name,
            app_slug,
        }
        | AppLaunch::IosDevice {
            destination_id,
            bundle_id,
            app_name,
            app_slug,
        } => capture_ios_logs_for_launch(
            runner,
            repo_root,
            destination_id,
            bundle_id,
            app_name,
            app_slug,
            seconds,
        ),
        AppLaunch::Android {
            serial,
            application_id,
        } => {
            let pid = wait_for_app_pid(runner, repo_root, serial, application_id)?;
            capture_tool(
                runner,
                repo_root,
                "adb",
                &["-s", serial, "logcat", "--pid", &pid, "-d"],
            )
        }
    }
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

fn capture_ios_logs_for_launch(
    runner: &mut (impl ToolRunner + ?Sized),
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
    launch: &AppLaunch,
    output_path: &Utf8Path,
    seconds: u64,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    match launch {
        AppLaunch::IosSimulator { destination_id, .. }
        | AppLaunch::IosDevice { destination_id, .. } => {
            let mut child = spawn_idb_video(repo_root, destination_id, output_path)?;
            thread::sleep(Duration::from_secs(seconds));
            stop_recording_process(repo_root, &mut child, DestinationPlatform::Ios)?;
            ensure_video_artifact(output_path)?;
            Ok(())
        }
        AppLaunch::Android { serial, .. } => {
            let remote = format!("/sdcard/atom-video-{}.mp4", timestamp_suffix());
            run_tool(
                runner,
                repo_root,
                "adb",
                &[
                    "-s",
                    serial,
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
                &["-s", serial, "pull", &remote, output_path.as_str()],
            )?;
            run_tool(
                runner,
                repo_root,
                "adb",
                &["-s", serial, "shell", "rm", "-f", &remote],
            )?;
            Ok(())
        }
    }
}

struct VideoCapture {
    output_path: Utf8PathBuf,
    child: Child,
    remote_path: Option<String>,
    platform: DestinationPlatform,
    serial: Option<String>,
}

fn start_video_capture(
    repo_root: &Utf8Path,
    launch: &AppLaunch,
    output_path: &Utf8Path,
) -> AtomResult<VideoCapture> {
    write_parent_dir(output_path)?;
    match launch {
        AppLaunch::IosSimulator { destination_id, .. }
        | AppLaunch::IosDevice { destination_id, .. } => {
            let child = spawn_idb_video(repo_root, destination_id, output_path)?;
            Ok(VideoCapture {
                output_path: output_path.to_owned(),
                child,
                remote_path: None,
                platform: DestinationPlatform::Ios,
                serial: None,
            })
        }
        AppLaunch::Android { serial, .. } => {
            let remote_path = format!("/sdcard/atom-video-{}.mp4", timestamp_suffix());
            let child = Command::new("adb")
                .args(["-s", serial, "shell", "screenrecord", &remote_path])
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
                remote_path: Some(remote_path),
                platform: DestinationPlatform::Android,
                serial: Some(serial.clone()),
            })
        }
    }
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

fn stop_video_capture(
    repo_root: &Utf8Path,
    video: VideoCapture,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<Utf8PathBuf> {
    let mut child = video.child;
    if video.platform == DestinationPlatform::Android {
        if let Some(serial) = video.serial.as_deref() {
            stop_android_screenrecord(repo_root, serial, &mut child, runner)?;
        } else {
            stop_recording_process(repo_root, &mut child, video.platform)?;
        }
    } else {
        stop_recording_process(repo_root, &mut child, video.platform)?;
    }

    if video.platform == DestinationPlatform::Android
        && let (Some(serial), Some(remote_path)) =
            (video.serial.as_deref(), video.remote_path.as_deref())
    {
        run_tool(
            runner,
            repo_root,
            "adb",
            &[
                "-s",
                serial,
                "pull",
                remote_path,
                video.output_path.as_str(),
            ],
        )?;
        run_tool(
            runner,
            repo_root,
            "adb",
            &["-s", serial, "shell", "rm", "-f", remote_path],
        )?;
    }

    ensure_video_artifact(&video.output_path)?;
    Ok(video.output_path)
}

fn stop_android_screenrecord(
    repo_root: &Utf8Path,
    serial: &str,
    child: &mut Child,
    runner: &mut (impl ToolRunner + ?Sized),
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

fn stop_recording_process(
    repo_root: &Utf8Path,
    child: &mut Child,
    platform: DestinationPlatform,
) -> AtomResult<()> {
    if wait_for_child_exit(child, Duration::from_millis(100))? {
        return Ok(());
    }

    let (primary_signal, secondary_signal) = match platform {
        DestinationPlatform::Ios | DestinationPlatform::Android => ("INT", "TERM"),
    };

    let _ = signal_child(repo_root, child, primary_signal);
    if wait_for_child_exit(child, VIDEO_STOP_TIMEOUT)? {
        return Ok(());
    }

    let _ = signal_child(repo_root, child, secondary_signal);
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

fn write_json<T: Serialize>(path: &Utf8Path, value: &T) -> AtomResult<()> {
    let contents = serde_json::to_string_pretty(value).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::InternalBug,
            format!("failed to encode JSON output: {error}"),
            path.as_str(),
        )
    })?;
    fs::write(path, contents).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to write JSON output: {error}"),
            path.as_str(),
        )
    })
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

fn timestamp_suffix() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use crate::destinations::DestinationKind;
    use atom_backends::ToolRunner;
    use atom_ffi::{AtomError, AtomErrorCode};
    use atom_manifest::{AndroidConfig, AppConfig, BuildConfig, IosConfig, NormalizedManifest};
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    use super::{
        AppLaunch, DestinationCapability, DestinationDescriptor, DestinationPlatform,
        EvaluationPlan, EvaluationStep, InteractionRequest, ScreenInfo, UiBounds, UiNode,
        UiSnapshot, attach_ios_app, capture_logs_for_launch, capture_screenshot_for_launch,
        interact_with_idb, load_evaluation_plan, require_plan_capabilities,
        snapshot_is_launch_ready, snapshot_matches_ios_app, video_artifact_name,
    };

    #[derive(Default)]
    struct FakeToolRunner {
        calls: Vec<(String, Vec<String>)>,
        captures: VecDeque<String>,
        capture_errors: VecDeque<AtomError>,
        run_errors: VecDeque<AtomError>,
    }

    impl ToolRunner for FakeToolRunner {
        fn run(
            &mut self,
            _repo_root: &camino::Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            if let Some(error) = self.run_errors.pop_front() {
                return Err(error);
            }
            Ok(())
        }

        fn capture(
            &mut self,
            _repo_root: &camino::Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            if let Some(error) = self.capture_errors.pop_front() {
                return Err(error);
            }
            Ok(self
                .captures
                .pop_front()
                .expect("expected captured output for command"))
        }

        fn capture_json_file(
            &mut self,
            _repo_root: &camino::Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            if let Some(error) = self.capture_errors.pop_front() {
                return Err(error);
            }
            Ok(self
                .captures
                .pop_front()
                .expect("expected captured JSON output for command"))
        }

        fn stream(
            &mut self,
            _repo_root: &camino::Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            Ok(())
        }
    }

    fn runnable_manifest(root: &Utf8PathBuf) -> NormalizedManifest {
        NormalizedManifest {
            repo_root: root.clone(),
            target_label: "//examples/hello-world/apps/hello_atom:hello_atom".to_owned(),
            metadata_path: root.join("bazel-out/hello_atom.atom.app.json"),
            app: AppConfig {
                name: "Hello Atom".to_owned(),
                slug: "hello-atom".to_owned(),
                entry_crate_label: "//examples/hello-world/apps/hello_atom:hello_atom".to_owned(),
                entry_crate_name: "hello_atom".to_owned(),
            },
            ios: IosConfig {
                enabled: true,
                bundle_id: Some("build.atom.hello".to_owned()),
                deployment_target: Some("17.0".to_owned()),
            },
            android: AndroidConfig {
                enabled: true,
                application_id: Some("build.atom.hello".to_owned()),
                min_sdk: Some(28),
                target_sdk: Some(35),
            },
            build: BuildConfig {
                generated_root: Utf8PathBuf::from("generated"),
                watch: false,
            },
            modules: Vec::new(),
            config_plugins: Vec::new(),
        }
    }

    #[test]
    fn load_evaluation_plan_reads_json_steps() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let plan_path = root.join("plan.json");
        std::fs::write(
            &plan_path,
            r#"{"steps":[{"kind":"launch"},{"kind":"tap","target_id":"atom.demo.primary_button"}]}"#,
        )
        .expect("plan");

        let plan = load_evaluation_plan(&plan_path).expect("plan should parse");

        assert_eq!(
            plan,
            EvaluationPlan {
                steps: vec![
                    EvaluationStep::Launch,
                    EvaluationStep::Tap {
                        target_id: Some("atom.demo.primary_button".to_owned()),
                        x: None,
                        y: None,
                    },
                ],
            }
        );
    }

    #[test]
    fn require_plan_capabilities_rejects_unsupported_steps() {
        let descriptor = DestinationDescriptor {
            backend_id: "ios".to_owned(),
            id: "ios-device".to_owned(),
            platform: DestinationPlatform::Ios,
            kind: DestinationKind::Device,
            display_name: "Device: Alex's iPhone".to_owned(),
            available: true,
            debug_state: "ready".to_owned(),
            capabilities: vec![DestinationCapability::Launch],
        };
        let plan = EvaluationPlan {
            steps: vec![EvaluationStep::CollectLogs {
                name: None,
                seconds: Some(5),
            }],
        };

        let error =
            require_plan_capabilities(&descriptor, &plan).expect_err("logs should be rejected");

        assert_eq!(error.code, AtomErrorCode::AutomationUnavailable);
        assert!(error.message.contains("does not support"));
    }

    #[test]
    fn idb_interactions_round_coordinates_to_integer_strings() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                r#"{"elements":[{"AXUniqueId":"atom.demo.primary_button","type":"button","AXLabel":"Tap me","AXValue":"Tap me","visible":true,"enabled":true,"frame":{"x":100.4,"y":240.6,"width":201.2,"height":84.8}}]}"#
                    .to_owned(),
                r#"{"elements":[]}"#.to_owned(),
            ]),
            capture_errors: VecDeque::new(),
            run_errors: VecDeque::new(),
        };

        interact_with_idb(
            &root,
            "SIM-123",
            &mut runner,
            InteractionRequest::Tap {
                target_id: Some("atom.demo.primary_button".to_owned()),
                x: None,
                y: None,
            },
        )
        .expect("tap should succeed");

        assert_eq!(
            runner.calls[1],
            (
                "idb".to_owned(),
                vec![
                    "ui".to_owned(),
                    "tap".to_owned(),
                    "--udid".to_owned(),
                    "SIM-123".to_owned(),
                    "201".to_owned(),
                    "283".to_owned(),
                ],
            )
        );
    }

    #[test]
    fn launch_readiness_checks_the_foreground_ios_application_node() {
        let app_snapshot = UiSnapshot {
            screen: ScreenInfo {
                width: 402.0,
                height: 874.0,
            },
            nodes: vec![UiNode {
                id: "idb-node-0".to_owned(),
                role: "Application".to_owned(),
                label: "Hello Atom".to_owned(),
                text: "Hello Atom".to_owned(),
                visible: true,
                enabled: true,
                bounds: UiBounds {
                    x: 0.0,
                    y: 0.0,
                    width: 402.0,
                    height: 874.0,
                },
            }],
            screenshot_path: None,
        };
        let springboard_snapshot = UiSnapshot {
            screen: app_snapshot.screen.clone(),
            nodes: vec![UiNode {
                label: "Home Screen".to_owned(),
                text: "Home Screen".to_owned(),
                ..app_snapshot.nodes[0].clone()
            }],
            screenshot_path: None,
        };

        assert!(snapshot_matches_ios_app(
            &app_snapshot,
            "Hello Atom",
            "hello-atom"
        ));
        assert!(!snapshot_matches_ios_app(
            &springboard_snapshot,
            "Hello Atom",
            "hello-atom"
        ));
    }

    #[test]
    fn log_capture_maps_backend_failures_to_automation_log_capture_failed() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let output = root.join("logs.txt");
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::new(),
            capture_errors: VecDeque::from([
                AtomError::new(AtomErrorCode::ExternalToolFailed, "idb log failed"),
                AtomError::new(AtomErrorCode::ExternalToolFailed, "idb log failed"),
            ]),
            run_errors: VecDeque::new(),
        };

        let error = capture_logs_for_launch(
            &root,
            &AppLaunch::IosSimulator {
                destination_id: "SIM-123".to_owned(),
                bundle_id: "build.atom.hello".to_owned(),
                app_name: "Hello Atom".to_owned(),
                app_slug: "hello-atom".to_owned(),
            },
            &output,
            5,
            &mut runner,
        )
        .expect_err("log capture should fail");

        assert_eq!(error.code, AtomErrorCode::AutomationLogCaptureFailed);
        assert!(error.message.contains("failed to collect logs"));
    }

    #[test]
    fn ios_log_capture_falls_back_to_filtered_syslog_when_process_scope_fails() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let output = root.join("logs.txt");
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([concat!(
                "Mar 11 10:48:12 simulatord Noise line\n",
                "Mar 11 10:48:13 hello-atom AtomRuntime: app line\n",
                "Mar 11 10:48:14 launchd build.atom.hello foreground transition\n"
            )
            .to_owned()]),
            capture_errors: VecDeque::from([AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "idb log --process failed",
            )]),
            run_errors: VecDeque::new(),
        };

        capture_logs_for_launch(
            &root,
            &AppLaunch::IosSimulator {
                destination_id: "SIM-123".to_owned(),
                bundle_id: "build.atom.hello".to_owned(),
                app_name: "Hello Atom".to_owned(),
                app_slug: "hello-atom".to_owned(),
            },
            &output,
            5,
            &mut runner,
        )
        .expect("log capture should succeed");

        let contents = std::fs::read_to_string(&output).expect("captured logs");
        assert!(!contents.contains("simulatord Noise line"));
        assert!(contents.contains("hello-atom AtomRuntime: app line"));
        assert!(contents.contains("build.atom.hello foreground transition"));
        assert_eq!(
            runner.calls[0],
            (
                "idb".to_owned(),
                vec![
                    "log".to_owned(),
                    "--udid".to_owned(),
                    "SIM-123".to_owned(),
                    "--".to_owned(),
                    "--style".to_owned(),
                    "syslog".to_owned(),
                    "--process".to_owned(),
                    "hello-atom".to_owned(),
                    "--timeout".to_owned(),
                    "5s".to_owned(),
                ],
            )
        );
    }

    #[test]
    fn ios_simulator_screenshot_falls_back_to_simctl_when_idb_fails() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let output = root.join("screenshot.png");
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::new(),
            capture_errors: VecDeque::new(),
            run_errors: VecDeque::from([AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "No Image available to encode",
            )]),
        };

        capture_screenshot_for_launch(
            &root,
            &AppLaunch::IosSimulator {
                destination_id: "SIM-123".to_owned(),
                bundle_id: "build.atom.hello".to_owned(),
                app_name: "Hello Atom".to_owned(),
                app_slug: "hello-atom".to_owned(),
            },
            &output,
            &mut runner,
        )
        .expect("simctl fallback should succeed");

        assert_eq!(
            runner.calls,
            vec![
                (
                    "idb".to_owned(),
                    vec![
                        "screenshot".to_owned(),
                        "--udid".to_owned(),
                        "SIM-123".to_owned(),
                        output.as_str().to_owned(),
                    ],
                ),
                (
                    "xcrun".to_owned(),
                    vec![
                        "simctl".to_owned(),
                        "io".to_owned(),
                        "SIM-123".to_owned(),
                        "screenshot".to_owned(),
                        output.as_str().to_owned(),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn ios_device_screenshot_retries_idb_until_the_surface_is_ready() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let output = root.join("device-screenshot.png");
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::new(),
            capture_errors: VecDeque::new(),
            run_errors: VecDeque::from([
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    "No Image available to encode",
                ),
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    "No Image available to encode",
                ),
            ]),
        };

        capture_screenshot_for_launch(
            &root,
            &AppLaunch::IosDevice {
                destination_id: "DEVICE-123".to_owned(),
                bundle_id: "build.atom.hello".to_owned(),
                app_name: "Hello Atom".to_owned(),
                app_slug: "hello-atom".to_owned(),
            },
            &output,
            &mut runner,
        )
        .expect("device screenshot should eventually succeed");

        let screenshot_calls = runner
            .calls
            .iter()
            .filter(|(tool, args)| {
                tool == "idb"
                    && args
                        .first()
                        .is_some_and(|subcommand| subcommand == "screenshot")
            })
            .count();
        assert_eq!(screenshot_calls, 3);
    }

    #[test]
    fn video_artifact_name_uses_platform_specific_extensions() {
        let ios_name = video_artifact_name(Some("session.mp4".to_owned()), 2, "mov");
        let android_name = video_artifact_name(Some("session".to_owned()), 2, "mp4");

        assert_eq!(ios_name, "session.mov");
        assert_eq!(android_name, "session.mp4");
    }

    #[test]
    fn attach_ios_app_reuses_the_foreground_snapshot() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([r#"{"elements":[{"AXUniqueId":"idb-node-0","type":"Application","AXLabel":"Hello Atom","AXValue":"Hello Atom","visible":true,"enabled":true,"frame":{"x":0,"y":0,"width":402,"height":874}},{"AXUniqueId":"atom.demo.title","type":"StaticText","AXLabel":"Hello Atom","AXValue":"Hello Atom","visible":true,"enabled":true,"frame":{"x":24,"y":96,"width":140,"height":28}}]}"#.to_owned()]),
            capture_errors: VecDeque::new(),
            run_errors: VecDeque::new(),
        };

        let launch =
            attach_ios_app(&root, &manifest, "SIM-123", &mut runner).expect("attach should work");

        assert!(matches!(
            launch,
            Some(AppLaunch::IosSimulator { destination_id, .. }) if destination_id == "SIM-123"
        ));
        assert!(
            runner.calls.iter().all(|(tool, _args)| tool != "bazelisk"),
            "attach should not rebuild when the app is already running"
        );
    }

    #[test]
    fn launch_readiness_requires_visible_content_beyond_application_root() {
        let root_only = UiSnapshot {
            screen: ScreenInfo {
                width: 402.0,
                height: 874.0,
            },
            nodes: vec![UiNode {
                id: "idb-node-0".to_owned(),
                role: "Application".to_owned(),
                label: "Hello Atom Plain".to_owned(),
                text: "Hello Atom Plain".to_owned(),
                visible: true,
                enabled: true,
                bounds: UiBounds {
                    x: 0.0,
                    y: 0.0,
                    width: 402.0,
                    height: 874.0,
                },
            }],
            screenshot_path: None,
        };
        let ready = UiSnapshot {
            screen: ScreenInfo {
                width: 402.0,
                height: 874.0,
            },
            nodes: vec![
                root_only.nodes[0].clone(),
                UiNode {
                    id: "idb-node-1".to_owned(),
                    role: "StaticText".to_owned(),
                    label: "hello-atom-plain".to_owned(),
                    text: "hello-atom-plain".to_owned(),
                    visible: true,
                    enabled: true,
                    bounds: UiBounds {
                        x: 136.0,
                        y: 448.0,
                        width: 128.0,
                        height: 15.0,
                    },
                },
            ],
            screenshot_path: None,
        };

        assert!(!snapshot_is_launch_ready(&root_only));
        assert!(snapshot_is_launch_ready(&ready));
    }
}
