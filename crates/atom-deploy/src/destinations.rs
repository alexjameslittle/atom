use std::fmt::Write as _;

use atom_ffi::AtomResult;
use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::devices::android::{AndroidDestination, list_android_destinations};
use crate::devices::ios::{IosDestination, IosDestinationKind, list_ios_destinations};
use crate::tools::ToolRunner;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationPlatform {
    Ios,
    Android,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationKind {
    Simulator,
    Device,
    Emulator,
    Avd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationCapability {
    Launch,
    Logs,
    Screenshot,
    Video,
    InspectUi,
    Interact,
    Evaluate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationDescriptor {
    pub id: String,
    pub platform: DestinationPlatform,
    pub kind: DestinationKind,
    pub display_name: String,
    pub available: bool,
    pub debug_state: String,
    pub capabilities: Vec<DestinationCapability>,
}

/// # Errors
///
/// Returns an error if iOS or Android destination discovery fails.
pub fn list_destinations(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<DestinationDescriptor>> {
    let mut destinations = list_ios_destination_descriptors(repo_root, runner)?;
    destinations.extend(list_android_destination_descriptors(repo_root, runner)?);
    Ok(destinations)
}

/// # Errors
///
/// Returns an error if destination discovery for the requested platform fails.
pub fn list_platform_destinations(
    repo_root: &Utf8Path,
    platform: DestinationPlatform,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<DestinationDescriptor>> {
    match platform {
        DestinationPlatform::Ios => list_ios_destination_descriptors(repo_root, runner),
        DestinationPlatform::Android => list_android_destination_descriptors(repo_root, runner),
    }
}

#[must_use]
pub fn render_destination_lines(destinations: &[DestinationDescriptor]) -> String {
    let mut output = String::new();
    for destination in destinations {
        let capabilities = destination
            .capabilities
            .iter()
            .map(|capability| match capability {
                DestinationCapability::Launch => "launch",
                DestinationCapability::Logs => "logs",
                DestinationCapability::Screenshot => "screenshot",
                DestinationCapability::Video => "video",
                DestinationCapability::InspectUi => "inspect_ui",
                DestinationCapability::Interact => "interact",
                DestinationCapability::Evaluate => "evaluate",
            })
            .collect::<Vec<_>>()
            .join(",");
        let _ = writeln!(
            output,
            "{} [{} {:?}; available={}; state={}; capabilities={}]",
            destination.display_name,
            destination.id,
            destination.kind,
            destination.available,
            destination.debug_state,
            capabilities
        );
    }
    output
}

fn list_ios_destination_descriptors(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<DestinationDescriptor>> {
    Ok(list_ios_destinations(repo_root, runner)?
        .into_iter()
        .map(destination_descriptor_from_ios)
        .collect())
}

fn list_android_destination_descriptors(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
) -> AtomResult<Vec<DestinationDescriptor>> {
    Ok(list_android_destinations(repo_root, runner)?
        .into_iter()
        .map(destination_descriptor_from_android)
        .collect())
}

fn destination_descriptor_from_ios(destination: IosDestination) -> DestinationDescriptor {
    let display_name = destination.display_label();
    let id = destination.id.clone();
    let kind = match destination.kind {
        IosDestinationKind::Simulator => DestinationKind::Simulator,
        IosDestinationKind::Device => DestinationKind::Device,
    };
    let capabilities = match destination.kind {
        IosDestinationKind::Simulator => vec![
            DestinationCapability::Launch,
            DestinationCapability::Logs,
            DestinationCapability::Screenshot,
            DestinationCapability::Video,
            DestinationCapability::InspectUi,
            DestinationCapability::Interact,
            DestinationCapability::Evaluate,
        ],
        IosDestinationKind::Device => vec![DestinationCapability::Launch],
    };

    DestinationDescriptor {
        id,
        platform: DestinationPlatform::Ios,
        kind,
        display_name,
        available: destination.is_available,
        debug_state: destination.state,
        capabilities,
    }
}

pub(crate) fn android_destination_descriptor(
    destination: AndroidDestination,
) -> DestinationDescriptor {
    let display_name = destination.display_label();
    let id = destination.destination_id();
    let kind = if destination.state == "avd" {
        DestinationKind::Avd
    } else if destination.is_emulator {
        DestinationKind::Emulator
    } else {
        DestinationKind::Device
    };
    let capabilities = match kind {
        DestinationKind::Avd | DestinationKind::Emulator | DestinationKind::Device => vec![
            DestinationCapability::Launch,
            DestinationCapability::Logs,
            DestinationCapability::Screenshot,
            DestinationCapability::Video,
            DestinationCapability::InspectUi,
            DestinationCapability::Interact,
            DestinationCapability::Evaluate,
        ],
        DestinationKind::Simulator => Vec::new(),
    };

    DestinationDescriptor {
        id,
        platform: DestinationPlatform::Android,
        kind,
        display_name,
        available: destination.state == "device" || destination.state == "avd",
        debug_state: destination.state,
        capabilities,
    }
}

fn destination_descriptor_from_android(destination: AndroidDestination) -> DestinationDescriptor {
    android_destination_descriptor(destination)
}
