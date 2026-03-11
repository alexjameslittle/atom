use atom_backends::{DeployBackend, DeployBackendRegistry, LaunchMode, ToolRunner};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::Utf8Path;

/// # Errors
///
/// Returns an error if the backend id is unknown, disabled for the app, or deployment fails.
pub fn deploy_backend(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    requested_device: Option<&str>,
    launch_mode: LaunchMode,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let backend = resolve_backend(registry, backend_id)?;
    require_enabled_backend(backend.platform(), backend.is_enabled(manifest))?;
    backend.deploy(repo_root, manifest, requested_device, launch_mode, runner)
}

/// # Errors
///
/// Returns an error if the backend id is unknown, disabled for the app, or stopping fails.
pub fn stop_backend(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    requested_device: Option<&str>,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let backend = resolve_backend(registry, backend_id)?;
    require_enabled_backend(backend.platform(), backend.is_enabled(manifest))?;
    backend.stop(repo_root, manifest, requested_device, runner)
}

fn resolve_backend<'a>(
    registry: &'a DeployBackendRegistry,
    backend_id: &str,
) -> AtomResult<&'a dyn DeployBackend> {
    registry.get(backend_id).map(Box::as_ref).ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::CliUsageError,
            format!("unknown backend id: {backend_id}"),
            backend_id,
        )
    })
}

fn require_enabled_backend(platform: &str, enabled: bool) -> AtomResult<()> {
    if enabled {
        Ok(())
    } else {
        Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            format!("{platform} platform is not enabled"),
            platform,
        ))
    }
}

#[must_use]
pub fn generated_target(manifest: &NormalizedManifest, backend_id: &str) -> String {
    format!(
        "//{}/{}/{}:app",
        manifest.build.generated_root.as_str(),
        backend_id,
        manifest.app.slug
    )
}
