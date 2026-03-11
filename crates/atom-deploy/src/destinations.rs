use std::fmt::Write as _;

use atom_backends::{DeployBackendRegistry, ToolRunner};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::Utf8Path;

pub use atom_backends::{DestinationCapability, DestinationDescriptor};

/// # Errors
///
/// Returns an error if destination discovery for any registered backend fails.
pub fn list_destinations(
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
/// Returns an error if the requested backend id is unknown or destination discovery fails.
pub fn list_backend_destinations(
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
            "{} [{} {}; backend={}; available={}; state={}; capabilities={}]",
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
