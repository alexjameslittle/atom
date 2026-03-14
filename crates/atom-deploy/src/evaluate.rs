use std::fs;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atom_backends::{
    AppSessionBuildProfile, AppSessionOptions, BackendAppSession, DebugSessionRequest,
    DebugSessionResponse, DeployBackendRegistry, DestinationCapability, DestinationDescriptor,
    ToolRunner,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::destinations::list_backend_destinations;

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
    let mut session = AppSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        AppSessionOptions {
            launch_behavior: SessionLaunchBehavior::AttachOrLaunch,
            build_profile: AppSessionBuildProfile::Standard,
        },
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
    let mut session = AppSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        AppSessionOptions {
            launch_behavior: SessionLaunchBehavior::AttachOrLaunch,
            build_profile: AppSessionBuildProfile::Standard,
        },
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
    let mut session = AppSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        AppSessionOptions {
            launch_behavior: SessionLaunchBehavior::AttachOrLaunch,
            build_profile: AppSessionBuildProfile::Standard,
        },
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
    let mut session = AppSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        AppSessionOptions {
            launch_behavior: SessionLaunchBehavior::AttachOrLaunch,
            build_profile: AppSessionBuildProfile::Standard,
        },
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
    let mut session = AppSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        AppSessionOptions {
            launch_behavior: SessionLaunchBehavior::AttachOrLaunch,
            build_profile: AppSessionBuildProfile::Standard,
        },
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
    require_debugger_profile(&plan)?;
    write_parent_dir(artifacts_dir)?;

    let descriptor =
        resolve_destination_descriptor(repo_root, registry, backend_id, destination_id, runner)?;
    require_plan_capabilities(&descriptor, &plan)?;

    let started_at_ms = timestamp_millis();
    let mut session = AppSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        runner,
        descriptor,
        AppSessionOptions {
            launch_behavior: SessionLaunchBehavior::LaunchOnly,
            build_profile: plan.build_profile,
        },
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

#[expect(
    clippy::too_many_lines,
    reason = "The evaluation dispatcher keeps the public plan step mapping in one place."
)]
fn execute_step(
    index: usize,
    step: EvaluationStep,
    artifacts_dir: &Utf8Path,
    session: &mut AppSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
) -> AtomResult<StepRecord> {
    let started_at_ms = timestamp_millis();
    match step {
        EvaluationStep::Launch => execute_launch_step(index, started_at_ms, session),
        EvaluationStep::DebugAttach { name } => execute_debug_step(
            index,
            "debug_attach",
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            name,
            DebugSessionRequest::Attach,
        ),
        EvaluationStep::DebugWaitForStop { name, timeout_ms } => execute_debug_step(
            index,
            "debug_wait_for_stop",
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            name,
            DebugSessionRequest::WaitForStop {
                timeout_ms: timeout_ms.unwrap_or(5_000),
            },
        ),
        EvaluationStep::DebugPause { name } => execute_debug_step(
            index,
            "debug_pause",
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            name,
            DebugSessionRequest::Pause,
        ),
        EvaluationStep::DebugResume { name } => execute_debug_step(
            index,
            "debug_resume",
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            name,
            DebugSessionRequest::Resume,
        ),
        EvaluationStep::DebugThreads { name } => execute_debug_step(
            index,
            "debug_threads",
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            name,
            DebugSessionRequest::ListThreads,
        ),
        EvaluationStep::DebugBacktrace { name, thread_id } => execute_debug_step(
            index,
            "debug_backtrace",
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            name,
            DebugSessionRequest::ListFrames { thread_id },
        ),
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
    session: &mut AppSession<'_>,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    Ok(simple_step(index, "launch", started_at_ms))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Debugger steps carry the same explicit artifact/session inputs as other evaluation helpers."
)]
fn execute_debug_step(
    index: usize,
    kind: &str,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut AppSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
    name: Option<String>,
    request: DebugSessionRequest,
) -> AtomResult<StepRecord> {
    let response = session.execute_debug_request(request)?;
    let artifact_name = artifact_name(name, index, kind, "json");
    let output_path = artifacts_dir.join(&artifact_name);
    write_json(&output_path, &response)?;
    artifacts.push(ArtifactRecord {
        name: artifact_name.clone(),
        kind: kind.to_owned(),
        path: output_path.as_str().to_owned(),
    });
    Ok(step_with_artifacts(
        index,
        kind,
        started_at_ms,
        vec![artifact_name],
    ))
}

fn execute_wait_for_ui_step(
    index: usize,
    started_at_ms: u128,
    session: &mut AppSession<'_>,
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
    session: &mut AppSession<'_>,
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
    session: &mut AppSession<'_>,
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
    session: &mut AppSession<'_>,
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
    session: &mut AppSession<'_>,
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
    session: &mut AppSession<'_>,
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
    session: &mut AppSession<'_>,
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
    session: &mut AppSession<'_>,
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
            EvaluationStep::DebugAttach { .. }
            | EvaluationStep::DebugWaitForStop { .. }
            | EvaluationStep::DebugPause { .. }
            | EvaluationStep::DebugResume { .. }
            | EvaluationStep::DebugThreads { .. }
            | EvaluationStep::DebugBacktrace { .. } => continue,
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

fn require_debugger_profile(plan: &EvaluationPlan) -> AtomResult<()> {
    if plan.build_profile == AppSessionBuildProfile::Debugger
        || !plan.steps.iter().any(is_debug_step)
    {
        return Ok(());
    }

    Err(AtomError::new(
        AtomErrorCode::CliUsageError,
        "debugger evaluation steps require plan.build_profile to be set to \"debugger\"",
    ))
}

fn is_debug_step(step: &EvaluationStep) -> bool {
    matches!(
        step,
        EvaluationStep::DebugAttach { .. }
            | EvaluationStep::DebugWaitForStop { .. }
            | EvaluationStep::DebugPause { .. }
            | EvaluationStep::DebugResume { .. }
            | EvaluationStep::DebugThreads { .. }
            | EvaluationStep::DebugBacktrace { .. }
    )
}

fn step_kind(step: &EvaluationStep) -> &'static str {
    match step {
        EvaluationStep::Launch => "launch",
        EvaluationStep::DebugAttach { .. } => "debug_attach",
        EvaluationStep::DebugWaitForStop { .. } => "debug_wait_for_stop",
        EvaluationStep::DebugPause { .. } => "debug_pause",
        EvaluationStep::DebugResume { .. } => "debug_resume",
        EvaluationStep::DebugThreads { .. } => "debug_threads",
        EvaluationStep::DebugBacktrace { .. } => "debug_backtrace",
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

struct AppSession<'a> {
    descriptor: DestinationDescriptor,
    backend: Box<dyn BackendAppSession + 'a>,
}

impl<'a> AppSession<'a> {
    #[expect(
        clippy::too_many_arguments,
        reason = "App sessions are assembled from explicit repo, manifest, registry, destination, and launch inputs."
    )]
    fn new(
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        registry: &DeployBackendRegistry,
        backend_id: &'a str,
        destination_id: &'a str,
        runner: &'a mut dyn ToolRunner,
        descriptor: DestinationDescriptor,
        options: AppSessionOptions,
    ) -> AtomResult<Self> {
        debug_assert_eq!(descriptor.id, destination_id);
        let backend = app_session_with_registry(
            registry,
            repo_root,
            manifest,
            backend_id,
            destination_id,
            runner,
            options,
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

    fn execute_debug_request(
        &mut self,
        request: DebugSessionRequest,
    ) -> AtomResult<DebugSessionResponse> {
        let response = {
            let Some(debug_session) = self.backend.debug_session()? else {
                return Err(AtomError::new(
                    AtomErrorCode::AutomationUnavailable,
                    "this evaluation session does not support debugger control",
                ));
            };
            debug_session.execute(request)?
        };
        self.ensure_launched()?;
        Ok(response)
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

fn app_session_with_registry<'a>(
    registry: &DeployBackendRegistry,
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    backend_id: &'a str,
    destination_id: &'a str,
    runner: &'a mut dyn ToolRunner,
    options: AppSessionOptions,
) -> AtomResult<Box<dyn BackendAppSession + 'a>> {
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
    backend.new_app_session(repo_root, manifest, destination_id, runner, options)
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

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;

    use atom_backends::{
        AppSessionBuildProfile, AppSessionOptions, BackendAppSession, BackendDebugSession,
        BackendDefinition, DebugFrame, DebugSessionRequest, DebugSessionResponse,
        DebugSessionState, DebugThread, DeployBackend, DeployBackendRegistry,
        DestinationCapability, DestinationDescriptor, ToolRunner,
    };
    use atom_manifest::{NormalizedManifest, testing::fixture_manifest};
    use camino::{Utf8Path, Utf8PathBuf};
    use tempfile::tempdir;

    use super::{
        EvaluationPlan, EvaluationStep, InteractionRequest, ScreenInfo, UiBounds, UiNode,
        UiSnapshot, load_evaluation_plan, require_debugger_profile, require_plan_capabilities,
        video_artifact_name,
    };

    #[derive(Default)]
    struct FakeToolRunner;

    impl ToolRunner for FakeToolRunner {
        fn run(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn capture(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            Ok(String::new())
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

    struct FixtureBackend;

    impl BackendDefinition for FixtureBackend {
        fn id(&self) -> &'static str {
            "fixture"
        }

        fn platform(&self) -> &'static str {
            "fixture"
        }
    }

    impl DeployBackend for FixtureBackend {
        fn is_enabled(&self, _manifest: &NormalizedManifest) -> bool {
            true
        }

        fn list_destinations(
            &self,
            _repo_root: &Utf8Path,
            _runner: &mut dyn ToolRunner,
        ) -> atom_ffi::AtomResult<Vec<DestinationDescriptor>> {
            Ok(vec![DestinationDescriptor {
                platform: "fixture-platform".to_owned(),
                backend_id: "fixture".to_owned(),
                id: "fixture-1".to_owned(),
                kind: "fixture-target".to_owned(),
                display_name: "Fixture".to_owned(),
                available: true,
                debug_state: "ready".to_owned(),
                capabilities: vec![
                    DestinationCapability::Launch,
                    DestinationCapability::InspectUi,
                    DestinationCapability::Interact,
                    DestinationCapability::Screenshot,
                    DestinationCapability::Video,
                    DestinationCapability::Logs,
                    DestinationCapability::Evaluate,
                ],
            }])
        }

        fn deploy(
            &self,
            _repo_root: &Utf8Path,
            _manifest: &NormalizedManifest,
            _requested_destination: Option<&str>,
            _launch_mode: atom_backends::LaunchMode,
            _runner: &mut dyn ToolRunner,
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn stop(
            &self,
            _repo_root: &Utf8Path,
            _manifest: &NormalizedManifest,
            _requested_destination: Option<&str>,
            _runner: &mut dyn ToolRunner,
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn new_app_session<'a>(
            &self,
            _repo_root: &'a Utf8Path,
            _manifest: &'a NormalizedManifest,
            _destination_id: &'a str,
            _runner: &'a mut dyn ToolRunner,
            _options: AppSessionOptions,
        ) -> atom_ffi::AtomResult<Box<dyn BackendAppSession + 'a>> {
            Ok(Box::new(FixtureSession::default()))
        }
    }

    struct FixtureDebugSession {
        state: DebugSessionState,
    }

    impl Default for FixtureDebugSession {
        fn default() -> Self {
            Self {
                state: DebugSessionState::Unknown,
            }
        }
    }

    impl BackendDebugSession for FixtureDebugSession {
        fn execute(
            &mut self,
            request: DebugSessionRequest,
        ) -> atom_ffi::AtomResult<DebugSessionResponse> {
            match request {
                DebugSessionRequest::Attach => {
                    self.state = DebugSessionState::Running;
                    Ok(DebugSessionResponse::Attached { state: self.state })
                }
                DebugSessionRequest::InspectState => {
                    Ok(DebugSessionResponse::State { state: self.state })
                }
                DebugSessionRequest::WaitForStop { .. } => {
                    self.state = DebugSessionState::Stopped;
                    Ok(DebugSessionResponse::Stopped { state: self.state })
                }
                DebugSessionRequest::Pause => {
                    self.state = DebugSessionState::Stopped;
                    Ok(DebugSessionResponse::Paused)
                }
                DebugSessionRequest::Resume => {
                    self.state = DebugSessionState::Running;
                    Ok(DebugSessionResponse::Resumed)
                }
                DebugSessionRequest::ListThreads => Ok(DebugSessionResponse::Threads {
                    threads: vec![DebugThread {
                        id: "thread-1".to_owned(),
                        name: Some("main".to_owned()),
                        selected: true,
                    }],
                }),
                DebugSessionRequest::ListFrames { thread_id } => {
                    let thread_id = thread_id.unwrap_or_else(|| "thread-1".to_owned());
                    Ok(DebugSessionResponse::Frames {
                        thread_id,
                        frames: vec![DebugFrame {
                            index: 0,
                            function: "fixture::main".to_owned(),
                            source_path: Some("src/main.rs".to_owned()),
                            line: Some(42),
                            column: Some(7),
                        }],
                    })
                }
            }
        }
    }

    #[derive(Default)]
    struct FixtureSession {
        snapshots: VecDeque<UiSnapshot>,
        debug_session: FixtureDebugSession,
    }

    impl BackendAppSession for FixtureSession {
        fn video_extension(&self) -> &'static str {
            "mp4"
        }

        fn ensure_launched(&mut self) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn interact(
            &mut self,
            _request: InteractionRequest,
        ) -> atom_ffi::AtomResult<atom_backends::InteractionResult> {
            let snapshot = self.snapshots.pop_front().unwrap_or(UiSnapshot {
                screen: ScreenInfo {
                    width: 100.0,
                    height: 100.0,
                },
                nodes: vec![UiNode {
                    id: "fixture".to_owned(),
                    role: "button".to_owned(),
                    label: "Fixture".to_owned(),
                    text: "Fixture".to_owned(),
                    visible: true,
                    enabled: true,
                    bounds: UiBounds {
                        x: 0.0,
                        y: 0.0,
                        width: 20.0,
                        height: 20.0,
                    },
                }],
                screenshot_path: None,
            });
            Ok(atom_backends::InteractionResult {
                ok: true,
                snapshot,
                message: None,
            })
        }

        fn capture_auto_screenshot(&mut self) -> atom_ffi::AtomResult<Utf8PathBuf> {
            Ok(Utf8PathBuf::from("fixture.png"))
        }

        fn capture_screenshot(&mut self, _output_path: &Utf8Path) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn capture_logs(
            &mut self,
            _output_path: &Utf8Path,
            _seconds: u64,
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn capture_video(
            &mut self,
            _output_path: &Utf8Path,
            _seconds: u64,
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn start_video(&mut self, _output_path: &Utf8Path) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn stop_video(&mut self) -> atom_ffi::AtomResult<Utf8PathBuf> {
            Ok(Utf8PathBuf::from("fixture.mp4"))
        }

        fn shutdown_video(&mut self) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn debug_session(&mut self) -> atom_ffi::AtomResult<Option<&mut dyn BackendDebugSession>> {
            Ok(Some(&mut self.debug_session))
        }
    }

    fn runnable_manifest(root: &Utf8PathBuf) -> NormalizedManifest {
        fixture_manifest(root)
    }

    #[test]
    fn plan_capabilities_require_matching_destination_support() {
        let descriptor = DestinationDescriptor {
            platform: "fixture-platform".to_owned(),
            backend_id: "fixture".to_owned(),
            id: "fixture-1".to_owned(),
            kind: "fixture-target".to_owned(),
            display_name: "Fixture".to_owned(),
            available: true,
            debug_state: "ready".to_owned(),
            capabilities: vec![DestinationCapability::Launch],
        };
        let plan = EvaluationPlan {
            build_profile: AppSessionBuildProfile::Standard,
            steps: vec![EvaluationStep::Screenshot { name: None }],
        };

        let error =
            require_plan_capabilities(&descriptor, &plan).expect_err("screenshot should fail");
        assert_eq!(error.code, atom_ffi::AtomErrorCode::AutomationUnavailable);
    }

    #[test]
    fn video_artifact_name_preserves_or_rewrites_extensions() {
        assert_eq!(
            video_artifact_name(Some("clip".to_owned()), 0, "mp4"),
            "clip.mp4"
        );
        assert_eq!(
            video_artifact_name(Some("clip.mov".to_owned()), 0, "mp4"),
            "clip.mp4"
        );
        assert_eq!(video_artifact_name(None, 3, "mp4"), "03-video.mp4");
    }

    #[test]
    fn evaluation_plan_loader_reads_json() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8");
        let plan_path = root.join("plan.json");
        fs::write(
            &plan_path,
            r#"{
  "build_profile": "debugger",
  "steps": [
    { "kind": "debug_attach", "name": "attach.json" },
    { "kind": "debug_backtrace" }
  ]
}"#,
        )
        .expect("write");

        let plan = load_evaluation_plan(&plan_path).expect("plan should load");
        assert_eq!(plan.build_profile, AppSessionBuildProfile::Debugger);
        assert_eq!(
            plan.steps,
            vec![
                EvaluationStep::DebugAttach {
                    name: Some("attach.json".to_owned()),
                },
                EvaluationStep::DebugBacktrace {
                    name: None,
                    thread_id: None,
                },
            ]
        );
    }

    #[test]
    fn debugger_steps_require_debugger_build_profile() {
        let error = require_debugger_profile(&EvaluationPlan {
            build_profile: AppSessionBuildProfile::Standard,
            steps: vec![EvaluationStep::DebugPause { name: None }],
        })
        .expect_err("debugger step should require debugger build profile");

        assert_eq!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
    }

    #[test]
    fn evaluate_run_dispatches_through_registered_backend_sessions() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8");
        let manifest = runnable_manifest(&root);
        let plan_path = root.join("plan.json");
        fs::write(&plan_path, "{ \"steps\": [ { \"kind\": \"launch\" } ] }").expect("write");
        let artifacts_dir = root.join("artifacts");
        let mut registry = DeployBackendRegistry::new();
        registry
            .register(Box::new(FixtureBackend))
            .expect("fixture backend should register");
        let mut runner = FakeToolRunner;

        let output = super::evaluate_run(
            &root,
            &manifest,
            &registry,
            "fixture",
            "fixture-1",
            &plan_path,
            &artifacts_dir,
            &mut runner,
        )
        .expect("evaluation should run");

        assert_eq!(output.manifest.destination.backend_id, "fixture");
        assert!(artifacts_dir.join("steps.json").exists());
        assert!(artifacts_dir.join("manifest.json").exists());
    }

    #[test]
    fn evaluate_run_writes_debugger_artifacts() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8");
        let manifest = runnable_manifest(&root);
        let plan_path = root.join("plan.json");
        fs::write(
            &plan_path,
            r#"{
  "build_profile": "debugger",
  "steps": [
    { "kind": "debug_attach" },
    { "kind": "debug_pause" },
    { "kind": "debug_threads", "name": "threads.json" },
    { "kind": "debug_backtrace" },
    { "kind": "debug_resume" }
  ]
}"#,
        )
        .expect("write");
        let artifacts_dir = root.join("artifacts");
        let mut registry = DeployBackendRegistry::new();
        registry
            .register(Box::new(FixtureBackend))
            .expect("fixture backend should register");
        let mut runner = FakeToolRunner;

        let output = super::evaluate_run(
            &root,
            &manifest,
            &registry,
            "fixture",
            "fixture-1",
            &plan_path,
            &artifacts_dir,
            &mut runner,
        )
        .expect("debugger evaluation should run");

        assert!(artifacts_dir.join("00-debug_attach.json").exists());
        assert!(artifacts_dir.join("01-debug_pause.json").exists());
        assert!(artifacts_dir.join("threads.json").exists());
        assert!(artifacts_dir.join("03-debug_backtrace.json").exists());
        assert!(artifacts_dir.join("04-debug_resume.json").exists());
        assert_eq!(
            output.manifest.steps[2].artifacts,
            vec!["threads.json".to_owned()]
        );
    }
}
