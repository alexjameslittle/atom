use atom_backends::{
    BackendDoctorReport, CommandInvocation, DoctorCheck, DoctorSeverity, DoctorSystem,
    combined_command_output, first_version_token,
};
use camino::Utf8Path;

const PLATFORM: &str = "android";

pub(crate) fn collect_doctor_report(
    repo_root: &Utf8Path,
    system: &dyn DoctorSystem,
) -> BackendDoctorReport {
    let android_sdk = android_sdk_check(repo_root, system);
    let java = java_check(repo_root, system);
    let ready = android_sdk.is_ok() && java.is_ok();

    BackendDoctorReport::new(PLATFORM, ready, vec![android_sdk, java])
}

fn android_sdk_check(repo_root: &Utf8Path, system: &dyn DoctorSystem) -> DoctorCheck {
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
            vec![combined_command_output(&output)],
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

fn java_check(repo_root: &Utf8Path, system: &dyn DoctorSystem) -> DoctorCheck {
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
            if looks_like_missing_java(&combined_command_output(&output)) {
                "not found"
            } else {
                "version check failed"
            },
            vec![
                "Install JDK 21+ or run: mise install java".to_owned(),
                combined_command_output(&output),
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

fn parse_java_major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

fn count_adb_devices(output: &str) -> usize {
    output
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("List of devices attached")
                && line.split_whitespace().nth(1) == Some("device")
        })
        .count()
}

fn looks_like_missing_java(detail: &str) -> bool {
    detail.contains("Unable to locate a Java Runtime")
        || detail.contains("No Java runtime present")
        || detail.contains("not found")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use atom_backends::{
        CapturedCommand, CommandInvocation, DoctorSeverity, DoctorStatus, DoctorSystem,
    };
    use camino::Utf8Path;

    use super::{collect_doctor_report, count_adb_devices};

    #[derive(Default)]
    struct FakeDoctorSystem {
        env: BTreeMap<String, String>,
        commands: BTreeMap<CommandKey, CommandInvocation>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct CommandKey {
        tool: String,
        args: Vec<String>,
    }

    impl FakeDoctorSystem {
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
        fn read_file(&self, _path: &Utf8Path) -> Result<String, String> {
            unreachable!("android doctor does not read files")
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

    #[test]
    fn ready_host_reports_android_platform_ready() {
        let system = FakeDoctorSystem::default()
            .with_env("ANDROID_HOME", "/Users/test/Library/Android/sdk")
            .with_output(
                "adb",
                &["devices", "-l"],
                "List of devices attached\nemulator-5554 device product:sdk_gphone64_arm64\n",
                "",
            )
            .with_output("javac", &["--version"], "javac 21.0.2\n", "");

        let report = collect_doctor_report(Utf8Path::new("/repo"), &system);

        assert!(report.ready);
        assert_eq!(report.platform, "android");
        assert_eq!(report.checks.len(), 2);
        assert_eq!(report.checks[0].status, DoctorStatus::Ok);
        assert_eq!(report.checks[0].summary, "1 devices available");
        assert_eq!(report.checks[1].severity, DoctorSeverity::Platform);
    }

    #[test]
    fn adb_parser_ignores_offline_and_headers() {
        let adb_output = "List of devices attached\nemulator-5554 device product:sdk_gphone64_arm64\nusb-serial offline transport_id:1\n";

        assert_eq!(count_adb_devices(adb_output), 1);
    }
}
