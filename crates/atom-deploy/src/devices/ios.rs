use atom_backends::ToolRunner;
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::Utf8Path;
use serde_json::Value;

use crate::devices::{choose_from_menu, should_prompt_interactively};
use crate::tools::{capture_tool, run_tool};

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
    pub architecture: Option<String>,
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
    runner: &mut (impl ToolRunner + ?Sized),
    requested_device: Option<&str>,
) -> AtomResult<IosDestination> {
    if let Some(requested_device) = requested_device {
        return resolve_requested_ios_destination(repo_root, runner, requested_device);
    }

    let destinations = list_ios_destinations(repo_root, runner)?;
    if should_prompt_interactively() {
        return choose_from_menu(
            "Select iOS destination",
            &destinations,
            IosDestination::display_label,
        );
    }

    select_default_ios_destination(&destinations).ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            "idb list-targets did not report an available iOS target",
        )
    })
}

fn resolve_requested_ios_destination(
    repo_root: &Utf8Path,
    runner: &mut (impl ToolRunner + ?Sized),
    requested_device: &str,
) -> AtomResult<IosDestination> {
    let destinations = list_ios_destinations(repo_root, runner)?;
    if requested_device == "booted" {
        return select_booted_ios_destination(&destinations)
            .or_else(|| select_default_ios_destination(&destinations))
            .ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    "idb list-targets did not report a booted iOS target",
                )
            });
    }

    if let Some(simulator) = destinations
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
        architecture: None,
        is_available: true,
    })
}

/// # Errors
///
/// Returns an error if `idb list-targets` fails or returns invalid JSON.
pub fn list_ios_simulators(
    repo_root: &Utf8Path,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<Vec<IosDestination>> {
    Ok(list_idb_targets(repo_root, runner)?
        .into_iter()
        .filter(|destination| {
            destination.kind == IosDestinationKind::Simulator && destination.is_available
        })
        .collect())
}

/// # Errors
///
/// Returns an error if `idb list-targets` fails or returns invalid JSON.
pub fn list_ios_destinations(
    repo_root: &Utf8Path,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<Vec<IosDestination>> {
    let mut destinations = list_idb_targets(repo_root, runner)?;
    sort_ios_destinations(&mut destinations);
    Ok(destinations)
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

pub(crate) fn sort_ios_destinations(destinations: &mut [IosDestination]) {
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
    runner: &mut (impl ToolRunner + ?Sized),
    destination: &IosDestination,
) -> AtomResult<String> {
    let simulator = destination.id.clone();
    if !destination.is_booted_simulator() {
        run_tool(runner, repo_root, "idb", &["boot", &simulator])?;
    }
    Ok(simulator)
}

fn list_idb_targets(
    repo_root: &Utf8Path,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<Vec<IosDestination>> {
    parse_idb_targets(&capture_tool(
        runner,
        repo_root,
        "idb",
        &["list-targets", "--json"],
    )?)
}

fn parse_idb_targets(json: &str) -> AtomResult<Vec<IosDestination>> {
    let targets = parse_idb_target_values(json)?;

    let mut destinations = Vec::new();
    for target in targets {
        let Some(id) = target
            .get("udid")
            .and_then(Value::as_str)
            .or_else(|| target.get("identifier").and_then(Value::as_str))
        else {
            continue;
        };
        let name = target
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_owned();
        let state = target
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let target_type = json_string(
            target
                .get("target_type")
                .or_else(|| target.get("targetType"))
                .or_else(|| target.get("type")),
        )
        .unwrap_or_else(|| "device".to_owned())
        .to_ascii_lowercase();
        let kind = if target_type.contains("simulator") {
            IosDestinationKind::Simulator
        } else {
            IosDestinationKind::Device
        };
        let is_available = !matches!(
            state.to_ascii_lowercase().as_str(),
            "unavailable" | "disconnected"
        );

        destinations.push(IosDestination {
            kind,
            id: id.to_owned(),
            alternate_id: None,
            name,
            state,
            runtime: if kind == IosDestinationKind::Simulator {
                json_string(
                    target
                        .get("os_version")
                        .or_else(|| target.get("osVersion"))
                        .or_else(|| target.get("runtime")),
                )
            } else {
                None
            },
            architecture: json_string(target.get("architecture")),
            is_available,
        });
    }

    Ok(destinations)
}

fn parse_idb_target_values(json: &str) -> AtomResult<Vec<Value>> {
    let trimmed = json.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if trimmed.starts_with('[') {
        let parsed: Value = serde_json::from_str(trimmed).map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to parse idb list-targets JSON: {error}"),
            )
        })?;
        return Ok(parsed.as_array().cloned().unwrap_or_default());
    }

    if trimmed.starts_with('{') && !trimmed.contains('\n') {
        let parsed: Value = serde_json::from_str(trimmed).map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to parse idb list-targets JSON: {error}"),
            )
        })?;
        if let Some(targets) = parsed.get("targets").and_then(Value::as_array) {
            return Ok(targets.clone());
        }
        return Ok(vec![parsed]);
    }

    trimmed
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to parse idb list-targets JSON line: {error}"),
                )
            })
        })
        .collect()
}

fn json_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        Some(Value::Bool(value)) => Some(value.to_string()),
        _ => None,
    }
}
