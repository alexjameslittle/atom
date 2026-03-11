use atom_backends::BackendDefinition;
use atom_ffi::AtomResult;
use atom_manifest::NormalizedManifest;
use camino::Utf8Path;

use crate::backends::{BackendAutomationSession, DeployBackend, DeployBackendRegistry};
use crate::deploy::{LaunchMode, deploy_ios, stop_ios};
use crate::destinations::{
    DestinationCapability, DestinationDescriptor, DestinationKind, DestinationPlatform,
};
use crate::devices::ios::{IosDestination, IosDestinationKind, list_ios_destinations};
use crate::evaluate::{SessionLaunchBehavior, new_ios_automation_session};
use crate::tools::ToolRunner;

const BACKEND_ID: &str = "ios";

pub(super) struct IosDestinationBackend;

/// # Errors
///
/// Returns an error if the backend id is registered more than once.
pub fn register(registry: &mut DeployBackendRegistry) -> AtomResult<()> {
    registry.register(Box::new(IosDestinationBackend))
}

impl BackendDefinition for IosDestinationBackend {
    fn id(&self) -> &'static str {
        BACKEND_ID
    }

    fn platform(&self) -> &'static str {
        "ios"
    }
}

impl DeployBackend for IosDestinationBackend {
    fn is_enabled(&self, manifest: &NormalizedManifest) -> bool {
        manifest.ios.enabled
    }

    fn list_destinations(
        &self,
        repo_root: &Utf8Path,
        runner: &mut dyn ToolRunner,
    ) -> AtomResult<Vec<DestinationDescriptor>> {
        Ok(list_ios_destinations(repo_root, runner)?
            .into_iter()
            .map(destination_descriptor_from_ios)
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

    fn new_automation_session<'a>(
        &self,
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        destination_id: &'a str,
        runner: &'a mut dyn ToolRunner,
        launch_behavior: SessionLaunchBehavior,
    ) -> AtomResult<Box<dyn BackendAutomationSession + 'a>> {
        Ok(new_ios_automation_session(
            repo_root,
            manifest,
            destination_id,
            runner,
            launch_behavior,
        ))
    }
}

pub(crate) fn destination_descriptor_from_ios(
    destination: IosDestination,
) -> DestinationDescriptor {
    let display_name = destination.display_label();
    let id = destination.id.clone();
    let kind = match destination.kind {
        IosDestinationKind::Simulator => DestinationKind::Simulator,
        IosDestinationKind::Device => DestinationKind::Device,
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
        IosDestinationKind::Device => vec![DestinationCapability::Launch],
    };

    DestinationDescriptor {
        backend_id: BACKEND_ID.to_owned(),
        id,
        platform: DestinationPlatform::Ios,
        kind,
        display_name,
        available: destination.is_available,
        debug_state: destination.state,
        capabilities,
    }
}
