use std::fs;
use std::io;
use std::process::Command;

use camino::Utf8Path;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorStatus {
    Ok,
    Issue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSeverity {
    Critical,
    Platform,
    Recommended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorCheck {
    pub id: String,
    pub label: String,
    pub status: DoctorStatus,
    pub severity: DoctorSeverity,
    pub summary: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub remediation: Vec<String>,
}

impl DoctorCheck {
    #[must_use]
    pub fn ok(
        id: impl Into<String>,
        label: impl Into<String>,
        severity: DoctorSeverity,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            status: DoctorStatus::Ok,
            severity,
            summary: summary.into(),
            remediation: Vec::new(),
        }
    }

    #[must_use]
    pub fn issue(
        id: impl Into<String>,
        label: impl Into<String>,
        severity: DoctorSeverity,
        summary: impl Into<String>,
        remediation: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            status: DoctorStatus::Issue,
            severity,
            summary: summary.into(),
            remediation,
        }
    }

    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.status == DoctorStatus::Ok
    }

    #[must_use]
    pub fn is_issue(&self) -> bool {
        self.status == DoctorStatus::Issue
    }

    #[must_use]
    pub fn is_critical_issue(&self) -> bool {
        self.status == DoctorStatus::Issue && self.severity == DoctorSeverity::Critical
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendDoctorReport {
    pub platform: String,
    pub ready: bool,
    pub checks: Vec<DoctorCheck>,
}

impl BackendDoctorReport {
    #[must_use]
    pub fn new(platform: impl Into<String>, ready: bool, checks: Vec<DoctorCheck>) -> Self {
        Self {
            platform: platform.into(),
            ready,
            checks,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedCommand {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandInvocation {
    Output(CapturedCommand),
    Missing,
    Failed(String),
}

#[expect(
    clippy::missing_errors_doc,
    reason = "Trait methods document the shared doctor probing contract once at the trait boundary."
)]
pub trait DoctorSystem {
    fn read_file(&self, path: &Utf8Path) -> Result<String, String>;
    fn env_var(&self, key: &str) -> Option<String>;
    fn run_command(&self, repo_root: &Utf8Path, tool: &str, args: &[&str]) -> CommandInvocation;
}

pub struct ProcessDoctorSystem;

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

#[must_use]
pub fn combined_command_output(output: &CapturedCommand) -> String {
    let stdout = output.stdout.trim();
    let stderr = output.stderr.trim();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n{stderr}"),
        (false, true) => stdout.to_owned(),
        (true, false) => stderr.to_owned(),
        (true, true) => format!("command exited with status {}", output.status),
    }
}

#[must_use]
pub fn first_version_token(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        token
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
            .then(|| token.to_owned())
    })
}
