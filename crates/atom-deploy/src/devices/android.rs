use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::Utf8Path;

use crate::devices::{choose_from_menu, should_prompt_interactively};
use crate::tools::{ToolRunner, capture_tool};

const ANDROID_BOOT_TIMEOUT_ATTEMPTS: usize = 60;
const ANDROID_POLL_INTERVAL: Duration = Duration::from_secs(2);

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
    pub fn destination_id(&self) -> String {
        self.avd_name
            .as_deref()
            .map_or_else(|| self.serial.clone(), |avd| format!("avd:{avd}"))
    }

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
        if self.is_emulator
            && let Some(avd) = self.avd_name.as_deref()
        {
            return match model {
                Some(model) => format!("Emulator: {model} [AVD: {avd}; {}]", self.serial),
                None => format!("Emulator: {avd} [{}]", self.serial),
            };
        }
        match model {
            Some(model) => format!("{kind}: {model} [{kind}; {}]", self.serial),
            None => format!("{kind}: {} [{}]", self.serial, self.state),
        }
    }
}

pub(crate) fn find_android_destination(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    requested: &str,
) -> AtomResult<Option<AndroidDestination>> {
    let running_devices = list_android_devices(repo_root, runner)?;
    if let Some(avd_name) = requested.strip_prefix("avd:") {
        if let Some(destination) = running_devices
            .iter()
            .find(|destination| destination.avd_name.as_deref() == Some(avd_name))
            .cloned()
        {
            return Ok(Some(destination));
        }
        let avds = list_avds(repo_root, runner)?;
        return Ok(avds
            .into_iter()
            .find(|candidate| candidate == avd_name)
            .map(|avd_name| AndroidDestination {
                serial: format!("avd:{avd_name}"),
                state: "avd".to_owned(),
                model: None,
                device_name: None,
                is_emulator: true,
                avd_name: Some(avd_name),
            }));
    }
    Ok(running_devices
        .into_iter()
        .find(|destination| destination.serial == requested))
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
        if requested.strip_prefix("avd:").is_some() {
            if let Some(destination) = find_android_destination(repo_root, runner, requested)? {
                return Ok(destination);
            }
            return Err(AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                format!("unknown Android destination id: {requested}"),
                requested,
            ));
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
        if destination.is_emulator {
            wait_for_android_boot(repo_root, runner, &destination.serial)?;
        }
        return Ok(destination.serial.clone());
    }

    let avd = destination.avd_name.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "Android destination has no AVD name and is not running",
        )
    })?;

    if let Some(serial) = running_emulator_serial_for_avd(repo_root, runner, avd)? {
        wait_for_android_boot(repo_root, runner, &serial)?;
        return Ok(serial);
    }

    Command::new("emulator")
        .args(["-avd", avd, "-no-snapshot-load"])
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

    let serial = wait_for_android_emulator_serial(repo_root, runner, avd)?;
    wait_for_android_boot(repo_root, runner, &serial)?;
    Ok(serial)
}

pub(crate) fn list_android_devices(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<AndroidDestination>> {
    let mut destinations =
        parse_android_devices(&capture_tool(runner, repo_root, "adb", &["devices", "-l"])?)
            .into_iter()
            .filter(|destination| destination.state == "device")
            .collect::<Vec<_>>();
    for destination in &mut destinations {
        if destination.is_emulator {
            destination.avd_name = emulator_avd_name(repo_root, runner, &destination.serial);
        }
    }
    Ok(destinations)
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

fn running_emulator_serial_for_avd(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    avd_name: &str,
) -> AtomResult<Option<String>> {
    Ok(list_android_devices(repo_root, runner)?
        .into_iter()
        .find(|destination| {
            destination.is_emulator && destination.avd_name.as_deref() == Some(avd_name)
        })
        .map(|destination| destination.serial))
}

fn wait_for_android_emulator_serial(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    avd_name: &str,
) -> AtomResult<String> {
    for _ in 0..ANDROID_BOOT_TIMEOUT_ATTEMPTS {
        if let Some(serial) = running_emulator_serial_for_avd(repo_root, runner, avd_name)? {
            return Ok(serial);
        }
        thread::sleep(ANDROID_POLL_INTERVAL);
    }
    Err(AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        format!("Android emulator {avd_name} started but was not found by adb"),
    ))
}

fn wait_for_android_boot(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    serial: &str,
) -> AtomResult<()> {
    for _ in 0..ANDROID_BOOT_TIMEOUT_ATTEMPTS {
        if let Ok(output) = capture_tool(
            runner,
            repo_root,
            "adb",
            &["-s", serial, "shell", "getprop", "sys.boot_completed"],
        ) && output.trim() == "1"
        {
            return Ok(());
        }
        thread::sleep(ANDROID_POLL_INTERVAL);
    }
    Err(AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        format!("Android emulator {serial} did not finish booting within 120 seconds"),
    ))
}

fn emulator_avd_name(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    serial: &str,
) -> Option<String> {
    capture_tool(
        runner,
        repo_root,
        "adb",
        &["-s", serial, "emu", "avd", "name"],
    )
    .ok()
    .and_then(|output| parse_emulator_avd_name(&output))
}

fn parse_emulator_avd_name(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.eq_ignore_ascii_case("ok"))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use camino::{Utf8Path, Utf8PathBuf};

    use super::{
        AndroidDestination, find_android_destination, parse_emulator_avd_name,
        prepare_android_emulator,
    };
    use crate::tools::ToolRunner;

    #[derive(Default)]
    struct FakeToolRunner {
        calls: Vec<(String, Vec<String>)>,
        captures: VecDeque<String>,
    }

    impl ToolRunner for FakeToolRunner {
        fn run(
            &mut self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            Ok(())
        }

        fn capture(
            &mut self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            Ok(self
                .captures
                .pop_front()
                .expect("expected captured output for command"))
        }

        fn capture_json_file(
            &mut self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            Ok(self
                .captures
                .pop_front()
                .expect("expected captured output for command"))
        }

        fn stream(
            &mut self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            self.calls.push((tool.to_owned(), args.to_vec()));
            Ok(())
        }
    }

    fn root() -> Utf8PathBuf {
        Utf8PathBuf::from(".")
    }

    #[test]
    fn running_emulator_uses_stable_avd_destination_id() {
        let destination = AndroidDestination {
            serial: "emulator-5554".to_owned(),
            state: "device".to_owned(),
            model: Some("Pixel 9".to_owned()),
            device_name: None,
            is_emulator: true,
            avd_name: Some("atom_35".to_owned()),
        };

        assert_eq!(destination.destination_id(), "avd:atom_35");
        assert_eq!(
            destination.display_label(),
            "Emulator: Pixel 9 [AVD: atom_35; emulator-5554]"
        );
    }

    #[test]
    fn requested_avd_resolves_to_running_emulator() {
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                "List of devices attached\nemulator-5554\tdevice model:Pixel_9 device:emu64a\n"
                    .to_owned(),
                "atom_35\n".to_owned(),
            ]),
        };

        let destination = find_android_destination(&root(), &mut runner, "avd:atom_35")
            .expect("lookup")
            .expect("destination");

        assert_eq!(destination.serial, "emulator-5554");
        assert_eq!(destination.avd_name.as_deref(), Some("atom_35"));
    }

    #[test]
    fn prepare_android_emulator_reuses_matching_running_avd() {
        let mut runner = FakeToolRunner {
            calls: Vec::new(),
            captures: VecDeque::from([
                "List of devices attached\nemulator-5554\tdevice model:Pixel_9 device:emu64a\n"
                    .to_owned(),
                "atom_35\n".to_owned(),
                "1\n".to_owned(),
            ]),
        };
        let destination = AndroidDestination {
            serial: "avd:atom_35".to_owned(),
            state: "avd".to_owned(),
            model: None,
            device_name: None,
            is_emulator: true,
            avd_name: Some("atom_35".to_owned()),
        };

        let serial =
            prepare_android_emulator(&root(), &mut runner, &destination).expect("reuse emulator");

        assert_eq!(serial, "emulator-5554");
        assert_eq!(
            runner.calls,
            vec![
                (
                    "adb".to_owned(),
                    vec!["devices".to_owned(), "-l".to_owned()],
                ),
                (
                    "adb".to_owned(),
                    vec![
                        "-s".to_owned(),
                        "emulator-5554".to_owned(),
                        "emu".to_owned(),
                        "avd".to_owned(),
                        "name".to_owned(),
                    ],
                ),
                (
                    "adb".to_owned(),
                    vec![
                        "-s".to_owned(),
                        "emulator-5554".to_owned(),
                        "shell".to_owned(),
                        "getprop".to_owned(),
                        "sys.boot_completed".to_owned(),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn parse_emulator_avd_name_ignores_empty_and_ok_lines() {
        assert_eq!(
            parse_emulator_avd_name("\nOK\natom_35\n"),
            Some("atom_35".to_owned())
        );
    }
}
