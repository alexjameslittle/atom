mod doctor;

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use atom_backend_android::{
    register_deploy_backend as register_android_deploy_backend,
    register_generation_backend as register_android_generation_backend,
};
use atom_backend_ios::{
    register_deploy_backend as register_ios_deploy_backend,
    register_generation_backend as register_ios_generation_backend,
};
use atom_backends::{DeployBackendRegistry, GenerationBackendRegistry, InteractionRequest};
use atom_cng::{ConfigPluginRegistry, build_generation_plan, emit_host_tree, render_prebuild_plan};
pub use atom_deploy::CommandOutput;
use atom_deploy::destinations::{list_backend_destinations, render_destination_lines};
use atom_deploy::evaluate::{
    capture_logs, capture_screenshot, capture_video, evaluate_run, inspect_ui, interact,
};
use atom_deploy::progress::run_step;
use atom_deploy::{
    LaunchMode, ProcessRunner, ToolRunner, deploy_backend, ensure_backend_enabled, run_bazel,
    stop_backend,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::load_manifest;
use atom_modules::resolve_modules;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, Parser, Subcommand, ValueEnum};
use doctor::DoctorArgs;
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(name = "atom")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Doctor(DoctorArgs),
    Prebuild(PrebuildArgs),
    Run(RunArgs),
    Stop(StopArgs),
    Test,
    Destinations(ListDestinationsArgs),
    Devices(DevicesArgs),
    Evidence(EvidenceArgs),
    Inspect(InspectArgs),
    Interact(InteractArgs),
    Evaluate(EvaluateArgs),
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
    #[arg(long, value_enum)]
    platform: PlatformArg,
    #[command(flatten)]
    target: TargetArgs,
}

#[derive(Debug, Args)]
struct TargetArgs {
    #[arg(long)]
    target: String,
    #[arg(long, alias = "device")]
    destination: Option<String>,
    #[arg(long, default_value_t = false)]
    detach: bool,
}

#[derive(Debug, Args)]
struct StopArgs {
    #[arg(long, value_enum)]
    platform: PlatformArg,
    #[command(flatten)]
    target: StopTargetArgs,
}

#[derive(Debug, Args)]
struct StopTargetArgs {
    #[arg(long)]
    target: String,
    #[arg(long, alias = "device")]
    destination: Option<String>,
}

#[derive(Debug, Args)]
struct ListDestinationsArgs {
    #[arg(long, value_enum)]
    platform: PlatformArg,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
struct DevicesArgs {
    #[arg(long, value_enum)]
    platform: PlatformArg,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PlatformArg {
    Ios,
    Android,
}

impl PlatformArg {
    fn as_backend_id(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
        }
    }
}

#[derive(Debug, Args)]
struct EvidenceArgs {
    #[command(subcommand)]
    command: EvidenceCommand,
}

#[derive(Debug, Subcommand)]
enum EvidenceCommand {
    Logs(EvidenceLogsArgs),
    Screenshot(EvidenceOutputArgs),
    Video(EvidenceVideoArgs),
}

#[derive(Debug, Args)]
struct EvidenceOutputArgs {
    #[command(flatten)]
    target: TargetDestinationArgs,
    #[arg(long)]
    output: Utf8PathBuf,
}

#[derive(Debug, Args)]
struct EvidenceLogsArgs {
    #[command(flatten)]
    target: TargetDestinationArgs,
    #[arg(long)]
    output: Utf8PathBuf,
    #[arg(long, default_value_t = 60)]
    seconds: u64,
}

#[derive(Debug, Args)]
struct EvidenceVideoArgs {
    #[command(flatten)]
    target: TargetDestinationArgs,
    #[arg(long)]
    output: Utf8PathBuf,
    #[arg(long, default_value_t = 5)]
    seconds: u64,
}

#[derive(Debug, Args)]
struct TargetDestinationArgs {
    #[arg(long, value_enum)]
    platform: PlatformArg,
    #[arg(long)]
    target: String,
    #[arg(long, alias = "device")]
    destination: String,
}

#[derive(Debug, Args)]
struct InspectArgs {
    #[command(subcommand)]
    command: InspectCommand,
}

#[derive(Debug, Subcommand)]
enum InspectCommand {
    Ui(InspectUiArgs),
}

#[derive(Debug, Args)]
struct InspectUiArgs {
    #[command(flatten)]
    target: TargetDestinationArgs,
    #[arg(long)]
    output: Option<Utf8PathBuf>,
}

#[derive(Debug, Args)]
struct InteractArgs {
    #[command(subcommand)]
    command: InteractCommand,
}

#[derive(Debug, Subcommand)]
enum InteractCommand {
    Tap(PointOrTargetArgs),
    LongPress(PointOrTargetArgs),
    Swipe(PointArgs),
    Drag(PointArgs),
    TypeText(TypeTextArgs),
}

#[derive(Debug, Args)]
struct PointOrTargetArgs {
    #[command(flatten)]
    target: TargetDestinationArgs,
    #[arg(long)]
    target_id: Option<String>,
    #[arg(long)]
    x: Option<f64>,
    #[arg(long)]
    y: Option<f64>,
}

#[derive(Debug, Args)]
struct PointArgs {
    #[command(flatten)]
    target: TargetDestinationArgs,
    #[arg(long)]
    x: Option<f64>,
    #[arg(long)]
    y: Option<f64>,
}

#[derive(Debug, Args)]
struct TypeTextArgs {
    #[command(flatten)]
    target: TargetDestinationArgs,
    #[arg(long)]
    target_id: Option<String>,
    #[arg(long)]
    text: String,
}

#[derive(Debug, Args)]
struct EvaluateArgs {
    #[command(subcommand)]
    command: EvaluateCommand,
}

#[derive(Debug, Subcommand)]
enum EvaluateCommand {
    Run(EvaluateRunArgs),
}

#[derive(Debug, Args)]
struct EvaluateRunArgs {
    #[command(flatten)]
    target: TargetDestinationArgs,
    #[arg(long)]
    plan: Utf8PathBuf,
    #[arg(long)]
    artifacts_dir: Utf8PathBuf,
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
        Commands::Doctor(args) => execute_doctor(cwd, args),
        Commands::Prebuild(args) => execute_prebuild(cwd, args),
        Commands::Run(args) => execute_run(cwd, args, runner),
        Commands::Stop(args) => execute_stop(cwd, args, runner),
        Commands::Test => execute_test(cwd, runner),
        Commands::Destinations(args) => execute_destinations(cwd, args, runner),
        Commands::Devices(args) => execute_devices(cwd, args, runner),
        Commands::Evidence(args) => execute_evidence(cwd, args, runner),
        Commands::Inspect(args) => execute_inspect(cwd, args, runner),
        Commands::Interact(args) => execute_interact(cwd, args, runner),
        Commands::Evaluate(args) => execute_evaluate(cwd, args, runner),
    }
}

fn execute_doctor(cwd: &Utf8Path, args: &DoctorArgs) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    doctor::execute(&repo_root, args)
}

fn execute_prebuild(cwd: &Utf8Path, args: &PrebuildArgs) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    let manifest = load_manifest(&repo_root, &args.target)?;
    let modules = resolve_modules(&repo_root, &manifest.modules)?;
    let generation_registry = first_party_generation_backend_registry()?;
    let plan = build_generation_plan(
        &manifest,
        &modules,
        &default_config_plugin_registry(),
        &generation_registry,
    )?;

    if args.dry_run {
        return Ok(CommandOutput {
            stdout: render_prebuild_plan(&plan),
            stderr: Vec::new(),
            exit_code: 0,
        });
    }

    let roots = emit_host_tree(&repo_root, &plan, &generation_registry)?;
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
    let platform = args.platform.as_backend_id();
    let target = &args.target;
    let manifest = load_manifest(&repo_root, &target.target)?;
    let launch_mode = if target.detach {
        LaunchMode::Detached
    } else {
        LaunchMode::Attached
    };
    let deploy_registry = first_party_deploy_backend_registry()?;

    preflight_run_backend(&manifest, &deploy_registry, platform, || {
        let modules = resolve_modules(&repo_root, &manifest.modules)?;
        let generation_registry = first_party_generation_backend_registry()?;
        let plan = build_generation_plan(
            &manifest,
            &modules,
            &default_config_plugin_registry(),
            &generation_registry,
        )?;
        emit_host_tree(&repo_root, &plan, &generation_registry).map(|_| ())
    })?;

    deploy_backend(
        &repo_root,
        &manifest,
        &deploy_registry,
        platform,
        target.destination.as_deref(),
        launch_mode,
        runner,
    )?;

    Ok(success_output(Vec::new()))
}

fn preflight_run_backend(
    manifest: &atom_manifest::NormalizedManifest,
    deploy_registry: &DeployBackendRegistry,
    backend_id: &str,
    generate: impl FnOnce() -> AtomResult<()>,
) -> AtomResult<()> {
    ensure_backend_enabled(manifest, deploy_registry, backend_id)?;
    run_step(
        "Generating build files...",
        "Build files generated",
        "Code generation failed",
        generate,
    )
}

fn execute_stop(
    cwd: &Utf8Path,
    args: &StopArgs,
    runner: &mut impl ToolRunner,
) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    let platform = args.platform.as_backend_id();
    let target = &args.target;
    let manifest = load_manifest(&repo_root, &target.target)?;
    let deploy_registry = first_party_deploy_backend_registry()?;
    stop_backend(
        &repo_root,
        &manifest,
        &deploy_registry,
        platform,
        target.destination.as_deref(),
        runner,
    )?;

    Ok(success_output(Vec::new()))
}

fn execute_test(cwd: &Utf8Path, runner: &mut impl ToolRunner) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    run_step("Running tests...", "Tests passed", "Tests failed", || {
        run_bazel(runner, &repo_root, &["test", "//..."])
    })?;
    Ok(success_output(Vec::new()))
}

fn execute_destinations(
    cwd: &Utf8Path,
    args: &ListDestinationsArgs,
    runner: &mut impl ToolRunner,
) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    let deploy_registry = first_party_deploy_backend_registry()?;
    let destinations = list_backend_destinations(
        &repo_root,
        &deploy_registry,
        args.platform.as_backend_id(),
        runner,
    )?;
    if args.json {
        return json_output(&destinations);
    }
    Ok(text_output(render_destination_lines(&destinations)))
}

fn execute_devices(
    cwd: &Utf8Path,
    args: &DevicesArgs,
    runner: &mut impl ToolRunner,
) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    let deploy_registry = first_party_deploy_backend_registry()?;
    let destinations = list_backend_destinations(
        &repo_root,
        &deploy_registry,
        args.platform.as_backend_id(),
        runner,
    )?;
    if args.json {
        return json_output(&destinations);
    }
    Ok(text_output(render_destination_lines(&destinations)))
}

fn execute_evidence(
    cwd: &Utf8Path,
    args: &EvidenceArgs,
    runner: &mut impl ToolRunner,
) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    let deploy_registry = first_party_deploy_backend_registry()?;
    match &args.command {
        EvidenceCommand::Logs(args) => {
            let manifest = load_manifest(&repo_root, &args.target.target)?;
            let output = resolve_cli_path(&repo_root, &args.output);
            capture_logs(
                &repo_root,
                &manifest,
                &deploy_registry,
                args.target.platform.as_backend_id(),
                &args.target.destination,
                &output,
                args.seconds,
                runner,
            )?;
            Ok(text_output(format!("{output}\n")))
        }
        EvidenceCommand::Screenshot(args) => {
            let manifest = load_manifest(&repo_root, &args.target.target)?;
            let output = resolve_cli_path(&repo_root, &args.output);
            capture_screenshot(
                &repo_root,
                &manifest,
                &deploy_registry,
                args.target.platform.as_backend_id(),
                &args.target.destination,
                &output,
                runner,
            )?;
            Ok(text_output(format!("{output}\n")))
        }
        EvidenceCommand::Video(args) => {
            let manifest = load_manifest(&repo_root, &args.target.target)?;
            let output = resolve_cli_path(&repo_root, &args.output);
            capture_video(
                &repo_root,
                &manifest,
                &deploy_registry,
                args.target.platform.as_backend_id(),
                &args.target.destination,
                &output,
                args.seconds,
                runner,
            )?;
            Ok(text_output(format!("{output}\n")))
        }
    }
}

fn execute_inspect(
    cwd: &Utf8Path,
    args: &InspectArgs,
    runner: &mut impl ToolRunner,
) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    let deploy_registry = first_party_deploy_backend_registry()?;
    match &args.command {
        InspectCommand::Ui(args) => {
            let manifest = load_manifest(&repo_root, &args.target.target)?;
            let snapshot = inspect_ui(
                &repo_root,
                &manifest,
                &deploy_registry,
                args.target.platform.as_backend_id(),
                &args.target.destination,
                runner,
            )?;
            if let Some(output) = args
                .output
                .as_ref()
                .map(|path| resolve_cli_path(&repo_root, path))
            {
                if let Some(parent) = output.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        AtomError::with_path(
                            AtomErrorCode::ExternalToolFailed,
                            format!("failed to create inspect output directory: {error}"),
                            parent.as_str(),
                        )
                    })?;
                }
                fs::write(
                    &output,
                    serde_json::to_string_pretty(&snapshot).map_err(|error| {
                        AtomError::new(
                            AtomErrorCode::InternalBug,
                            format!("failed to encode UI snapshot: {error}"),
                        )
                    })?,
                )
                .map_err(|error| {
                    AtomError::with_path(
                        AtomErrorCode::ExternalToolFailed,
                        format!("failed to write UI snapshot: {error}"),
                        output.as_str(),
                    )
                })?;
                return Ok(text_output(format!("{output}\n")));
            }
            json_output(&snapshot)
        }
    }
}

fn execute_interact(
    cwd: &Utf8Path,
    args: &InteractArgs,
    runner: &mut impl ToolRunner,
) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    let deploy_registry = first_party_deploy_backend_registry()?;
    let (target, request) = match &args.command {
        InteractCommand::Tap(args) => (
            &args.target,
            InteractionRequest::Tap {
                target_id: args.target_id.clone(),
                x: args.x,
                y: args.y,
            },
        ),
        InteractCommand::LongPress(args) => (
            &args.target,
            InteractionRequest::LongPress {
                target_id: args.target_id.clone(),
                x: args.x,
                y: args.y,
            },
        ),
        InteractCommand::Swipe(args) => (
            &args.target,
            InteractionRequest::Swipe {
                x: args.x,
                y: args.y,
            },
        ),
        InteractCommand::Drag(args) => (
            &args.target,
            InteractionRequest::Drag {
                x: args.x,
                y: args.y,
            },
        ),
        InteractCommand::TypeText(args) => (
            &args.target,
            InteractionRequest::TypeText {
                target_id: args.target_id.clone(),
                text: args.text.clone(),
            },
        ),
    };
    let manifest = load_manifest(&repo_root, &target.target)?;
    let result = interact(
        &repo_root,
        &manifest,
        &deploy_registry,
        target.platform.as_backend_id(),
        &target.destination,
        request,
        runner,
    )?;
    json_output(&result)
}

fn execute_evaluate(
    cwd: &Utf8Path,
    args: &EvaluateArgs,
    runner: &mut impl ToolRunner,
) -> AtomResult<CommandOutput> {
    let repo_root = resolve_workspace_root(cwd)?;
    let deploy_registry = first_party_deploy_backend_registry()?;
    match &args.command {
        EvaluateCommand::Run(args) => {
            let manifest = load_manifest(&repo_root, &args.target.target)?;
            let plan = resolve_cli_path(&repo_root, &args.plan);
            let artifacts_dir = resolve_cli_path(&repo_root, &args.artifacts_dir);
            let result = evaluate_run(
                &repo_root,
                &manifest,
                &deploy_registry,
                args.target.platform.as_backend_id(),
                &args.target.destination,
                &plan,
                &artifacts_dir,
                runner,
            )?;
            json_output(&result.manifest)
        }
    }
}

fn json_output<T: Serialize>(value: &T) -> AtomResult<CommandOutput> {
    let stdout = serde_json::to_vec_pretty(value).map_err(|error| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            format!("failed to encode JSON output: {error}"),
        )
    })?;
    Ok(success_output(stdout))
}

fn text_output(value: String) -> CommandOutput {
    success_output(value.into_bytes())
}

fn success_output(stdout: Vec<u8>) -> CommandOutput {
    CommandOutput {
        stdout,
        stderr: Vec::new(),
        exit_code: 0,
    }
}

fn default_config_plugin_registry() -> ConfigPluginRegistry {
    let mut registry = ConfigPluginRegistry::new();
    atom_cng_app_icon::register(&mut registry);
    registry
}

fn first_party_deploy_backend_registry() -> AtomResult<DeployBackendRegistry> {
    let mut registry = DeployBackendRegistry::new();
    register_ios_deploy_backend(&mut registry)?;
    register_android_deploy_backend(&mut registry)?;
    Ok(registry)
}

fn first_party_generation_backend_registry() -> AtomResult<GenerationBackendRegistry> {
    let mut registry = GenerationBackendRegistry::new();
    register_ios_generation_backend(&mut registry)?;
    register_android_generation_backend(&mut registry)?;
    Ok(registry)
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

fn resolve_cli_path(repo_root: &Utf8Path, path: &Utf8Path) -> Utf8PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        repo_root.join(path)
    }
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
    use std::fs;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use atom_backends::{
        BackendAutomationSession, BackendDefinition, DeployBackend, DeployBackendRegistry,
        LaunchMode, SessionLaunchBehavior, ToolRunner,
    };
    use atom_manifest::{NormalizedManifest, testing::fixture_manifest};
    use camino::{Utf8Path, Utf8PathBuf};
    use clap::Parser;
    use tempfile::tempdir;

    use super::{
        Cli, find_workspace_root, preflight_run_backend, resolve_cli_path,
        resolve_workspace_root_with_workspace_dir, run_from_args,
    };

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
        ) -> atom_ffi::AtomResult<Vec<atom_backends::DestinationDescriptor>> {
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

        fn new_automation_session<'a>(
            &self,
            _repo_root: &'a Utf8Path,
            _manifest: &'a NormalizedManifest,
            _destination_id: &'a str,
            _runner: &'a mut dyn ToolRunner,
            _launch_behavior: SessionLaunchBehavior,
        ) -> atom_ffi::AtomResult<Box<dyn BackendAutomationSession + 'a>> {
            unreachable!("CLI tests do not construct automation sessions")
        }
    }

    fn parse_cli(args: &[&str]) {
        Cli::try_parse_from(args).expect("clap should accept the command");
    }

    fn runnable_manifest(root: &Utf8Path) -> NormalizedManifest {
        fixture_manifest(root)
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
    fn run_command_accepts_destination_alias() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        fs::write(root.join("MODULE.bazel"), "module(name = \"atom\")\n").expect("workspace");

        let error = run_from_args(
            [
                "atom",
                "run",
                "--platform",
                "ios",
                "--target",
                "//examples/hello-world/apps/hello_atom:hello_atom",
                "--destination",
                "SIM-123",
            ],
            &root,
        )
        .expect_err("missing manifest should fail after clap accepts the command");

        assert_ne!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
    }

    #[test]
    fn run_command_accepts_detach_flag() {
        parse_cli(&[
            "atom",
            "run",
            "--platform",
            "android",
            "--target",
            "//examples/hello-world/apps/hello_atom:hello_atom",
            "--detach",
        ]);
    }

    #[test]
    fn doctor_command_accepts_json_flag() {
        parse_cli(&["atom", "doctor", "--json"]);
    }

    #[test]
    fn stop_command_accepts_destination_alias() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        fs::write(root.join("MODULE.bazel"), "module(name = \"atom\")\n").expect("workspace");

        let error = run_from_args(
            [
                "atom",
                "stop",
                "--platform",
                "ios",
                "--target",
                "//examples/hello-world/apps/hello_atom:hello_atom",
                "--destination",
                "SIM-123",
            ],
            &root,
        )
        .expect_err("missing manifest should fail after clap accepts the command");

        assert_ne!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
    }

    #[test]
    fn destinations_command_accepts_json_flag() {
        parse_cli(&["atom", "destinations", "--platform", "ios", "--json"]);
    }

    #[test]
    fn devices_command_accepts_platform_and_json_flag() {
        parse_cli(&["atom", "devices", "--platform", "ios", "--json"]);
        parse_cli(&["atom", "devices", "--platform", "android", "--json"]);
    }

    #[test]
    fn disabled_run_backend_fails_before_generation() {
        let mut registry = DeployBackendRegistry::new();
        registry
            .register(Box::new(DisabledFixtureBackend))
            .expect("fixture backend should register");
        let root = Utf8PathBuf::from(".");
        let manifest = runnable_manifest(&root);
        let generated = Arc::new(AtomicBool::new(false));
        let generated_flag = Arc::clone(&generated);

        let error = preflight_run_backend(&manifest, &registry, "fixture", move || {
            generated_flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .expect_err("disabled backend should fail before generation");

        assert_eq!(error.code, atom_ffi::AtomErrorCode::ManifestInvalidValue);
        assert!(!generated.load(Ordering::SeqCst));
    }

    #[test]
    fn evidence_commands_accept_required_flags() {
        parse_cli(&[
            "atom",
            "evidence",
            "logs",
            "--platform",
            "ios",
            "--target",
            "//examples/hello-world/apps/hello_atom:hello_atom",
            "--destination",
            "SIM-123",
            "--output",
            "tmp/logs.txt",
            "--seconds",
            "10",
        ]);
        parse_cli(&[
            "atom",
            "evidence",
            "screenshot",
            "--platform",
            "ios",
            "--target",
            "//examples/hello-world/apps/hello_atom:hello_atom",
            "--destination",
            "SIM-123",
            "--output",
            "tmp/screenshot.png",
        ]);
        parse_cli(&[
            "atom",
            "evidence",
            "video",
            "--platform",
            "ios",
            "--target",
            "//examples/hello-world/apps/hello_atom:hello_atom",
            "--destination",
            "SIM-123",
            "--output",
            "tmp/video.mp4",
            "--seconds",
            "3",
        ]);
    }

    #[test]
    fn inspect_ui_accepts_output_flag() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        fs::write(root.join("MODULE.bazel"), "module(name = \"atom\")\n").expect("workspace");

        let error = run_from_args(
            [
                "atom",
                "inspect",
                "ui",
                "--platform",
                "ios",
                "--target",
                "//examples/hello-world/apps/hello_atom:hello_atom",
                "--destination",
                "SIM-123",
                "--output",
                "tmp/out.json",
            ],
            &root,
        )
        .expect_err("missing manifest should fail after clap accepts the command");

        assert_ne!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
    }

    #[test]
    fn interact_commands_accept_supported_shapes() {
        parse_cli(&[
            "atom",
            "interact",
            "tap",
            "--platform",
            "ios",
            "--target",
            "//examples/hello-world/apps/hello_atom:hello_atom",
            "--destination",
            "SIM-123",
            "--target-id",
            "atom.demo.primary_button",
        ]);
        parse_cli(&[
            "atom",
            "interact",
            "type-text",
            "--platform",
            "ios",
            "--target",
            "//examples/hello-world/apps/hello_atom:hello_atom",
            "--destination",
            "SIM-123",
            "--target-id",
            "atom.demo.input",
            "--text",
            "hello",
        ]);
    }

    #[test]
    fn evaluate_run_requires_plan_and_artifacts_dir() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let error = run_from_args(
            [
                "atom",
                "evaluate",
                "run",
                "--platform",
                "ios",
                "--target",
                "//examples/hello-world/apps/hello_atom:hello_atom",
                "--destination",
                "SIM-123",
            ],
            &root,
        )
        .expect_err("missing evaluate args should fail");
        assert_eq!(error.code, atom_ffi::AtomErrorCode::CliUsageError);
    }

    #[test]
    fn resolve_cli_path_joins_repo_relative_paths() {
        let repo_root = Utf8PathBuf::from("/tmp/atom-workspace");
        let resolved = resolve_cli_path(&repo_root, Utf8Path::new("examples/plan.json"));

        assert_eq!(resolved, repo_root.join("examples/plan.json"));
    }

    #[test]
    fn resolve_cli_path_preserves_absolute_paths() {
        let repo_root = Utf8PathBuf::from("/tmp/atom-workspace");
        let absolute = Utf8PathBuf::from("/tmp/output.json");

        assert_eq!(resolve_cli_path(&repo_root, &absolute), absolute);
    }
}
