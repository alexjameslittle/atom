use std::fmt::Write as _;

use atom_backends::{
    CommandInvocation, DeployBackendRegistry, DoctorCheck, DoctorSeverity, DoctorStatus,
    DoctorSystem, ProcessDoctorSystem, combined_command_output, first_version_token,
};
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
                dotted_label(&check.label),
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

/// # Errors
///
/// Returns an error if the CLI output cannot be encoded.
pub(super) fn execute(
    repo_root: &Utf8Path,
    args: &DoctorArgs,
    deploy_registry: &DeployBackendRegistry,
) -> AtomResult<CommandOutput> {
    execute_with_system(repo_root, args, &ProcessDoctorSystem, deploy_registry)
}

fn execute_with_system(
    repo_root: &Utf8Path,
    args: &DoctorArgs,
    system: &dyn DoctorSystem,
    deploy_registry: &DeployBackendRegistry,
) -> AtomResult<CommandOutput> {
    let report = collect_report(repo_root, system, deploy_registry);
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

fn collect_report(
    repo_root: &Utf8Path,
    system: &dyn DoctorSystem,
    deploy_registry: &DeployBackendRegistry,
) -> DoctorReport {
    let mut checks = vec![
        bazel_check(repo_root, system),
        rust_check(repo_root, system),
        mise_check(repo_root, system),
    ];
    let mut ready_platforms = Vec::new();

    for backend in deploy_registry.iter() {
        let report = backend.doctor(repo_root, system);
        if report.ready {
            ready_platforms.push(report.platform);
        }
        checks.extend(report.checks);
    }

    let issue_count = checks.iter().filter(|check| check.is_issue()).count();
    let critical_issue_count = checks
        .iter()
        .filter(|check| check.is_critical_issue())
        .count();

    DoctorReport {
        checks,
        ready_platforms,
        issue_count,
        critical_issue_count,
    }
}

fn bazel_check(repo_root: &Utf8Path, system: &dyn DoctorSystem) -> DoctorCheck {
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
                format!("Expected Bazel {expected}"),
                combined_command_output(&output),
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

fn rust_check(repo_root: &Utf8Path, system: &dyn DoctorSystem) -> DoctorCheck {
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
                combined_command_output(&output),
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

fn mise_check(repo_root: &Utf8Path, system: &dyn DoctorSystem) -> DoctorCheck {
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
            vec![combined_command_output(&output)],
        ),
        CommandInvocation::Output(output) => DoctorCheck::ok(
            "mise",
            "mise",
            DoctorSeverity::Recommended,
            first_version_token(&output.stdout).unwrap_or_else(|| "installed".to_owned()),
        ),
    }
}

fn read_expected_tool_version(
    repo_root: &Utf8Path,
    system: &dyn DoctorSystem,
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
    system: &dyn DoctorSystem,
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
    contents.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == tool).then(|| value.trim().trim_matches('"').to_owned())
    })
}

fn parse_bazel_build_label(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("Build label:").map(str::trim))
        .map(str::to_owned)
}

fn dotted_label(label: &str) -> String {
    format!(
        "{label} {}",
        ".".repeat(19usize.saturating_sub(label.len() + 1))
    )
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

    use atom_backends::{
        BackendAutomationSession, BackendDefinition, BackendDoctorReport, CapturedCommand,
        CommandInvocation, DeployBackend, DeployBackendRegistry, DoctorCheck, DoctorSeverity,
        DoctorSystem, LaunchMode, SessionLaunchBehavior, ToolRunner,
    };
    use atom_manifest::NormalizedManifest;
    use camino::Utf8Path;
    use serde_json::Value;

    use super::{DoctorArgs, execute_with_system};

    #[derive(Default)]
    struct FakeDoctorSystem {
        files: BTreeMap<String, String>,
        commands: BTreeMap<CommandKey, CommandInvocation>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct CommandKey {
        tool: String,
        args: Vec<String>,
    }

    struct FixtureDoctorBackend {
        id: &'static str,
        platform: &'static str,
        ready: bool,
        checks: Vec<DoctorCheck>,
    }

    impl FakeDoctorSystem {
        fn with_file(mut self, path: &str, contents: &str) -> Self {
            self.files.insert(path.to_owned(), contents.to_owned());
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

        fn env_var(&self, _key: &str) -> Option<String> {
            None
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

    impl BackendDefinition for FixtureDoctorBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn platform(&self) -> &'static str {
            self.platform
        }
    }

    impl DeployBackend for FixtureDoctorBackend {
        fn is_enabled(&self, _manifest: &NormalizedManifest) -> bool {
            true
        }

        fn doctor(&self, _repo_root: &Utf8Path, _system: &dyn DoctorSystem) -> BackendDoctorReport {
            BackendDoctorReport::new(self.platform, self.ready, self.checks.clone())
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
            unreachable!("doctor tests do not construct automation sessions")
        }
    }

    fn registry_with_backends(backends: Vec<FixtureDoctorBackend>) -> DeployBackendRegistry {
        let mut registry = DeployBackendRegistry::new();
        for backend in backends {
            registry
                .register(Box::new(backend))
                .expect("fixture backend should register");
        }
        registry
    }

    fn repo_root() -> &'static Utf8Path {
        Utf8Path::new("/repo")
    }

    #[test]
    fn clean_ios_only_host_is_non_critical() {
        let system = FakeDoctorSystem::default()
            .with_file("/repo/.bazelversion", "8.4.2\n")
            .with_file("/repo/mise.toml", "[tools]\nrust = \"1.92.0\"\n")
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
            .with_output("mise", &["--version"], "2025.12.13 macos-arm64\n", "");
        let registry = registry_with_backends(vec![
            FixtureDoctorBackend {
                id: "ios",
                platform: "ios",
                ready: true,
                checks: vec![
                    DoctorCheck::ok("xcode", "Xcode", DoctorSeverity::Platform, "16.2"),
                    DoctorCheck::ok(
                        "ios_simulators",
                        "iOS Simulators",
                        DoctorSeverity::Platform,
                        "2 available",
                    ),
                ],
            },
            FixtureDoctorBackend {
                id: "android",
                platform: "android",
                ready: false,
                checks: vec![
                    DoctorCheck::issue(
                        "android_sdk",
                        "Android SDK",
                        DoctorSeverity::Platform,
                        "not configured",
                        vec!["Set ANDROID_HOME or run: scripts/setup-android-sdk.sh".to_owned()],
                    ),
                    DoctorCheck::issue(
                        "java",
                        "Java",
                        DoctorSeverity::Platform,
                        "not found",
                        vec!["Install JDK 21+ or run: mise install java".to_owned()],
                    ),
                ],
            },
        ]);

        let output =
            execute_with_system(repo_root(), &DoctorArgs { json: false }, &system, &registry)
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
            .with_file("/repo/mise.toml", "[tools]\nrust = \"1.92.0\"\n")
            .with_missing("bazelisk", &["version"])
            .with_output(
                "rustc",
                &["--version"],
                "rustc 1.92.0 (hash 2025-12-08)\n",
                "",
            )
            .with_output("mise", &["--version"], "2025.12.13 macos-arm64\n", "");
        let registry = registry_with_backends(vec![FixtureDoctorBackend {
            id: "ios",
            platform: "ios",
            ready: true,
            checks: vec![
                DoctorCheck::ok("xcode", "Xcode", DoctorSeverity::Platform, "16.2"),
                DoctorCheck::ok(
                    "ios_simulators",
                    "iOS Simulators",
                    DoctorSeverity::Platform,
                    "1 available",
                ),
            ],
        }]);

        let output =
            execute_with_system(repo_root(), &DoctorArgs { json: false }, &system, &registry)
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
            .with_file("/repo/mise.toml", "[tools]\nrust = \"1.92.0\"\n")
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
            .with_missing("mise", &["--version"]);
        let registry = registry_with_backends(vec![
            FixtureDoctorBackend {
                id: "ios",
                platform: "ios",
                ready: true,
                checks: vec![
                    DoctorCheck::ok("xcode", "Xcode", DoctorSeverity::Platform, "16.2"),
                    DoctorCheck::ok(
                        "ios_simulators",
                        "iOS Simulators",
                        DoctorSeverity::Platform,
                        "1 available",
                    ),
                ],
            },
            FixtureDoctorBackend {
                id: "android",
                platform: "android",
                ready: true,
                checks: vec![
                    DoctorCheck::ok(
                        "android_sdk",
                        "Android SDK",
                        DoctorSeverity::Platform,
                        "1 devices available",
                    ),
                    DoctorCheck::ok("java", "Java", DoctorSeverity::Platform, "21.0.2"),
                ],
            },
        ]);

        let output =
            execute_with_system(repo_root(), &DoctorArgs { json: true }, &system, &registry)
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
    fn nonzero_rust_probe_reports_issue() {
        let system = FakeDoctorSystem::default()
            .with_file("/repo/.bazelversion", "8.4.2\n")
            .with_file("/repo/mise.toml", "[tools]\nrust = \"1.92.0\"\n")
            .with_output(
                "bazelisk",
                &["version"],
                "Bazelisk version: v1.28.1\nBuild label: 8.4.2\n",
                "",
            )
            .with_status("rustc", &["--version"], 1, "", "permission denied")
            .with_output("mise", &["--version"], "2025.12.13 macos-arm64\n", "");
        let registry = registry_with_backends(Vec::new());

        let output =
            execute_with_system(repo_root(), &DoctorArgs { json: false }, &system, &registry)
                .expect("doctor should render");
        let stdout = String::from_utf8(output.stdout).expect("utf8");

        assert_eq!(output.exit_code, 1);
        assert!(stdout.contains("Rust"));
        assert!(stdout.contains("permission denied"));
        assert!(stdout.contains("1 issue found (1 critical)"));
    }
}
