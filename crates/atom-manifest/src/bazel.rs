use std::process::Command;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};

/// # Errors
///
/// Returns an error if `label` is not a valid absolute Bazel label.
pub fn metadata_target(label: &str, suffix: &str) -> AtomResult<String> {
    let (repository, rest) = if let Some(rest) = label.strip_prefix("//") {
        ("", rest)
    } else if let Some((repository, rest)) = label.split_once("//") {
        if repository.starts_with('@') && !repository[1..].is_empty() {
            (repository, rest)
        } else {
            return Err(AtomError::new(
                AtomErrorCode::CliUsageError,
                "atom targets must use absolute Bazel labels like //pkg:target",
            ));
        }
    } else {
        return Err(AtomError::new(
            AtomErrorCode::CliUsageError,
            "atom targets must use absolute Bazel labels like //pkg:target",
        ));
    };

    let (package, target) = if let Some((package, target)) = rest.split_once(':') {
        (package, target)
    } else {
        let inferred = rest.rsplit('/').next().unwrap_or(rest);
        (rest, inferred)
    };

    if target.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::CliUsageError,
            "atom targets must include a non-empty Bazel target name",
        ));
    }

    Ok(if package.is_empty() {
        format!("{repository}//:{target}{suffix}")
    } else {
        format!("{repository}//{package}:{target}{suffix}")
    })
}

/// # Errors
///
/// Returns an error if bazel invocation fails or the output path is invalid.
pub fn build_metadata_output(repo_root: &Utf8Path, target: &str) -> AtomResult<Utf8PathBuf> {
    invoke_bazel(repo_root, &["build", target])?;
    let output = capture_bazel(repo_root, &["cquery", target, "--output=files"])?;
    let Some(first_line) = output.lines().find(|line| !line.trim().is_empty()) else {
        return Err(AtomError::new(
            AtomErrorCode::ManifestNotFound,
            format!("bazelisk did not return an output path for {target}"),
        ));
    };

    if first_line.starts_with('/') {
        Utf8PathBuf::from_path_buf(first_line.into()).map_err(|_| {
            AtomError::new(
                AtomErrorCode::ManifestParseError,
                "bazelisk returned a non-UTF-8 metadata path",
            )
        })
    } else {
        Ok(repo_root.join(first_line))
    }
}

fn invoke_bazel(repo_root: &Utf8Path, args: &[&str]) -> AtomResult<()> {
    let status = Command::new("bazelisk")
        .args(args)
        .current_dir(repo_root)
        .status()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to invoke bazelisk: {error}"),
            )
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("bazelisk {} exited with status {}", args.join(" "), status),
        ))
    }
}

fn capture_bazel(repo_root: &Utf8Path, args: &[&str]) -> AtomResult<String> {
    let output = Command::new("bazelisk")
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to invoke bazelisk: {error}"),
            )
        })?;

    if !output.status.success() {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!(
                "bazelisk {} exited with status {}",
                args.join(" "),
                output.status
            ),
        ));
    }

    String::from_utf8(output.stdout).map_err(|_| {
        AtomError::new(
            AtomErrorCode::ManifestParseError,
            "bazelisk returned non-UTF-8 output",
        )
    })
}
