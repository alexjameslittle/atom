use std::fmt::Write as _;

use atom_backends::BackendDefinition;
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::backends::{DeployBackendRegistry, first_party_deploy_backend_registry};
use crate::tools::ToolRunner;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationPlatform {
    Ios,
    Android,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationKind {
    Simulator,
    Device,
    Emulator,
    Avd,
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
    pub backend_id: String,
    pub id: String,
    pub platform: DestinationPlatform,
    pub kind: DestinationKind,
    pub display_name: String,
    pub available: bool,
    pub debug_state: String,
    pub capabilities: Vec<DestinationCapability>,
}

/// # Errors
///
/// Returns an error if iOS or Android destination discovery fails.
pub fn list_destinations(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<DestinationDescriptor>> {
    let registry = first_party_deploy_backend_registry();
    list_destinations_with_registry(repo_root, &registry, runner)
}

/// # Errors
///
/// Returns an error if destination discovery for any registered backend fails.
pub(crate) fn list_destinations_with_registry(
    repo_root: &Utf8Path,
    registry: &DeployBackendRegistry,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<DestinationDescriptor>> {
    let mut destinations = Vec::new();
    for backend in registry.iter() {
        destinations.extend(backend.list_destinations(repo_root, runner)?);
    }
    Ok(destinations)
}

/// # Errors
///
/// Returns an error if destination discovery for the requested platform fails.
pub fn list_platform_destinations(
    repo_root: &Utf8Path,
    platform: DestinationPlatform,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<DestinationDescriptor>> {
    let registry = first_party_deploy_backend_registry();
    list_platform_destinations_with_registry(repo_root, &registry, platform, runner)
}

/// # Errors
///
/// Returns an error if destination discovery for a backend on the requested platform fails.
pub(crate) fn list_platform_destinations_with_registry(
    repo_root: &Utf8Path,
    registry: &DeployBackendRegistry,
    platform: DestinationPlatform,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<DestinationDescriptor>> {
    let mut destinations = Vec::new();
    for backend in registry
        .iter()
        .filter(|backend| backend.platform() == platform.as_str())
    {
        destinations.extend(backend.list_destinations(repo_root, runner)?);
    }
    Ok(destinations)
}

/// # Errors
///
/// Returns an error if the requested backend id is unknown or destination discovery fails.
pub fn list_backend_destinations(
    repo_root: &Utf8Path,
    backend_id: &str,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<DestinationDescriptor>> {
    let registry = first_party_deploy_backend_registry();
    list_backend_destinations_with_registry(repo_root, &registry, backend_id, runner)
}

/// # Errors
///
/// Returns an error if the requested backend id is unknown or destination discovery fails.
pub(crate) fn list_backend_destinations_with_registry(
    repo_root: &Utf8Path,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<DestinationDescriptor>> {
    let backend = registry.get(backend_id).ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::CliUsageError,
            format!("unknown backend id: {backend_id}"),
            backend_id,
        )
    })?;
    backend.list_destinations(repo_root, runner)
}

#[must_use]
pub fn render_destination_lines(destinations: &[DestinationDescriptor]) -> String {
    let mut output = String::new();
    for destination in destinations {
        let capabilities = destination
            .capabilities
            .iter()
            .map(|capability| match capability {
                DestinationCapability::Launch => "launch",
                DestinationCapability::Logs => "logs",
                DestinationCapability::Screenshot => "screenshot",
                DestinationCapability::Video => "video",
                DestinationCapability::InspectUi => "inspect_ui",
                DestinationCapability::Interact => "interact",
                DestinationCapability::Evaluate => "evaluate",
            })
            .collect::<Vec<_>>()
            .join(",");
        let _ = writeln!(
            output,
            "{} [{} {:?}; backend={}; available={}; state={}; capabilities={}]",
            destination.display_name,
            destination.id,
            destination.kind,
            destination.backend_id,
            destination.available,
            destination.debug_state,
            capabilities
        );
    }
    output
}

impl DestinationPlatform {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }
}
