use atom_backends::{
    BackendAutomationSession, BackendDefinition, DeployBackend, DeployBackendRegistry,
    DestinationCapability, DestinationDescriptor, DestinationKind, DestinationPlatform, LaunchMode,
    SessionLaunchBehavior, ToolRunner,
};
use atom_deploy::devices::android::{AndroidDestination, list_android_destinations};
use atom_deploy::{deploy_android, new_android_automation_session, stop_android};
use atom_ffi::AtomResult;
use atom_manifest::NormalizedManifest;
use camino::Utf8Path;

const BACKEND_ID: &str = "android";

struct AndroidDeployBackend;

pub fn register(registry: &mut DeployBackendRegistry) -> AtomResult<()> {
    registry.register(Box::new(AndroidDeployBackend))
}

impl BackendDefinition for AndroidDeployBackend {
    fn id(&self) -> &'static str {
        BACKEND_ID
    }

    fn platform(&self) -> &'static str {
        "android"
    }
}

impl DeployBackend for AndroidDeployBackend {
    fn is_enabled(&self, manifest: &NormalizedManifest) -> bool {
        manifest.android.enabled
    }

    fn list_destinations(
        &self,
        repo_root: &Utf8Path,
        runner: &mut dyn ToolRunner,
    ) -> AtomResult<Vec<DestinationDescriptor>> {
        Ok(list_android_destinations(repo_root, runner)?
            .into_iter()
            .map(destination_descriptor_from_android)
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
        deploy_android(
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
        stop_android(repo_root, manifest, requested_destination, runner)
    }

    fn new_automation_session<'a>(
        &self,
        repo_root: &'a Utf8Path,
        manifest: &'a NormalizedManifest,
        destination_id: &'a str,
        runner: &'a mut dyn ToolRunner,
        launch_behavior: SessionLaunchBehavior,
    ) -> AtomResult<Box<dyn BackendAutomationSession + 'a>> {
        Ok(new_android_automation_session(
            repo_root,
            manifest,
            destination_id,
            runner,
            launch_behavior,
        ))
    }
}

fn destination_descriptor_from_android(destination: AndroidDestination) -> DestinationDescriptor {
    let display_name = destination.display_label();
    let id = destination.destination_id();
    let kind = if destination.state == "avd" {
        DestinationKind::Avd
    } else if destination.is_emulator {
        DestinationKind::Emulator
    } else {
        DestinationKind::Device
    };
    let capabilities = match kind {
        DestinationKind::Avd | DestinationKind::Emulator | DestinationKind::Device => vec![
            DestinationCapability::Launch,
            DestinationCapability::Logs,
            DestinationCapability::Screenshot,
            DestinationCapability::Video,
            DestinationCapability::InspectUi,
            DestinationCapability::Interact,
            DestinationCapability::Evaluate,
        ],
        DestinationKind::Simulator => Vec::new(),
    };

    DestinationDescriptor {
        backend_id: BACKEND_ID.to_owned(),
        id,
        platform: DestinationPlatform::Android,
        kind,
        display_name,
        available: destination.state == "device" || destination.state == "avd",
        debug_state: destination.state,
        capabilities,
    }
}
