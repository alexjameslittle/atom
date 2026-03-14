use std::time::Duration;

use atom_ffi::AtomResult;
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{BackendDefinition, BackendRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Attached,
    Detached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLaunchBehavior {
    AttachOrLaunch,
    LaunchOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppSessionBuildProfile {
    #[default]
    Standard,
    Debugger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppSessionOptions {
    pub launch_behavior: SessionLaunchBehavior,
    pub build_profile: AppSessionBuildProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationCapability {
    Launch,
    Logs,
    Screenshot,
    Video,
    InspectUi,
    Interact,
    Evaluate,
    DebugSession,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationDescriptor {
    pub platform: String,
    pub backend_id: String,
    pub id: String,
    pub kind: String,
    pub display_name: String,
    pub available: bool,
    pub debug_state: String,
    pub capabilities: Vec<DestinationCapability>,
}

pub trait ToolRunner {
    /// # Errors
    ///
    /// Returns an error if the tool invocation fails.
    fn run(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()>;

    /// # Errors
    ///
    /// Returns an error if the tool invocation fails.
    fn capture(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<String>;

    /// # Errors
    ///
    /// Returns an error if the tool invocation fails.
    fn capture_json_file(
        &mut self,
        repo_root: &Utf8Path,
        tool: &str,
        args: &[String],
    ) -> AtomResult<String>;

    /// # Errors
    ///
    /// Returns an error if the tool invocation fails or exits with a non-zero status.
    fn stream(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()>;
}

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
    AttachDebugger,
    InspectDebuggerState,
    WaitForDebuggerStop {
        #[serde(default)]
        timeout_ms: Option<u64>,
    },
    PauseDebugger,
    ResumeDebugger,
    ListDebuggerThreads,
    ListDebuggerFrames {
        #[serde(default)]
        thread_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationPlan {
    #[serde(default)]
    pub build_profile: AppSessionBuildProfile,
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

// Debug-session requests live in the backend contract crate so later LLDB/JVM orchestration can
// stay behind backend implementations while atom-deploy remains a generic coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebugSessionState {
    Running,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugThread {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub path: String,
    pub line: u32,
    #[serde(default)]
    pub column: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResolvedSourceLocation {
    FileLine {
        location: SourceLocation,
        #[serde(default)]
        symbol_file: Option<String>,
    },
    ClassLine {
        location: SourceLocation,
        class_name: String,
        #[serde(default)]
        symbol_file: Option<String>,
    },
    Symbol {
        location: SourceLocation,
        symbol_name: String,
        #[serde(default)]
        symbol_file: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugFrame {
    pub index: usize,
    pub function: String,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub column: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DebugSessionRequest {
    Attach,
    InspectState,
    WaitForStop {
        timeout_ms: u64,
    },
    Pause,
    Resume,
    ListThreads,
    ListFrames {
        #[serde(default)]
        thread_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DebugSessionResponse {
    Attached {
        state: DebugSessionState,
    },
    State {
        state: DebugSessionState,
    },
    Stopped {
        state: DebugSessionState,
    },
    Paused,
    Resumed,
    Threads {
        threads: Vec<DebugThread>,
    },
    Frames {
        thread_id: String,
        frames: Vec<DebugFrame>,
    },
}

#[expect(
    clippy::missing_errors_doc,
    reason = "Trait methods document the shared debug-session contract once at the trait boundary."
)]
pub trait BackendDebugSession {
    fn execute(&mut self, request: DebugSessionRequest) -> AtomResult<DebugSessionResponse>;

    fn wait_for_stop(&mut self, timeout: Duration) -> AtomResult<DebugSessionResponse> {
        self.execute(DebugSessionRequest::WaitForStop {
            timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        })
    }
}

#[expect(
    clippy::missing_errors_doc,
    reason = "Trait methods document the shared app-session contract once at the trait boundary."
)]
pub trait BackendAppSession {
    fn video_extension(&self) -> &'static str;

    fn ensure_launched(&mut self) -> AtomResult<()>;

    fn interact(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult>;

    fn capture_auto_screenshot(&mut self) -> AtomResult<Utf8PathBuf>;

    fn capture_screenshot(&mut self, output_path: &Utf8Path) -> AtomResult<()>;

    fn capture_logs(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()>;

    fn capture_video(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()>;

    fn start_video(&mut self, output_path: &Utf8Path) -> AtomResult<()>;

    fn stop_video(&mut self) -> AtomResult<Utf8PathBuf>;

    fn shutdown_video(&mut self) -> AtomResult<()>;

    fn debug_session(&mut self) -> AtomResult<Option<Box<dyn BackendDebugSession>>> {
        Ok(None)
    }
}

#[expect(
    clippy::missing_errors_doc,
    reason = "Trait methods document the shared deploy backend contract once at the trait boundary."
)]
pub trait DeployBackend: BackendDefinition {
    fn is_enabled(&self, manifest: &NormalizedManifest) -> bool;

    fn list_destinations(
        &self,
        repo_root: &Utf8Path,
        runner: &mut dyn ToolRunner,
    ) -> AtomResult<Vec<DestinationDescriptor>>;

    fn deploy(
        &self,
        repo_root: &Utf8Path,
        manifest: &NormalizedManifest,
        requested_destination: Option<&str>,
        launch_mode: LaunchMode,
        runner: &mut dyn ToolRunner,
    ) -> AtomResult<()>;

    fn stop(
        &self,
        repo_root: &Utf8Path,
        manifest: &NormalizedManifest,
        requested_destination: Option<&str>,
        runner: &mut dyn ToolRunner,
    ) -> AtomResult<()>;

    fn new_app_session<'a>(
        &self,
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        destination_id: &'a str,
        runner: &'a mut dyn ToolRunner,
        options: AppSessionOptions,
    ) -> AtomResult<Box<dyn BackendAppSession + 'a>>;
}

pub type DeployBackendRegistry = BackendRegistry<Box<dyn DeployBackend>>;
