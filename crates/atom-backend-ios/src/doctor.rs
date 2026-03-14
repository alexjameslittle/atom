use atom_backends::{
    BackendDoctorReport, CommandInvocation, DoctorCheck, DoctorSeverity, DoctorSystem,
    combined_command_output,
};
use camino::Utf8Path;
use serde_json::Value;

const PLATFORM: &str = "ios";

pub(crate) fn collect_doctor_report(
    repo_root: &Utf8Path,
    system: &dyn DoctorSystem,
) -> BackendDoctorReport {
    let xcode = xcode_check(repo_root, system);
    let simulators = ios_simulators_check(repo_root, system);
    let ready = xcode.is_ok() && simulators.is_ok();

    BackendDoctorReport::new(PLATFORM, ready, vec![xcode, simulators])
}

fn xcode_check(repo_root: &Utf8Path, system: &dyn DoctorSystem) -> DoctorCheck {
    let selected_path = match system.run_command(repo_root, "xcode-select", &["-p"]) {
        CommandInvocation::Missing => {
            return DoctorCheck::issue(
                "xcode",
                "Xcode",
                DoctorSeverity::Platform,
                "not found",
                vec![
                    "Install Xcode, then run xcode-select --switch /Applications/Xcode.app/Contents/Developer"
                        .to_owned(),
                ],
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
                    combined_command_output(&output),
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
            vec![combined_command_output(&output)],
        ),
        CommandInvocation::Output(output) => DoctorCheck::ok(
            "xcode",
            "Xcode",
            DoctorSeverity::Platform,
            parse_xcode_version(&output.stdout).unwrap_or(selected_path),
        ),
    }
}

fn ios_simulators_check(repo_root: &Utf8Path, system: &dyn DoctorSystem) -> DoctorCheck {
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
            vec![combined_command_output(&output)],
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

fn parse_xcode_version(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("Xcode ").map(str::trim))
        .map(str::to_owned)
}

fn count_available_simulators(output: &str) -> Option<usize> {
    let value: Value = serde_json::from_str(output).ok()?;
    let devices = value.get("devices")?.as_object()?;
    Some(
        devices
            .values()
            .filter_map(Value::as_array)
            .flatten()
            .filter(|device| {
                device
                    .get("isAvailable")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .count(),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use atom_backends::{
        CommandInvocation, DoctorSeverity, DoctorStatus, DoctorSystem, first_version_token,
    };
    use camino::Utf8Path;

    use super::{collect_doctor_report, count_available_simulators};

    #[derive(Default)]
    struct FakeDoctorSystem {
        commands: BTreeMap<CommandKey, CommandInvocation>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct CommandKey {
        tool: String,
        args: Vec<String>,
    }

    impl FakeDoctorSystem {
        fn with_output(mut self, tool: &str, args: &[&str], stdout: &str, stderr: &str) -> Self {
            self.commands.insert(
                CommandKey::new(tool, args),
                CommandInvocation::Output(atom_backends::CapturedCommand {
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
            unreachable!("ios doctor does not read files")
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

    #[test]
    fn ready_host_reports_ios_platform_ready() {
        let system = FakeDoctorSystem::default()
            .with_output(
                "xcode-select",
                &["-p"],
                "/Applications/Xcode.app/Contents/Developer\n",
                "",
            )
            .with_output(
                "xcodebuild",
                &["-version"],
                "Xcode 16.2\nBuild version 16C5032a\n",
                "",
            )
            .with_output(
                "xcrun",
                &["simctl", "list", "devices", "available", "--json"],
                "{\"devices\":{\"runtime\":[{\"isAvailable\":true},{\"isAvailable\":true}]}}",
                "",
            );

        let report = collect_doctor_report(Utf8Path::new("/repo"), &system);

        assert!(report.ready);
        assert_eq!(report.platform, "ios");
        assert_eq!(report.checks.len(), 2);
        assert_eq!(report.checks[0].status, DoctorStatus::Ok);
        assert_eq!(report.checks[1].summary, "2 available");
        assert_eq!(report.checks[1].severity, DoctorSeverity::Platform);
        assert_eq!(
            first_version_token(&report.checks[0].summary),
            Some("16.2".to_owned())
        );
    }

    #[test]
    fn simulator_counter_ignores_unavailable_entries() {
        let simulator_json = "{\"devices\":{\"runtime\":[{\"isAvailable\":true},{\"isAvailable\":false},{\"isAvailable\":true}]}}";

        assert_eq!(count_available_simulators(simulator_json), Some(2));
    }
}
