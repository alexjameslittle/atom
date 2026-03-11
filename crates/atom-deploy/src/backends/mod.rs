pub mod android;
pub mod ios;

use atom_backends::{BackendDefinition, BackendRegistry};
use atom_ffi::AtomResult;
use atom_manifest::NormalizedManifest;
use camino::Utf8Path;
use camino::Utf8PathBuf;

use crate::deploy::LaunchMode;
use crate::destinations::DestinationDescriptor;
use crate::evaluate::{InteractionRequest, InteractionResult, SessionLaunchBehavior};
use crate::tools::ToolRunner;

pub(crate) trait BackendAutomationSession {
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

pub(crate) trait DeployBackend: BackendDefinition {
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
        runner: &'a mut dyn ToolRunner,
        launch_behavior: SessionLaunchBehavior,
    ) -> AtomResult<Box<dyn BackendAutomationSession + 'a>>;
}

pub(crate) type DeployBackendRegistry = BackendRegistry<Box<dyn DeployBackend>>;

#[must_use]
pub(crate) fn first_party_deploy_backend_registry() -> DeployBackendRegistry {
    let mut registry = DeployBackendRegistry::new();
    ios::register(&mut registry).expect("first-party iOS backend id should be unique");
    android::register(&mut registry).expect("first-party Android backend id should be unique");
    registry
}
