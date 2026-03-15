mod cng;
mod deploy;
mod doctor;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};

pub use crate::cng::{
    BackendContribution, BackendPlan, ContributedFile, FileSource, GenerationBackend,
    GenerationBackendRegistry, GenerationPlan, PlannedBackend, SchemaPlan,
};
pub use crate::deploy::{
    ArtifactRecord, BackendAutomationSession, DeployBackend, DeployBackendRegistry,
    DestinationCapability, DestinationDescriptor, EvaluationBundleManifest, EvaluationPlan,
    EvaluationStep, InteractionRequest, InteractionResult, LaunchMode, ScreenInfo,
    SessionLaunchBehavior, StepRecord, ToolRunner, UiBounds, UiNode, UiSnapshot,
};
pub use crate::doctor::{
    BackendDoctorReport, CapturedCommand, CommandInvocation, DoctorCheck, DoctorSeverity,
    DoctorStatus, DoctorSystem, ProcessDoctorSystem, combined_command_output, first_version_token,
};

pub trait BackendDefinition {
    fn id(&self) -> &'static str;
    fn platform(&self) -> &'static str;
}

impl<B> BackendDefinition for Box<B>
where
    B: BackendDefinition + ?Sized,
{
    fn id(&self) -> &'static str {
        (**self).id()
    }

    fn platform(&self) -> &'static str {
        (**self).platform()
    }
}

pub struct BackendRegistry<B> {
    backends: Vec<B>,
}

impl<B> Default for BackendRegistry<B> {
    fn default() -> Self {
        Self {
            backends: Vec::new(),
        }
    }
}

impl<B> BackendRegistry<B>
where
    B: BackendDefinition,
{
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// # Errors
    ///
    /// Returns an error if a backend with the same id is already registered.
    pub fn register(&mut self, backend: B) -> AtomResult<()> {
        let id = backend.id();
        if self.backends.iter().any(|existing| existing.id() == id) {
            return Err(AtomError::new(
                AtomErrorCode::InternalBug,
                format!("backend registry already contains id {id}"),
            ));
        }
        self.backends.push(backend);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&B> {
        self.backends.iter().find(|backend| backend.id() == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &B> {
        self.backends.iter()
    }
}

#[cfg(test)]
mod tests {
    use atom_ffi::AtomErrorCode;
    use serde_json::json;

    use super::{BackendDefinition, BackendRegistry, DestinationCapability, DestinationDescriptor};

    struct FixtureBackend {
        id: &'static str,
        platform: &'static str,
    }

    impl BackendDefinition for FixtureBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn platform(&self) -> &'static str {
            self.platform
        }
    }

    #[test]
    fn duplicate_backend_ids_are_rejected() {
        let mut registry = BackendRegistry::new();
        registry
            .register(FixtureBackend {
                id: "fixture-backend",
                platform: "fixture-platform",
            })
            .expect("first registration should succeed");

        let error = registry
            .register(FixtureBackend {
                id: "fixture-backend",
                platform: "fixture-platform",
            })
            .expect_err("duplicate id should fail");

        assert_eq!(error.code, AtomErrorCode::InternalBug);
        assert!(
            error
                .message
                .contains("backend registry already contains id fixture-backend")
        );
    }

    #[test]
    fn destination_descriptor_serialization_preserves_platform_and_backend_id() {
        let descriptor = DestinationDescriptor {
            platform: "fixture-platform".to_owned(),
            backend_id: "fixture-backend".to_owned(),
            id: "fixture-1".to_owned(),
            kind: "fixture-target".to_owned(),
            display_name: "Fixture Device".to_owned(),
            available: true,
            debug_state: "ready".to_owned(),
            capabilities: vec![DestinationCapability::Launch],
        };

        let value =
            serde_json::to_value(&descriptor).expect("destination descriptor should encode");

        assert_eq!(
            value,
            json!({
                "platform": "fixture-platform",
                "backend_id": "fixture-backend",
                "id": "fixture-1",
                "kind": "fixture-target",
                "display_name": "Fixture Device",
                "available": true,
                "debug_state": "ready",
                "capabilities": ["launch"],
            })
        );
    }
}
