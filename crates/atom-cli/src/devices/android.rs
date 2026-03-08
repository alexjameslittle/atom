use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::Utf8Path;

use crate::devices::{choose_from_menu, should_prompt_interactively};
use crate::tools::{ToolRunner, capture_tool};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AndroidDestination {
    pub(crate) serial: String,
    pub(crate) state: String,
    pub(crate) model: Option<String>,
    pub(crate) device_name: Option<String>,
    pub(crate) is_emulator: bool,
}

impl AndroidDestination {
    pub(crate) fn display_label(&self) -> String {
        let kind = if self.is_emulator {
            "Emulator"
        } else {
            "Device"
        };
        let model = self.model.as_deref().or(self.device_name.as_deref());
        match model {
            Some(model) => format!("{kind}: {model} [{}]", self.serial),
            None => format!("{kind}: {}", self.serial),
        }
    }
}

pub(crate) fn resolve_android_device(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    requested_device: Option<&str>,
) -> AtomResult<Option<String>> {
    if let Some(requested_device) = requested_device {
        return Ok(Some(requested_device.to_owned()));
    }

    if !should_prompt_interactively() {
        return Ok(None);
    }

    let destinations = list_android_devices(repo_root, runner)?;
    if destinations.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            "adb did not report any connected emulators or devices",
        ));
    }
    choose_from_menu(
        "Select Android destination",
        &destinations,
        AndroidDestination::display_label,
    )
    .map(|destination| Some(destination.serial))
}

fn list_android_devices(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<AndroidDestination>> {
    Ok(
        parse_android_devices(&capture_tool(runner, repo_root, "adb", &["devices", "-l"])?)
            .into_iter()
            .filter(|destination| destination.state == "device")
            .collect(),
    )
}

pub(crate) fn parse_android_devices(output: &str) -> Vec<AndroidDestination> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("List of devices attached"))
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let serial = parts.next()?;
            let state = parts.next()?;
            let mut model = None;
            let mut device_name = None;
            for part in parts {
                if let Some(value) = part.strip_prefix("model:") {
                    model = Some(value.replace('_', " "));
                }
                if let Some(value) = part.strip_prefix("device:") {
                    device_name = Some(value.replace('_', " "));
                }
            }
            Some(AndroidDestination {
                serial: serial.to_owned(),
                state: state.to_owned(),
                model,
                device_name,
                is_emulator: serial.starts_with("emulator-"),
            })
        })
        .collect()
}
