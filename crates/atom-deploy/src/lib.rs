mod deploy;
pub mod destinations;
pub mod devices;
pub mod evaluate;
pub mod progress;
mod tools;

pub use crate::deploy::{deploy_backend, generated_target, stop_backend};
pub use crate::tools::{
    CommandOutput, ProcessRunner, capture_bazel, capture_bazel_owned, capture_json_tool,
    capture_tool, find_bazel_output, find_bazel_output_owned, run_bazel, run_bazel_owned, run_tool,
    stream_tool,
};
pub use atom_backends::{LaunchMode, ToolRunner};

#[cfg(test)]
mod tests {
    use atom_backends::{
        BackendAutomationSession, BackendDefinition, DeployBackend, DeployBackendRegistry,
        DestinationCapability, DestinationDescriptor, LaunchMode, SessionLaunchBehavior,
        ToolRunner,
    };
    use atom_manifest::{AndroidConfig, AppConfig, BuildConfig, IosConfig, NormalizedManifest};
    use camino::{Utf8Path, Utf8PathBuf};

    use crate::deploy::{deploy_backend, stop_backend};
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
                backend_id: "fixture".to_owned(),
                id: "fixture-1".to_owned(),
                kind: "device".to_owned(),
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

        fn new_automation_session<'a>(
            &self,
            _repo_root: &'a Utf8Path,
            _manifest: &'a NormalizedManifest,
            _destination_id: &'a str,
            _runner: &'a mut dyn ToolRunner,
            _launch_behavior: SessionLaunchBehavior,
        ) -> atom_ffi::AtomResult<Box<dyn BackendAutomationSession + 'a>> {
            unreachable!("deploy core tests do not construct automation sessions")
        }
    }

    fn runnable_manifest(root: &Utf8PathBuf) -> NormalizedManifest {
        NormalizedManifest {
            repo_root: root.clone(),
            target_label: "//apps/fixture:fixture".to_owned(),
            metadata_path: root.join("fixture.atom.app.json"),
            app: AppConfig {
                name: "Fixture".to_owned(),
                slug: "fixture".to_owned(),
                entry_crate_label: "//apps/fixture:fixture".to_owned(),
                entry_crate_name: "fixture".to_owned(),
            },
            ios: IosConfig {
                enabled: true,
                bundle_id: Some("build.atom.fixture".to_owned()),
                deployment_target: Some("17.0".to_owned()),
            },
            android: AndroidConfig {
                enabled: true,
                application_id: Some("build.atom.fixture".to_owned()),
                min_sdk: Some(28),
                target_sdk: Some(35),
            },
            build: BuildConfig {
                generated_root: Utf8PathBuf::from("generated"),
                watch: false,
            },
            modules: Vec::new(),
            config_plugins: Vec::new(),
        }
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
        assert_eq!(destinations[0].kind, "device");
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
}
