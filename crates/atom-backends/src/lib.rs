mod cng;
mod deploy;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};

pub use crate::cng::{
    BackendContribution, BackendPlan, ContributedFile, FileSource, GenerationBackend,
    GenerationBackendRegistry, GenerationPlan, PlannedBackend, SchemaFilePlan, SchemaPlan,
};
pub use crate::deploy::{
    AppSessionBuildProfile, AppSessionOptions, ArtifactRecord, BackendAppSession,
    BackendDebugSession, DebugFrame, DebugSessionRequest, DebugSessionResponse, DebugSessionState,
    DebugThread, DeployBackend, DeployBackendRegistry, DestinationCapability,
    DestinationDescriptor, EvaluationBundleManifest, EvaluationPlan, EvaluationStep,
    InteractionRequest, InteractionResult, LaunchMode, ResolvedSourceLocation, ScreenInfo,
    SessionLaunchBehavior, SourceLocation, StepRecord, ToolRunner, UiBounds, UiNode, UiSnapshot,
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
    use std::time::Duration;

    use atom_ffi::AtomErrorCode;
    use serde_json::json;

    use super::{
        BackendDebugSession, BackendDefinition, BackendRegistry, DebugFrame, DebugSessionRequest,
        DebugSessionResponse, DebugSessionState, DestinationCapability, DestinationDescriptor,
        ResolvedSourceLocation, SourceLocation,
    };

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

    #[test]
    fn debug_session_wire_payloads_use_stable_snake_case_tags() {
        let request = DebugSessionRequest::ListFrames {
            thread_id: Some("thread-1".to_owned()),
        };
        let response = DebugSessionResponse::Frames {
            thread_id: "thread-1".to_owned(),
            frames: vec![DebugFrame {
                index: 0,
                function: "demo::main".to_owned(),
                source_path: Some("src/main.rs".to_owned()),
                line: Some(42),
                column: Some(7),
            }],
        };

        assert_eq!(
            serde_json::to_value(&request).expect("request should encode"),
            json!({
                "kind": "list_frames",
                "thread_id": "thread-1",
            })
        );
        assert_eq!(
            serde_json::to_value(&response).expect("response should encode"),
            json!({
                "kind": "frames",
                "thread_id": "thread-1",
                "frames": [{
                    "index": 0,
                    "function": "demo::main",
                    "source_path": "src/main.rs",
                    "line": 42,
                    "column": 7,
                }],
            })
        );
    }

    #[test]
    fn debug_session_wait_for_stop_forwards_timeout_millis() {
        struct RecordingDebugSession {
            requests: Vec<DebugSessionRequest>,
        }

        impl BackendDebugSession for RecordingDebugSession {
            fn execute(
                &mut self,
                request: DebugSessionRequest,
            ) -> atom_ffi::AtomResult<DebugSessionResponse> {
                self.requests.push(request);
                Ok(DebugSessionResponse::Stopped {
                    state: DebugSessionState::Stopped,
                })
            }
        }

        let mut session = RecordingDebugSession {
            requests: Vec::new(),
        };
        let response = session
            .wait_for_stop(Duration::from_millis(250))
            .expect("wait_for_stop should delegate");

        assert_eq!(
            session.requests,
            vec![DebugSessionRequest::WaitForStop { timeout_ms: 250 }]
        );
        assert_eq!(
            response,
            DebugSessionResponse::Stopped {
                state: DebugSessionState::Stopped,
            }
        );
    }

    #[test]
    fn source_location_payloads_encode_backend_neutral_shapes() {
        let location = SourceLocation {
            path: "src/demo.rs".to_owned(),
            line: 42,
            column: Some(7),
        };
        let resolved = ResolvedSourceLocation::ClassLine {
            location: location.clone(),
            class_name: "build.atom.hello.DemoSurfaceKt".to_owned(),
            symbol_file: Some("bazel-out/demo/app_deploy.jar".to_owned()),
        };

        assert_eq!(
            serde_json::to_value(&location).expect("location should encode"),
            json!({
                "path": "src/demo.rs",
                "line": 42,
                "column": 7,
            })
        );
        assert_eq!(
            serde_json::to_value(&resolved).expect("resolved location should encode"),
            json!({
                "kind": "class_line",
                "location": {
                    "path": "src/demo.rs",
                    "line": 42,
                    "column": 7,
                },
                "class_name": "build.atom.hello.DemoSurfaceKt",
                "symbol_file": "bazel-out/demo/app_deploy.jar",
            })
        );
    }
}
