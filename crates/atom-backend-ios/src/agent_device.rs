use std::thread;
use std::time::Duration;

use atom_backends::{
    InteractionRequest, InteractionResult, ToolRunner, UiBounds, UiNode, UiSnapshot,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::Utf8Path;
use serde_json::Value;

use crate::deploy::resolve_interaction_point;

const TOOL: &str = "agent-device";
const ACTION_SETTLE_DELAY: Duration = Duration::from_millis(250);
const LONG_PRESS_DURATION_MS: &str = "1000";
const SWIPE_DURATION_MS: &str = "300";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentDeviceDestinationKind {
    Simulator,
    Device,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentDeviceDestination {
    pub(crate) kind: AgentDeviceDestinationKind,
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) state: String,
}

pub(crate) fn is_available(repo_root: &Utf8Path, runner: &mut (impl ToolRunner + ?Sized)) -> bool {
    runner
        .capture(repo_root, TOOL, &["--version".to_owned()])
        .is_ok()
}

pub(crate) fn list_destinations(
    repo_root: &Utf8Path,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<Vec<AgentDeviceDestination>> {
    let raw = runner.capture(
        repo_root,
        TOOL,
        &[
            "devices".to_owned(),
            "--platform".to_owned(),
            "ios".to_owned(),
            "--json".to_owned(),
        ],
    )?;
    parse_destinations(&raw)
}

pub(crate) fn open_app(
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    bundle_id: &str,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    run_agent_device(
        runner,
        repo_root,
        destination_id,
        session_name,
        &["open".to_owned(), bundle_id.to_owned()],
    )
}

pub(crate) fn close_session(
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    run_agent_device(
        runner,
        repo_root,
        destination_id,
        session_name,
        &["close".to_owned()],
    )
}

pub(crate) fn inspect_ui_with_agent_device(
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<UiSnapshot> {
    let raw = capture_agent_device(
        runner,
        repo_root,
        destination_id,
        session_name,
        &["snapshot".to_owned()],
    )?;
    parse_snapshot(&raw)
}

#[expect(
    clippy::too_many_lines,
    reason = "The agent-device adapter keeps command translation in one iOS-specific place."
)]
pub(crate) fn interact_with_agent_device(
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    runner: &mut (impl ToolRunner + ?Sized),
    request: InteractionRequest,
) -> AtomResult<InteractionResult> {
    match request {
        InteractionRequest::InspectUi => Ok(InteractionResult {
            ok: true,
            snapshot: inspect_ui_with_agent_device(
                repo_root,
                destination_id,
                session_name,
                runner,
            )?,
            message: None,
        }),
        InteractionRequest::Tap { target_id, x, y } => {
            let snapshot =
                inspect_ui_with_agent_device(repo_root, destination_id, session_name, runner)?;
            let (tap_x, tap_y) = resolve_interaction_point(&snapshot, target_id.as_deref(), x, y)?;
            run_agent_device(
                runner,
                repo_root,
                destination_id,
                session_name,
                &[
                    "click".to_owned(),
                    format_coordinate(tap_x),
                    format_coordinate(tap_y),
                ],
            )?;
            thread::sleep(ACTION_SETTLE_DELAY);
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_agent_device(
                    repo_root,
                    destination_id,
                    session_name,
                    runner,
                )?,
                message: None,
            })
        }
        InteractionRequest::LongPress { target_id, x, y } => {
            let snapshot =
                inspect_ui_with_agent_device(repo_root, destination_id, session_name, runner)?;
            let (press_x, press_y) =
                resolve_interaction_point(&snapshot, target_id.as_deref(), x, y)?;
            run_agent_device(
                runner,
                repo_root,
                destination_id,
                session_name,
                &[
                    "longpress".to_owned(),
                    format_coordinate(press_x),
                    format_coordinate(press_y),
                    LONG_PRESS_DURATION_MS.to_owned(),
                ],
            )?;
            thread::sleep(ACTION_SETTLE_DELAY);
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_agent_device(
                    repo_root,
                    destination_id,
                    session_name,
                    runner,
                )?,
                message: None,
            })
        }
        InteractionRequest::TypeText { target_id, text } => {
            if let Some(target_id) = target_id.as_deref() {
                let snapshot =
                    inspect_ui_with_agent_device(repo_root, destination_id, session_name, runner)?;
                let (tap_x, tap_y) =
                    resolve_interaction_point(&snapshot, Some(target_id), None, None)?;
                run_agent_device(
                    runner,
                    repo_root,
                    destination_id,
                    session_name,
                    &[
                        "click".to_owned(),
                        format_coordinate(tap_x),
                        format_coordinate(tap_y),
                    ],
                )?;
                thread::sleep(Duration::from_millis(150));
            }
            run_agent_device(
                runner,
                repo_root,
                destination_id,
                session_name,
                &["type".to_owned(), text],
            )?;
            thread::sleep(ACTION_SETTLE_DELAY);
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_agent_device(
                    repo_root,
                    destination_id,
                    session_name,
                    runner,
                )?,
                message: None,
            })
        }
        InteractionRequest::Swipe { x, y } | InteractionRequest::Drag { x, y } => {
            let snapshot =
                inspect_ui_with_agent_device(repo_root, destination_id, session_name, runner)?;
            let start_x = snapshot.screen.width / 2.0;
            let start_y = snapshot.screen.height * 0.75;
            let end_x = x.unwrap_or(start_x);
            let end_y = y.unwrap_or(snapshot.screen.height * 0.25);
            run_agent_device(
                runner,
                repo_root,
                destination_id,
                session_name,
                &[
                    "swipe".to_owned(),
                    format_coordinate(start_x),
                    format_coordinate(start_y),
                    format_coordinate(end_x),
                    format_coordinate(end_y),
                    SWIPE_DURATION_MS.to_owned(),
                ],
            )?;
            thread::sleep(ACTION_SETTLE_DELAY);
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_agent_device(
                    repo_root,
                    destination_id,
                    session_name,
                    runner,
                )?,
                message: None,
            })
        }
    }
}

pub(crate) fn capture_screenshot(
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    output_path: &Utf8Path,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    run_agent_device(
        runner,
        repo_root,
        destination_id,
        session_name,
        &[
            "screenshot".to_owned(),
            "--out".to_owned(),
            output_path.as_str().to_owned(),
        ],
    )
}

pub(crate) fn start_video_capture(
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    output_path: &Utf8Path,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    run_agent_device(
        runner,
        repo_root,
        destination_id,
        session_name,
        &[
            "record".to_owned(),
            "start".to_owned(),
            output_path.as_str().to_owned(),
        ],
    )
}

pub(crate) fn stop_video_capture(
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    run_agent_device(
        runner,
        repo_root,
        destination_id,
        session_name,
        &["record".to_owned(), "stop".to_owned()],
    )
}

fn run_agent_device(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    subcommand: &[String],
) -> AtomResult<()> {
    let args = agent_device_args(destination_id, session_name, subcommand);
    runner.run(repo_root, TOOL, &args)
}

fn capture_agent_device(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    destination_id: &str,
    session_name: &str,
    subcommand: &[String],
) -> AtomResult<String> {
    let args = agent_device_args(destination_id, session_name, subcommand);
    runner.capture(repo_root, TOOL, &args)
}

fn agent_device_args(
    destination_id: &str,
    session_name: &str,
    subcommand: &[String],
) -> Vec<String> {
    let mut args = Vec::with_capacity(subcommand.len() + 7);
    args.extend(subcommand.iter().cloned());
    args.extend([
        "--platform".to_owned(),
        "ios".to_owned(),
        "--udid".to_owned(),
        destination_id.to_owned(),
        "--session".to_owned(),
        session_name.to_owned(),
        "--json".to_owned(),
    ]);
    args
}

fn parse_snapshot(raw: &str) -> AtomResult<UiSnapshot> {
    let parsed: Value = serde_json::from_str(raw).map_err(|error| {
        AtomError::new(
            AtomErrorCode::AutomationUnavailable,
            format!("failed to parse agent-device snapshot JSON: {error}"),
        )
    })?;
    let nodes = parsed
        .get("data")
        .and_then(|data| data.get("nodes"))
        .and_then(Value::as_array)
        .map_or_else(|| &[][..], Vec::as_slice)
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| node_from_value(entry, index))
        .collect::<Vec<_>>();

    let mut width = 0.0_f64;
    let mut height = 0.0_f64;
    for node in &nodes {
        width = width.max(node.bounds.x + node.bounds.width);
        height = height.max(node.bounds.y + node.bounds.height);
    }

    Ok(UiSnapshot {
        screen: atom_backends::ScreenInfo {
            width: width.max(1.0),
            height: height.max(1.0),
        },
        nodes,
        screenshot_path: None,
    })
}

fn parse_destinations(raw: &str) -> AtomResult<Vec<AgentDeviceDestination>> {
    let parsed: Value = serde_json::from_str(raw).map_err(|error| {
        AtomError::new(
            AtomErrorCode::AutomationUnavailable,
            format!("failed to parse agent-device destinations JSON: {error}"),
        )
    })?;
    Ok(parsed
        .get("data")
        .and_then(|data| data.get("devices"))
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |devices| {
            devices
                .iter()
                .filter_map(destination_from_value)
                .collect::<Vec<_>>()
        }))
}

fn destination_from_value(entry: &Value) -> Option<AgentDeviceDestination> {
    let id = json_string(entry.get("id"))?;
    let name = json_string(entry.get("name"))?;
    let kind = match json_string(entry.get("kind")).as_deref()? {
        "simulator" => AgentDeviceDestinationKind::Simulator,
        "device" => AgentDeviceDestinationKind::Device,
        _ => return None,
    };
    let booted = json_bool(entry.get("booted")).unwrap_or(false);
    let state = match kind {
        AgentDeviceDestinationKind::Simulator => {
            if booted {
                "Booted"
            } else {
                "Shutdown"
            }
        }
        AgentDeviceDestinationKind::Device => {
            if booted {
                "Ready"
            } else {
                "Disconnected"
            }
        }
    };
    Some(AgentDeviceDestination {
        kind,
        id,
        name,
        state: state.to_owned(),
    })
}

fn node_from_value(entry: &Value, index: usize) -> Option<UiNode> {
    let bounds = entry.get("rect").and_then(Value::as_object)?;
    let x = json_f64(bounds.get("x"))?;
    let y = json_f64(bounds.get("y"))?;
    let width = json_f64(bounds.get("width"))?;
    let height = json_f64(bounds.get("height"))?;
    let label = json_string(entry.get("label")).unwrap_or_default();
    let text = json_string(entry.get("value")).unwrap_or_else(|| label.clone());
    Some(UiNode {
        id: json_string(entry.get("identifier"))
            .or_else(|| json_string(entry.get("ref")))
            .unwrap_or_else(|| format!("agent-device-node-{index}")),
        role: json_string(entry.get("type")).unwrap_or_else(|| "unknown".to_owned()),
        label,
        text,
        visible: json_bool(entry.get("hittable")).unwrap_or(true),
        enabled: json_bool(entry.get("enabled")).unwrap_or(true),
        bounds: UiBounds {
            x,
            y,
            width,
            height,
        },
    })
}

fn json_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        Some(Value::Bool(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn json_f64(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(value)) => value.as_f64(),
        Some(Value::String(value)) => value.parse::<f64>().ok(),
        _ => None,
    }
}

fn json_bool(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn format_coordinate(value: f64) -> String {
    value.round().to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use atom_backends::ToolRunner;
    use atom_ffi::{AtomError, AtomErrorCode};
    use camino::Utf8Path;

    use super::{
        AgentDeviceDestinationKind, agent_device_args, inspect_ui_with_agent_device,
        interact_with_agent_device, list_destinations,
    };

    #[derive(Default)]
    struct FakeToolRunner {
        captures: VecDeque<String>,
        runs: Vec<(String, Vec<String>)>,
    }

    impl ToolRunner for FakeToolRunner {
        fn run(
            &mut self,
            _repo_root: &Utf8Path,
            tool: &str,
            args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            self.runs.push((tool.to_owned(), args.to_vec()));
            Ok(())
        }

        fn capture(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            self.captures.pop_front().ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::InternalBug,
                    "missing fake capture response for agent-device test",
                )
            })
        }

        fn capture_json_file(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<String> {
            Err(AtomError::new(
                AtomErrorCode::InternalBug,
                "capture_json_file is not used in agent-device tests",
            ))
        }

        fn stream(
            &mut self,
            _repo_root: &Utf8Path,
            _tool: &str,
            _args: &[String],
        ) -> atom_ffi::AtomResult<()> {
            Err(AtomError::new(
                AtomErrorCode::InternalBug,
                "stream is not used in agent-device tests",
            ))
        }
    }

    #[test]
    fn parses_agent_device_destinations() {
        let root = Utf8Path::new("/tmp");
        let mut runner = FakeToolRunner {
            captures: VecDeque::from([r#"{
  "success": true,
  "data": {
    "devices": [
      {
        "platform": "ios",
        "id": "SIM-1",
        "name": "iPhone 17",
        "kind": "simulator",
        "booted": true
      },
      {
        "platform": "ios",
        "id": "DEV-1",
        "name": "Fixture Phone",
        "kind": "device",
        "booted": true
      }
    ]
  }
}"#
            .to_owned()]),
            runs: Vec::new(),
        };

        let destinations = list_destinations(root, &mut runner).expect("destinations");

        assert_eq!(destinations.len(), 2);
        assert_eq!(destinations[0].kind, AgentDeviceDestinationKind::Simulator);
        assert_eq!(destinations[0].state, "Booted");
        assert_eq!(destinations[1].kind, AgentDeviceDestinationKind::Device);
        assert_eq!(destinations[1].state, "Ready");
    }

    #[test]
    fn parses_agent_device_snapshot_into_ui_snapshot() {
        let root = Utf8Path::new("/tmp");
        let mut runner = FakeToolRunner {
            captures: VecDeque::from([r#"{
  "success": true,
  "data": {
    "nodes": [
      {
        "identifier": "settings-nav",
        "label": "Settings",
        "type": "NavigationBar",
        "enabled": true,
        "hittable": false,
        "rect": { "x": 0, "y": 62, "width": 402, "height": 106 }
      }
    ]
  }
}"#
            .to_owned()]),
            runs: Vec::new(),
        };

        let snapshot = inspect_ui_with_agent_device(root, "SIM-1", "atom-test", &mut runner)
            .expect("snapshot");

        assert_eq!(snapshot.screen.width, 402.0);
        assert_eq!(snapshot.screen.height, 168.0);
        assert_eq!(snapshot.nodes[0].id, "settings-nav");
        assert_eq!(snapshot.nodes[0].role, "NavigationBar");
        assert_eq!(snapshot.nodes[0].label, "Settings");
        assert!(!snapshot.nodes[0].visible);
    }

    #[test]
    fn tap_interaction_uses_target_coordinates() {
        let root = Utf8Path::new("/tmp");
        let mut runner = FakeToolRunner {
            captures: VecDeque::from([
                r#"{
  "success": true,
  "data": {
    "nodes": [
      {
        "identifier": "settings-nav",
        "label": "Settings",
        "type": "NavigationBar",
        "enabled": true,
        "hittable": true,
        "rect": { "x": 10, "y": 20, "width": 100, "height": 40 }
      }
    ]
  }
}"#
                .to_owned(),
                r#"{
  "success": true,
  "data": {
    "nodes": [
      {
        "identifier": "settings-nav",
        "label": "Settings",
        "type": "NavigationBar",
        "enabled": true,
        "hittable": true,
        "rect": { "x": 10, "y": 20, "width": 100, "height": 40 }
      }
    ]
  }
}"#
                .to_owned(),
            ]),
            runs: Vec::new(),
        };

        let result = interact_with_agent_device(
            root,
            "SIM-1",
            "atom-test",
            &mut runner,
            atom_backends::InteractionRequest::Tap {
                target_id: Some("settings-nav".to_owned()),
                x: None,
                y: None,
            },
        )
        .expect("tap should succeed");

        assert!(result.ok);
        assert_eq!(result.snapshot.nodes[0].id, "settings-nav");
        assert_eq!(runner.runs.len(), 1);
        assert_eq!(
            runner.runs[0],
            (
                "agent-device".to_owned(),
                agent_device_args(
                    "SIM-1",
                    "atom-test",
                    &["click".to_owned(), "60".to_owned(), "40".to_owned()]
                ),
            )
        );
    }
}
