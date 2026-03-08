use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::Utf8Path;
use serde_json::Value;

use crate::devices::{choose_from_menu, should_prompt_interactively};
use crate::tools::{ToolRunner, capture_json_tool, capture_tool, run_tool};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IosSimulator {
    pub runtime: String,
    pub name: String,
    pub udid: String,
    pub state: String,
    pub is_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IosDestinationKind {
    Simulator,
    Device,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IosDestination {
    pub kind: IosDestinationKind,
    pub id: String,
    pub alternate_id: Option<String>,
    pub name: String,
    pub state: String,
    pub runtime: Option<String>,
    pub is_available: bool,
}

impl IosDestination {
    #[must_use]
    pub fn matches_identifier(&self, value: &str) -> bool {
        self.id == value || self.alternate_id.as_deref() == Some(value) || self.name == value
    }

    #[must_use]
    pub fn is_booted_simulator(&self) -> bool {
        self.kind == IosDestinationKind::Simulator && self.state == "Booted"
    }

    #[must_use]
    pub fn display_label(&self) -> String {
        match self.kind {
            IosDestinationKind::Simulator => match &self.runtime {
                Some(runtime) => format!("Simulator: {} [{}; {}]", self.name, runtime, self.state),
                None => format!("Simulator: {} [{}]", self.name, self.state),
            },
            IosDestinationKind::Device => format!("Device: {} [{}]", self.name, self.state),
        }
    }
}

/// # Errors
///
/// Returns an error if no simulators or devices are available.
pub fn resolve_ios_destination(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    requested_device: Option<&str>,
) -> AtomResult<IosDestination> {
    if let Some(requested_device) = requested_device {
        return resolve_requested_ios_destination(repo_root, runner, requested_device);
    }

    let simulators = list_ios_simulators(repo_root, runner)?;
    if should_prompt_interactively() {
        let mut destinations = simulators;
        destinations.extend(list_ios_physical_devices(repo_root, runner).unwrap_or_default());
        sort_ios_destinations(&mut destinations);
        return choose_from_menu(
            "Select iOS destination",
            &destinations,
            IosDestination::display_label,
        );
    }

    select_default_ios_destination(&simulators).ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            "xcrun simctl did not report an available simulator",
        )
    })
}

fn resolve_requested_ios_destination(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    requested_device: &str,
) -> AtomResult<IosDestination> {
    let simulators = list_ios_simulators(repo_root, runner)?;
    if requested_device == "booted" {
        return select_booted_ios_destination(&simulators)
            .or_else(|| select_default_ios_destination(&simulators))
            .ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    "xcrun simctl did not report a bootable simulator",
                )
            });
    }

    if let Some(simulator) = simulators
        .into_iter()
        .find(|simulator| simulator.matches_identifier(requested_device))
    {
        return Ok(simulator);
    }

    Ok(IosDestination {
        kind: IosDestinationKind::Device,
        id: requested_device.to_owned(),
        alternate_id: None,
        name: requested_device.to_owned(),
        state: "requested".to_owned(),
        runtime: None,
        is_available: true,
    })
}

/// # Errors
///
/// Returns an error if `xcrun simctl` fails or returns invalid JSON.
pub fn list_ios_simulators(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<IosDestination>> {
    Ok(parse_ios_simulators(&capture_tool(
        runner,
        repo_root,
        "xcrun",
        &["simctl", "list", "devices", "available", "-j"],
    )?)?
    .into_iter()
    .filter(|simulator| simulator.is_available)
    .map(|simulator| IosDestination {
        kind: IosDestinationKind::Simulator,
        id: simulator.udid.clone(),
        alternate_id: None,
        name: simulator.name,
        state: simulator.state,
        runtime: Some(simulator.runtime),
        is_available: simulator.is_available,
    })
    .collect())
}

fn list_ios_physical_devices(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<IosDestination>> {
    parse_ios_physical_devices(&capture_json_tool(
        runner,
        repo_root,
        "xcrun",
        &["devicectl", "list", "devices"],
    )?)
}

#[must_use]
pub fn select_booted_ios_destination(destinations: &[IosDestination]) -> Option<IosDestination> {
    destinations
        .iter()
        .find(|destination| destination.is_booted_simulator())
        .cloned()
}

#[must_use]
pub fn select_default_ios_destination(destinations: &[IosDestination]) -> Option<IosDestination> {
    let mut simulators = destinations
        .iter()
        .filter(|destination| destination.kind == IosDestinationKind::Simulator)
        .cloned()
        .collect::<Vec<_>>();
    simulators.sort_by(|left, right| {
        right
            .runtime
            .cmp(&left.runtime)
            .then_with(|| right.is_booted_simulator().cmp(&left.is_booted_simulator()))
            .then_with(|| left.name.cmp(&right.name))
    });

    select_booted_ios_destination(&simulators)
        .or_else(|| {
            simulators
                .iter()
                .find(|simulator| simulator.name.contains("iPhone"))
                .cloned()
        })
        .or_else(|| simulators.first().cloned())
}

fn sort_ios_destinations(destinations: &mut [IosDestination]) {
    destinations.sort_by(|left, right| {
        right
            .is_available
            .cmp(&left.is_available)
            .then_with(|| right.is_booted_simulator().cmp(&left.is_booted_simulator()))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| right.runtime.cmp(&left.runtime))
            .then_with(|| left.name.cmp(&right.name))
    });
}

/// # Errors
///
/// Returns an error if the simulator cannot be booted.
pub fn prepare_ios_simulator(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    destination: &IosDestination,
) -> AtomResult<String> {
    let simulator = destination.id.clone();
    if !destination.is_booted_simulator() {
        run_tool(runner, repo_root, "xcrun", &["simctl", "boot", &simulator])?;
        run_tool(
            runner,
            repo_root,
            "xcrun",
            &["simctl", "bootstatus", &simulator, "-b"],
        )?;
    }
    Ok(simulator)
}

/// # Errors
///
/// Returns an error if the JSON is malformed or missing the devices map.
pub fn parse_ios_simulators(json: &str) -> AtomResult<Vec<IosSimulator>> {
    let parsed: Value = serde_json::from_str(json).map_err(|error| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to parse xcrun simctl JSON: {error}"),
        )
    })?;
    let devices = parsed
        .get("devices")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "xcrun simctl JSON did not contain a devices map",
            )
        })?;

    let mut simulators = Vec::new();
    for (runtime, entries) in devices {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            let Some(name) = entry.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(udid) = entry.get("udid").and_then(Value::as_str) else {
                continue;
            };
            simulators.push(IosSimulator {
                runtime: runtime.clone(),
                name: name.to_owned(),
                udid: udid.to_owned(),
                state: entry
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                is_available: entry
                    .get("isAvailable")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            });
        }
    }
    Ok(simulators)
}

/// # Errors
///
/// Returns an error if the JSON is malformed or missing the devices array.
pub fn parse_ios_physical_devices(json: &str) -> AtomResult<Vec<IosDestination>> {
    let parsed: Value = serde_json::from_str(json).map_err(|error| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to parse xcrun devicectl JSON: {error}"),
        )
    })?;
    let devices = parsed
        .get("result")
        .and_then(|result| result.get("devices"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                "xcrun devicectl JSON did not contain a devices array",
            )
        })?;

    let mut destinations = Vec::new();
    for device in devices {
        let platform = device
            .get("hardwareProperties")
            .and_then(|value| value.get("platform"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if platform != "iOS" {
            continue;
        }

        let identifier = device.get("identifier").and_then(Value::as_str);
        let udid = device
            .get("hardwareProperties")
            .and_then(|value| value.get("udid"))
            .and_then(Value::as_str);
        let name = device
            .get("deviceProperties")
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str);
        let ddi_available = device
            .get("deviceProperties")
            .and_then(|value| value.get("ddiServicesAvailable"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let tunnel_state = device
            .get("connectionProperties")
            .and_then(|value| value.get("tunnelState"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        let Some(primary_id) = udid.or(identifier) else {
            continue;
        };
        let Some(name) = name else {
            continue;
        };
        let is_available = ddi_available || tunnel_state == "connected";
        if !is_available {
            continue;
        }

        destinations.push(IosDestination {
            kind: IosDestinationKind::Device,
            id: primary_id.to_owned(),
            alternate_id: identifier
                .filter(|identifier| *identifier != primary_id)
                .map(str::to_owned),
            name: name.to_owned(),
            state: if ddi_available {
                "ready".to_owned()
            } else {
                tunnel_state.to_owned()
            },
            runtime: None,
            is_available,
        });
    }
    Ok(destinations)
}
