use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::Utf8Path;

use crate::devices::{choose_from_menu, should_prompt_interactively};
use crate::tools::{ToolRunner, capture_tool, run_tool};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidDestination {
    pub serial: String,
    pub state: String,
    pub model: Option<String>,
    pub device_name: Option<String>,
    pub is_emulator: bool,
    pub avd_name: Option<String>,
}

impl AndroidDestination {
    #[must_use]
    pub fn display_label(&self) -> String {
        if self.state == "avd" {
            let avd = self.avd_name.as_deref().unwrap_or("unknown");
            return format!("AVD: {avd} (not running)");
        }
        let kind = if self.is_emulator {
            "Emulator"
        } else {
            "Device"
        };
        let model = self.model.as_deref().or(self.device_name.as_deref());
        match model {
            Some(model) => format!("{kind}: {model} [{kind}; {}]", self.serial),
            None => format!("{kind}: {} [{}]", self.serial, self.state),
        }
    }
}

/// # Errors
///
/// Returns an error if no devices or AVDs are available.
pub fn resolve_android_device(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    requested_device: Option<&str>,
) -> AtomResult<AndroidDestination> {
    if let Some(requested) = requested_device {
        if let Some(avd_name) = requested.strip_prefix("avd:") {
            return Ok(AndroidDestination {
                serial: requested.to_owned(),
                state: "avd".to_owned(),
                model: None,
                device_name: None,
                is_emulator: true,
                avd_name: Some(avd_name.to_owned()),
            });
        }
        return Ok(AndroidDestination {
            serial: requested.to_owned(),
            state: "device".to_owned(),
            model: None,
            device_name: None,
            is_emulator: requested.starts_with("emulator-"),
            avd_name: None,
        });
    }

    let mut destinations = list_android_devices(repo_root, runner)?;

    // Include available AVDs that aren't already running, mirroring how iOS
    // lists non-booted simulators alongside booted ones.
    let running_avds = destinations
        .iter()
        .filter_map(|d| d.avd_name.as_deref())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if let Ok(avds) = list_avds(repo_root, runner) {
        for avd in avds {
            if !running_avds.contains(&avd) {
                destinations.push(AndroidDestination {
                    serial: String::new(),
                    state: "avd".to_owned(),
                    model: None,
                    device_name: None,
                    is_emulator: true,
                    avd_name: Some(avd),
                });
            }
        }
    }

    if destinations.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            "no Android devices or AVDs found — run scripts/setup-android-sdk.sh first",
        ));
    }

    if should_prompt_interactively() {
        return choose_from_menu(
            "Select Android destination",
            &destinations,
            AndroidDestination::display_label,
        );
    }

    select_default_android_destination(&destinations)
}

/// # Errors
///
/// Returns an error if `adb devices` cannot be read or parsed.
pub fn list_android_destinations(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<AndroidDestination>> {
    let mut destinations = list_android_devices(repo_root, runner)?;
    let running_avds = destinations
        .iter()
        .filter_map(|d| d.avd_name.as_deref())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if let Ok(avds) = list_avds(repo_root, runner) {
        for avd in avds {
            if !running_avds.contains(&avd) {
                destinations.push(AndroidDestination {
                    serial: format!("avd:{avd}"),
                    state: "avd".to_owned(),
                    model: None,
                    device_name: None,
                    is_emulator: true,
                    avd_name: Some(avd),
                });
            }
        }
    }
    Ok(destinations)
}

fn select_default_android_destination(
    destinations: &[AndroidDestination],
) -> AtomResult<AndroidDestination> {
    // Prefer a running device/emulator, then fall back to an AVD.
    destinations
        .iter()
        .find(|d| d.state == "device")
        .or_else(|| destinations.iter().find(|d| d.state == "avd"))
        .cloned()
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "no Android devices or AVDs available",
            )
        })
}

/// Ensure the selected Android destination is running. If it is an offline AVD,
/// launch the emulator and wait for the device to boot.
///
/// # Errors
///
/// Returns an error if the emulator cannot be launched or fails to boot.
pub fn prepare_android_emulator(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    destination: &AndroidDestination,
) -> AtomResult<String> {
    if destination.state == "device" {
        return Ok(destination.serial.clone());
    }

    let avd = destination.avd_name.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "Android destination has no AVD name and is not running",
        )
    })?;

    Command::new("emulator")
        .args(["-avd", avd])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to launch Android emulator: {error}"),
            )
        })?;

    // Wait for the emulator to appear as a device.
    run_tool(runner, repo_root, "adb", &["wait-for-device"])?;

    // Wait for boot to complete.
    wait_for_android_boot(repo_root, runner)?;

    // Return the serial of the emulator.
    let devices = list_android_devices(repo_root, runner)?;
    devices
        .into_iter()
        .find(|d| d.is_emulator)
        .map(|d| d.serial)
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "Android emulator started but was not found by adb",
            )
        })
}

pub(crate) fn list_android_devices(
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

#[must_use]
pub fn parse_android_devices(output: &str) -> Vec<AndroidDestination> {
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
                avd_name: None,
            })
        })
        .collect()
}

pub(crate) fn list_avds(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<String>> {
    let output = capture_tool(runner, repo_root, "emulator", &["-list-avds"])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn wait_for_android_boot(repo_root: &Utf8Path, runner: &mut impl ToolRunner) -> AtomResult<()> {
    for _ in 0..60 {
        if let Ok(output) = capture_tool(
            runner,
            repo_root,
            "adb",
            &["shell", "getprop", "sys.boot_completed"],
        ) && output.trim() == "1"
        {
            return Ok(());
        }
        thread::sleep(Duration::from_secs(2));
    }
    Err(AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        "Android emulator did not finish booting within 120 seconds",
    ))
}
