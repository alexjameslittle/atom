mod deploy;
mod devices;
mod tools;

use std::ffi::OsString;
use std::path::PathBuf;

use atom_cng::{build_generation_plan, emit_host_tree, render_prebuild_plan};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::load_manifest;
use atom_modules::resolve_modules;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, Parser, Subcommand};

use crate::deploy::{deploy_android, deploy_ios};
pub use crate::tools::CommandOutput;
use crate::tools::{ProcessRunner, ToolRunner, run_bazel};

#[derive(Debug, Parser)]
#[command(name = "atom")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Prebuild(PrebuildArgs),
    Run(RunArgs),
    Test,
}

#[derive(Debug, Args)]
struct PrebuildArgs {
    #[arg(long)]
    target: String,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Args)]
struct RunArgs {
    #[command(subcommand)]
    platform: RunPlatform,
}

#[derive(Debug, Args)]
struct TargetArgs {
    #[arg(long)]
    target: String,
    #[arg(long)]
    device: Option<String>,
}

#[derive(Debug, Subcommand)]
enum RunPlatform {
    Ios(TargetArgs),
    Android(TargetArgs),
}

/// # Errors
///
/// Returns an error if the CLI command fails.
pub fn run_from_env(cwd: &Utf8Path) -> AtomResult<CommandOutput> {
    run_from_args(std::env::args_os(), cwd)
}

#[must_use]
pub fn run_process() -> CommandOutput {
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
        .unwrap_or_else(|| Utf8PathBuf::from("."));

    match run_from_env(&cwd) {
        Ok(output) => output,
        Err(error) => CommandOutput {
            stdout: Vec::new(),
            stderr: error.encode(),
            exit_code: error.exit_code(),
        },
    }
}

/// # Errors
///
/// Returns an error if the CLI command fails.
pub fn run_from_args<I, T>(args: I, cwd: &Utf8Path) -> AtomResult<CommandOutput>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)
        .map_err(|error| AtomError::new(AtomErrorCode::CliUsageError, error.to_string()))?;
    let mut runner = ProcessRunner;
    execute(&cli, cwd, &mut runner)
}

fn execute(cli: &Cli, cwd: &Utf8Path, runner: &mut impl ToolRunner) -> AtomResult<CommandOutput> {
    match &cli.command {
        Commands::Prebuild(args) => execute_prebuild(cwd, args),
        Commands::Run(args) => execute_run(cwd, args, runner),
        Commands::Test => execute_test(cwd, runner),
    }
}

fn execute_prebuild(cwd: &Utf8Path, args: &PrebuildArgs) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    let manifest = load_manifest(&repo_root, &args.target)?;
    let modules = resolve_modules(&repo_root, &manifest.modules)?;
    let plan = build_generation_plan(&manifest, &modules)?;

    if args.dry_run {
        return Ok(CommandOutput {
            stdout: render_prebuild_plan(&plan),
            stderr: Vec::new(),
            exit_code: 0,
        });
    }

    let roots = emit_host_tree(&repo_root, &plan)?;
    let mut summary = String::new();
    for root in roots {
        summary.push_str(root.as_str());
        summary.push('\n');
    }
    Ok(CommandOutput {
        stdout: summary.into_bytes(),
        stderr: Vec::new(),
        exit_code: 0,
    })
}

fn execute_run(
    cwd: &Utf8Path,
    args: &RunArgs,
    runner: &mut impl ToolRunner,
) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    let (platform, target) = match &args.platform {
        RunPlatform::Ios(target) => ("ios", target),
        RunPlatform::Android(target) => ("android", target),
    };
    let manifest = load_manifest(&repo_root, &target.target)?;
    let enabled = match platform {
        "ios" => manifest.ios.enabled,
        "android" => manifest.android.enabled,
        _ => unreachable!("run platform should be validated by clap"),
    };
    if !enabled {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            format!("{platform} platform is not enabled"),
            platform,
        ));
    }

    let modules = resolve_modules(&repo_root, &manifest.modules)?;
    let plan = build_generation_plan(&manifest, &modules)?;
    let _ = emit_host_tree(&repo_root, &plan)?;

    match platform {
        "ios" => deploy_ios(&repo_root, &manifest, target.device.as_deref(), runner)?,
        "android" => deploy_android(&repo_root, &manifest, target.device.as_deref(), runner)?,
        _ => unreachable!("run platform should be validated by clap"),
    }

    Ok(CommandOutput {
        stdout: Vec::new(),
        stderr: Vec::new(),
        exit_code: 0,
    })
}

fn execute_test(cwd: &Utf8Path, runner: &mut impl ToolRunner) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    run_bazel(runner, &repo_root, &["test", "//..."])?;
    Ok(CommandOutput {
        stdout: Vec::new(),
        stderr: Vec::new(),
        exit_code: 0,
    })
}

fn resolve_workspace_root(cwd: &Utf8Path) -> AtomResult<Utf8PathBuf> {
    resolve_workspace_root_with_workspace_dir(cwd, workspace_directory().as_deref())
}

fn resolve_workspace_root_with_workspace_dir(
    cwd: &Utf8Path,
    workspace_directory: Option<&Utf8Path>,
) -> AtomResult<Utf8PathBuf> {
    let command_root = resolve_command_root(cwd, workspace_directory);
    find_workspace_root(&command_root).ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::CliUsageError,
            "atom commands must run inside a Bazel workspace that uses bzlmod",
        )
    })
}

fn workspace_directory() -> Option<Utf8PathBuf> {
    std::env::var_os("BUILD_WORKSPACE_DIRECTORY")
        .map(PathBuf::from)
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
}

fn resolve_command_root(cwd: &Utf8Path, workspace_directory: Option<&Utf8Path>) -> Utf8PathBuf {
    workspace_directory.map_or_else(|| cwd.to_owned(), Utf8PathBuf::from)
}

fn find_workspace_root(start: &Utf8Path) -> Option<Utf8PathBuf> {
    for candidate in start.ancestors() {
        if candidate.join("MODULE.bazel").exists() {
            return Some(candidate.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;

    use atom_manifest::{AndroidConfig, AppConfig, BuildConfig, IosConfig, NormalizedManifest};
    use camino::{Utf8Path, Utf8PathBuf};
    use tempfile::tempdir;

    use crate::deploy::{deploy_android, deploy_ios};
    use crate::devices::android::AndroidDestination;
    use crate::devices::ios::{IosDestination, IosDestinationKind, select_default_ios_destination};
    use crate::tools::ToolRunner;

    use super::{find_workspace_root, resolve_workspace_root_with_workspace_dir, run_from_args};

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
        }
    }

    #[test]
    fn workspace_root_prefers_nearest_module_file() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let outer = root.join("outer");
        let inner = outer.join("examples/hello-world");
        fs::create_dir_all(&inner).expect("example dir");
        fs::write(outer.join("MODULE.bazel"), "module(name = \"outer\")\n")
            .expect("outer workspace");
        fs::write(inner.join("MODULE.bazel"), "module(name = \"inner\")\n")
            .expect("inner workspace");

        let detected = find_workspace_root(&inner).expect("workspace root");
        assert_eq!(detected, inner);
    }

    #[test]
    fn invalid_command_maps_to_cli_usage_error() {
        let directory = tempdir().expect("tempdir");
        let cwd = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let error = run_from_args(["atom", "unknown"], &cwd).expect_err("invalid command");
        assert_eq!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
    }

    #[test]
    fn workspace_directory_is_used_for_bazel_run_context() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let runfiles_root =
            root.join("bazel-out/darwin_arm64-fastbuild/bin/crates/atom-cli/atom.runfiles/_main");
        fs::create_dir_all(&runfiles_root).expect("runfiles root");
        fs::write(root.join("MODULE.bazel"), "module(name = \"atom\")\n").expect("workspace");

        let repo_root = resolve_workspace_root_with_workspace_dir(&runfiles_root, Some(&root))
            .expect("workspace root");

        assert_eq!(repo_root, root);
    }

    #[test]
    fn run_command_accepts_target_after_platform_subcommand() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        fs::write(root.join("MODULE.bazel"), "module(name = \"atom\")\n").expect("workspace");

        let error = run_from_args(
            [
                "atom",
                "run",
                "ios",
                "--target",
                "//examples/hello-world/apps/hello_atom:hello_atom",
            ],
            &root,
        )
        .expect_err("missing manifest should fail after clap accepts the command");

        assert_ne!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
    }

    #[test]
    fn ios_deploy_sequence_builds_boots_installs_and_launches() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let manifest = runnable_manifest(&root);
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                "{\"devices\":{\"com.apple.CoreSimulator.SimRuntime.iOS-18-2\":[{\"name\":\"iPhone 16\",\"udid\":\"SIM-123\",\"state\":\"Shutdown\",\"isAvailable\":true}]}}\n".to_owned(),
                "bazel-bin/generated/ios/hello-atom/app.app\n".to_owned(),
            ]),
        };

        deploy_ios(&root, &manifest, Some("SIM-123"), &mut runner).expect("ios deploy");

        assert_eq!(
            runner.calls,
            vec![
                (
                    "xcrun".to_owned(),
                    vec![
                        "simctl".to_owned(),
                        "list".to_owned(),
                        "devices".to_owned(),
                        "available".to_owned(),
                        "-j".to_owned(),
                    ],
                ),
                (
                    "bazelisk".to_owned(),
                    vec![
                        "build".to_owned(),
                        "//generated/ios/hello-atom:app".to_owned(),
                        "--ios_multi_cpus=sim_arm64".to_owned(),
                    ],
                ),
                (
                    "bazelisk".to_owned(),
                    vec![
                        "cquery".to_owned(),
                        "//generated/ios/hello-atom:app".to_owned(),
                        "--ios_multi_cpus=sim_arm64".to_owned(),
                        "--output=files".to_owned(),
                    ],
                ),
                (
                    "xcrun".to_owned(),
                    vec!["simctl".to_owned(), "boot".to_owned(), "SIM-123".to_owned()],
                ),
                (
                    "xcrun".to_owned(),
                    vec![
                        "simctl".to_owned(),
                        "bootstatus".to_owned(),
                        "SIM-123".to_owned(),
                        "-b".to_owned(),
                    ],
                ),
                (
                    "xcrun".to_owned(),
                    vec![
                        "simctl".to_owned(),
                        "install".to_owned(),
                        "SIM-123".to_owned(),
                        root.join("bazel-bin/generated/ios/hello-atom/app.app")
                            .as_str()
                            .to_owned(),
                    ],
                ),
                (
                    "xcrun".to_owned(),
                    vec![
                        "simctl".to_owned(),
                        "launch".to_owned(),
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
                "{\"devices\":{\"com.apple.CoreSimulator.SimRuntime.iOS-18-2\":[{\"name\":\"iPhone 16\",\"udid\":\"SIM-123\",\"state\":\"Shutdown\",\"isAvailable\":true}]}}\n".to_owned(),
                "bazel-bin/generated/ios/hello-atom/app.app\n".to_owned(),
            ]),
        };

        deploy_ios(
            &root,
            &manifest,
            Some("00008130-001431E90A78001C"),
            &mut runner,
        )
        .expect("ios device deploy");

        assert_eq!(
            runner.calls,
            vec![
                (
                    "xcrun".to_owned(),
                    vec![
                        "simctl".to_owned(),
                        "list".to_owned(),
                        "devices".to_owned(),
                        "available".to_owned(),
                        "-j".to_owned(),
                    ],
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
                    "xcrun".to_owned(),
                    vec![
                        "devicectl".to_owned(),
                        "device".to_owned(),
                        "install".to_owned(),
                        "app".to_owned(),
                        "--device".to_owned(),
                        "00008130-001431E90A78001C".to_owned(),
                        root.join("bazel-bin/generated/ios/hello-atom/app.app")
                            .as_str()
                            .to_owned(),
                    ],
                ),
                (
                    "xcrun".to_owned(),
                    vec![
                        "devicectl".to_owned(),
                        "device".to_owned(),
                        "process".to_owned(),
                        "launch".to_owned(),
                        "--device".to_owned(),
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
                "{\"devices\":{\"com.apple.CoreSimulator.SimRuntime.iOS-18-2\":[{\"name\":\"iPhone 16\",\"udid\":\"SIM-123\",\"state\":\"Shutdown\",\"isAvailable\":true}]}}\n".to_owned(),
                "bazel-bin/generated/ios/hello-atom/app.ipa\n".to_owned(),
            ]),
        };

        deploy_ios(&root, &manifest, Some("SIM-123"), &mut runner).expect("ios deploy");

        assert_eq!(
            runner.calls[5],
            (
                "xcrun".to_owned(),
                vec![
                    "simctl".to_owned(),
                    "install".to_owned(),
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
            ]),
        };

        deploy_android(&root, &manifest, Some("emulator-5554"), &mut runner)
            .expect("android deploy");

        assert_eq!(
            runner.calls,
            vec![
                (
                    "bazelisk".to_owned(),
                    vec![
                        "build".to_owned(),
                        "//generated/android/hello-atom:app".to_owned()
                    ],
                ),
                (
                    "bazelisk".to_owned(),
                    vec![
                        "cquery".to_owned(),
                        "//generated/android/hello-atom:app".to_owned(),
                        "--output=files".to_owned(),
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
                        "-n".to_owned(),
                        "build.atom.hello/.MainActivity".to_owned(),
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
                is_available: true,
            },
            IosDestination {
                kind: IosDestinationKind::Simulator,
                id: "PHONE-1".to_owned(),
                alternate_id: None,
                name: "iPhone 16".to_owned(),
                state: "Shutdown".to_owned(),
                runtime: Some("com.apple.CoreSimulator.SimRuntime.iOS-18-2".to_owned()),
                is_available: true,
            },
            IosDestination {
                kind: IosDestinationKind::Device,
                id: "DEVICE-1".to_owned(),
                alternate_id: None,
                name: "Alex's iPhone".to_owned(),
                state: "ready".to_owned(),
                runtime: None,
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
        };

        assert_eq!(
            destination.display_label(),
            "Emulator: Pixel 9 [emulator-5554]"
        );
    }
}
