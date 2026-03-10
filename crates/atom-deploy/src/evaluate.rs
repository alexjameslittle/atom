use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::deploy::{generated_target, ios_bazel_args, resolve_ios_installable_artifact};
use crate::destinations::{
    DestinationCapability, DestinationDescriptor, DestinationKind, DestinationPlatform,
    list_destinations,
};
use crate::devices::android::{
    list_android_destinations, prepare_android_emulator, resolve_android_device,
};
use crate::devices::ios::{
    IosDestination, IosDestinationKind, list_ios_destinations, prepare_ios_simulator,
};
use crate::tools::{ToolRunner, capture_tool, find_bazel_output_owned, run_bazel_owned, run_tool};

const AUTOMATION_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const AUTOMATION_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiNode {
    pub id: String,
    pub role: String,
    pub label: String,
    pub text: String,
    pub visible: bool,
    pub enabled: bool,
    pub bounds: UiBounds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenInfo {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub screen: ScreenInfo,
    pub nodes: Vec<UiNode>,
    #[serde(default)]
    pub screenshot_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InteractionRequest {
    InspectUi,
    Tap {
        #[serde(default)]
        target_id: Option<String>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    LongPress {
        #[serde(default)]
        target_id: Option<String>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    Swipe {
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    Drag {
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    TypeText {
        #[serde(default)]
        target_id: Option<String>,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionResult {
    pub ok: bool,
    pub snapshot: UiSnapshot,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvaluationStep {
    Launch,
    WaitForUi {
        #[serde(default)]
        target_id: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    Tap {
        #[serde(default)]
        target_id: Option<String>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    LongPress {
        #[serde(default)]
        target_id: Option<String>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    Swipe {
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    Drag {
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    TypeText {
        #[serde(default)]
        target_id: Option<String>,
        text: String,
    },
    Screenshot {
        #[serde(default)]
        name: Option<String>,
    },
    InspectUi {
        #[serde(default)]
        name: Option<String>,
    },
    StartVideo {
        #[serde(default)]
        name: Option<String>,
    },
    StopVideo,
    CollectLogs {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        seconds: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationPlan {
    pub steps: Vec<EvaluationStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub name: String,
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepRecord {
    pub index: usize,
    pub kind: String,
    pub ok: bool,
    pub started_at_ms: u128,
    pub finished_at_ms: u128,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationBundleManifest {
    pub target_label: String,
    pub destination: DestinationDescriptor,
    pub started_at_ms: u128,
    pub finished_at_ms: u128,
    pub transcript_path: String,
    pub steps: Vec<StepRecord>,
    pub artifacts: Vec<ArtifactRecord>,
}

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
    destination_id: &str,
    runner: &mut impl ToolRunner,
) -> AtomResult<UiSnapshot> {
    let descriptor = resolve_destination_descriptor(repo_root, destination_id, runner)?;
    require_capability(&descriptor, DestinationCapability::InspectUi)?;
    let mut session =
        AutomationSession::new(repo_root, manifest, destination_id, runner, descriptor);
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
    destination_id: &str,
    request: InteractionRequest,
    runner: &mut impl ToolRunner,
) -> AtomResult<InteractionResult> {
    let descriptor = resolve_destination_descriptor(repo_root, destination_id, runner)?;
    require_capability(&descriptor, DestinationCapability::Interact)?;
    let mut session =
        AutomationSession::new(repo_root, manifest, destination_id, runner, descriptor);
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
    destination_id: &str,
    output_path: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let descriptor = resolve_destination_descriptor(repo_root, destination_id, runner)?;
    require_capability(&descriptor, DestinationCapability::Screenshot)?;
    let app = AppLaunch::launch(repo_root, manifest, destination_id, runner, None)?;
    capture_screenshot_for_launch(repo_root, &app, output_path, runner)
}

/// # Errors
///
/// Returns an error if destination resolution, app launch, or log capture fails.
pub fn capture_logs(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination_id: &str,
    output_path: &Utf8Path,
    seconds: u64,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let descriptor = resolve_destination_descriptor(repo_root, destination_id, runner)?;
    require_capability(&descriptor, DestinationCapability::Logs)?;
    let app = AppLaunch::launch(repo_root, manifest, destination_id, runner, None)?;
    capture_logs_for_launch(repo_root, &app, output_path, seconds, runner)
}

/// # Errors
///
/// Returns an error if destination resolution, app launch, or video capture fails.
pub fn capture_video(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination_id: &str,
    output_path: &Utf8Path,
    seconds: u64,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let descriptor = resolve_destination_descriptor(repo_root, destination_id, runner)?;
    require_capability(&descriptor, DestinationCapability::Video)?;
    let app = AppLaunch::launch(repo_root, manifest, destination_id, runner, None)?;
    capture_video_for_launch(repo_root, &app, output_path, seconds, runner)
}

/// # Errors
///
/// Returns an error if the evaluation plan cannot be loaded, a required capability is unavailable,
/// or any step fails while executing.
pub fn evaluate_run(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination_id: &str,
    plan_path: &Utf8Path,
    artifacts_dir: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<EvaluateCommandOutput> {
    let plan = load_evaluation_plan(plan_path)?;
    write_parent_dir(artifacts_dir)?;

    let descriptor = resolve_destination_descriptor(repo_root, destination_id, runner)?;
    require_plan_capabilities(&descriptor, &plan)?;

    let started_at_ms = timestamp_millis();
    let mut session =
        AutomationSession::new(repo_root, manifest, destination_id, runner, descriptor);
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

fn execute_step<R: ToolRunner>(
    index: usize,
    step: EvaluationStep,
    artifacts_dir: &Utf8Path,
    session: &mut AutomationSession<'_, R>,
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

fn execute_launch_step<R: ToolRunner>(
    index: usize,
    started_at_ms: u128,
    session: &mut AutomationSession<'_, R>,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    Ok(simple_step(index, "launch", started_at_ms))
}

fn execute_wait_for_ui_step<R: ToolRunner>(
    index: usize,
    started_at_ms: u128,
    session: &mut AutomationSession<'_, R>,
    target_id: Option<&str>,
    text: Option<&str>,
    timeout_ms: u64,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    wait_for_ui(session, target_id, text, timeout_ms)?;
    Ok(simple_step(index, "wait_for_ui", started_at_ms))
}

fn execute_interaction_step<R: ToolRunner>(
    index: usize,
    kind: &str,
    started_at_ms: u128,
    session: &mut AutomationSession<'_, R>,
    request: InteractionRequest,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    session.interact(request)?;
    Ok(simple_step(index, kind, started_at_ms))
}

fn execute_screenshot_step<R: ToolRunner>(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut AutomationSession<'_, R>,
    artifacts: &mut Vec<ArtifactRecord>,
    name: Option<String>,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    let artifact_name = artifact_name(name, index, "screenshot", "png");
    let output_path = artifacts_dir.join(&artifact_name);
    capture_screenshot_for_launch(
        session.repo_root,
        session.launch.as_ref().expect("launch"),
        &output_path,
        session.runner,
    )?;
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

fn execute_inspect_ui_step<R: ToolRunner>(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut AutomationSession<'_, R>,
    artifacts: &mut Vec<ArtifactRecord>,
    name: Option<String>,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    let artifact_file_name = artifact_name(name, index, "inspect", "json");
    let output_path = artifacts_dir.join(&artifact_file_name);
    let mut snapshot = session.interact(InteractionRequest::InspectUi)?.snapshot;
    let screenshot_name = artifact_name(None, index, "inspect", "png");
    let screenshot_path = artifacts_dir.join(&screenshot_name);
    capture_screenshot_for_launch(
        session.repo_root,
        session.launch.as_ref().expect("launch"),
        &screenshot_path,
        session.runner,
    )?;
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

fn execute_start_video_step<R: ToolRunner>(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut AutomationSession<'_, R>,
    name: Option<String>,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    let artifact_name = artifact_name(name, index, "video", "mp4");
    let output_path = artifacts_dir.join(&artifact_name);
    session.start_video(&output_path)?;
    Ok(step_with_artifacts(
        index,
        "start_video",
        started_at_ms,
        vec![artifact_name],
    ))
}

fn execute_stop_video_step<R: ToolRunner>(
    index: usize,
    started_at_ms: u128,
    session: &mut AutomationSession<'_, R>,
    artifacts: &mut Vec<ArtifactRecord>,
) -> AtomResult<StepRecord> {
    let output_path = session.stop_video()?;
    let artifact_name = output_path
        .file_name()
        .map_or_else(|| "video.mp4".to_owned(), ToOwned::to_owned);
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

fn execute_collect_logs_step<R: ToolRunner>(
    index: usize,
    started_at_ms: u128,
    artifacts_dir: &Utf8Path,
    session: &mut AutomationSession<'_, R>,
    artifacts: &mut Vec<ArtifactRecord>,
    name: Option<String>,
    seconds: u64,
) -> AtomResult<StepRecord> {
    session.ensure_launched()?;
    let artifact_name = artifact_name(name, index, "logs", "txt");
    let output_path = artifacts_dir.join(&artifact_name);
    capture_logs_for_launch(
        session.repo_root,
        session.launch.as_ref().expect("launch"),
        &output_path,
        seconds,
        session.runner,
    )?;
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

fn wait_for_ui<R: ToolRunner>(
    session: &mut AutomationSession<'_, R>,
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
    destination_id: &str,
    runner: &mut impl ToolRunner,
) -> AtomResult<DestinationDescriptor> {
    list_destinations(repo_root, runner)?
        .into_iter()
        .find(|destination| destination.id == destination_id)
        .ok_or_else(|| {
            AtomError::with_path(
                AtomErrorCode::AutomationUnavailable,
                format!("unknown destination id: {destination_id}"),
                destination_id,
            )
        })
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

struct AutomationSession<'a, R: ToolRunner> {
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    destination_id: &'a str,
    runner: &'a mut R,
    descriptor: DestinationDescriptor,
    launch: Option<AppLaunch>,
    automation_server: Option<AutomationServer>,
    video_capture: Option<VideoCapture>,
}

impl<'a, R: ToolRunner> AutomationSession<'a, R> {
    fn new(
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        destination_id: &'a str,
        runner: &'a mut R,
        descriptor: DestinationDescriptor,
    ) -> Self {
        Self {
            repo_root,
            manifest,
            destination_id,
            runner,
            descriptor,
            launch: None,
            automation_server: None,
            video_capture: None,
        }
    }

    fn ensure_launched(&mut self) -> AtomResult<()> {
        if self.launch.is_some() {
            return Ok(());
        }
        if uses_idb_ios_simulator(&self.descriptor) {
            self.launch = Some(AppLaunch::launch(
                self.repo_root,
                self.manifest,
                self.destination_id,
                self.runner,
                None,
            )?);
            return Ok(());
        }
        if !self.manifest.app.automation_fixture {
            return Err(AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                "this app target does not enable the automation fixture",
            ));
        }

        let server = AutomationServer::new()?;
        let launch_parameters = server.launch_parameters(self.descriptor.platform);
        let launch = AppLaunch::launch(
            self.repo_root,
            self.manifest,
            self.destination_id,
            self.runner,
            Some(&launch_parameters),
        )?;
        server.wait_for_registration(AUTOMATION_CONNECT_TIMEOUT)?;
        self.automation_server = Some(server);
        self.launch = Some(launch);
        Ok(())
    }

    fn interact(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult> {
        self.ensure_launched()?;
        if uses_idb_ios_simulator(&self.descriptor) {
            return interact_with_idb(
                self.repo_root,
                self.descriptor.id.as_str(),
                self.runner,
                request,
            );
        }
        let server = self.automation_server.as_mut().expect("automation server");
        server.send_command(request)
    }

    fn capture_auto_screenshot(&mut self) -> AtomResult<Utf8PathBuf> {
        let root = self.repo_root.join("cng-output").join("artifacts");
        write_parent_dir(&root)?;
        let path = root.join(format!("inspect-{}.png", timestamp_suffix()));
        capture_screenshot_for_launch(
            self.repo_root,
            self.launch.as_ref().expect("launch"),
            &path,
            self.runner,
        )?;
        Ok(path)
    }

    fn start_video(&mut self, output_path: &Utf8Path) -> AtomResult<()> {
        let launch = self.launch.as_ref().ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                "app must be launched before recording video",
            )
        })?;
        self.video_capture = Some(start_video_capture(self.repo_root, launch, output_path)?);
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

#[derive(Clone)]
struct LaunchParameters {
    base_url: String,
    token: String,
}

struct AutomationServer {
    shared: Arc<(Mutex<ServerState>, Condvar)>,
    handle: Option<thread::JoinHandle<()>>,
    port: u16,
    token: String,
}

#[derive(Default)]
struct ServerState {
    connected: bool,
    stop: bool,
    next_command: Option<ServerCommand>,
    completed: BTreeMap<String, InteractionResult>,
    command_index: u64,
}

struct ServerCommand {
    payload: String,
    claimed: bool,
}

impl AutomationServer {
    fn new() -> AtomResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| {
            AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                format!("failed to bind automation session port: {error}"),
            )
        })?;
        listener.set_nonblocking(true).map_err(|error| {
            AtomError::new(AtomErrorCode::AutomationUnavailable, error.to_string())
        })?;
        let port = listener
            .local_addr()
            .map_err(|error| {
                AtomError::new(AtomErrorCode::AutomationUnavailable, error.to_string())
            })?
            .port();
        let shared = Arc::new((Mutex::new(ServerState::default()), Condvar::new()));
        let thread_shared = Arc::clone(&shared);
        let handle = thread::spawn(move || {
            server_loop(&listener, &thread_shared);
        });

        Ok(Self {
            shared,
            handle: Some(handle),
            port,
            token: format!("atom-{}", timestamp_suffix()),
        })
    }

    fn launch_parameters(&self, _platform: DestinationPlatform) -> LaunchParameters {
        let base_url = format!("http://127.0.0.1:{}/", self.port);
        LaunchParameters {
            base_url,
            token: self.token.clone(),
        }
    }

    fn wait_for_registration(&self, timeout: Duration) -> AtomResult<()> {
        let (lock, cvar) = &*self.shared;
        let deadline = Instant::now() + timeout;
        let mut state = lock.lock().expect("lock");
        while !state.connected {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let (next_state, _) = cvar.wait_timeout(state, remaining).expect("wait");
            state = next_state;
        }
        if state.connected {
            Ok(())
        } else {
            Err(AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                "automation fixture did not connect back to the host session",
            ))
        }
    }

    fn send_command(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult> {
        let command_id = {
            let (lock, cvar) = &*self.shared;
            let mut state = lock.lock().expect("lock");
            state.command_index += 1;
            let id = format!("cmd-{}", state.command_index);
            let envelope = AutomationCommandEnvelope {
                id: id.clone(),
                request,
            };
            let payload = serde_json::to_string(&envelope).map_err(|error| {
                AtomError::new(
                    AtomErrorCode::AutomationUnavailable,
                    format!("failed to encode automation command: {error}"),
                )
            })?;
            state.next_command = Some(ServerCommand {
                payload: payload.clone(),
                claimed: false,
            });
            cvar.notify_all();
            id
        };

        let (lock, cvar) = &*self.shared;
        let deadline = Instant::now() + AUTOMATION_COMMAND_TIMEOUT;
        let mut state = lock.lock().expect("lock");
        while !state.completed.contains_key(&command_id) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let (next_state, _) = cvar.wait_timeout(state, remaining).expect("wait");
            state = next_state;
        }
        state.completed.remove(&command_id).ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                "automation command timed out waiting for the app fixture",
            )
        })
    }
}

impl Drop for AutomationServer {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.stop = true;
            cvar.notify_all();
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn server_loop(listener: &TcpListener, shared: &Arc<(Mutex<ServerState>, Condvar)>) {
    loop {
        {
            let (lock, _) = &**shared;
            if lock.lock().map(|state| state.stop).unwrap_or(true) {
                break;
            }
        }

        match listener.accept() {
            Ok((stream, _)) => {
                let _ = handle_stream(stream, shared);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "The lightweight host HTTP bridge keeps request parsing and dispatch together."
)]
fn handle_stream(
    mut stream: TcpStream,
    shared: &Arc<(Mutex<ServerState>, Condvar)>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    if request_line.trim().is_empty() {
        return Ok(());
    }
    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }
    let body = String::from_utf8_lossy(&body).to_string();

    let mut segments = request_line.split_whitespace();
    let method = segments.next().unwrap_or_default();
    let path = segments.next().unwrap_or_default();

    let response = match (method, path.split('?').next().unwrap_or_default()) {
        ("POST", "/register") => {
            if let Ok(payload) = serde_json::from_str::<Value>(&body) {
                let token = payload
                    .get("token")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let (lock, cvar) = &**shared;
                let mut state = lock.lock().expect("lock");
                if !token.is_empty() {
                    state.connected = true;
                    cvar.notify_all();
                }
            }
            http_response(200, "{\"status\":\"ok\"}")
        }
        ("GET", "/next") => {
            let token = path
                .split_once('?')
                .map(|(_, query)| query)
                .unwrap_or_default();
            if token.contains("token=") {
                let (lock, cvar) = &**shared;
                let mut state = lock.lock().expect("lock");
                if let Some(command) = &mut state.next_command
                    && !command.claimed
                {
                    command.claimed = true;
                    cvar.notify_all();
                    http_response(200, &command.payload)
                } else {
                    http_empty_response(204)
                }
            } else {
                http_empty_response(400)
            }
        }
        ("POST", "/result") => {
            if let Ok(payload) = serde_json::from_str::<Value>(&body) {
                let command_id = payload
                    .get("command_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
                let message = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let snapshot = payload.get("snapshot").cloned().unwrap_or(Value::Null);
                if !command_id.is_empty()
                    && let Ok(snapshot) = serde_json::from_value::<UiSnapshot>(snapshot)
                {
                    let result = InteractionResult {
                        ok,
                        snapshot,
                        message,
                    };
                    let (lock, cvar) = &**shared;
                    let mut state = lock.lock().expect("lock");
                    state.completed.insert(command_id.clone(), result);
                    state.next_command = None;
                    cvar.notify_all();
                }
            }
            http_response(200, "{\"status\":\"ok\"}")
        }
        _ => http_empty_response(404),
    };

    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn http_response(status: u16, body: &str) -> String {
    let reason = match status {
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn http_empty_response(status: u16) -> String {
    let reason = match status {
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    format!("HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}

#[derive(Debug, Serialize, Deserialize)]
struct AutomationCommandEnvelope {
    id: String,
    #[serde(flatten)]
    request: InteractionRequest,
}

fn uses_idb_ios_simulator(descriptor: &DestinationDescriptor) -> bool {
    descriptor.platform == DestinationPlatform::Ios && descriptor.kind == DestinationKind::Simulator
}

#[expect(
    clippy::too_many_lines,
    reason = "The idb adapter keeps per-command translation in one place for the iOS backend."
)]
fn interact_with_idb(
    repo_root: &Utf8Path,
    destination_id: &str,
    runner: &mut impl ToolRunner,
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
    runner: &mut impl ToolRunner,
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
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    destination_id: &str,
    subcommand: &[String],
) -> AtomResult<()> {
    let args = idb_args(destination_id, subcommand);
    runner.run(repo_root, "idb", &args)
}

fn capture_idb(
    runner: &mut impl ToolRunner,
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

enum AppLaunch {
    IosSimulator { destination_id: String },
    IosDevice { destination_id: String },
    Android { serial: String },
}

impl AppLaunch {
    fn launch(
        repo_root: &Utf8Path,
        manifest: &NormalizedManifest,
        destination_id: &str,
        runner: &mut impl ToolRunner,
        launch_parameters: Option<&LaunchParameters>,
    ) -> AtomResult<Self> {
        if let Some(destination) = list_ios_destinations(repo_root, runner)?
            .into_iter()
            .find(|destination| destination.id == destination_id)
        {
            return launch_ios_app(repo_root, manifest, destination, runner, launch_parameters);
        }
        if list_android_destinations(repo_root, runner)?
            .into_iter()
            .any(|destination| destination.serial == destination_id)
        {
            return launch_android_app(
                repo_root,
                manifest,
                destination_id,
                runner,
                launch_parameters,
            );
        }
        Err(AtomError::new(
            AtomErrorCode::AutomationUnavailable,
            format!("unknown destination id: {destination_id}"),
        ))
    }
}

fn launch_ios_app(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination: IosDestination,
    runner: &mut impl ToolRunner,
    launch_parameters: Option<&LaunchParameters>,
) -> AtomResult<AppLaunch> {
    if !manifest.ios.enabled {
        return Err(AtomError::new(
            AtomErrorCode::ManifestInvalidValue,
            "iOS is not enabled for this target",
        ));
    }
    if destination.kind == IosDestinationKind::Device && launch_parameters.is_some() {
        return Err(AtomError::new(
            AtomErrorCode::AutomationUnavailable,
            "automation sessions are only supported on iOS simulators in this phase",
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
            })
        }
    }
}

fn launch_android_app(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    destination_id: &str,
    runner: &mut impl ToolRunner,
    launch_parameters: Option<&LaunchParameters>,
) -> AtomResult<AppLaunch> {
    if !manifest.android.enabled {
        return Err(AtomError::new(
            AtomErrorCode::ManifestInvalidValue,
            "Android is not enabled for this target",
        ));
    }

    let destination = resolve_android_device(repo_root, runner, Some(destination_id))?;
    let serial = prepare_android_emulator(repo_root, runner, &destination)?;
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
    if let Some(parameters) = launch_parameters
        && let Some(port) = parameters.base_url.rsplit(':').next()
    {
        let port = port.trim_end_matches('/');
        run_tool(
            runner,
            repo_root,
            "adb",
            &[
                "-s",
                &serial,
                "reverse",
                &format!("tcp:{port}"),
                &format!("tcp:{port}"),
            ],
        )?;
    }
    let component = format!("{application_id}/.MainActivity");
    let mut args = vec![
        "-s".to_owned(),
        serial.clone(),
        "shell".to_owned(),
        "am".to_owned(),
        "start".to_owned(),
        "-n".to_owned(),
        component,
    ];
    if let Some(parameters) = launch_parameters {
        args.push("--es".to_owned());
        args.push("atomAutomationUrl".to_owned());
        args.push(parameters.base_url.clone());
        args.push("--es".to_owned());
        args.push("atomAutomationToken".to_owned());
        args.push(parameters.token.clone());
    }
    runner.run(repo_root, "adb", &args)?;

    let _ = application_id;
    Ok(AppLaunch::Android { serial })
}

fn capture_screenshot_for_launch(
    repo_root: &Utf8Path,
    launch: &AppLaunch,
    output_path: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    match launch {
        AppLaunch::IosSimulator { destination_id } | AppLaunch::IosDevice { destination_id } => {
            run_idb(
                runner,
                repo_root,
                destination_id,
                &["screenshot".to_owned(), output_path.as_str().to_owned()],
            )
        }
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

fn capture_logs_for_launch(
    repo_root: &Utf8Path,
    launch: &AppLaunch,
    output_path: &Utf8Path,
    seconds: u64,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    let contents = match launch {
        AppLaunch::IosSimulator { destination_id } | AppLaunch::IosDevice { destination_id } => {
            capture_idb(
                runner,
                repo_root,
                destination_id,
                &[
                    "log".to_owned(),
                    "--".to_owned(),
                    "--style".to_owned(),
                    "syslog".to_owned(),
                    "--timeout".to_owned(),
                    format!("{seconds}s"),
                ],
            )?
        }
        AppLaunch::Android { serial } => {
            capture_tool(runner, repo_root, "adb", &["-s", serial, "logcat", "-d"])?
        }
    };
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
    launch: &AppLaunch,
    output_path: &Utf8Path,
    seconds: u64,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    write_parent_dir(output_path)?;
    match launch {
        AppLaunch::IosSimulator { destination_id } | AppLaunch::IosDevice { destination_id } => {
            let mut child = spawn_idb_video(repo_root, destination_id, output_path)?;
            thread::sleep(Duration::from_secs(seconds));
            let _ = child.kill();
            let _ = child.wait();
            Ok(())
        }
        AppLaunch::Android { serial } => {
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
        AppLaunch::IosSimulator { destination_id } | AppLaunch::IosDevice { destination_id } => {
            let child = spawn_idb_video(repo_root, destination_id, output_path)?;
            Ok(VideoCapture {
                output_path: output_path.to_owned(),
                child,
                remote_path: None,
                platform: DestinationPlatform::Ios,
                serial: None,
            })
        }
        AppLaunch::Android { serial } => {
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
    runner: &mut impl ToolRunner,
) -> AtomResult<Utf8PathBuf> {
    let mut child = video.child;
    let _ = child.kill();
    let _ = child.wait();

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

    Ok(video.output_path)
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
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::thread;
    use std::time::Duration;

    use crate::destinations::DestinationKind;
    use crate::tools::ToolRunner;
    use atom_ffi::AtomErrorCode;
    use camino::Utf8PathBuf;
    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::{
        AutomationServer, DestinationCapability, DestinationDescriptor, DestinationPlatform,
        EvaluationPlan, EvaluationStep, InteractionRequest, interact_with_idb,
        load_evaluation_plan, require_plan_capabilities,
    };

    #[derive(Default)]
    struct FakeToolRunner {
        calls: Vec<(String, Vec<String>)>,
        captures: VecDeque<String>,
    }

    impl ToolRunner for FakeToolRunner {
        fn run(
            &mut self,
            _repo_root: &camino::Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            Ok(())
        }

        fn capture(
            &mut self,
            _repo_root: &camino::Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            self.calls.push((tool.to_owned(), args.to_vec()));
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

    fn send_http_request(port: u16, request_line: &str, body: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        let request = format!(
            "{request_line}\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).expect("write request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        response
    }

    fn response_body(response: &str) -> &str {
        response.split("\r\n\r\n").nth(1).unwrap_or_default()
    }

    #[test]
    fn load_evaluation_plan_reads_json_steps() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let plan_path = root.join("plan.json");
        std::fs::write(
            &plan_path,
            r#"{"steps":[{"kind":"launch"},{"kind":"tap","target_id":"atom.fixture.primary_button"}]}"#,
        )
        .expect("plan");

        let plan = load_evaluation_plan(&plan_path).expect("plan should parse");

        assert_eq!(
            plan,
            EvaluationPlan {
                steps: vec![
                    EvaluationStep::Launch,
                    EvaluationStep::Tap {
                        target_id: Some("atom.fixture.primary_button".to_owned()),
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
    fn automation_server_registers_and_completes_commands() {
        let mut server = AutomationServer::new().expect("server");
        let launch = server.launch_parameters(DestinationPlatform::Ios);
        let port = server.port;
        let token = launch.token.clone();

        let worker = thread::spawn(move || {
            let register_body = json!({
                "token": token,
                "platform": "ios",
            })
            .to_string();
            let register_response =
                send_http_request(port, "POST /register HTTP/1.1", &register_body);
            assert!(register_response.contains("200 OK"));

            let command_payload = loop {
                let response = send_http_request(
                    port,
                    &format!("GET /next?token={} HTTP/1.1", launch.token),
                    "",
                );
                if response.contains("204 No Content") {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                break response_body(&response).to_owned();
            };
            let command: Value =
                serde_json::from_str(&command_payload).expect("command payload should parse");
            assert_eq!(
                command.get("kind").and_then(Value::as_str),
                Some("inspect_ui")
            );

            let result_body = json!({
                "token": launch.token,
                "command_id": command.get("id").and_then(Value::as_str).unwrap_or_default(),
                "ok": true,
                "snapshot": {
                    "screen": {
                        "width": 393.0,
                        "height": 852.0
                    },
                    "nodes": [
                        {
                            "id": "atom.fixture.title",
                            "role": "text",
                            "label": "Hello Atom",
                            "text": "Hello Atom",
                            "visible": true,
                            "enabled": true,
                            "bounds": {
                                "x": 24.0,
                                "y": 112.0,
                                "width": 200.0,
                                "height": 32.0
                            }
                        }
                    ]
                }
            })
            .to_string();
            let result_response = send_http_request(port, "POST /result HTTP/1.1", &result_body);
            assert!(result_response.contains("200 OK"));
        });

        server
            .wait_for_registration(Duration::from_secs(2))
            .expect("fixture should register");
        let result = server
            .send_command(InteractionRequest::InspectUi)
            .expect("command should complete");

        assert!(result.ok);
        assert_eq!(result.snapshot.nodes[0].id, "atom.fixture.title");
        worker.join().expect("worker should exit cleanly");
    }

    #[test]
    fn idb_interactions_round_coordinates_to_integer_strings() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                r#"{"elements":[{"AXUniqueId":"atom.fixture.primary_button","type":"button","AXLabel":"Tap me","AXValue":"Tap me","visible":true,"enabled":true,"frame":{"x":100.4,"y":240.6,"width":201.2,"height":84.8}}]}"#
                    .to_owned(),
                r#"{"elements":[]}"#.to_owned(),
            ]),
        };

        interact_with_idb(
            &root,
            "SIM-123",
            &mut runner,
            InteractionRequest::Tap {
                target_id: Some("atom.fixture.primary_button".to_owned()),
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
}
