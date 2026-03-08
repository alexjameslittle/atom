use std::ffi::OsString;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use atom_cng::{build_generation_plan, emit_host_tree, render_prebuild_plan};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::{NormalizedManifest, load_manifest};
use atom_modules::resolve_modules;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, Parser, Subcommand};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}

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

trait ToolRunner {
    fn run(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()>;
    fn capture(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<String>;
    fn capture_json_file(
        &mut self,
        repo_root: &Utf8Path,
        tool: &str,
        args: &[String],
    ) -> AtomResult<String>;
}

struct ProcessRunner;

#[derive(Debug, Clone, PartialEq, Eq)]
struct IosSimulator {
    runtime: String,
    name: String,
    udid: String,
    state: String,
    is_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum IosDestinationKind {
    Simulator,
    Device,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IosDestination {
    kind: IosDestinationKind,
    id: String,
    alternate_id: Option<String>,
    name: String,
    state: String,
    runtime: Option<String>,
    is_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AndroidDestination {
    serial: String,
    state: String,
    model: Option<String>,
    device_name: Option<String>,
    is_emulator: bool,
}

impl IosDestination {
    fn matches_identifier(&self, value: &str) -> bool {
        self.id == value || self.alternate_id.as_deref() == Some(value) || self.name == value
    }

    fn is_booted_simulator(&self) -> bool {
        self.kind == IosDestinationKind::Simulator && self.state == "Booted"
    }

    fn display_label(&self) -> String {
        match self.kind {
            IosDestinationKind::Simulator => match &self.runtime {
                Some(runtime) => format!("Simulator: {} [{}; {}]", self.name, runtime, self.state),
                None => format!("Simulator: {} [{}]", self.name, self.state),
            },
            IosDestinationKind::Device => format!("Device: {} [{}]", self.name, self.state),
        }
    }
}

impl AndroidDestination {
    fn display_label(&self) -> String {
        let kind = if self.is_emulator {
            "Emulator"
        } else {
            "Device"
        };
        let model = self.model.as_deref().or(self.device_name.as_deref());
        match model {
            Some(model) => format!("{kind}: {model} [{}]", self.serial),
            None => format!("{kind}: {}", self.serial),
        }
    }
}

impl ToolRunner for ProcessRunner {
    fn run(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()> {
        let status = Command::new(tool)
            .args(args)
            .current_dir(repo_root)
            .status()
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to invoke {tool}: {error}"),
                )
            })?;

        if status.success() {
            Ok(())
        } else {
            Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("{tool} {} exited with status {status}", args.join(" ")),
            ))
        }
    }

    fn capture(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<String> {
        let output = Command::new(tool)
            .args(args)
            .current_dir(repo_root)
            .output()
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to invoke {tool}: {error}"),
                )
            })?;

        if !output.status.success() {
            return Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!(
                    "{tool} {} exited with status {}",
                    args.join(" "),
                    output.status
                ),
            ));
        }

        String::from_utf8(output.stdout).map_err(|_| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("{tool} returned non-UTF-8 output"),
            )
        })
    }

    fn capture_json_file(
        &mut self,
        repo_root: &Utf8Path,
        tool: &str,
        args: &[String],
    ) -> AtomResult<String> {
        let path = temp_json_output_path(tool);
        let status = Command::new(tool)
            .args(args)
            .arg("--json-output")
            .arg(&path)
            .current_dir(repo_root)
            .status()
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to invoke {tool}: {error}"),
                )
            })?;

        if !status.success() {
            let _ = fs::remove_file(&path);
            return Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("{tool} {} exited with status {status}", args.join(" ")),
            ));
        }

        let contents = fs::read_to_string(&path).map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to read {tool} JSON output: {error}"),
            )
        })?;
        let _ = fs::remove_file(&path);
        Ok(contents)
    }
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

fn deploy_ios(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_device: Option<&str>,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let destination = resolve_ios_destination(repo_root, runner, requested_device)?;
    let target = generated_target(manifest, "ios");
    let build_args = ios_bazel_args(&target, destination.kind);
    run_bazel_owned(runner, repo_root, &build_args)?;

    let app_bundle = find_bazel_output_owned(
        runner,
        repo_root,
        &build_args,
        &target,
        &[".app", ".ipa"],
        "iOS app artifact",
    )?;
    let installable_app = resolve_ios_installable_artifact(&app_bundle)?;
    let bundle_id = manifest.ios.bundle_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated iOS manifest is missing bundle_id",
        )
    })?;

    match destination.kind {
        IosDestinationKind::Simulator => {
            let simulator = prepare_ios_simulator(repo_root, runner, &destination)?;
            run_tool(
                runner,
                repo_root,
                "xcrun",
                &["simctl", "install", &simulator, installable_app.as_str()],
            )?;
            run_tool(
                runner,
                repo_root,
                "xcrun",
                &["simctl", "launch", &simulator, bundle_id],
            )?;
        }
        IosDestinationKind::Device => {
            run_tool(
                runner,
                repo_root,
                "xcrun",
                &[
                    "devicectl",
                    "device",
                    "install",
                    "app",
                    "--device",
                    &destination.id,
                    installable_app.as_str(),
                ],
            )?;
            run_tool(
                runner,
                repo_root,
                "xcrun",
                &[
                    "devicectl",
                    "device",
                    "process",
                    "launch",
                    "--device",
                    &destination.id,
                    bundle_id,
                ],
            )?;
        }
    }
    Ok(())
}

fn deploy_android(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_device: Option<&str>,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let target = generated_target(manifest, "android");
    run_bazel(runner, repo_root, &["build", &target])?;

    let apk = find_bazel_output(runner, repo_root, &target, &["app.apk", ".apk"], "APK")?;
    let application_id = manifest.android.application_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated Android manifest is missing application_id",
        )
    })?;

    let selected_serial = resolve_android_device(repo_root, runner, requested_device)?;
    let component = format!("{application_id}/.MainActivity");
    if let Some(serial) = selected_serial.as_deref() {
        run_tool(
            runner,
            repo_root,
            "adb",
            &["-s", serial, "install", "-r", apk.as_str()],
        )?;
        run_tool(
            runner,
            repo_root,
            "adb",
            &["-s", serial, "shell", "am", "start", "-n", &component],
        )?;
    } else {
        run_tool(runner, repo_root, "adb", &["install", "-r", apk.as_str()])?;
        run_tool(
            runner,
            repo_root,
            "adb",
            &["shell", "am", "start", "-n", &component],
        )?;
    }
    Ok(())
}

fn generated_target(manifest: &NormalizedManifest, platform: &str) -> String {
    format!(
        "//{}/{}/{}:app",
        manifest.build.generated_root.as_str(),
        platform,
        manifest.app.slug
    )
}

fn resolve_ios_destination(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    requested_device: Option<&str>,
) -> AtomResult<IosDestination> {
    if let Some(requested_device) = requested_device {
        return resolve_requested_ios_destination(repo_root, runner, requested_device);
    }

    let simulators = list_ios_simulators(repo_root, runner)?;
    if should_prompt_interactively() {
        let mut destinations = simulators;
        destinations.extend(list_ios_physical_devices(repo_root, runner).unwrap_or_default());
        sort_ios_destinations(&mut destinations);
        return choose_from_menu(
            "Select iOS destination",
            &destinations,
            IosDestination::display_label,
        );
    }

    select_default_ios_destination(&simulators).ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            "xcrun simctl did not report an available simulator",
        )
    })
}

fn resolve_requested_ios_destination(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    requested_device: &str,
) -> AtomResult<IosDestination> {
    let simulators = list_ios_simulators(repo_root, runner)?;
    if requested_device == "booted" {
        return select_booted_ios_destination(&simulators)
            .or_else(|| select_default_ios_destination(&simulators))
            .ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    "xcrun simctl did not report a bootable simulator",
                )
            });
    }

    if let Some(simulator) = simulators
        .into_iter()
        .find(|simulator| simulator.matches_identifier(requested_device))
    {
        return Ok(simulator);
    }

    Ok(IosDestination {
        kind: IosDestinationKind::Device,
        id: requested_device.to_owned(),
        alternate_id: None,
        name: requested_device.to_owned(),
        state: "requested".to_owned(),
        runtime: None,
        is_available: true,
    })
}

fn list_ios_simulators(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<IosDestination>> {
    Ok(parse_ios_simulators(&capture_tool(
        runner,
        repo_root,
        "xcrun",
        &["simctl", "list", "devices", "available", "-j"],
    )?)?
    .into_iter()
    .filter(|simulator| simulator.is_available)
    .map(|simulator| IosDestination {
        kind: IosDestinationKind::Simulator,
        id: simulator.udid.clone(),
        alternate_id: None,
        name: simulator.name,
        state: simulator.state,
        runtime: Some(simulator.runtime),
        is_available: simulator.is_available,
    })
    .collect())
}

fn list_ios_physical_devices(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<IosDestination>> {
    parse_ios_physical_devices(&capture_json_tool(
        runner,
        repo_root,
        "xcrun",
        &["devicectl", "list", "devices"],
    )?)
}

fn select_booted_ios_destination(destinations: &[IosDestination]) -> Option<IosDestination> {
    destinations
        .iter()
        .find(|destination| destination.is_booted_simulator())
        .cloned()
}

fn select_default_ios_destination(destinations: &[IosDestination]) -> Option<IosDestination> {
    let mut simulators = destinations
        .iter()
        .filter(|destination| destination.kind == IosDestinationKind::Simulator)
        .cloned()
        .collect::<Vec<_>>();
    simulators.sort_by(|left, right| {
        right
            .runtime
            .cmp(&left.runtime)
            .then_with(|| right.is_booted_simulator().cmp(&left.is_booted_simulator()))
            .then_with(|| left.name.cmp(&right.name))
    });

    select_booted_ios_destination(&simulators)
        .or_else(|| {
            simulators
                .iter()
                .find(|simulator| simulator.name.contains("iPhone"))
                .cloned()
        })
        .or_else(|| simulators.first().cloned())
}

fn sort_ios_destinations(destinations: &mut [IosDestination]) {
    destinations.sort_by(|left, right| {
        right
            .is_available
            .cmp(&left.is_available)
            .then_with(|| right.is_booted_simulator().cmp(&left.is_booted_simulator()))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| right.runtime.cmp(&left.runtime))
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn prepare_ios_simulator(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    destination: &IosDestination,
) -> AtomResult<String> {
    let simulator = destination.id.clone();
    if !destination.is_booted_simulator() {
        run_tool(runner, repo_root, "xcrun", &["simctl", "boot", &simulator])?;
        run_tool(
            runner,
            repo_root,
            "xcrun",
            &["simctl", "bootstatus", &simulator, "-b"],
        )?;
    }
    Ok(simulator)
}

fn resolve_android_device(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    requested_device: Option<&str>,
) -> AtomResult<Option<String>> {
    if let Some(requested_device) = requested_device {
        return Ok(Some(requested_device.to_owned()));
    }

    if !should_prompt_interactively() {
        return Ok(None);
    }

    let destinations = list_android_devices(repo_root, runner)?;
    if destinations.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            "adb did not report any connected emulators or devices",
        ));
    }
    choose_from_menu(
        "Select Android destination",
        &destinations,
        AndroidDestination::display_label,
    )
    .map(|destination| Some(destination.serial))
}

fn list_android_devices(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<AndroidDestination>> {
    Ok(
        parse_android_devices(&capture_tool(runner, repo_root, "adb", &["devices", "-l"])?)
            .into_iter()
            .filter(|destination| destination.state == "device")
            .collect(),
    )
}

fn choose_from_menu<T, F>(title: &str, options: &[T], render: F) -> AtomResult<T>
where
    T: Clone,
    F: Fn(&T) -> String,
{
    if options.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("{title} could not find any choices"),
        ));
    }

    if options.len() == 1 {
        return Ok(options[0].clone());
    }

    let mut stdout = io::stdout();
    writeln!(stdout, "{title}:").map_err(|error| io_error_to_cli_error(&error))?;
    for (index, option) in options.iter().enumerate() {
        writeln!(stdout, "  {}. {}", index + 1, render(option))
            .map_err(|error| io_error_to_cli_error(&error))?;
    }
    loop {
        write!(stdout, "Enter selection [1-{}]: ", options.len())
            .map_err(|error| io_error_to_cli_error(&error))?;
        stdout
            .flush()
            .map_err(|error| io_error_to_cli_error(&error))?;

        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .map_err(|error| io_error_to_cli_error(&error))?;
        let trimmed = line.trim();
        if let Ok(selection) = trimmed.parse::<usize>()
            && (1..=options.len()).contains(&selection)
        {
            return Ok(options[selection - 1].clone());
        }
        writeln!(stdout, "Invalid selection.").map_err(|error| io_error_to_cli_error(&error))?;
    }
}

fn should_prompt_interactively() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn io_error_to_cli_error(error: &io::Error) -> AtomError {
    AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        format!("interactive device selection failed: {error}"),
    )
}

fn parse_ios_simulators(json: &str) -> AtomResult<Vec<IosSimulator>> {
    let parsed: Value = serde_json::from_str(json).map_err(|error| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to parse xcrun simctl JSON: {error}"),
        )
    })?;
    let devices = parsed
        .get("devices")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "xcrun simctl JSON did not contain a devices map",
            )
        })?;

    let mut simulators = Vec::new();
    for (runtime, entries) in devices {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            let Some(name) = entry.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(udid) = entry.get("udid").and_then(Value::as_str) else {
                continue;
            };
            simulators.push(IosSimulator {
                runtime: runtime.clone(),
                name: name.to_owned(),
                udid: udid.to_owned(),
                state: entry
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                is_available: entry
                    .get("isAvailable")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            });
        }
    }
    Ok(simulators)
}

fn parse_ios_physical_devices(json: &str) -> AtomResult<Vec<IosDestination>> {
    let parsed: Value = serde_json::from_str(json).map_err(|error| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to parse xcrun devicectl JSON: {error}"),
        )
    })?;
    let devices = parsed
        .get("result")
        .and_then(|result| result.get("devices"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "xcrun devicectl JSON did not contain a devices array",
            )
        })?;

    let mut destinations = Vec::new();
    for device in devices {
        let platform = device
            .get("hardwareProperties")
            .and_then(|value| value.get("platform"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if platform != "iOS" {
            continue;
        }

        let identifier = device.get("identifier").and_then(Value::as_str);
        let udid = device
            .get("hardwareProperties")
            .and_then(|value| value.get("udid"))
            .and_then(Value::as_str);
        let name = device
            .get("deviceProperties")
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str);
        let ddi_available = device
            .get("deviceProperties")
            .and_then(|value| value.get("ddiServicesAvailable"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let tunnel_state = device
            .get("connectionProperties")
            .and_then(|value| value.get("tunnelState"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        let Some(primary_id) = udid.or(identifier) else {
            continue;
        };
        let Some(name) = name else {
            continue;
        };
        let is_available = ddi_available || tunnel_state == "connected";
        if !is_available {
            continue;
        }

        destinations.push(IosDestination {
            kind: IosDestinationKind::Device,
            id: primary_id.to_owned(),
            alternate_id: identifier
                .filter(|identifier| *identifier != primary_id)
                .map(str::to_owned),
            name: name.to_owned(),
            state: if ddi_available {
                "ready".to_owned()
            } else {
                tunnel_state.to_owned()
            },
            runtime: None,
            is_available,
        });
    }
    Ok(destinations)
}

fn parse_android_devices(output: &str) -> Vec<AndroidDestination> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("List of devices attached"))
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?;
            let state = parts.next()?;
            let mut model = None;
            let mut device_name = None;
            for part in parts {
                if let Some(value) = part.strip_prefix("model:") {
                    model = Some(value.replace('_', " "));
                }
                if let Some(value) = part.strip_prefix("device:") {
                    device_name = Some(value.replace('_', " "));
                }
            }
            Some(AndroidDestination {
                serial: serial.to_owned(),
                state: state.to_owned(),
                model,
                device_name,
                is_emulator: serial.starts_with("emulator-"),
            })
        })
        .collect()
}

fn find_bazel_output(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    target: &str,
    suffixes: &[&str],
    artifact_name: &str,
) -> AtomResult<Utf8PathBuf> {
    let output = capture_bazel(runner, repo_root, &["cquery", target, "--output=files"])?;
    select_bazel_output_path(repo_root, &output, suffixes, artifact_name, target)
}

fn find_bazel_output_owned(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    build_args: &[String],
    target: &str,
    suffixes: &[&str],
    artifact_name: &str,
) -> AtomResult<Utf8PathBuf> {
    let mut args = build_args.to_vec();
    "cquery".clone_into(&mut args[0]);
    args.push("--output=files".to_owned());
    let output = capture_bazel_owned(runner, repo_root, &args)?;
    select_bazel_output_path(repo_root, &output, suffixes, artifact_name, target)
}

fn select_bazel_output_path(
    repo_root: &Utf8Path,
    output: &str,
    suffixes: &[&str],
    artifact_name: &str,
    target: &str,
) -> AtomResult<Utf8PathBuf> {
    let selected = select_bazel_output(output, suffixes).ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("bazelisk cquery did not return a {artifact_name} for {target}"),
        )
    })?;

    if selected.starts_with('/') {
        Utf8PathBuf::from_path_buf(selected.into()).map_err(|_| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("bazelisk returned a non-UTF-8 {artifact_name} path"),
            )
        })
    } else {
        Ok(repo_root.join(selected))
    }
}

fn select_bazel_output<'a>(output: &'a str, suffixes: &[&str]) -> Option<&'a str> {
    let lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    for suffix in suffixes {
        if let Some(line) = lines.iter().copied().find(|line| line.ends_with(suffix)) {
            return Some(line);
        }
    }
    None
}

fn resolve_ios_installable_artifact(path: &Utf8Path) -> AtomResult<Utf8PathBuf> {
    if path.extension() == Some("app") {
        return Ok(path.to_owned());
    }
    if path.extension() != Some("ipa") {
        return Err(AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "bazelisk did not produce an installable iOS artifact",
            path.as_str(),
        ));
    }

    find_descendant_with_suffix(
        path.parent().ok_or_else(|| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                "bazelisk returned an invalid iOS artifact path",
                path.as_str(),
            )
        })?,
        ".app",
    )?
    .ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "could not locate an unpacked .app bundle next to the built .ipa",
            path.as_str(),
        )
    })
}

fn find_descendant_with_suffix(root: &Utf8Path, suffix: &str) -> AtomResult<Option<Utf8PathBuf>> {
    for entry in fs::read_dir(root).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to inspect generated iOS outputs: {error}"),
            root.as_str(),
        )
    })? {
        let entry = entry.map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to inspect generated iOS outputs: {error}"),
                root.as_str(),
            )
        })?;
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|_| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                "generated iOS output path was not valid UTF-8",
                root.as_str(),
            )
        })?;
        if path.as_str().ends_with(suffix) {
            return Ok(Some(path));
        }
        if path.is_dir()
            && let Some(found) = find_descendant_with_suffix(&path, suffix)?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn ios_bazel_args(target: &str, destination: IosDestinationKind) -> Vec<String> {
    let cpu = match destination {
        IosDestinationKind::Simulator => "sim_arm64",
        IosDestinationKind::Device => "arm64",
    };
    vec![
        "build".to_owned(),
        target.to_owned(),
        format!("--ios_multi_cpus={cpu}"),
    ]
}

fn run_bazel(runner: &mut impl ToolRunner, repo_root: &Utf8Path, args: &[&str]) -> AtomResult<()> {
    run_tool(runner, repo_root, "bazelisk", args)
}

fn run_bazel_owned(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    args: &[String],
) -> AtomResult<()> {
    runner.run(repo_root, "bazelisk", args)
}

fn capture_bazel(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    args: &[&str],
) -> AtomResult<String> {
    capture_tool(runner, repo_root, "bazelisk", args)
}

fn capture_bazel_owned(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    args: &[String],
) -> AtomResult<String> {
    runner.capture(repo_root, "bazelisk", args)
}

fn run_tool(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    tool: &str,
    args: &[&str],
) -> AtomResult<()> {
    runner.run(
        repo_root,
        tool,
        &args
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
    )
}

fn capture_json_tool(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    tool: &str,
    args: &[&str],
) -> AtomResult<String> {
    runner.capture_json_file(
        repo_root,
        tool,
        &args
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
    )
}

fn capture_tool(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    tool: &str,
    args: &[&str],
) -> AtomResult<String> {
    runner.capture(
        repo_root,
        tool,
        &args
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
    )
}

fn temp_json_output_path(tool: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("atom-{tool}-{timestamp}.json"))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;

    use atom_manifest::{AndroidConfig, AppConfig, BuildConfig, IosConfig, NormalizedManifest};
    use camino::{Utf8Path, Utf8PathBuf};
    use tempfile::tempdir;

    use super::{
        AndroidDestination, IosDestination, IosDestinationKind, ToolRunner, deploy_android,
        deploy_ios, find_workspace_root, resolve_workspace_root_with_workspace_dir, run_from_args,
        select_default_ios_destination,
    };

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
