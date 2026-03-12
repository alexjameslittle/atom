use std::collections::BTreeMap;
use std::fs;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atom_backends::{
    BackendAutomationSession, BackendDebugSession, DebugBacktrace, DebugBreakpoint,
    DebugSourceLocation, DebugStop, DebugThread, DebuggerKind, DeployBackendRegistry,
    DestinationCapability, DestinationDescriptor, SharedToolRunner, ToolRunner,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

use crate::debugger::debug_session_with_registry;
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
    let shared_runner = SharedToolRunner::new(runner);
    let descriptor = resolve_destination_descriptor(
        repo_root,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
    )?;
    require_capability(&descriptor, DestinationCapability::InspectUi)?;
    let mut session = EvaluationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
        descriptor,
        SessionLaunchBehavior::AttachOrLaunch,
    )?;
    session.ensure_launched()?;
    let mut snapshot = session.interact(InteractionRequest::InspectUi)?;
    session.shutdown_video()?;
    session.shutdown_debug_sessions()?;
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
    let shared_runner = SharedToolRunner::new(runner);
    let descriptor = resolve_destination_descriptor(
        repo_root,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
    )?;
    require_capability(&descriptor, DestinationCapability::Interact)?;
    let mut session = EvaluationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
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
    let shared_runner = SharedToolRunner::new(runner);
    let descriptor = resolve_destination_descriptor(
        repo_root,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
    )?;
    require_capability(&descriptor, DestinationCapability::Screenshot)?;
    let mut session = EvaluationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
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
    let shared_runner = SharedToolRunner::new(runner);
    let descriptor = resolve_destination_descriptor(
        repo_root,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
    )?;
    require_capability(&descriptor, DestinationCapability::Logs)?;
    let mut session = EvaluationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
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
    let shared_runner = SharedToolRunner::new(runner);
    let descriptor = resolve_destination_descriptor(
        repo_root,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
    )?;
    require_capability(&descriptor, DestinationCapability::Video)?;
    let mut session = EvaluationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
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
    let shared_runner = SharedToolRunner::new(runner);

    let descriptor = resolve_destination_descriptor(
        repo_root,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
    )?;
    require_plan_capabilities(&descriptor, &plan)?;

    let started_at_ms = timestamp_millis();
    let mut session = EvaluationSession::new(
        repo_root,
        manifest,
        registry,
        backend_id,
        destination_id,
        &shared_runner,
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

#[expect(
    clippy::too_many_lines,
    reason = "Step dispatch enumerates every public evaluation step in one place for the evaluator."
)]
fn execute_step(
    index: usize,
    step: EvaluationStep,
    artifacts_dir: &Utf8Path,
    session: &mut EvaluationSession<'_>,
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
        EvaluationStep::DebugTap { target_id, x, y } => execute_debug_interaction_step(
            index,
            "debug_tap",
            started_at_ms,
            session,
            InteractionRequest::Tap { target_id, x, y },
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
        EvaluationStep::DebugLaunch { debugger } => {
            execute_debug_launch_step(index, started_at_ms, session, debugger)
        }
        EvaluationStep::DebugAttach { debugger } => {
            execute_debug_attach_step(index, started_at_ms, session, debugger)
        }
        EvaluationStep::DebugSetBreakpoint {
            debugger,
            file,
            line,
            name,
        } => execute_debug_set_breakpoint_step(
            index,
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            debugger,
            file,
            line,
            name,
        ),
        EvaluationStep::DebugClearBreakpoint {
            debugger,
            file,
            line,
        } => {
            execute_debug_clear_breakpoint_step(index, started_at_ms, session, debugger, file, line)
        }
        EvaluationStep::DebugWaitForStop {
            debugger,
            timeout_ms,
            name,
        } => execute_debug_wait_for_stop_step(
            index,
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            debugger,
            timeout_ms,
            name,
        ),
        EvaluationStep::DebugThreads { debugger, name } => execute_debug_threads_step(
            index,
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            debugger,
            name,
        ),
        EvaluationStep::DebugBacktrace {
            debugger,
            thread_id,
            name,
        } => execute_debug_backtrace_step(
            index,
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            debugger,
            thread_id.as_deref(),
            name,
        ),
        EvaluationStep::DebugPause { debugger, name } => execute_debug_pause_step(
            index,
            started_at_ms,
            artifacts_dir,
            session,
            artifacts,
            debugger,
            name,
        ),
        EvaluationStep::DebugResume { debugger } => {
            execute_debug_resume_step(index, started_at_ms, session, debugger)
        }
    }
}

fn execute_launch_step(
    index: usize,
    started_at_ms: u128,
    session: &mut EvaluationSession<'_>,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    Ok(simple_step(index, "launch", started_at_ms))
}

fn execute_wait_for_ui_step(
    index: usize,
    started_at_ms: u128,
    session: &mut EvaluationSession<'_>,
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
    session: &mut EvaluationSession<'_>,
    request: InteractionRequest,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    session.interact(request)?;
    Ok(simple_step(index, kind, started_at_ms))
}

fn execute_debug_interaction_step(
    index: usize,
    kind: &str,
    started_at_ms: u128,
    session: &mut EvaluationSession<'_>,
    request: InteractionRequest,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    session.interact_without_snapshot(request)?;
    Ok(simple_step(index, kind, started_at_ms))
}

fn execute_screenshot_step(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut EvaluationSession<'_>,
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
    session: &mut EvaluationSession<'_>,
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
    session: &mut EvaluationSession<'_>,
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
    session: &mut EvaluationSession<'_>,
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
    session: &mut EvaluationSession<'_>,
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

fn execute_debug_launch_step(
    index: usize,
    started_at_ms: u128,
    session: &mut EvaluationSession<'_>,
    debugger: DebuggerKind,
) -> AtomResult<StepRecord> {
    session.debug_launch(debugger)?;
    Ok(simple_step(index, "debug_launch", started_at_ms))
}

fn execute_debug_attach_step(
    index: usize,
    started_at_ms: u128,
    session: &mut EvaluationSession<'_>,
    debugger: DebuggerKind,
) -> AtomResult<StepRecord> {
    session.debug_attach(debugger)?;
    Ok(simple_step(index, "debug_attach", started_at_ms))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Debug breakpoint steps need explicit artifact, session, and source location inputs."
)]
fn execute_debug_set_breakpoint_step(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut EvaluationSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
    debugger: DebuggerKind,
    file: String,
    line: u32,
    name: Option<String>,
) -> AtomResult<StepRecord> {
    let breakpoint = session.debug_set_breakpoint(debugger, file, line)?;
    let artifact_name = artifact_name(name, index, "breakpoint", "json");
    let output_path = artifacts_dir.join(&artifact_name);
    write_json(&output_path, &breakpoint)?;
    artifacts.push(ArtifactRecord {
        name: artifact_name.clone(),
        kind: "debug_breakpoint".to_owned(),
        path: output_path.as_str().to_owned(),
    });
    Ok(step_with_artifacts(
        index,
        "debug_set_breakpoint",
        started_at_ms,
        vec![artifact_name],
    ))
}

fn execute_debug_clear_breakpoint_step(
    index: usize,
    started_at_ms: u128,
    session: &mut EvaluationSession<'_>,
    debugger: DebuggerKind,
    file: String,
    line: u32,
) -> AtomResult<StepRecord> {
    session.debug_clear_breakpoint(debugger, file, line)?;
    Ok(simple_step(index, "debug_clear_breakpoint", started_at_ms))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Debug stop steps need explicit artifact, session, and timeout inputs."
)]
fn execute_debug_wait_for_stop_step(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut EvaluationSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
    debugger: DebuggerKind,
    timeout_ms: Option<u64>,
    name: Option<String>,
) -> AtomResult<StepRecord> {
    let stop = session.debug_wait_for_stop(debugger, timeout_ms)?;
    write_debug_stop_artifact(
        index,
        "debug_wait_for_stop",
        started_at_ms,
        artifacts_dir,
        artifacts,
        name,
        "stop",
        &stop,
    )
}

fn execute_debug_threads_step(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut EvaluationSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
    debugger: DebuggerKind,
    name: Option<String>,
) -> AtomResult<StepRecord> {
    let threads = session.debug_threads(debugger)?;
    write_debug_json_artifact(
        index,
        "debug_threads",
        started_at_ms,
        artifacts_dir,
        artifacts,
        name,
        "threads",
        "debug_threads",
        &threads,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "Debug backtrace steps need explicit artifact, session, debugger, and thread selection inputs."
)]
fn execute_debug_backtrace_step(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut EvaluationSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
    debugger: DebuggerKind,
    thread_id: Option<&str>,
    name: Option<String>,
) -> AtomResult<StepRecord> {
    let backtrace = session.debug_backtrace(debugger, thread_id)?;
    write_debug_json_artifact(
        index,
        "debug_backtrace",
        started_at_ms,
        artifacts_dir,
        artifacts,
        name,
        "backtrace",
        "debug_backtrace",
        &backtrace,
    )
}

fn execute_debug_pause_step(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut EvaluationSession<'_>,
    artifacts: &mut Vec<ArtifactRecord>,
    debugger: DebuggerKind,
    name: Option<String>,
) -> AtomResult<StepRecord> {
    let stop = session.debug_pause(debugger)?;
    write_debug_stop_artifact(
        index,
        "debug_pause",
        started_at_ms,
        artifacts_dir,
        artifacts,
        name,
        "pause",
        &stop,
    )
}

fn execute_debug_resume_step(
    index: usize,
    started_at_ms: u128,
    session: &mut EvaluationSession<'_>,
    debugger: DebuggerKind,
) -> AtomResult<StepRecord> {
    session.debug_resume(debugger)?;
    Ok(simple_step(index, "debug_resume", started_at_ms))
}

#[expect(
    clippy::too_many_arguments,
    reason = "Debug stop artifacts need explicit step metadata plus stop payload details."
)]
fn write_debug_stop_artifact(
    index: usize,
    kind: &str,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    artifacts: &mut Vec<ArtifactRecord>,
    name: Option<String>,
    prefix: &str,
    stop: &DebugStop,
) -> AtomResult<StepRecord> {
    write_debug_json_artifact(
        index,
        kind,
        started_at_ms,
        artifacts_dir,
        artifacts,
        name,
        prefix,
        "debug_stop",
        stop,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "Debug JSON artifacts need explicit step metadata, output naming, and payload inputs."
)]
fn write_debug_json_artifact<T: Serialize>(
    index: usize,
    kind: &str,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    artifacts: &mut Vec<ArtifactRecord>,
    name: Option<String>,
    prefix: &str,
    artifact_kind: &str,
    value: &T,
) -> AtomResult<StepRecord> {
    let artifact_name = artifact_name(name, index, prefix, "json");
    let output_path = artifacts_dir.join(&artifact_name);
    write_json(&output_path, value)?;
    artifacts.push(ArtifactRecord {
        name: artifact_name.clone(),
        kind: artifact_kind.to_owned(),
        path: output_path.as_str().to_owned(),
    });
    Ok(step_with_artifacts(
        index,
        kind,
        started_at_ms,
        vec![artifact_name],
    ))
}

fn wait_for_ui(
    session: &mut EvaluationSession<'_>,
    target_id: Option<&str>,
    text: Option<&str>,
    timeout_ms: u64,
) -> AtomResult<()> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        match session.interact(InteractionRequest::InspectUi) {
            Ok(result) => {
                if snapshot_matches(&result.snapshot, target_id, text) {
                    return Ok(());
                }
            }
            Err(error) if error.code == AtomErrorCode::AutomationUnavailable => {}
            Err(error) => return Err(error),
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
    runner: &SharedToolRunner<'_>,
) -> AtomResult<DestinationDescriptor> {
    let destinations = {
        let mut runner = runner.borrow_mut();
        list_backend_destinations(repo_root, registry, backend_id, &mut **runner)?
    };
    if let Some(destination) = destinations
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
            | EvaluationStep::TypeText { .. }
            | EvaluationStep::DebugTap { .. } => DestinationCapability::Interact,
            EvaluationStep::Screenshot { .. } => DestinationCapability::Screenshot,
            EvaluationStep::StartVideo { .. } | EvaluationStep::StopVideo => {
                DestinationCapability::Video
            }
            EvaluationStep::CollectLogs { .. } => DestinationCapability::Logs,
            EvaluationStep::DebugLaunch { .. }
            | EvaluationStep::DebugAttach { .. }
            | EvaluationStep::DebugSetBreakpoint { .. }
            | EvaluationStep::DebugClearBreakpoint { .. }
            | EvaluationStep::DebugWaitForStop { .. }
            | EvaluationStep::DebugThreads { .. }
            | EvaluationStep::DebugBacktrace { .. }
            | EvaluationStep::DebugPause { .. }
            | EvaluationStep::DebugResume { .. } => DestinationCapability::Debug,
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
        EvaluationStep::DebugTap { .. } => "debug_tap",
        EvaluationStep::Screenshot { .. } => "screenshot",
        EvaluationStep::InspectUi { .. } => "inspect_ui",
        EvaluationStep::StartVideo { .. } => "start_video",
        EvaluationStep::StopVideo => "stop_video",
        EvaluationStep::CollectLogs { .. } => "collect_logs",
        EvaluationStep::DebugLaunch { .. } => "debug_launch",
        EvaluationStep::DebugAttach { .. } => "debug_attach",
        EvaluationStep::DebugSetBreakpoint { .. } => "debug_set_breakpoint",
        EvaluationStep::DebugClearBreakpoint { .. } => "debug_clear_breakpoint",
        EvaluationStep::DebugWaitForStop { .. } => "debug_wait_for_stop",
        EvaluationStep::DebugThreads { .. } => "debug_threads",
        EvaluationStep::DebugBacktrace { .. } => "debug_backtrace",
        EvaluationStep::DebugPause { .. } => "debug_pause",
        EvaluationStep::DebugResume { .. } => "debug_resume",
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

struct EvaluationSession<'a> {
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    registry: &'a DeployBackendRegistry,
    backend_id: &'a str,
    destination_id: &'a str,
    runner: &'a SharedToolRunner<'a>,
    descriptor: DestinationDescriptor,
    automation: Box<dyn BackendAutomationSession + 'a>,
    automation_started: bool,
    debug_sessions: BTreeMap<DebuggerKind, Box<dyn BackendDebugSession + 'a>>,
}

impl<'a> EvaluationSession<'a> {
    #[expect(
        clippy::too_many_arguments,
        reason = "Automation sessions are assembled from explicit repo, manifest, registry, destination, and launch inputs."
    )]
    fn new(
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        registry: &'a DeployBackendRegistry,
        backend_id: &'a str,
        destination_id: &'a str,
        runner: &'a SharedToolRunner<'a>,
        descriptor: DestinationDescriptor,
        launch_behavior: SessionLaunchBehavior,
    ) -> AtomResult<Self> {
        debug_assert_eq!(descriptor.id, destination_id);
        let automation = automation_session_with_registry(
            registry,
            repo_root,
            manifest,
            backend_id,
            destination_id,
            runner,
            launch_behavior,
        )?;
        Ok(Self {
            repo_root,
            manifest,
            registry,
            backend_id,
            destination_id,
            runner,
            descriptor,
            automation,
            automation_started: false,
            debug_sessions: BTreeMap::new(),
        })
    }

    fn ensure_launched(&mut self) -> AtomResult<()> {
        if !self.automation_started
            && !self.debug_sessions.is_empty()
            && self.automation.attach_existing()?
        {
            self.automation_started = true;
            return Ok(());
        }
        self.automation.ensure_launched()?;
        self.automation_started = true;
        Ok(())
    }

    fn interact(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult> {
        self.automation.interact(request)
    }

    fn interact_without_snapshot(&mut self, request: InteractionRequest) -> AtomResult<()> {
        self.automation.interact_without_snapshot(request)
    }

    fn video_extension(&self) -> &'static str {
        self.automation.video_extension()
    }

    fn capture_auto_screenshot(&mut self) -> AtomResult<Utf8PathBuf> {
        self.automation.capture_auto_screenshot()
    }

    fn capture_screenshot(&mut self, output_path: &Utf8Path) -> AtomResult<()> {
        self.automation.capture_screenshot(output_path)
    }

    fn capture_logs(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()> {
        self.automation.capture_logs(output_path, seconds)
    }

    fn capture_video(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()> {
        self.automation.capture_video(output_path, seconds)
    }

    fn start_video(&mut self, output_path: &Utf8Path) -> AtomResult<()> {
        self.automation.start_video(output_path)
    }

    fn stop_video(&mut self) -> AtomResult<Utf8PathBuf> {
        self.automation.stop_video()
    }

    fn shutdown_video(&mut self) -> AtomResult<()> {
        self.automation.shutdown_video()
    }

    fn debug_launch(&mut self, debugger: DebuggerKind) -> AtomResult<()> {
        self.debug_session(debugger, SessionLaunchBehavior::LaunchOnly)?
            .launch()
    }

    fn debug_attach(&mut self, debugger: DebuggerKind) -> AtomResult<()> {
        self.debug_session(debugger, SessionLaunchBehavior::AttachOrLaunch)?
            .attach()
    }

    fn debug_set_breakpoint(
        &mut self,
        debugger: DebuggerKind,
        file: String,
        line: u32,
    ) -> AtomResult<DebugBreakpoint> {
        self.require_debug_session(debugger)?
            .set_breakpoint(DebugSourceLocation { file, line })
    }

    fn debug_clear_breakpoint(
        &mut self,
        debugger: DebuggerKind,
        file: String,
        line: u32,
    ) -> AtomResult<()> {
        self.require_debug_session(debugger)?
            .clear_breakpoint(DebugSourceLocation { file, line })
    }

    fn debug_wait_for_stop(
        &mut self,
        debugger: DebuggerKind,
        timeout_ms: Option<u64>,
    ) -> AtomResult<DebugStop> {
        self.require_debug_session(debugger)?
            .wait_for_stop(timeout_ms)
    }

    fn debug_threads(&mut self, debugger: DebuggerKind) -> AtomResult<Vec<DebugThread>> {
        self.require_debug_session(debugger)?.threads()
    }

    fn debug_backtrace(
        &mut self,
        debugger: DebuggerKind,
        thread_id: Option<&str>,
    ) -> AtomResult<DebugBacktrace> {
        self.require_debug_session(debugger)?.backtrace(thread_id)
    }

    fn debug_pause(&mut self, debugger: DebuggerKind) -> AtomResult<DebugStop> {
        self.require_debug_session(debugger)?.pause()
    }

    fn debug_resume(&mut self, debugger: DebuggerKind) -> AtomResult<()> {
        self.require_debug_session(debugger)?.resume()
    }

    fn shutdown_debug_sessions(&mut self) -> AtomResult<()> {
        for session in self.debug_sessions.values_mut() {
            session.shutdown()?;
        }
        Ok(())
    }

    fn debug_session(
        &mut self,
        debugger: DebuggerKind,
        launch_behavior: SessionLaunchBehavior,
    ) -> AtomResult<&mut (dyn BackendDebugSession + '_)> {
        use std::collections::btree_map::Entry;

        match self.debug_sessions.entry(debugger) {
            Entry::Occupied(entry) => Ok(entry.into_mut().as_mut()),
            Entry::Vacant(entry) => {
                let session = debug_session_with_registry(
                    self.registry,
                    self.repo_root,
                    self.manifest,
                    self.backend_id,
                    self.destination_id,
                    self.runner,
                    launch_behavior,
                    debugger,
                )?;
                Ok(entry.insert(session).as_mut())
            }
        }
    }

    fn require_debug_session(
        &mut self,
        debugger: DebuggerKind,
    ) -> AtomResult<&mut (dyn BackendDebugSession + '_)> {
        let session = self.debug_sessions.get_mut(&debugger).ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                format!("debugger session {debugger:?} has not been attached or launched"),
            )
        })?;
        Ok(&mut **session)
    }
}

fn automation_session_with_registry<'a>(
    registry: &DeployBackendRegistry,
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    backend_id: &'a str,
    destination_id: &'a str,
    runner: &'a SharedToolRunner<'a>,
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
        BackendAutomationSession, BackendDebugSession, BackendDefinition, DebuggerKind,
        DeployBackend, DeployBackendRegistry, DestinationCapability, DestinationDescriptor,
        SharedToolRunner, ToolRunner,
    };
    use atom_manifest::{NormalizedManifest, testing::fixture_manifest};
    use camino::{Utf8Path, Utf8PathBuf};
    use tempfile::tempdir;

    use super::{
        EvaluationPlan, EvaluationStep, InteractionRequest, ScreenInfo, SessionLaunchBehavior,
        UiBounds, UiNode, UiSnapshot, load_evaluation_plan, require_plan_capabilities,
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
                debuggers: Vec::new(),
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

        fn new_automation_session<'a>(
            &self,
            _repo_root: &'a Utf8Path,
            _manifest: &'a NormalizedManifest,
            _destination_id: &'a str,
            _runner: &'a SharedToolRunner<'a>,
            _launch_behavior: SessionLaunchBehavior,
        ) -> atom_ffi::AtomResult<Box<dyn BackendAutomationSession + 'a>> {
            Ok(Box::new(FixtureSession::default()))
        }

        fn new_debug_session<'a>(
            &self,
            _repo_root: &'a Utf8Path,
            _manifest: &'a NormalizedManifest,
            _destination_id: &'a str,
            _runner: &'a SharedToolRunner<'a>,
            _launch_behavior: SessionLaunchBehavior,
            _debugger: DebuggerKind,
        ) -> atom_ffi::AtomResult<Box<dyn BackendDebugSession + 'a>> {
            unreachable!("evaluation tests do not construct debug sessions")
        }
    }

    #[derive(Default)]
    struct FixtureSession {
        snapshots: VecDeque<UiSnapshot>,
    }

    impl BackendAutomationSession for FixtureSession {
        fn video_extension(&self) -> &'static str {
            "mp4"
        }

        fn attach_existing(&mut self) -> atom_ffi::AtomResult<bool> {
            Ok(false)
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

        fn interact_without_snapshot(
            &mut self,
            _request: InteractionRequest,
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
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
            debuggers: Vec::new(),
        };
        let plan = EvaluationPlan {
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
        fs::write(&plan_path, "{ \"steps\": [ { \"kind\": \"launch\" } ] }").expect("write");

        let plan = load_evaluation_plan(&plan_path).expect("plan should load");
        assert_eq!(plan.steps, vec![EvaluationStep::Launch]);
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
}
