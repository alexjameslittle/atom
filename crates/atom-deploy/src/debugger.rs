use atom_backends::{
    BackendDebugSession, DebuggerKind, DeployBackendRegistry, SessionLaunchBehavior,
    SharedToolRunner,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::Utf8Path;

pub(crate) fn debug_session_with_registry<'a>(
    registry: &DeployBackendRegistry,
    repo_root: &'a Utf8Path,
    manifest: &'a NormalizedManifest,
    backend_id: &'a str,
    destination_id: &'a str,
    runner: &'a SharedToolRunner<'a>,
    launch_behavior: SessionLaunchBehavior,
    debugger: DebuggerKind,
) -> AtomResult<Box<dyn BackendDebugSession + 'a>> {
    let backend = registry.get(backend_id).map(Box::as_ref).ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::CliUsageError,
            format!("unknown backend id: {backend_id}"),
            backend_id,
        )
    })?;
    if !backend.is_enabled(manifest) {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            format!("{backend_id} platform is not enabled"),
            backend_id,
        ));
    }
    backend.new_debug_session(
        repo_root,
        manifest,
        destination_id,
        runner,
        launch_behavior,
        debugger,
    )
}
