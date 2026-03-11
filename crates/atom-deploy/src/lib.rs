mod deploy;
pub mod destinations;
pub mod devices;
pub mod evaluate;
pub mod progress;
mod tools;

pub use crate::deploy::{
    deploy_android, deploy_backend, deploy_ios, generated_target, stop_android, stop_backend,
    stop_ios,
};
pub use crate::evaluate::{new_android_automation_session, new_ios_automation_session};
pub use crate::tools::{
    CommandOutput, ProcessRunner, capture_bazel, capture_bazel_owned, capture_json_tool,
    capture_tool, find_bazel_output, find_bazel_output_owned, run_bazel, run_bazel_owned, run_tool,
};
pub use atom_backends::{LaunchMode, ToolRunner};

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;

    use atom_backend_android::register_deploy_backend as register_android_deploy_backend;
    use atom_backend_ios::register_deploy_backend as register_ios_deploy_backend;
    use atom_backends::{
        BackendAutomationSession, BackendDefinition, DeployBackend, DeployBackendRegistry,
        DestinationCapability, DestinationDescriptor, DestinationKind, DestinationPlatform,
        LaunchMode, SessionLaunchBehavior, ToolRunner,
    };
    use atom_manifest::{AndroidConfig, AppConfig, BuildConfig, IosConfig, NormalizedManifest};
    use camino::{Utf8Path, Utf8PathBuf};
    use tempfile::tempdir;

    use crate::deploy::{deploy_android, deploy_backend, deploy_ios, stop_android, stop_ios};
    use crate::destinations::list_destinations;
    use crate::devices::android::AndroidDestination;
    use crate::devices::ios::{IosDestination, IosDestinationKind, select_default_ios_destination};

    #[derive(Default)]
    struct FakeToolRunner {
        calls: Vec<(String, Vec<String>)>,
        captures: VecDeque<String>,
    }

    impl ToolRunner for FakeToolRunner {
        fn run(
            &mut self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            Ok(())
        }

        fn capture(
            &mut self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            Ok(self
                .captures
                .pop_front()
                .expect("expected captured output for command"))
        }

        fn capture_json_file(
            &mut self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            Ok(self
                .captures
                .pop_front()
                .expect("expected captured JSON output for command"))
        }

        fn stream(
            &mut self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            Ok(())
        }
    }

    fn runnable_manifest(root: &Utf8PathBuf) -> NormalizedManifest {
        NormalizedManifest {
            repo_root: root.clone(),
            target_label: "//examples/hello-world/apps/hello_atom:hello_atom".to_owned(),
            metadata_path: root.join("bazel-out/hello_atom.atom.app.json"),
            app: AppConfig {
                name: "Hello Atom".to_owned(),
                slug: "hello-atom".to_owned(),
                entry_crate_label: "//examples/hello-world/apps/hello_atom:hello_atom".to_owned(),
                entry_crate_name: "hello_atom".to_owned(),
            },
            ios: IosConfig {
                enabled: true,
                bundle_id: Some("build.atom.hello".to_owned()),
                deployment_target: Some("17.0".to_owned()),
            },
            android: AndroidConfig {
                enabled: true,
                application_id: Some("build.atom.hello".to_owned()),
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

    fn idb_targets_json(simulator_state: &str) -> String {
        format!(
            r#"{{"udid":"SIM-123","name":"iPhone 16","state":"{simulator_state}","type":"simulator","os_version":"18.2","architecture":"x86_64"}}
{{"udid":"00008130-001431E90A78001C","name":"Alex's iPhone","state":"Booted","type":"device","os_version":"18.2","architecture":"arm64"}}"#
        )
    }

    fn first_party_registry() -> DeployBackendRegistry {
        let mut registry = DeployBackendRegistry::new();
        register_ios_deploy_backend(&mut registry).expect("ios backend should register");
        register_android_deploy_backend(&mut registry).expect("android backend should register");
        registry
    }

    #[test]
    fn ios_deploy_sequence_builds_boots_installs_and_launches() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                idb_targets_json("Shutdown"),
                "bazel-bin/generated/ios/hello-atom/app.app\n".to_owned(),
            ]),
        };

        deploy_ios(
            &root,
            &manifest,
            Some("SIM-123"),
            LaunchMode::Attached,
            &mut runner,
        )
        .expect("ios deploy");

        assert_eq!(
            runner.calls,
            vec![
                (
                    "idb".to_owned(),
                    vec!["list-targets".to_owned(), "--json".to_owned(),],
                ),
                (
                    "bazelisk".to_owned(),
                    vec![
                        "build".to_owned(),
                        "//generated/ios/hello-atom:app".to_owned(),
                        "--ios_multi_cpus=sim_arm64,x86_64".to_owned(),
                    ],
                ),
                (
                    "bazelisk".to_owned(),
                    vec![
                        "cquery".to_owned(),
                        "//generated/ios/hello-atom:app".to_owned(),
                        "--ios_multi_cpus=sim_arm64,x86_64".to_owned(),
                        "--output=files".to_owned(),
                    ],
                ),
                (
                    "idb".to_owned(),
                    vec!["boot".to_owned(), "SIM-123".to_owned()],
                ),
                (
                    "idb".to_owned(),
                    vec![
                        "install".to_owned(),
                        "--udid".to_owned(),
                        "SIM-123".to_owned(),
                        root.join("bazel-bin/generated/ios/hello-atom/app.app")
                            .as_str()
                            .to_owned(),
                    ],
                ),
                (
                    "idb".to_owned(),
                    vec![
                        "terminate".to_owned(),
                        "--udid".to_owned(),
                        "SIM-123".to_owned(),
                        "build.atom.hello".to_owned(),
                    ],
                ),
                (
                    "idb".to_owned(),
                    vec![
                        "launch".to_owned(),
                        "-f".to_owned(),
                        "-w".to_owned(),
                        "--udid".to_owned(),
                        "SIM-123".to_owned(),
                        "build.atom.hello".to_owned(),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn ios_device_deploy_sequence_builds_installs_and_launches() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                idb_targets_json("Shutdown"),
                "bazel-bin/generated/ios/hello-atom/app.app\n".to_owned(),
            ]),
        };

        deploy_ios(
            &root,
            &manifest,
            Some("00008130-001431E90A78001C"),
            LaunchMode::Attached,
            &mut runner,
        )
        .expect("ios device deploy");

        assert_eq!(
            runner.calls,
            vec![
                (
                    "idb".to_owned(),
                    vec!["list-targets".to_owned(), "--json".to_owned(),],
                ),
                (
                    "bazelisk".to_owned(),
                    vec![
                        "build".to_owned(),
                        "//generated/ios/hello-atom:app".to_owned(),
                        "--ios_multi_cpus=arm64".to_owned(),
                    ],
                ),
                (
                    "bazelisk".to_owned(),
                    vec![
                        "cquery".to_owned(),
                        "//generated/ios/hello-atom:app".to_owned(),
                        "--ios_multi_cpus=arm64".to_owned(),
                        "--output=files".to_owned(),
                    ],
                ),
                (
                    "idb".to_owned(),
                    vec![
                        "install".to_owned(),
                        "--udid".to_owned(),
                        "00008130-001431E90A78001C".to_owned(),
                        root.join("bazel-bin/generated/ios/hello-atom/app.app")
                            .as_str()
                            .to_owned(),
                    ],
                ),
                (
                    "idb".to_owned(),
                    vec![
                        "terminate".to_owned(),
                        "--udid".to_owned(),
                        "00008130-001431E90A78001C".to_owned(),
                        "build.atom.hello".to_owned(),
                    ],
                ),
                (
                    "idb".to_owned(),
                    vec![
                        "launch".to_owned(),
                        "-f".to_owned(),
                        "-w".to_owned(),
                        "--udid".to_owned(),
                        "00008130-001431E90A78001C".to_owned(),
                        "build.atom.hello".to_owned(),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn ios_simulator_deploy_uses_unpacked_app_when_cquery_returns_ipa() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let manifest = runnable_manifest(&root);
        let app_bundle =
            root.join("bazel-bin/generated/ios/hello-atom/app_archive-root/Payload/app.app");
        fs::create_dir_all(&app_bundle).expect("app bundle");
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                idb_targets_json("Shutdown"),
                "bazel-bin/generated/ios/hello-atom/app.ipa\n".to_owned(),
            ]),
        };

        deploy_ios(
            &root,
            &manifest,
            Some("SIM-123"),
            LaunchMode::Attached,
            &mut runner,
        )
        .expect("ios deploy");

        assert_eq!(
            runner.calls[4],
            (
                "idb".to_owned(),
                vec![
                    "install".to_owned(),
                    "--udid".to_owned(),
                    "SIM-123".to_owned(),
                    app_bundle.as_str().to_owned(),
                ],
            )
        );
    }

    #[test]
    fn android_deploy_sequence_builds_installs_and_launches() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                "bazel-bin/generated/android/hello-atom/app_unsigned.apk\nbazel-bin/generated/android/hello-atom/app.apk\n".to_owned(),
                "1\n".to_owned(),
                "4793\n".to_owned(),
            ]),
        };

        deploy_android(
            &root,
            &manifest,
            Some("emulator-5554"),
            LaunchMode::Attached,
            &mut runner,
        )
        .expect("android deploy");

        assert_eq!(
            runner.calls,
            vec![
                (
                    "bazelisk".to_owned(),
                    vec![
                        "build".to_owned(),
                        "//generated/android/hello-atom:app".to_owned(),
                        "--android_platforms=//platforms:arm64-v8a".to_owned(),
                    ],
                ),
                (
                    "bazelisk".to_owned(),
                    vec![
                        "cquery".to_owned(),
                        "//generated/android/hello-atom:app".to_owned(),
                        "--android_platforms=//platforms:arm64-v8a".to_owned(),
                        "--output=files".to_owned(),
                    ],
                ),
                (
                    "adb".to_owned(),
                    vec![
                        "-s".to_owned(),
                        "emulator-5554".to_owned(),
                        "shell".to_owned(),
                        "getprop".to_owned(),
                        "sys.boot_completed".to_owned(),
                    ],
                ),
                (
                    "adb".to_owned(),
                    vec![
                        "-s".to_owned(),
                        "emulator-5554".to_owned(),
                        "install".to_owned(),
                        "-r".to_owned(),
                        root.join("bazel-bin/generated/android/hello-atom/app.apk")
                            .as_str()
                            .to_owned(),
                    ],
                ),
                (
                    "adb".to_owned(),
                    vec![
                        "-s".to_owned(),
                        "emulator-5554".to_owned(),
                        "logcat".to_owned(),
                        "-c".to_owned(),
                    ],
                ),
                (
                    "adb".to_owned(),
                    vec![
                        "-s".to_owned(),
                        "emulator-5554".to_owned(),
                        "shell".to_owned(),
                        "am".to_owned(),
                        "start".to_owned(),
                        "-W".to_owned(),
                        "-n".to_owned(),
                        "build.atom.hello/.MainActivity".to_owned(),
                    ],
                ),
                (
                    "adb".to_owned(),
                    vec![
                        "-s".to_owned(),
                        "emulator-5554".to_owned(),
                        "shell".to_owned(),
                        "pidof".to_owned(),
                        "build.atom.hello".to_owned(),
                    ],
                ),
                (
                    "adb".to_owned(),
                    vec![
                        "-s".to_owned(),
                        "emulator-5554".to_owned(),
                        "logcat".to_owned(),
                        "--pid".to_owned(),
                        "4793".to_owned(),
                        "-s".to_owned(),
                        "AtomRuntime:*".to_owned(),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn default_ios_destination_prefers_an_iphone_simulator() {
        let destinations = vec![
            IosDestination {
                kind: IosDestinationKind::Simulator,
                id: "PAD-1".to_owned(),
                alternate_id: None,
                name: "iPad Pro".to_owned(),
                state: "Shutdown".to_owned(),
                runtime: Some("com.apple.CoreSimulator.SimRuntime.iOS-18-2".to_owned()),
                architecture: Some("x86_64".to_owned()),
                is_available: true,
            },
            IosDestination {
                kind: IosDestinationKind::Simulator,
                id: "PHONE-1".to_owned(),
                alternate_id: None,
                name: "iPhone 16".to_owned(),
                state: "Shutdown".to_owned(),
                runtime: Some("com.apple.CoreSimulator.SimRuntime.iOS-18-2".to_owned()),
                architecture: Some("x86_64".to_owned()),
                is_available: true,
            },
            IosDestination {
                kind: IosDestinationKind::Device,
                id: "DEVICE-1".to_owned(),
                alternate_id: None,
                name: "Alex's iPhone".to_owned(),
                state: "ready".to_owned(),
                runtime: None,
                architecture: Some("arm64".to_owned()),
                is_available: true,
            },
        ];

        let selected = select_default_ios_destination(&destinations).expect("destination");
        assert_eq!(selected.id, "PHONE-1");
    }

    #[test]
    fn android_destination_display_includes_model_when_available() {
        let destination = AndroidDestination {
            serial: "emulator-5554".to_owned(),
            state: "device".to_owned(),
            model: Some("Pixel 9".to_owned()),
            device_name: None,
            is_emulator: true,
            avd_name: None,
        };

        assert_eq!(
            destination.display_label(),
            "Emulator: Pixel 9 [Emulator; emulator-5554]"
        );
    }

    #[test]
    fn destination_listing_reports_capabilities_and_platforms() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                idb_targets_json("Booted"),
                "List of devices attached\nemulator-5554\tdevice model:Pixel_9 device:emu64a\n"
                    .to_owned(),
                "atom_35\n".to_owned(),
                "atom_35\nPixel_9_API_35\n".to_owned(),
            ]),
        };

        let registry = first_party_registry();
        let destinations = list_destinations(&root, &registry, &mut runner).expect("destinations");

        assert!(destinations.iter().any(|destination| {
            destination.id == "SIM-123"
                && destination.backend_id == "ios"
                && destination.platform == DestinationPlatform::Ios
                && destination.kind == DestinationKind::Simulator
                && destination
                    .capabilities
                    .contains(&DestinationCapability::Evaluate)
        }));
        assert!(destinations.iter().any(|destination| {
            destination.id == "00008130-001431E90A78001C"
                && destination.backend_id == "ios"
                && destination.platform == DestinationPlatform::Ios
                && destination.kind == DestinationKind::Device
                && destination.capabilities == vec![DestinationCapability::Launch]
        }));
        assert!(destinations.iter().any(|destination| {
            destination.id == "avd:atom_35"
                && destination.backend_id == "android"
                && destination.platform == DestinationPlatform::Android
                && destination.kind == DestinationKind::Emulator
                && destination
                    .capabilities
                    .contains(&DestinationCapability::InspectUi)
        }));
        assert!(destinations.iter().any(|destination| {
            destination.id == "avd:Pixel_9_API_35"
                && destination.backend_id == "android"
                && destination.platform == DestinationPlatform::Android
                && destination.kind == DestinationKind::Avd
                && destination.available
        }));

        let json = serde_json::to_string(&destinations).expect("destinations json");
        assert!(json.contains("\"backend_id\":\"ios\""));
        assert!(json.contains("\"platform\":\"ios\""));
        assert!(json.contains("\"capabilities\":[\"launch\""));
    }

    #[test]
    fn list_destinations_uses_registered_destination_backends() {
        struct FixtureBackend {
            id: &'static str,
            platform: &'static str,
            destination_id: &'static str,
        }

        impl BackendDefinition for FixtureBackend {
            fn id(&self) -> &'static str {
                self.id
            }

            fn platform(&self) -> &'static str {
                self.platform
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
                let platform = match self.platform {
                    "ios" => DestinationPlatform::Ios,
                    "android" => DestinationPlatform::Android,
                    value => panic!("unexpected test platform {value}"),
                };
                Ok(vec![DestinationDescriptor {
                    backend_id: self.id.to_owned(),
                    id: self.destination_id.to_owned(),
                    platform,
                    kind: DestinationKind::Device,
                    display_name: format!("{} destination", self.id),
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
                panic!("test backend should not deploy")
            }

            fn stop(
                &self,
                _repo_root: &Utf8Path,
                _manifest: &NormalizedManifest,
                _requested_destination: Option<&str>,
                _runner: &mut dyn ToolRunner,
            ) -> atom_ffi::AtomResult<()> {
                panic!("test backend should not stop")
            }

            fn new_automation_session<'a>(
                &self,
                _repo_root: &'a Utf8Path,
                _manifest: &'a NormalizedManifest,
                _destination_id: &'a str,
                _runner: &'a mut dyn ToolRunner,
                _launch_behavior: SessionLaunchBehavior,
            ) -> atom_ffi::AtomResult<Box<dyn BackendAutomationSession + 'a>> {
                panic!("test backend should not create automation sessions")
            }
        }

        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let mut runner = FakeToolRunner::default();
        let mut registry = DeployBackendRegistry::new();
        registry
            .register(Box::new(FixtureBackend {
                id: "alpha",
                platform: "ios",
                destination_id: "ALPHA-1",
            }))
            .expect("alpha backend should register");
        registry
            .register(Box::new(FixtureBackend {
                id: "beta",
                platform: "android",
                destination_id: "BETA-1",
            }))
            .expect("beta backend should register");

        let destinations = list_destinations(&root, &registry, &mut runner).expect("destinations");

        assert_eq!(destinations.len(), 2);
        assert_eq!(
            destinations
                .iter()
                .map(|destination| destination.backend_id.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
    }

    #[test]
    fn deploy_backend_uses_registered_backend_dispatch() {
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
                Ok(Vec::new())
            }

            fn deploy(
                &self,
                repo_root: &Utf8Path,
                _manifest: &NormalizedManifest,
                requested_destination: Option<&str>,
                _launch_mode: LaunchMode,
                runner: &mut dyn ToolRunner,
            ) -> atom_ffi::AtomResult<()> {
                runner.run(
                    repo_root,
                    "fixture",
                    &[requested_destination.unwrap_or("default").to_owned()],
                )
            }

            fn stop(
                &self,
                _repo_root: &Utf8Path,
                _manifest: &NormalizedManifest,
                _requested_destination: Option<&str>,
                _runner: &mut dyn ToolRunner,
            ) -> atom_ffi::AtomResult<()> {
                panic!("stop should not be used in this test")
            }

            fn new_automation_session<'a>(
                &self,
                _repo_root: &'a Utf8Path,
                _manifest: &'a NormalizedManifest,
                _destination_id: &'a str,
                _runner: &'a mut dyn ToolRunner,
                _launch_behavior: SessionLaunchBehavior,
            ) -> atom_ffi::AtomResult<Box<dyn BackendAutomationSession + 'a>> {
                panic!("automation session should not be used in this test")
            }
        }

        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner::default();
        let mut registry = DeployBackendRegistry::new();
        registry
            .register(Box::new(FixtureBackend))
            .expect("fixture backend should register");

        deploy_backend(
            &root,
            &manifest,
            &registry,
            "fixture",
            Some("DEVICE-123"),
            LaunchMode::Detached,
            &mut runner,
        )
        .expect("fixture backend deploy");

        assert_eq!(
            runner.calls,
            vec![("fixture".to_owned(), vec!["DEVICE-123".to_owned()])]
        );
    }

    #[test]
    fn ios_detached_deploy_waits_for_ui_readiness_before_returning() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                idb_targets_json("Shutdown"),
                "bazel-bin/generated/ios/hello-atom/app.app\n".to_owned(),
                r#"{"elements":[{"AXUniqueId":"idb-node-0","type":"Application","AXLabel":"Hello Atom","AXValue":"Hello Atom","visible":true,"enabled":true,"frame":{"x":0,"y":0,"width":402,"height":874}},{"AXUniqueId":"atom.demo.title","type":"StaticText","AXLabel":"Hello Atom","AXValue":"Hello Atom","visible":true,"enabled":true,"frame":{"x":24,"y":96,"width":140,"height":28}}]}"#
                    .to_owned(),
            ]),
        };

        deploy_ios(
            &root,
            &manifest,
            Some("SIM-123"),
            LaunchMode::Detached,
            &mut runner,
        )
        .expect("ios deploy");

        assert!(runner.calls.contains(&(
            "idb".to_owned(),
            vec![
                "launch".to_owned(),
                "-f".to_owned(),
                "--udid".to_owned(),
                "SIM-123".to_owned(),
                "build.atom.hello".to_owned(),
            ],
        )));
        assert!(runner.calls.contains(&(
            "idb".to_owned(),
            vec![
                "ui".to_owned(),
                "describe-all".to_owned(),
                "--udid".to_owned(),
                "SIM-123".to_owned(),
            ],
        )));
    }

    #[test]
    fn android_detached_deploy_skips_log_streaming() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                "bazel-bin/generated/android/hello-atom/app_unsigned.apk\nbazel-bin/generated/android/hello-atom/app.apk\n".to_owned(),
                "1\n".to_owned(),
            ]),
        };

        deploy_android(
            &root,
            &manifest,
            Some("emulator-5554"),
            LaunchMode::Detached,
            &mut runner,
        )
        .expect("android deploy");

        assert_eq!(
            runner.calls,
            vec![
                (
                    "bazelisk".to_owned(),
                    vec![
                        "build".to_owned(),
                        "//generated/android/hello-atom:app".to_owned(),
                        "--android_platforms=//platforms:arm64-v8a".to_owned(),
                    ],
                ),
                (
                    "bazelisk".to_owned(),
                    vec![
                        "cquery".to_owned(),
                        "//generated/android/hello-atom:app".to_owned(),
                        "--android_platforms=//platforms:arm64-v8a".to_owned(),
                        "--output=files".to_owned(),
                    ],
                ),
                (
                    "adb".to_owned(),
                    vec![
                        "-s".to_owned(),
                        "emulator-5554".to_owned(),
                        "shell".to_owned(),
                        "getprop".to_owned(),
                        "sys.boot_completed".to_owned(),
                    ],
                ),
                (
                    "adb".to_owned(),
                    vec![
                        "-s".to_owned(),
                        "emulator-5554".to_owned(),
                        "install".to_owned(),
                        "-r".to_owned(),
                        root.join("bazel-bin/generated/android/hello-atom/app.apk")
                            .as_str()
                            .to_owned(),
                    ],
                ),
                (
                    "adb".to_owned(),
                    vec![
                        "-s".to_owned(),
                        "emulator-5554".to_owned(),
                        "shell".to_owned(),
                        "am".to_owned(),
                        "start".to_owned(),
                        "-W".to_owned(),
                        "-n".to_owned(),
                        "build.atom.hello/.MainActivity".to_owned(),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn ios_stop_only_terminates_when_app_is_running() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                idb_targets_json("Booted"),
                "build.atom.hello | hello-atom | user | arm64 | Running | Not Debuggable | pid=42\n"
                    .to_owned(),
            ]),
        };

        stop_ios(&root, &manifest, Some("SIM-123"), &mut runner).expect("ios stop");

        assert_eq!(
            runner.calls,
            vec![
                (
                    "idb".to_owned(),
                    vec!["list-targets".to_owned(), "--json".to_owned(),],
                ),
                (
                    "idb".to_owned(),
                    vec![
                        "list-apps".to_owned(),
                        "--udid".to_owned(),
                        "SIM-123".to_owned(),
                    ],
                ),
                (
                    "idb".to_owned(),
                    vec![
                        "terminate".to_owned(),
                        "--udid".to_owned(),
                        "SIM-123".to_owned(),
                        "build.atom.hello".to_owned(),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn android_stop_force_stops_running_destination() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner::default();

        stop_android(&root, &manifest, Some("emulator-5554"), &mut runner).expect("android stop");

        assert_eq!(
            runner.calls,
            vec![(
                "adb".to_owned(),
                vec![
                    "-s".to_owned(),
                    "emulator-5554".to_owned(),
                    "shell".to_owned(),
                    "am".to_owned(),
                    "force-stop".to_owned(),
                    "build.atom.hello".to_owned(),
                ],
            )]
        );
    }
}
