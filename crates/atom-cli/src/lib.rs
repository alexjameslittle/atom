use std::ffi::OsString;
use std::path::PathBuf;

use atom_cng::{ConfigPluginRegistry, build_generation_plan, emit_host_tree, render_prebuild_plan};
pub use atom_deploy::CommandOutput;
use atom_deploy::progress::run_step;
use atom_deploy::{ProcessRunner, ToolRunner, deploy_android, deploy_ios, run_bazel};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::load_manifest;
use atom_modules::resolve_modules;
use camino::{Utf8Path, Utf8PathBuf};
use clap::{Args, Parser, Subcommand};

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
    let plan = build_generation_plan(&manifest, &modules, &default_config_plugin_registry())?;

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

    run_step(
        "Generating build files...",
        "Build files generated",
        "Code generation failed",
        || {
            let modules = resolve_modules(&repo_root, &manifest.modules)?;
            let plan = build_generation_plan(&manifest, &modules, &default_config_plugin_registry())?;
            emit_host_tree(&repo_root, &plan)
        },
    )?;

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
    run_step("Running tests...", "Tests passed", "Tests failed", || {
        run_bazel(runner, &repo_root, &["test", "//..."])
    })?;
    Ok(CommandOutput {
        stdout: Vec::new(),
        stderr: Vec::new(),
        exit_code: 0,
    })
}

fn default_config_plugin_registry() -> ConfigPluginRegistry {
    let mut registry = ConfigPluginRegistry::new();
    atom_cng_app_icon::register(&mut registry);
    registry
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
    use std::fs;

    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    use super::{find_workspace_root, resolve_workspace_root_with_workspace_dir, run_from_args};

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
}
