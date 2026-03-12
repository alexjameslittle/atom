use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use atom_backends::{ToolCommandOutput, ToolRunner};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}

pub struct ProcessRunner;

impl ToolRunner for ProcessRunner {
    fn run(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()> {
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

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = if stderr.trim().is_empty() {
                format!(
                    "{tool} {} exited with status {}",
                    args.join(" "),
                    output.status
                )
            } else {
                format!(
                    "{tool} {} exited with status {}:\n{stderr}",
                    args.join(" "),
                    output.status
                )
            };
            Err(AtomError::new(AtomErrorCode::ExternalToolFailed, detail))
        }
    }

    fn capture_output(
        &mut self,
        repo_root: &Utf8Path,
        tool: &str,
        args: &[String],
    ) -> AtomResult<ToolCommandOutput> {
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

        Ok(ToolCommandOutput {
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    fn capture(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<String> {
        let output = self.capture_output(repo_root, tool, args)?;

        if output.exit_code != 0 {
            return Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!(
                    "{tool} {} exited with status {}",
                    args.join(" "),
                    output.exit_code
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

    fn stream(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()> {
        let status = Command::new(tool)
            .args(args)
            .current_dir(repo_root)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
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
                format!("{tool} {} exited with status {status}", args.join(" "),),
            ))
        }
    }

    fn capture_json_file(
        &mut self,
        repo_root: &Utf8Path,
        tool: &str,
        args: &[String],
    ) -> AtomResult<String> {
        let path = temp_json_output_path(tool);
        let output = Command::new(tool)
            .args(args)
            .arg("--json-output")
            .arg(&path)
            .current_dir(repo_root)
            .output()
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to invoke {tool}: {error}"),
                )
            })?;

        if !output.status.success() {
            let _ = fs::remove_file(&path);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = if stderr.trim().is_empty() {
                format!(
                    "{tool} {} exited with status {}",
                    args.join(" "),
                    output.status
                )
            } else {
                format!(
                    "{tool} {} exited with status {}:\n{stderr}",
                    args.join(" "),
                    output.status
                )
            };
            return Err(AtomError::new(AtomErrorCode::ExternalToolFailed, detail));
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
/// Returns an error if the tool invocation fails.
pub fn run_tool(
    runner: &mut (impl ToolRunner + ?Sized),
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

/// # Errors
///
/// Returns an error if the tool invocation fails.
pub fn stream_tool(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    tool: &str,
    args: &[&str],
) -> AtomResult<()> {
    runner.stream(
        repo_root,
        tool,
        &args
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
    )
}

/// # Errors
///
/// Returns an error if the tool invocation fails.
pub fn capture_tool(
    runner: &mut (impl ToolRunner + ?Sized),
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

/// # Errors
///
/// Returns an error if the tool invocation fails.
pub fn capture_json_tool(
    runner: &mut (impl ToolRunner + ?Sized),
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

/// # Errors
///
/// Returns an error if bazelisk fails.
pub fn run_bazel(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    args: &[&str],
) -> AtomResult<()> {
    run_tool(runner, repo_root, "bazelisk", args)
}

/// # Errors
///
/// Returns an error if bazelisk fails.
pub fn run_bazel_owned(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    args: &[String],
) -> AtomResult<()> {
    runner.run(repo_root, "bazelisk", args)
}

/// # Errors
///
/// Returns an error if bazelisk fails.
pub fn capture_bazel(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    args: &[&str],
) -> AtomResult<String> {
    capture_tool(runner, repo_root, "bazelisk", args)
}

/// # Errors
///
/// Returns an error if bazelisk fails.
pub fn capture_bazel_owned(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    args: &[String],
) -> AtomResult<String> {
    runner.capture(repo_root, "bazelisk", args)
}

/// # Errors
///
/// Returns an error if bazelisk cquery fails or no matching artifact is found.
pub fn find_bazel_output(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    target: &str,
    suffixes: &[&str],
    artifact_name: &str,
) -> AtomResult<Utf8PathBuf> {
    let output = capture_bazel(runner, repo_root, &["cquery", target, "--output=files"])?;
    select_bazel_output_path(repo_root, &output, suffixes, artifact_name, target)
}

/// # Errors
///
/// Returns an error if bazelisk cquery fails or no matching artifact is found.
pub fn find_bazel_output_owned(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    build_args: &[String],
    target: &str,
    suffixes: &[&str],
    artifact_name: &str,
) -> AtomResult<Utf8PathBuf> {
    let output = capture_bazel_outputs_owned(runner, repo_root, build_args, target)?;
    select_bazel_output_path(repo_root, &output, suffixes, artifact_name, target)
}

/// # Errors
///
/// Returns an error if bazelisk cquery fails or any returned path is not valid UTF-8.
pub fn list_bazel_outputs_owned(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    build_args: &[String],
    target: &str,
) -> AtomResult<Vec<Utf8PathBuf>> {
    let output = capture_bazel_outputs_owned(runner, repo_root, build_args, target)?;
    bazel_output_paths(repo_root, &output)
}

fn capture_bazel_outputs_owned(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    build_args: &[String],
    target: &str,
) -> AtomResult<String> {
    let mut args = build_args.to_vec();
    "cquery".clone_into(&mut args[0]);
    if args.len() > 1 {
        args[1] = target.to_owned();
    } else {
        args.push(target.to_owned());
    }
    args.push("--output=files".to_owned());
    capture_bazel_owned(runner, repo_root, &args)
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

fn bazel_output_paths(repo_root: &Utf8Path, output: &str) -> AtomResult<Vec<Utf8PathBuf>> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            if line.starts_with('/') {
                Utf8PathBuf::from_path_buf(line.into()).map_err(|_| {
                    AtomError::new(
                        AtomErrorCode::ExternalToolFailed,
                        "bazelisk returned a non-UTF-8 output path",
                    )
                })
            } else {
                Ok(repo_root.join(line))
            }
        })
        .collect()
}

/// # Errors
///
/// Returns an error if the artifact path cannot be canonicalized as valid UTF-8.
pub fn bazel_source_map_prefix(path: &Utf8Path) -> AtomResult<Option<String>> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to canonicalize debug artifact path: {error}"),
            path.as_str(),
        )
    })?;
    let canonical = Utf8PathBuf::from_path_buf(canonical).map_err(|_| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "debug artifact path contained non-UTF-8 bytes",
            path.as_str(),
        )
    })?;

    let mut saw_bazel_out = false;
    let mut config = None::<String>;
    let mut saw_bin = false;
    for component in canonical.components() {
        let component = component.as_str();
        if !saw_bazel_out {
            saw_bazel_out = component == "bazel-out";
            continue;
        }
        if config.is_none() {
            config = Some(component.to_owned());
            continue;
        }
        if component == "bin" {
            saw_bin = true;
            break;
        }
    }

    Ok(match (saw_bazel_out, config, saw_bin) {
        (true, Some(config), true) => Some(format!("bazel-out/{config}/bin")),
        _ => None,
    })
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

fn temp_json_output_path(tool: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("atom-{tool}-{timestamp}.json"))
}
