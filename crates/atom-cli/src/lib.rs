mod doctor;
mod new_project;
mod templates;

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

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
use doctor::{DoctorArgs, execute as execute_doctor};
use new_project::scaffold_project;
use serde::Serialize;

const ATOM_FRAMEWORK_VERSION: &str = env!("ATOM_FRAMEWORK_VERSION");
const ATOM_RUST_VERSION: &str = env!("ATOM_RUST_VERSION");
const ATOM_BUILD_BAZEL_VERSION: &str = env!("ATOM_BUILD_BAZEL_VERSION");
const ATOM_MISE_BAZELISK_VERSION: &str = env!("ATOM_MISE_BAZELISK_VERSION");
const ATOM_MISE_RUST_TOOLCHAIN_VERSION: &str = env!("ATOM_MISE_RUST_TOOLCHAIN_VERSION");
const ATOM_MISE_JAVA_VERSION: &str = env!("ATOM_MISE_JAVA_VERSION");
const ATOM_RULES_RUST_VERSION: &str = env!("ATOM_RULES_RUST_VERSION");
const ATOM_APPLE_SUPPORT_VERSION: &str = env!("ATOM_APPLE_SUPPORT_VERSION");
const ATOM_RULES_JAVA_VERSION: &str = env!("ATOM_RULES_JAVA_VERSION");
const ATOM_RULES_KOTLIN_VERSION: &str = env!("ATOM_RULES_KOTLIN_VERSION");
const ATOM_RULES_ANDROID_VERSION: &str = env!("ATOM_RULES_ANDROID_VERSION");
const ATOM_RULES_ANDROID_NDK_VERSION: &str = env!("ATOM_RULES_ANDROID_NDK_VERSION");
const ATOM_RULES_APPLE_VERSION: &str = env!("ATOM_RULES_APPLE_VERSION");
const ATOM_RULES_SWIFT_VERSION: &str = env!("ATOM_RULES_SWIFT_VERSION");
const ATOM_JAVA_RUNTIME_VERSION: &str = env!("ATOM_JAVA_RUNTIME_VERSION");

#[derive(Debug, Parser)]
#[command(name = "atom", disable_version_flag = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    New(NewArgs),
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
struct NewArgs {
    name: String,
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
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    if is_version_request(&args) {
        return Ok(version_output(cwd));
    }

    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => {
            return match error.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                    Ok(CommandOutput {
                        stdout: error.to_string().into_bytes(),
                        stderr: Vec::new(),
                        exit_code: 0,
                    })
                }
                _ => Err(AtomError::new(
                    AtomErrorCode::CliUsageError,
                    error.to_string(),
                )),
            };
        }
    };
    let mut runner = ProcessRunner;
    execute(&cli, cwd, &mut runner)
}

fn is_version_request(args: &[OsString]) -> bool {
    args.len() == 2
        && matches!(
            args[1].as_os_str(),
            flag if flag == OsStr::new("--version") || flag == OsStr::new("-V")
        )
}

fn version_output(cwd: &Utf8Path) -> CommandOutput {
    let bazel_version = resolve_bazel_version_with(cwd, detect_runtime_bazel_version);
    text_output(format!(
        "atom {ATOM_FRAMEWORK_VERSION}\nrust {ATOM_RUST_VERSION}\nbazel {bazel_version}\n"
    ))
}

fn resolve_bazel_version_with(
    cwd: &Utf8Path,
    detect_bazel_version: impl FnOnce() -> Option<String>,
) -> String {
    let command_root = resolve_command_root(cwd, workspace_directory().as_deref());
    read_bazel_version_file(&command_root)
        .or_else(detect_bazel_version)
        .unwrap_or_else(|| ATOM_BUILD_BAZEL_VERSION.to_owned())
}

fn read_bazel_version_file(start: &Utf8Path) -> Option<String> {
    start.ancestors().find_map(|candidate| {
        let bazelversion_path = candidate.join(".bazelversion");
        let contents = fs::read_to_string(&bazelversion_path).ok()?;
        contents
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_owned)
    })
}

fn detect_runtime_bazel_version() -> Option<String> {
    let output = Command::new("bazelisk").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    extract_version_token(&stdout)
        .or_else(|| extract_version_token(&stderr))
        .map(str::to_owned)
}

fn extract_version_token(output: &str) -> Option<&str> {
    output.split_whitespace().find(|token| {
        token
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
    })
}

fn execute(cli: &Cli, cwd: &Utf8Path, runner: &mut impl ToolRunner) -> AtomResult<CommandOutput> {
    match &cli.command {
        Commands::New(args) => execute_new(cwd, args),
        Commands::Doctor(args) => {
            let repo_root = resolve_workspace_root(cwd)?;
            execute_doctor(&repo_root, args, &first_party_deploy_backend_registry()?)
        }
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

fn execute_new(cwd: &Utf8Path, args: &NewArgs) -> AtomResult<CommandOutput> {
    let project_root = scaffold_project(cwd, &args.name)?;
    Ok(text_output(format!("{project_root}\n")))
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
        ATOM_BUILD_BAZEL_VERSION, ATOM_FRAMEWORK_VERSION, ATOM_RUST_VERSION, Cli,
        find_workspace_root, preflight_run_backend, resolve_bazel_version_with, resolve_cli_path,
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
    fn help_flag_returns_success_output() {
        let directory = tempdir().expect("tempdir");
        let cwd = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let output = run_from_args(["atom", "--help"], &cwd).expect("help should succeed");

        assert_eq!(output.exit_code, 0);
        assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: atom"));
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn doctor_command_accepts_json_flag() {
        parse_cli(&["atom", "doctor", "--json"]);
    }

    #[test]
    fn version_flag_succeeds_without_a_workspace() {
        let directory = tempdir().expect("tempdir");
        let cwd = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        fs::write(cwd.join(".bazelversion"), "8.4.2\n").expect("bazelversion");

        let output = run_from_args(["atom", "--version"], &cwd).expect("version output");
        let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");

        assert_eq!(output.exit_code, 0);
        assert_eq!(
            stdout,
            format!("atom {ATOM_FRAMEWORK_VERSION}\nrust {ATOM_RUST_VERSION}\nbazel 8.4.2\n")
        );
    }

    #[test]
    fn version_flag_prefers_the_nearest_bazelversion_file() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let inner = root.join("nested/project");
        fs::create_dir_all(&inner).expect("inner directory");
        fs::write(root.join(".bazelversion"), "7.1.0\n").expect("outer bazelversion");
        fs::write(inner.join(".bazelversion"), "8.4.2\n").expect("inner bazelversion");

        let detected = resolve_bazel_version_with(&inner, || Some(String::from("9.0.0")));

        assert_eq!(detected, "8.4.2");
    }

    #[test]
    fn version_flag_falls_back_to_runtime_detection_when_bazelversion_is_missing() {
        let directory = tempdir().expect("tempdir");
        let cwd = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

        let detected = resolve_bazel_version_with(&cwd, || Some(String::from("8.9.0")));

        assert_eq!(detected, "8.9.0");
    }

    #[test]
    fn version_flag_falls_back_to_build_bazel_version_when_runtime_detection_is_unavailable() {
        let directory = tempdir().expect("tempdir");
        let cwd = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");

        let detected = resolve_bazel_version_with(&cwd, || None);

        assert_eq!(detected, ATOM_BUILD_BAZEL_VERSION);
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
