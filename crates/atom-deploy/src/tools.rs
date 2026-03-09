use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}

pub trait ToolRunner {
    /// # Errors
    ///
    /// Returns an error if the tool invocation fails.
    fn run(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()>;
    /// # Errors
    ///
    /// Returns an error if the tool invocation fails.
    fn capture(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<String>;
    /// # Errors
    ///
    /// Returns an error if the tool invocation fails.
    fn capture_json_file(
        &mut self,
        repo_root: &Utf8Path,
        tool: &str,
        args: &[String],
    ) -> AtomResult<String>;
    /// Run a tool with stdout and stderr inherited from the current process, streaming
    /// output directly to the terminal. Blocks until the process exits.
    ///
    /// # Errors
    ///
    /// Returns an error if the tool invocation fails or exits with a non-zero status.
    fn stream(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()>;
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

/// # Errors
///
/// Returns an error if the tool invocation fails.
pub fn stream_tool(
    runner: &mut impl ToolRunner,
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

/// # Errors
///
/// Returns an error if the tool invocation fails.
pub fn capture_json_tool(
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

/// # Errors
///
/// Returns an error if bazelisk fails.
pub fn run_bazel(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    args: &[&str],
) -> AtomResult<()> {
    run_tool(runner, repo_root, "bazelisk", args)
}

/// # Errors
///
/// Returns an error if bazelisk fails.
pub fn run_bazel_owned(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    args: &[String],
) -> AtomResult<()> {
    runner.run(repo_root, "bazelisk", args)
}

/// # Errors
///
/// Returns an error if bazelisk fails.
pub fn capture_bazel(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    args: &[&str],
) -> AtomResult<String> {
    capture_tool(runner, repo_root, "bazelisk", args)
}

/// # Errors
///
/// Returns an error if bazelisk fails.
pub fn capture_bazel_owned(
    runner: &mut impl ToolRunner,
    repo_root: &Utf8Path,
    args: &[String],
) -> AtomResult<String> {
    runner.capture(repo_root, "bazelisk", args)
}

/// # Errors
///
/// Returns an error if bazelisk cquery fails or no matching artifact is found.
pub fn find_bazel_output(
    runner: &mut impl ToolRunner,
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

fn temp_json_output_path(tool: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("atom-{tool}-{timestamp}.json"))
}
