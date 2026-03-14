mod agent_device;
mod android_uiautomator;
mod cng;
mod deploy;
mod templates;

use atom_backends::{DeployBackendRegistry, GenerationBackendRegistry};
use atom_ffi::AtomResult;

/// # Errors
///
/// Returns an error if the backend id is registered more than once.
pub fn register_deploy_backend(registry: &mut DeployBackendRegistry) -> AtomResult<()> {
    deploy::register(registry)
}

/// # Errors
///
/// Returns an error if the backend id is registered more than once.
pub fn register_generation_backend(registry: &mut GenerationBackendRegistry) -> AtomResult<()> {
    cng::register(registry)
}
