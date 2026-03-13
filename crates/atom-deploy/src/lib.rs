mod deploy;
pub mod destinations;
pub mod devices;
pub mod evaluate;
pub mod progress;
mod tools;

pub use crate::deploy::{deploy_backend, ensure_backend_enabled, generated_target, stop_backend};
pub use crate::tools::{
    CommandOutput, ProcessRunner, capture_bazel, capture_bazel_owned, capture_json_tool,
    capture_tool, find_bazel_output, find_bazel_output_owned, run_bazel, run_bazel_owned, run_tool,
    stream_tool,
};
pub use atom_backends::{LaunchMode, ToolRunner};

#[cfg(test)]
mod tests {
    use atom_backends::{
        BackendAppSession, BackendDefinition, DeployBackend, DeployBackendRegistry,
        DestinationCapability, DestinationDescriptor, LaunchMode, SessionLaunchBehavior,
        ToolRunner,
    };
    use atom_manifest::{NormalizedManifest, testing::fixture_manifest};
    use camino::{Utf8Path, Utf8PathBuf};

    use crate::deploy::{deploy_backend, ensure_backend_enabled, stop_backend};
    use crate::destinations::list_destinations;

    #[derive(Default)]
    struct FakeToolRunner;

    impl ToolRunner for FakeToolRunner {
        fn run(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn capture(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            Ok(String::new())
        }

        fn capture_json_file(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            Ok(String::new())
        }

        fn stream(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
        }
    }

    struct FixtureBackend;

    impl BackendDefinition for FixtureBackend {
        fn id(&self) -> &'static str {
            "fixture"
        }

        fn platform(&self) -> &'static str {
            "fixture"
        }
    }

    impl DeployBackend for FixtureBackend {
        fn is_enabled(&self, _manifest: &NormalizedManifest) -> bool {
            true
        }

        fn list_destinations(
            &self,
            _repo_root: &Utf8Path,
            _runner: &mut dyn ToolRunner,
        ) -> atom_ffi::AtomResult<Vec<DestinationDescriptor>> {
            Ok(vec![DestinationDescriptor {
                platform: "fixture-platform".to_owned(),
                backend_id: "fixture".to_owned(),
                id: "fixture-1".to_owned(),
                kind: "fixture-target".to_owned(),
                display_name: "Fixture Device".to_owned(),
                available: true,
                debug_state: "ready".to_owned(),
                capabilities: vec![DestinationCapability::Launch],
            }])
        }

        fn deploy(
            &self,
            _repo_root: &Utf8Path,
            _manifest: &NormalizedManifest,
            _requested_destination: Option<&str>,
            _launch_mode: LaunchMode,
            _runner: &mut dyn ToolRunner,
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn stop(
            &self,
            _repo_root: &Utf8Path,
            _manifest: &NormalizedManifest,
            _requested_destination: Option<&str>,
            _runner: &mut dyn ToolRunner,
        ) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn new_app_session<'a>(
            &self,
            _repo_root: &'a Utf8Path,
            _manifest: &'a NormalizedManifest,
            _destination_id: &'a str,
            _runner: &'a mut dyn ToolRunner,
            _launch_behavior: SessionLaunchBehavior,
        ) -> atom_ffi::AtomResult<Box<dyn BackendAppSession + 'a>> {
            unreachable!("deploy core tests do not construct app sessions")
        }
    }

    fn runnable_manifest(root: &Utf8PathBuf) -> NormalizedManifest {
        fixture_manifest(root)
    }

    #[test]
    fn destination_listing_uses_registered_backends() {
        let mut registry = DeployBackendRegistry::new();
        registry
            .register(Box::new(FixtureBackend))
            .expect("fixture backend should register");
        let root = Utf8PathBuf::from(".");
        let mut runner = FakeToolRunner;

        let destinations = list_destinations(&root, &registry, &mut runner).expect("list");
        assert_eq!(destinations.len(), 1);
        assert_eq!(destinations[0].backend_id, "fixture");
        assert_eq!(destinations[0].kind, "fixture-target");
    }

    #[test]
    fn deploy_backend_dispatches_through_registry() {
        let mut registry = DeployBackendRegistry::new();
        registry
            .register(Box::new(FixtureBackend))
            .expect("fixture backend should register");
        let root = Utf8PathBuf::from(".");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner;

        deploy_backend(
            &root,
            &manifest,
            &registry,
            "fixture",
            Some("fixture-1"),
            LaunchMode::Detached,
            &mut runner,
        )
        .expect("deploy should dispatch");
    }

    #[test]
    fn stop_backend_dispatches_through_registry() {
        let mut registry = DeployBackendRegistry::new();
        registry
            .register(Box::new(FixtureBackend))
            .expect("fixture backend should register");
        let root = Utf8PathBuf::from(".");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner;

        stop_backend(
            &root,
            &manifest,
            &registry,
            "fixture",
            Some("fixture-1"),
            &mut runner,
        )
        .expect("stop should dispatch");
    }

    #[test]
    fn ensure_backend_enabled_rejects_disabled_backends() {
        struct DisabledFixtureBackend;

        impl BackendDefinition for DisabledFixtureBackend {
            fn id(&self) -> &'static str {
                "fixture"
            }

            fn platform(&self) -> &'static str {
                "fixture-platform"
            }
        }

        impl DeployBackend for DisabledFixtureBackend {
            fn is_enabled(&self, _manifest: &NormalizedManifest) -> bool {
                false
            }

            fn list_destinations(
                &self,
                _repo_root: &Utf8Path,
                _runner: &mut dyn ToolRunner,
            ) -> atom_ffi::AtomResult<Vec<DestinationDescriptor>> {
                Ok(Vec::new())
            }

            fn deploy(
                &self,
                _repo_root: &Utf8Path,
                _manifest: &NormalizedManifest,
                _requested_destination: Option<&str>,
                _launch_mode: LaunchMode,
                _runner: &mut dyn ToolRunner,
            ) -> atom_ffi::AtomResult<()> {
                Ok(())
            }

            fn stop(
                &self,
                _repo_root: &Utf8Path,
                _manifest: &NormalizedManifest,
                _requested_destination: Option<&str>,
                _runner: &mut dyn ToolRunner,
            ) -> atom_ffi::AtomResult<()> {
                Ok(())
            }

            fn new_app_session<'a>(
                &self,
                _repo_root: &'a Utf8Path,
                _manifest: &'a NormalizedManifest,
                _destination_id: &'a str,
                _runner: &'a mut dyn ToolRunner,
                _launch_behavior: SessionLaunchBehavior,
            ) -> atom_ffi::AtomResult<Box<dyn BackendAppSession + 'a>> {
                unreachable!("deploy core tests do not construct app sessions")
            }
        }

        let mut registry = DeployBackendRegistry::new();
        registry
            .register(Box::new(DisabledFixtureBackend))
            .expect("fixture backend should register");
        let root = Utf8PathBuf::from(".");
        let manifest = runnable_manifest(&root);

        let error = ensure_backend_enabled(&manifest, &registry, "fixture")
            .expect_err("disabled backend should fail");

        assert_eq!(error.code, atom_ffi::AtomErrorCode::ManifestInvalidValue);
        assert!(
            error
                .message
                .contains("fixture-platform platform is not enabled")
        );
    }
}
