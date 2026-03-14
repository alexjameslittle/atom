use std::fmt::Write as _;
use std::fs;
use std::io;
use std::process::Command;

use atom_deploy::CommandOutput;
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::Utf8Path;
use clap::Args;
use serde::Serialize;

#[derive(Debug, Args)]
pub(super) struct DoctorArgs {
    #[arg(long)]
    pub(super) json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DoctorStatus {
    Ok,
    Issue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DoctorSeverity {
    Critical,
    Platform,
    Recommended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct DoctorCheck {
    id: &'static str,
    label: &'static str,
    status: DoctorStatus,
    severity: DoctorSeverity,
    summary: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    remediation: Vec<String>,
}

impl DoctorCheck {
    fn ok(
        id: &'static str,
        label: &'static str,
        severity: DoctorSeverity,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id,
            label,
            status: DoctorStatus::Ok,
            severity,
            summary: summary.into(),
            remediation: Vec::new(),
        }
    }

    fn issue(
        id: &'static str,
        label: &'static str,
        severity: DoctorSeverity,
        summary: impl Into<String>,
        remediation: Vec<String>,
    ) -> Self {
        Self {
            id,
            label,
            status: DoctorStatus::Issue,
            severity,
            summary: summary.into(),
            remediation,
        }
    }

    fn is_ok(&self) -> bool {
        self.status == DoctorStatus::Ok
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct DoctorReport {
    checks: Vec<DoctorCheck>,
    ready_platforms: Vec<String>,
    issue_count: usize,
    critical_issue_count: usize,
}

impl DoctorReport {
    fn exit_code(&self) -> i32 {
        i32::from(self.critical_issue_count > 0)
    }

    fn render_text(&self) -> String {
        let mut output = String::from("atom doctor\n\n");
        for check in &self.checks {
            let _ = writeln!(
                output,
                "  {} {} {}",
                dotted_label(check.label),
                check.summary,
                status_symbol(check.status),
            );
            for remediation in &check.remediation {
                let _ = writeln!(output, "    -> {remediation}");
            }
        }

        output.push('\n');
        let platform_word = if self.ready_platforms.len() == 1 {
            "platform"
        } else {
            "platforms"
        };
        if self.ready_platforms.is_empty() {
            output.push_str("  0 platforms ready\n");
        } else {
            let _ = writeln!(
                output,
                "  {} {platform_word} ready: {}",
                self.ready_platforms.len(),
                self.ready_platforms.join(", ")
            );
        }

        let issue_word = if self.issue_count == 1 {
            "issue"
        } else {
            "issues"
        };
        if self.critical_issue_count > 0 {
            let _ = writeln!(
                output,
                "  {} {issue_word} found ({} critical)",
                self.issue_count, self.critical_issue_count
            );
        } else {
            let _ = writeln!(output, "  {} {issue_word} found", self.issue_count);
        }

        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapturedCommand {
    status: i32,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandInvocation {
    Output(CapturedCommand),
    Missing,
    Failed(String),
}

trait DoctorSystem {
    fn read_file(&self, path: &Utf8Path) -> Result<String, String>;
    fn env_var(&self, key: &str) -> Option<String>;
    fn run_command(&self, repo_root: &Utf8Path, tool: &str, args: &[&str]) -> CommandInvocation;
}

struct ProcessDoctorSystem;

impl DoctorSystem for ProcessDoctorSystem {
    fn read_file(&self, path: &Utf8Path) -> Result<String, String> {
        fs::read_to_string(path).map_err(|error| error.to_string())
    }

    fn env_var(&self, key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }

    fn run_command(&self, repo_root: &Utf8Path, tool: &str, args: &[&str]) -> CommandInvocation {
        match Command::new(tool)
            .args(args)
            .current_dir(repo_root)
            .output()
        {
            Ok(output) => CommandInvocation::Output(CapturedCommand {
                status: output.status.code().unwrap_or(1),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => CommandInvocation::Missing,
            Err(error) => CommandInvocation::Failed(error.to_string()),
        }
    }
}

pub(super) fn execute(repo_root: &Utf8Path, args: &DoctorArgs) -> AtomResult<CommandOutput> {
    execute_with_system(repo_root, args, &ProcessDoctorSystem)
}

fn execute_with_system(
    repo_root: &Utf8Path,
    args: &DoctorArgs,
    system: &impl DoctorSystem,
) -> AtomResult<CommandOutput> {
    let report = collect_report(repo_root, system);
    let stdout = if args.json {
        serde_json::to_vec_pretty(&report).map_err(|error| {
            AtomError::new(
                AtomErrorCode::InternalBug,
                format!("failed to encode doctor JSON output: {error}"),
            )
        })?
    } else {
        report.render_text().into_bytes()
    };

    Ok(CommandOutput {
        stdout,
        stderr: Vec::new(),
        exit_code: report.exit_code(),
    })
}

fn collect_report(repo_root: &Utf8Path, system: &impl DoctorSystem) -> DoctorReport {
    let bazel = bazel_check(repo_root, system);
    let rust = rust_check(repo_root, system);
    let mise = mise_check(repo_root, system);
    let xcode = xcode_check(repo_root, system);
    let simulators = ios_simulators_check(repo_root, system);
    let android = android_sdk_check(repo_root, system);
    let java = java_check(repo_root, system);

    let ios_ready = xcode.is_ok() && simulators.is_ok();
    let android_ready = android.is_ok() && java.is_ok();
    let checks = vec![bazel, rust, mise, xcode, simulators, android, java];
    let issue_count = checks
        .iter()
        .filter(|check| check.status == DoctorStatus::Issue)
        .count();
    let critical_issue_count = checks
        .iter()
        .filter(|check| {
            check.status == DoctorStatus::Issue && check.severity == DoctorSeverity::Critical
        })
        .count();
    let mut ready_platforms = Vec::new();
    if ios_ready {
        ready_platforms.push("ios".to_owned());
    }
    if android_ready {
        ready_platforms.push("android".to_owned());
    }

    DoctorReport {
        checks,
        ready_platforms,
        issue_count,
        critical_issue_count,
    }
}

fn bazel_check(repo_root: &Utf8Path, system: &impl DoctorSystem) -> DoctorCheck {
    let expected = match read_expected_tool_version(repo_root, system, ".bazelversion", "bazel") {
        Ok(version) => version,
        Err(message) => {
            return DoctorCheck::issue(
                "bazel",
                "Bazel",
                DoctorSeverity::Critical,
                "expected version unavailable",
                vec![message],
            );
        }
    };

    match system.run_command(repo_root, "bazelisk", &["version"]) {
        CommandInvocation::Missing => DoctorCheck::issue(
            "bazel",
            "Bazel",
            DoctorSeverity::Critical,
            "not found",
            vec!["Install Bazelisk or run: mise install bazelisk".to_owned()],
        ),
        CommandInvocation::Failed(error) => DoctorCheck::issue(
            "bazel",
            "Bazel",
            DoctorSeverity::Critical,
            "version check failed",
            vec![format!("Failed to start bazelisk: {error}")],
        ),
        CommandInvocation::Output(output) if output.status != 0 => DoctorCheck::issue(
            "bazel",
            "Bazel",
            DoctorSeverity::Critical,
            "version check failed",
            vec![
                format!("Expected Bazel {}", expected),
                combined_output(&output),
            ],
        ),
        CommandInvocation::Output(output) => {
            let detected = parse_bazel_build_label(&output.stdout)
                .or_else(|| first_version_token(&output.stdout))
                .unwrap_or_else(|| "unknown".to_owned());
            if detected == expected {
                DoctorCheck::ok("bazel", "Bazel", DoctorSeverity::Critical, detected)
            } else {
                DoctorCheck::issue(
                    "bazel",
                    "Bazel",
                    DoctorSeverity::Critical,
                    format!("{detected} (expected {expected})"),
                    vec!["Run: mise install bazelisk".to_owned()],
                )
            }
        }
    }
}

fn rust_check(repo_root: &Utf8Path, system: &impl DoctorSystem) -> DoctorCheck {
    let expected = match read_expected_mise_version(repo_root, system, "rust") {
        Ok(version) => version,
        Err(message) => {
            return DoctorCheck::issue(
                "rust",
                "Rust",
                DoctorSeverity::Critical,
                "expected version unavailable",
                vec![message],
            );
        }
    };

    match system.run_command(repo_root, "rustc", &["--version"]) {
        CommandInvocation::Missing => DoctorCheck::issue(
            "rust",
            "Rust",
            DoctorSeverity::Critical,
            "not found",
            vec![format!("Install Rust {expected} or run: mise install rust")],
        ),
        CommandInvocation::Failed(error) => DoctorCheck::issue(
            "rust",
            "Rust",
            DoctorSeverity::Critical,
            "version check failed",
            vec![format!("Failed to start rustc: {error}")],
        ),
        CommandInvocation::Output(output) if output.status != 0 => DoctorCheck::issue(
            "rust",
            "Rust",
            DoctorSeverity::Critical,
            "version check failed",
            vec![
                format!("Expected Rust {expected}"),
                combined_output(&output),
            ],
        ),
        CommandInvocation::Output(output) => {
            let detected =
                first_version_token(&output.stdout).unwrap_or_else(|| "unknown".to_owned());
            if detected == expected {
                DoctorCheck::ok("rust", "Rust", DoctorSeverity::Critical, detected)
            } else {
                DoctorCheck::issue(
                    "rust",
                    "Rust",
                    DoctorSeverity::Critical,
                    format!("{detected} (expected {expected})"),
                    vec!["Run: mise install rust".to_owned()],
                )
            }
        }
    }
}

fn mise_check(repo_root: &Utf8Path, system: &impl DoctorSystem) -> DoctorCheck {
    match system.run_command(repo_root, "mise", &["--version"]) {
        CommandInvocation::Missing => DoctorCheck::issue(
            "mise",
            "mise",
            DoctorSeverity::Recommended,
            "not found",
            vec![
                "Install mise from https://mise.jdx.dev/".to_owned(),
                "Then run: ./scripts/bootstrap.sh".to_owned(),
            ],
        ),
        CommandInvocation::Failed(error) => DoctorCheck::issue(
            "mise",
            "mise",
            DoctorSeverity::Recommended,
            "version check failed",
            vec![format!("Failed to start mise: {error}")],
        ),
        CommandInvocation::Output(output) if output.status != 0 => DoctorCheck::issue(
            "mise",
            "mise",
            DoctorSeverity::Recommended,
            "version check failed",
            vec![combined_output(&output)],
        ),
        CommandInvocation::Output(output) => DoctorCheck::ok(
            "mise",
            "mise",
            DoctorSeverity::Recommended,
            first_version_token(&output.stdout).unwrap_or_else(|| "installed".to_owned()),
        ),
    }
}

fn xcode_check(repo_root: &Utf8Path, system: &impl DoctorSystem) -> DoctorCheck {
    let selected_path = match system.run_command(repo_root, "xcode-select", &["-p"]) {
        CommandInvocation::Missing => {
            return DoctorCheck::issue(
                "xcode",
                "Xcode",
                DoctorSeverity::Platform,
                "not found",
                vec!["Install Xcode, then run xcode-select --switch /Applications/Xcode.app/Contents/Developer".to_owned()],
            );
        }
        CommandInvocation::Failed(error) => {
            return DoctorCheck::issue(
                "xcode",
                "Xcode",
                DoctorSeverity::Platform,
                "xcode-select failed",
                vec![format!("Failed to start xcode-select: {error}")],
            );
        }
        CommandInvocation::Output(output) if output.status != 0 => {
            return DoctorCheck::issue(
                "xcode",
                "Xcode",
                DoctorSeverity::Platform,
                "not configured",
                vec![
                    "Run xcode-select --switch /Applications/Xcode.app/Contents/Developer"
                        .to_owned(),
                    combined_output(&output),
                ],
            );
        }
        CommandInvocation::Output(output) => output.stdout.trim().to_owned(),
    };

    match system.run_command(repo_root, "xcodebuild", &["-version"]) {
        CommandInvocation::Missing => DoctorCheck::issue(
            "xcode",
            "Xcode",
            DoctorSeverity::Platform,
            "not found",
            vec![
                "Install full Xcode, not just the command line tools.".to_owned(),
                format!("xcode-select currently points to {selected_path}"),
            ],
        ),
        CommandInvocation::Failed(error) => DoctorCheck::issue(
            "xcode",
            "Xcode",
            DoctorSeverity::Platform,
            "version check failed",
            vec![format!("Failed to start xcodebuild: {error}")],
        ),
        CommandInvocation::Output(output) if output.status != 0 => DoctorCheck::issue(
            "xcode",
            "Xcode",
            DoctorSeverity::Platform,
            "version check failed",
            vec![combined_output(&output)],
        ),
        CommandInvocation::Output(output) => DoctorCheck::ok(
            "xcode",
            "Xcode",
            DoctorSeverity::Platform,
            parse_xcode_version(&output.stdout).unwrap_or(selected_path),
        ),
    }
}

fn ios_simulators_check(repo_root: &Utf8Path, system: &impl DoctorSystem) -> DoctorCheck {
    match system.run_command(
        repo_root,
        "xcrun",
        &["simctl", "list", "devices", "available", "--json"],
    ) {
        CommandInvocation::Missing => DoctorCheck::issue(
            "ios_simulators",
            "iOS Simulators",
            DoctorSeverity::Platform,
            "simctl not found",
            vec!["Install Xcode and ensure xcrun is on PATH.".to_owned()],
        ),
        CommandInvocation::Failed(error) => DoctorCheck::issue(
            "ios_simulators",
            "iOS Simulators",
            DoctorSeverity::Platform,
            "simctl failed",
            vec![format!("Failed to start xcrun simctl: {error}")],
        ),
        CommandInvocation::Output(output) if output.status != 0 => DoctorCheck::issue(
            "ios_simulators",
            "iOS Simulators",
            DoctorSeverity::Platform,
            "simctl failed",
            vec![combined_output(&output)],
        ),
        CommandInvocation::Output(output) => match count_available_simulators(&output.stdout) {
            Some(0) => DoctorCheck::issue(
                "ios_simulators",
                "iOS Simulators",
                DoctorSeverity::Platform,
                "0 available",
                vec![
                    "Install an iOS simulator runtime in Xcode > Settings > Platforms.".to_owned(),
                ],
            ),
            Some(count) => DoctorCheck::ok(
                "ios_simulators",
                "iOS Simulators",
                DoctorSeverity::Platform,
                format!("{count} available"),
            ),
            None => DoctorCheck::issue(
                "ios_simulators",
                "iOS Simulators",
                DoctorSeverity::Platform,
                "list unavailable",
                vec!["xcrun simctl returned output that could not be parsed.".to_owned()],
            ),
        },
    }
}

fn android_sdk_check(repo_root: &Utf8Path, system: &impl DoctorSystem) -> DoctorCheck {
    let Some(android_home) = system.env_var("ANDROID_HOME") else {
        return DoctorCheck::issue(
            "android_sdk",
            "Android SDK",
            DoctorSeverity::Platform,
            "not configured",
            vec!["Set ANDROID_HOME or run: scripts/setup-android-sdk.sh".to_owned()],
        );
    };

    match system.run_command(repo_root, "adb", &["devices", "-l"]) {
        CommandInvocation::Missing => DoctorCheck::issue(
            "android_sdk",
            "Android SDK",
            DoctorSeverity::Platform,
            "adb not found",
            vec![
                format!("ANDROID_HOME is set to {android_home}"),
                "Add $ANDROID_HOME/platform-tools to PATH or run: scripts/setup-android-sdk.sh"
                    .to_owned(),
            ],
        ),
        CommandInvocation::Failed(error) => DoctorCheck::issue(
            "android_sdk",
            "Android SDK",
            DoctorSeverity::Platform,
            "adb failed",
            vec![format!("Failed to start adb: {error}")],
        ),
        CommandInvocation::Output(output) if output.status != 0 => DoctorCheck::issue(
            "android_sdk",
            "Android SDK",
            DoctorSeverity::Platform,
            "adb failed",
            vec![combined_output(&output)],
        ),
        CommandInvocation::Output(output) => {
            let device_count = count_adb_devices(&output.stdout);
            if device_count > 0 {
                DoctorCheck::ok(
                    "android_sdk",
                    "Android SDK",
                    DoctorSeverity::Platform,
                    format!("{device_count} devices available"),
                )
            } else {
                DoctorCheck::issue(
                    "android_sdk",
                    "Android SDK",
                    DoctorSeverity::Platform,
                    "0 devices available",
                    vec![
                        "Start an Android emulator or connect a device, then re-run atom doctor."
                            .to_owned(),
                    ],
                )
            }
        }
    }
}

fn java_check(repo_root: &Utf8Path, system: &impl DoctorSystem) -> DoctorCheck {
    match system.run_command(repo_root, "javac", &["--version"]) {
        CommandInvocation::Missing => DoctorCheck::issue(
            "java",
            "Java",
            DoctorSeverity::Platform,
            "not found",
            vec!["Install JDK 21+ or run: mise install java".to_owned()],
        ),
        CommandInvocation::Failed(error) => DoctorCheck::issue(
            "java",
            "Java",
            DoctorSeverity::Platform,
            "version check failed",
            vec![format!("Failed to start javac: {error}")],
        ),
        CommandInvocation::Output(output) if output.status != 0 => DoctorCheck::issue(
            "java",
            "Java",
            DoctorSeverity::Platform,
            if looks_like_missing_java(&combined_output(&output)) {
                "not found"
            } else {
                "version check failed"
            },
            vec![
                "Install JDK 21+ or run: mise install java".to_owned(),
                combined_output(&output),
            ],
        ),
        CommandInvocation::Output(output) => {
            let text = if output.stdout.trim().is_empty() {
                output.stderr.trim()
            } else {
                output.stdout.trim()
            };
            let detected = first_version_token(text).unwrap_or_else(|| "unknown".to_owned());
            match parse_java_major(&detected) {
                Some(major) if major >= 21 => {
                    DoctorCheck::ok("java", "Java", DoctorSeverity::Platform, detected)
                }
                _ => DoctorCheck::issue(
                    "java",
                    "Java",
                    DoctorSeverity::Platform,
                    format!("{detected} (need 21+)"),
                    vec!["Install JDK 21+ or run: mise install java".to_owned()],
                ),
            }
        }
    }
}

fn read_expected_tool_version(
    repo_root: &Utf8Path,
    system: &impl DoctorSystem,
    relative_path: &str,
    tool_name: &str,
) -> Result<String, String> {
    let path = repo_root.join(relative_path);
    system
        .read_file(&path)
        .map(|contents| contents.trim().to_owned())
        .map_err(|error| {
            format!("Restore {relative_path} so atom doctor can verify {tool_name}: {error}")
        })
}

fn read_expected_mise_version(
    repo_root: &Utf8Path,
    system: &impl DoctorSystem,
    tool: &str,
) -> Result<String, String> {
    let path = repo_root.join("mise.toml");
    let contents = system
        .read_file(&path)
        .map_err(|error| format!("Restore mise.toml so atom doctor can verify {tool}: {error}"))?;
    parse_mise_tool_version(&contents, tool).ok_or_else(|| {
        format!("Add a pinned `{tool}` entry to mise.toml so atom doctor can verify it.")
    })
}

fn parse_mise_tool_version(contents: &str, tool: &str) -> Option<String> {
    let needle = format!("{tool} = ");
    contents.lines().find_map(|line| {
        let remainder = line.trim().strip_prefix(&needle)?;
        let start = remainder.find('"')? + 1;
        let end = remainder[start..].find('"')? + start;
        Some(remainder[start..end].to_owned())
    })
}

fn parse_bazel_build_label(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Build label: "))
        .map(str::to_owned)
}

fn parse_xcode_version(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Xcode "))
        .map(str::to_owned)
}

fn first_version_token(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|token| {
            token.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '.' && character != '-'
            })
        })
        .find(|token| {
            token
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
        })
        .map(str::to_owned)
}

fn parse_java_major(version: &str) -> Option<u32> {
    version
        .split(['.', '-'])
        .next()
        .and_then(|segment| segment.parse().ok())
}

fn count_available_simulators(output: &str) -> Option<usize> {
    let value: serde_json::Value = serde_json::from_str(output).ok()?;
    let devices = value.get("devices")?.as_object()?;
    Some(
        devices
            .values()
            .filter_map(serde_json::Value::as_array)
            .flat_map(|entries| entries.iter())
            .filter(|device| {
                device
                    .get("isAvailable")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
            })
            .count(),
    )
}

fn count_adb_devices(output: &str) -> usize {
    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("List of devices attached") {
                return None;
            }
            let mut parts = trimmed.split_whitespace();
            let _serial = parts.next()?;
            let state = parts.next()?;
            (state == "device").then_some(())
        })
        .count()
}

fn combined_output(output: &CapturedCommand) -> String {
    let stdout = output.stdout.trim();
    let stderr = output.stderr.trim();
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => format!("Command exited with status {}", output.status),
        (false, true) => stdout.to_owned(),
        (true, false) => stderr.to_owned(),
        (false, false) => format!("{stdout} {stderr}"),
    }
}

fn looks_like_missing_java(detail: &str) -> bool {
    let lowered = detail.to_ascii_lowercase();
    lowered.contains("unable to locate a java runtime")
        || lowered.contains("no java runtime present")
}

fn dotted_label(label: &str) -> String {
    const LABEL_WIDTH: usize = 18;
    let dot_count = LABEL_WIDTH.saturating_sub(label.len()).max(1);
    format!("{label} {}", ".".repeat(dot_count))
}

fn status_symbol(status: DoctorStatus) -> &'static str {
    match status {
        DoctorStatus::Ok => "✓",
        DoctorStatus::Issue => "✗",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use camino::Utf8Path;
    use serde_json::Value;

    use super::{
        CapturedCommand, CommandInvocation, DoctorArgs, DoctorSystem, count_adb_devices,
        count_available_simulators, execute_with_system,
    };

    #[derive(Default)]
    struct FakeDoctorSystem {
        files: BTreeMap<String, String>,
        env: BTreeMap<String, String>,
        commands: BTreeMap<CommandKey, CommandInvocation>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct CommandKey {
        tool: String,
        args: Vec<String>,
    }

    impl FakeDoctorSystem {
        fn with_file(mut self, path: &str, contents: &str) -> Self {
            self.files.insert(path.to_owned(), contents.to_owned());
            self
        }

        fn with_env(mut self, key: &str, value: &str) -> Self {
            self.env.insert(key.to_owned(), value.to_owned());
            self
        }

        fn with_output(mut self, tool: &str, args: &[&str], stdout: &str, stderr: &str) -> Self {
            self.commands.insert(
                CommandKey::new(tool, args),
                CommandInvocation::Output(CapturedCommand {
                    status: 0,
                    stdout: stdout.to_owned(),
                    stderr: stderr.to_owned(),
                }),
            );
            self
        }

        fn with_status(
            mut self,
            tool: &str,
            args: &[&str],
            status: i32,
            stdout: &str,
            stderr: &str,
        ) -> Self {
            self.commands.insert(
                CommandKey::new(tool, args),
                CommandInvocation::Output(CapturedCommand {
                    status,
                    stdout: stdout.to_owned(),
                    stderr: stderr.to_owned(),
                }),
            );
            self
        }

        fn with_missing(mut self, tool: &str, args: &[&str]) -> Self {
            self.commands
                .insert(CommandKey::new(tool, args), CommandInvocation::Missing);
            self
        }
    }

    impl CommandKey {
        fn new(tool: &str, args: &[&str]) -> Self {
            Self {
                tool: tool.to_owned(),
                args: args.iter().map(|value| (*value).to_owned()).collect(),
            }
        }
    }

    impl DoctorSystem for FakeDoctorSystem {
        fn read_file(&self, path: &Utf8Path) -> Result<String, String> {
            self.files
                .get(path.as_str())
                .cloned()
                .ok_or_else(|| format!("missing {}", path))
        }

        fn env_var(&self, key: &str) -> Option<String> {
            self.env.get(key).cloned()
        }

        fn run_command(
            &self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[&str],
        ) -> CommandInvocation {
            self.commands
                .get(&CommandKey::new(tool, args))
                .cloned()
                .unwrap_or(CommandInvocation::Missing)
        }
    }

    fn repo_root() -> &'static Utf8Path {
        Utf8Path::new("/repo")
    }

    #[test]
    fn clean_ios_only_host_is_non_critical() {
        let system = FakeDoctorSystem::default()
            .with_file("/repo/.bazelversion", "8.4.2\n")
            .with_file(
                "/repo/mise.toml",
                "[tools]\nrust = \"1.92.0\"\njava = \"temurin-21\"\n",
            )
            .with_output(
                "bazelisk",
                &["version"],
                "Bazelisk version: v1.28.1\nBuild label: 8.4.2\n",
                "",
            )
            .with_output("rustc", &["--version"], "rustc 1.92.0 (hash 2025-12-08)\n", "")
            .with_output("mise", &["--version"], "2025.12.13 macos-arm64\n", "")
            .with_output("xcode-select", &["-p"], "/Applications/Xcode.app/Contents/Developer\n", "")
            .with_output("xcodebuild", &["-version"], "Xcode 26.1.1\nBuild version 17B100\n", "")
            .with_output(
                "xcrun",
                &["simctl", "list", "devices", "available", "--json"],
                "{\"devices\":{\"com.apple.CoreSimulator.SimRuntime.iOS-26-1\":[{\"isAvailable\":true},{\"isAvailable\":true}]}}",
                "",
            )
            .with_missing("javac", &["--version"]);

        let output = execute_with_system(repo_root(), &DoctorArgs { json: false }, &system)
            .expect("doctor should render");
        let stdout = String::from_utf8(output.stdout).expect("utf8");

        assert_eq!(output.exit_code, 0);
        assert!(stdout.contains("1 platform ready: ios"));
        assert!(stdout.contains("2 issues found"));
        assert!(stdout.contains("Set ANDROID_HOME or run: scripts/setup-android-sdk.sh"));
        assert!(stdout.contains("Install JDK 21+ or run: mise install java"));
    }

    #[test]
    fn missing_bazel_is_critical() {
        let system = FakeDoctorSystem::default()
            .with_file("/repo/.bazelversion", "8.4.2\n")
            .with_file(
                "/repo/mise.toml",
                "[tools]\nrust = \"1.92.0\"\njava = \"temurin-21\"\n",
            )
            .with_missing("bazelisk", &["version"])
            .with_output("rustc", &["--version"], "rustc 1.92.0 (hash 2025-12-08)\n", "")
            .with_output("mise", &["--version"], "2025.12.13 macos-arm64\n", "")
            .with_output("xcode-select", &["-p"], "/Applications/Xcode.app/Contents/Developer\n", "")
            .with_output("xcodebuild", &["-version"], "Xcode 26.1.1\nBuild version 17B100\n", "")
            .with_output(
                "xcrun",
                &["simctl", "list", "devices", "available", "--json"],
                "{\"devices\":{\"com.apple.CoreSimulator.SimRuntime.iOS-26-1\":[{\"isAvailable\":true}]}}",
                "",
            )
            .with_env("ANDROID_HOME", "/Users/test/.android/sdk")
            .with_output("adb", &["devices", "-l"], "List of devices attached\nemulator-5554 device product:sdk_gphone64_arm64\n", "")
            .with_output("javac", &["--version"], "javac 21.0.2\n", "");

        let output = execute_with_system(repo_root(), &DoctorArgs { json: false }, &system)
            .expect("doctor should render");
        let stdout = String::from_utf8(output.stdout).expect("utf8");

        assert_eq!(output.exit_code, 1);
        assert!(stdout.contains("Bazel"));
        assert!(stdout.contains("Install Bazelisk or run: mise install bazelisk"));
        assert!(stdout.contains("1 issue found (1 critical)"));
    }

    #[test]
    fn json_output_is_machine_readable() {
        let system = FakeDoctorSystem::default()
            .with_file("/repo/.bazelversion", "8.4.2\n")
            .with_file(
                "/repo/mise.toml",
                "[tools]\nrust = \"1.92.0\"\njava = \"temurin-21\"\n",
            )
            .with_output(
                "bazelisk",
                &["version"],
                "Bazelisk version: v1.28.1\nBuild label: 8.4.2\n",
                "",
            )
            .with_output("rustc", &["--version"], "rustc 1.92.0 (hash 2025-12-08)\n", "")
            .with_missing("mise", &["--version"])
            .with_output("xcode-select", &["-p"], "/Applications/Xcode.app/Contents/Developer\n", "")
            .with_output("xcodebuild", &["-version"], "Xcode 26.1.1\nBuild version 17B100\n", "")
            .with_output(
                "xcrun",
                &["simctl", "list", "devices", "available", "--json"],
                "{\"devices\":{\"com.apple.CoreSimulator.SimRuntime.iOS-26-1\":[{\"isAvailable\":true}]}}",
                "",
            )
            .with_env("ANDROID_HOME", "/Users/test/.android/sdk")
            .with_output("adb", &["devices", "-l"], "List of devices attached\nemulator-5554 device product:sdk_gphone64_arm64\n", "")
            .with_output("javac", &["--version"], "javac 21.0.2\n", "");

        let output = execute_with_system(repo_root(), &DoctorArgs { json: true }, &system)
            .expect("doctor should render");
        let value: Value = serde_json::from_slice(&output.stdout).expect("json output");

        assert_eq!(output.exit_code, 0);
        assert_eq!(
            value["ready_platforms"],
            serde_json::json!(["ios", "android"])
        );
        assert_eq!(value["issue_count"], 1);
        assert_eq!(value["checks"][2]["id"], "mise");
        assert_eq!(value["checks"][2]["status"], "issue");
        assert_eq!(value["checks"][2]["severity"], "recommended");
    }

    #[test]
    fn simulator_and_adb_parsers_handle_realistic_output() {
        let simulator_json = "{\"devices\":{\"runtime\":[{\"isAvailable\":true},{\"isAvailable\":false},{\"isAvailable\":true}]}}";
        assert_eq!(count_available_simulators(simulator_json), Some(2));

        let adb_output = "List of devices attached\nemulator-5554 device product:sdk_gphone64_arm64\nusb-serial offline transport_id:1\n";
        assert_eq!(count_adb_devices(adb_output), 1);
    }

    #[test]
    fn nonzero_java_probe_reports_issue() {
        let system = FakeDoctorSystem::default()
            .with_file("/repo/.bazelversion", "8.4.2\n")
            .with_file(
                "/repo/mise.toml",
                "[tools]\nrust = \"1.92.0\"\njava = \"temurin-21\"\n",
            )
            .with_output(
                "bazelisk",
                &["version"],
                "Bazelisk version: v1.28.1\nBuild label: 8.4.2\n",
                "",
            )
            .with_output(
                "rustc",
                &["--version"],
                "rustc 1.92.0 (hash 2025-12-08)\n",
                "",
            )
            .with_output("mise", &["--version"], "2025.12.13 macos-arm64\n", "")
            .with_output(
                "xcode-select",
                &["-p"],
                "/Applications/Xcode.app/Contents/Developer\n",
                "",
            )
            .with_output(
                "xcodebuild",
                &["-version"],
                "Xcode 26.1.1\nBuild version 17B100\n",
                "",
            )
            .with_output(
                "xcrun",
                &["simctl", "list", "devices", "available", "--json"],
                "{\"devices\":{\"runtime\":[{\"isAvailable\":true}]}}",
                "",
            )
            .with_env("ANDROID_HOME", "/Users/test/.android/sdk")
            .with_output(
                "adb",
                &["devices", "-l"],
                "List of devices attached\nemulator-5554 device product:sdk_gphone64_arm64\n",
                "",
            )
            .with_status(
                "javac",
                &["--version"],
                1,
                "",
                "Unable to locate a Java Runtime.\n",
            );

        let output = execute_with_system(repo_root(), &DoctorArgs { json: false }, &system)
            .expect("doctor should render");
        let stdout = String::from_utf8(output.stdout).expect("utf8");

        assert_eq!(output.exit_code, 0);
        assert!(stdout.contains("Java"));
        assert!(stdout.contains("not found"));
    }
}
