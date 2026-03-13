use std::fs;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use atom_backends::ToolRunner;
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
        let output = command_output(repo_root, tool, args)?;
        if !output.status.success() {
            return Err(AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                command_failure_with_stderr(tool, args, output.status, &output.stderr),
            ));
        }
        Ok(())
    }

    fn capture(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<String> {
        let output = command_output(repo_root, tool, args)?;
        require_success(tool, args, output.status)?;
        utf8_stdout(tool, output.stdout)
    }

    fn stream(&mut self, repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()> {
        let status = streamed_command_status(repo_root, tool, args)?;
        require_success(tool, args, status)
    }

    fn capture_json_file(
        &mut self,
        repo_root: &Utf8Path,
        tool: &str,
        args: &[String],
    ) -> AtomResult<String> {
        let path = temp_json_output_path(tool);
        let output = command_output_with_json_path(repo_root, tool, args, &path)?;

        if !output.status.success() {
            cleanup_path(&path);
            let detail = command_failure_with_stderr(tool, args, output.status, &output.stderr);
            return Err(AtomError::new(AtomErrorCode::ExternalToolFailed, detail));
        }

        let contents = fs::read_to_string(&path).map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to read {tool} JSON output: {error}"),
            )
        })?;
        cleanup_path(&path);
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
    runner.run(repo_root, tool, &owned_args(args))
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
    runner.stream(repo_root, tool, &owned_args(args))
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
    runner.capture(repo_root, tool, &owned_args(args))
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
    runner.capture_json_file(repo_root, tool, &owned_args(args))
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
/// Returns an error if bazelisk cquery fails.
pub fn capture_bazel_cquery_starlark_owned(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    build_args: &[String],
    expression: &str,
) -> AtomResult<String> {
    let mut args = bazel_cquery_base_args(build_args);
    args.push("--output=starlark".to_owned());
    args.push(format!("--starlark:expr={expression}"));
    capture_bazel_owned(runner, repo_root, &args)
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
    let args = bazel_cquery_args(build_args);
    let output = capture_bazel_owned(runner, repo_root, &args)?;
    select_bazel_output_path(repo_root, &output, suffixes, artifact_name, target)
}

/// # Errors
///
/// Returns an error if any returned path is invalid UTF-8.
pub fn parse_bazel_output_paths(
    repo_root: &Utf8Path,
    output: &str,
    artifact_name: &str,
) -> AtomResult<Vec<Utf8PathBuf>> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| normalize_bazel_output_path(repo_root, line, artifact_name))
        .collect()
}

fn owned_args(args: &[&str]) -> Vec<String> {
    args.iter().map(|value| (*value).to_owned()).collect()
}

fn command_output(repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<Output> {
    base_command(repo_root, tool, args)
        .output()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to invoke {tool}: {error}"),
            )
        })
}

fn command_output_with_json_path(
    repo_root: &Utf8Path,
    tool: &str,
    args: &[String],
    path: &PathBuf,
) -> AtomResult<Output> {
    base_command(repo_root, tool, args)
        .arg("--json-output")
        .arg(path)
        .output()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to invoke {tool}: {error}"),
            )
        })
}

fn streamed_command_status(
    repo_root: &Utf8Path,
    tool: &str,
    args: &[String],
) -> AtomResult<ExitStatus> {
    base_command(repo_root, tool, args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to invoke {tool}: {error}"),
            )
        })
}

fn base_command(repo_root: &Utf8Path, tool: &str, args: &[String]) -> Command {
    let mut command = Command::new(tool);
    command.args(args).current_dir(repo_root);
    command
}

fn require_success(tool: &str, args: &[String], status: ExitStatus) -> AtomResult<()> {
    if status.success() {
        Ok(())
    } else {
        Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            command_failure(tool, args, status),
        ))
    }
}

fn utf8_stdout(tool: &str, stdout: Vec<u8>) -> AtomResult<String> {
    String::from_utf8(stdout).map_err(|_| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("{tool} returned non-UTF-8 output"),
        )
    })
}

fn command_failure(tool: &str, args: &[String], status: ExitStatus) -> String {
    format!("{tool} {} exited with status {status}", args.join(" "))
}

fn command_failure_with_stderr(
    tool: &str,
    args: &[String],
    status: ExitStatus,
    stderr: &[u8],
) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    if stderr.trim().is_empty() {
        command_failure(tool, args, status)
    } else {
        format!("{}:\n{stderr}", command_failure(tool, args, status))
    }
}

fn cleanup_path(path: &PathBuf) {
    let _ = fs::remove_file(path);
}

fn bazel_cquery_base_args(build_args: &[String]) -> Vec<String> {
    let mut args = Vec::with_capacity(build_args.len().saturating_add(1));
    args.push("cquery".to_owned());
    args.extend(build_args.iter().skip(1).cloned());
    args
}

fn bazel_cquery_args(build_args: &[String]) -> Vec<String> {
    let mut args = bazel_cquery_base_args(build_args);
    args.push("--output=files".to_owned());
    args
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

    normalize_bazel_output_path(repo_root, selected, artifact_name)
}

fn normalize_bazel_output_path(
    repo_root: &Utf8Path,
    path: &str,
    artifact_name: &str,
) -> AtomResult<Utf8PathBuf> {
    if path.starts_with('/') {
        Utf8PathBuf::from_path_buf(path.into()).map_err(|_| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("bazelisk returned a non-UTF-8 {artifact_name} path"),
            )
        })
    } else {
        Ok(repo_root.join(path))
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

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use super::{
        bazel_cquery_args, capture_bazel_cquery_starlark_owned, parse_bazel_output_paths,
        select_bazel_output,
    };
    use atom_backends::ToolRunner;

    #[test]
    fn bazel_cquery_args_replaces_the_subcommand_and_preserves_flags() {
        let args = bazel_cquery_args(&[
            "build".to_owned(),
            "//apps/demo:demo".to_owned(),
            "--config=ci".to_owned(),
        ]);

        assert_eq!(
            args,
            vec![
                "cquery".to_owned(),
                "//apps/demo:demo".to_owned(),
                "--config=ci".to_owned(),
                "--output=files".to_owned(),
            ]
        );
    }

    #[test]
    fn select_bazel_output_prefers_suffix_priority_over_line_order() {
        let output = "\n  bazel-out/demo/app.apk\n  bazel-out/demo/app.aab\n";

        let selected = select_bazel_output(output, &[".aab", ".apk"]);

        assert_eq!(selected, Some("bazel-out/demo/app.aab"));
    }

    #[test]
    fn select_bazel_output_ignores_blank_lines() {
        let output = "\n\n  bazel-out/demo/app.apk  \n\n";

        let selected = select_bazel_output(output, &[".apk"]);

        assert_eq!(selected, Some("bazel-out/demo/app.apk"));
    }

    #[test]
    fn parse_bazel_output_paths_normalizes_relative_and_absolute_paths() {
        let repo_root = Utf8PathBuf::from("/repo");
        let output = "bazel-out/demo/app.apk\n/private/tmp/app_deploy.jar\n";

        let paths =
            parse_bazel_output_paths(&repo_root, output, "artifact").expect("paths should parse");

        assert_eq!(
            paths,
            vec![
                Utf8PathBuf::from("/repo/bazel-out/demo/app.apk"),
                Utf8PathBuf::from("/private/tmp/app_deploy.jar"),
            ]
        );
    }

    #[test]
    fn starlark_cquery_reuses_build_flags_from_build_args() {
        #[derive(Default)]
        struct RecordingRunner {
            args: Vec<String>,
        }

        impl ToolRunner for RecordingRunner {
            fn run(
                &mut self,
                _repo_root: &camino::Utf8Path,
                _tool: &str,
                _args: &[String],
            ) -> atom_ffi::AtomResult<()> {
                unreachable!("run is not used")
            }

            fn capture(
                &mut self,
                _repo_root: &camino::Utf8Path,
                _tool: &str,
                args: &[String],
            ) -> atom_ffi::AtomResult<String> {
                self.args = args.to_vec();
                Ok(String::new())
            }

            fn capture_json_file(
                &mut self,
                _repo_root: &camino::Utf8Path,
                _tool: &str,
                _args: &[String],
            ) -> atom_ffi::AtomResult<String> {
                unreachable!("capture_json_file is not used")
            }

            fn stream(
                &mut self,
                _repo_root: &camino::Utf8Path,
                _tool: &str,
                _args: &[String],
            ) -> atom_ffi::AtomResult<()> {
                unreachable!("stream is not used")
            }
        }

        let mut runner = RecordingRunner::default();
        let repo_root = Utf8PathBuf::from("/repo");
        let build_args = vec![
            "build".to_owned(),
            "//apps/demo:demo".to_owned(),
            "--compilation_mode=dbg".to_owned(),
        ];

        capture_bazel_cquery_starlark_owned(&mut runner, &repo_root, &build_args, "target.label")
            .expect("cquery should dispatch");

        assert_eq!(
            runner.args,
            vec![
                "cquery".to_owned(),
                "//apps/demo:demo".to_owned(),
                "--compilation_mode=dbg".to_owned(),
                "--output=starlark".to_owned(),
                "--starlark:expr=target.label".to_owned(),
            ]
        );
    }
}
