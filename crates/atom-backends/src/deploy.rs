use atom_ffi::AtomResult;
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::{BackendDefinition, BackendDoctorReport, BackendRegistry, DoctorSystem};

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

    fn ensure_launched(&mut self) -> AtomResult<()>;

    fn interact(&mut self, request: InteractionRequest) -> AtomResult<InteractionResult>;

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
    reason = "Trait methods document the shared deploy backend contract once at the trait boundary."
)]
pub trait DeployBackend: BackendDefinition {
    fn is_enabled(&self, manifest: &NormalizedManifest) -> bool;

    fn doctor(&self, _repo_root: &Utf8Path, _system: &dyn DoctorSystem) -> BackendDoctorReport {
        BackendDoctorReport::new(self.platform(), false, Vec::new())
    }

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
        runner: &'a mut dyn ToolRunner,
        launch_behavior: SessionLaunchBehavior,
    ) -> AtomResult<Box<dyn BackendAutomationSession + 'a>>;
}

pub type DeployBackendRegistry = BackendRegistry<Box<dyn DeployBackend>>;
