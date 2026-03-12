use std::cell::{RefCell, RefMut};

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
    Debug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DebuggerKind {
    Native,
    Jvm,
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
    #[serde(default)]
    pub debuggers: Vec<DebuggerKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}

pub trait ToolRunner {
    /// # Errors
    ///
    /// Returns an error if the tool invocation fails.
    fn run(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()>;

    /// # Errors
    ///
    /// Returns an error if the tool process could not be invoked.
    fn capture_output(
        &mut self,
        repo_root: &Utf8Path,
        tool: &str,
        args: &[String],
    ) -> AtomResult<ToolCommandOutput>;

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

pub struct SharedToolRunner<'a> {
    inner: RefCell<&'a mut dyn ToolRunner>,
}

impl<'a> SharedToolRunner<'a> {
    #[must_use]
    pub fn new(runner: &'a mut dyn ToolRunner) -> Self {
        Self {
            inner: RefCell::new(runner),
        }
    }

    pub fn borrow_mut(&self) -> RefMut<'_, &'a mut dyn ToolRunner> {
        self.inner.borrow_mut()
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugSourceLocation {
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugBreakpoint {
    pub debugger: DebuggerKind,
    pub file: String,
    pub line: u32,
    pub id: String,
    #[serde(default)]
    pub resolved_file: Option<String>,
    #[serde(default)]
    pub resolved_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugStop {
    pub debugger: DebuggerKind,
    pub reason: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub thread_name: Option<String>,
    #[serde(default)]
    pub breakpoint_id: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugThread {
    pub debugger: DebuggerKind,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugFrame {
    pub index: usize,
    pub function: String,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugBacktrace {
    pub debugger: DebuggerKind,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub thread_name: Option<String>,
    pub frames: Vec<DebugFrame>,
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
    DebugTap {
        #[serde(default)]
        target_id: Option<String>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
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
    DebugLaunch {
        debugger: DebuggerKind,
    },
    DebugAttach {
        debugger: DebuggerKind,
    },
    DebugSetBreakpoint {
        debugger: DebuggerKind,
        file: String,
        line: u32,
        #[serde(default)]
        name: Option<String>,
    },
    DebugClearBreakpoint {
        debugger: DebuggerKind,
        file: String,
        line: u32,
    },
    DebugWaitForStop {
        debugger: DebuggerKind,
        #[serde(default)]
        timeout_ms: Option<u64>,
        #[serde(default)]
        name: Option<String>,
    },
    DebugThreads {
        debugger: DebuggerKind,
        #[serde(default)]
        name: Option<String>,
    },
    DebugBacktrace {
        debugger: DebuggerKind,
        #[serde(default)]
        thread_id: Option<String>,
        #[serde(default)]
        name: Option<String>,
    },
    DebugPause {
        debugger: DebuggerKind,
        #[serde(default)]
        name: Option<String>,
    },
    DebugResume {
        debugger: DebuggerKind,
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

#[expect(
    clippy::missing_errors_doc,
    reason = "Trait methods document the shared automation session contract once at the trait boundary."
)]
pub trait BackendAutomationSession {
    fn video_extension(&self) -> &'static str;

    fn attach_existing(&mut self) -> AtomResult<bool>;

    fn ensure_launched(&mut self) -> AtomResult<()>;

    fn interact(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult>;

    fn interact_without_snapshot(&mut self, request: InteractionRequest) -> AtomResult<()>;

    fn capture_auto_screenshot(&mut self) -> AtomResult<Utf8PathBuf>;

    fn capture_screenshot(&mut self, output_path: &Utf8Path) -> AtomResult<()>;

    fn capture_logs(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()>;

    fn capture_video(&mut self, output_path: &Utf8Path, seconds: u64) -> AtomResult<()>;

    fn start_video(&mut self, output_path: &Utf8Path) -> AtomResult<()>;

    fn stop_video(&mut self) -> AtomResult<Utf8PathBuf>;

    fn shutdown_video(&mut self) -> AtomResult<()>;
}

#[expect(
    clippy::missing_errors_doc,
    reason = "Trait methods document the shared debugger session contract once at the trait boundary."
)]
pub trait BackendDebugSession {
    fn kind(&self) -> DebuggerKind;

    fn launch(&mut self) -> AtomResult<()>;

    fn attach(&mut self) -> AtomResult<()>;

    fn set_breakpoint(&mut self, location: DebugSourceLocation) -> AtomResult<DebugBreakpoint>;

    fn clear_breakpoint(&mut self, location: DebugSourceLocation) -> AtomResult<()>;

    fn wait_for_stop(&mut self, timeout_ms: Option<u64>) -> AtomResult<DebugStop>;

    fn threads(&mut self) -> AtomResult<Vec<DebugThread>>;

    fn backtrace(&mut self, thread_id: Option<&str>) -> AtomResult<DebugBacktrace>;

    fn pause(&mut self) -> AtomResult<DebugStop>;

    fn resume(&mut self) -> AtomResult<()>;

    fn shutdown(&mut self) -> AtomResult<()>;
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

    fn new_automation_session<'a>(
        &self,
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        destination_id: &'a str,
        runner: &'a SharedToolRunner<'a>,
        launch_behavior: SessionLaunchBehavior,
    ) -> AtomResult<Box<dyn BackendAutomationSession + 'a>>;

    fn new_debug_session<'a>(
        &self,
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        destination_id: &'a str,
        runner: &'a SharedToolRunner<'a>,
        launch_behavior: SessionLaunchBehavior,
        debugger: DebuggerKind,
    ) -> AtomResult<Box<dyn BackendDebugSession + 'a>>;
}

pub type DeployBackendRegistry = BackendRegistry<Box<dyn DeployBackend>>;
