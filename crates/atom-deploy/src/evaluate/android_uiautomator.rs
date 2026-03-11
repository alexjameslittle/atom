use std::collections::BTreeMap;
use std::thread;
use std::time::Duration;

use atom_backends::ToolRunner;
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::Utf8Path;

use crate::tools::{capture_tool, run_tool};

use super::{
    InteractionRequest, InteractionResult, ScreenInfo, UiBounds, UiNode, UiSnapshot,
    resolve_interaction_point, timestamp_suffix,
};

const ACTION_SETTLE_DELAY: Duration = Duration::from_millis(250);

pub(crate) struct AndroidUiSnapshot {
    pub(crate) snapshot: UiSnapshot,
    pub(crate) packages: Vec<String>,
}

struct AndroidUiElement {
    node: UiNode,
    package: String,
}

pub(crate) fn inspect_ui_with_android_uiautomator(
    repo_root: &Utf8Path,
    serial: &str,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<AndroidUiSnapshot> {
    let remote_path = format!("/sdcard/atom-ui-{}.xml", timestamp_suffix());
    run_tool(
        runner,
        repo_root,
        "adb",
        &["-s", serial, "shell", "uiautomator", "dump", &remote_path],
    )?;
    let raw = capture_tool(
        runner,
        repo_root,
        "adb",
        &["-s", serial, "exec-out", "cat", &remote_path],
    )?;
    let _ = run_tool(
        runner,
        repo_root,
        "adb",
        &["-s", serial, "shell", "rm", "-f", &remote_path],
    );
    parse_uiautomator_dump(&raw)
}

#[expect(
    clippy::too_many_lines,
    reason = "The Android adapter keeps per-command translation in one place for the UIAutomator backend."
)]
pub(crate) fn interact_with_android_uiautomator(
    repo_root: &Utf8Path,
    serial: &str,
    runner: &mut (impl ToolRunner + ?Sized),
    request: InteractionRequest,
) -> AtomResult<InteractionResult> {
    match request {
        InteractionRequest::InspectUi => Ok(InteractionResult {
            ok: true,
            snapshot: inspect_ui_with_android_uiautomator(repo_root, serial, runner)?.snapshot,
            message: None,
        }),
        InteractionRequest::Tap { target_id, x, y } => {
            let snapshot = inspect_ui_with_android_uiautomator(repo_root, serial, runner)?.snapshot;
            let (tap_x, tap_y) = resolve_interaction_point(&snapshot, target_id.as_deref(), x, y)?;
            run_tool(
                runner,
                repo_root,
                "adb",
                &[
                    "-s",
                    serial,
                    "shell",
                    "input",
                    "tap",
                    &format_coordinate(tap_x),
                    &format_coordinate(tap_y),
                ],
            )?;
            thread::sleep(ACTION_SETTLE_DELAY);
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_android_uiautomator(repo_root, serial, runner)?.snapshot,
                message: None,
            })
        }
        InteractionRequest::LongPress { target_id, x, y } => {
            let snapshot = inspect_ui_with_android_uiautomator(repo_root, serial, runner)?.snapshot;
            let (tap_x, tap_y) = resolve_interaction_point(&snapshot, target_id.as_deref(), x, y)?;
            let tap_x = format_coordinate(tap_x);
            let tap_y = format_coordinate(tap_y);
            run_tool(
                runner,
                repo_root,
                "adb",
                &[
                    "-s", serial, "shell", "input", "swipe", &tap_x, &tap_y, &tap_x, &tap_y, "800",
                ],
            )?;
            thread::sleep(ACTION_SETTLE_DELAY);
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_android_uiautomator(repo_root, serial, runner)?.snapshot,
                message: None,
            })
        }
        InteractionRequest::TypeText { target_id, text } => {
            if let Some(target_id) = target_id.as_deref() {
                let snapshot =
                    inspect_ui_with_android_uiautomator(repo_root, serial, runner)?.snapshot;
                let (tap_x, tap_y) =
                    resolve_interaction_point(&snapshot, Some(target_id), None, None)?;
                run_tool(
                    runner,
                    repo_root,
                    "adb",
                    &[
                        "-s",
                        serial,
                        "shell",
                        "input",
                        "tap",
                        &format_coordinate(tap_x),
                        &format_coordinate(tap_y),
                    ],
                )?;
                thread::sleep(Duration::from_millis(150));
            }
            let escaped = escape_input_text(&text);
            run_tool(
                runner,
                repo_root,
                "adb",
                &["-s", serial, "shell", "input", "text", &escaped],
            )?;
            thread::sleep(ACTION_SETTLE_DELAY);
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_android_uiautomator(repo_root, serial, runner)?.snapshot,
                message: None,
            })
        }
        InteractionRequest::Swipe { x, y } | InteractionRequest::Drag { x, y } => {
            let snapshot = inspect_ui_with_android_uiautomator(repo_root, serial, runner)?.snapshot;
            let start_x = snapshot.screen.width / 2.0;
            let start_y = snapshot.screen.height * 0.75;
            let end_x = x.unwrap_or(start_x);
            let end_y = y.unwrap_or(snapshot.screen.height * 0.25);
            run_tool(
                runner,
                repo_root,
                "adb",
                &[
                    "-s",
                    serial,
                    "shell",
                    "input",
                    "swipe",
                    &format_coordinate(start_x),
                    &format_coordinate(start_y),
                    &format_coordinate(end_x),
                    &format_coordinate(end_y),
                    "300",
                ],
            )?;
            thread::sleep(ACTION_SETTLE_DELAY);
            Ok(InteractionResult {
                ok: true,
                snapshot: inspect_ui_with_android_uiautomator(repo_root, serial, runner)?.snapshot,
                message: None,
            })
        }
    }
}

fn parse_uiautomator_dump(raw: &str) -> AtomResult<AndroidUiSnapshot> {
    let elements = extract_node_tags(raw)
        .into_iter()
        .enumerate()
        .filter_map(|(index, tag)| android_element_from_tag(tag, index))
        .collect::<Vec<_>>();

    if elements.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::AutomationUnavailable,
            "UIAutomator did not return any visible nodes",
        ));
    }

    let mut width = 0.0_f64;
    let mut height = 0.0_f64;
    let mut packages = Vec::new();
    for element in &elements {
        width = width.max(element.node.bounds.x + element.node.bounds.width);
        height = height.max(element.node.bounds.y + element.node.bounds.height);
        if !element.package.is_empty() {
            packages.push(element.package.clone());
        }
    }

    Ok(AndroidUiSnapshot {
        snapshot: UiSnapshot {
            screen: ScreenInfo {
                width: width.max(1.0),
                height: height.max(1.0),
            },
            nodes: elements.into_iter().map(|element| element.node).collect(),
            screenshot_path: None,
        },
        packages,
    })
}

fn extract_node_tags(raw: &str) -> Vec<&str> {
    let mut tags = Vec::new();
    let mut remainder = raw;
    while let Some(start) = remainder.find("<node ") {
        remainder = &remainder[start + 6..];
        let Some(end) = remainder.find("/>") else {
            break;
        };
        tags.push(&remainder[..end]);
        remainder = &remainder[end + 2..];
    }
    tags
}

fn android_element_from_tag(tag: &str, index: usize) -> Option<AndroidUiElement> {
    let attributes = parse_xml_attributes(tag);
    let bounds = parse_bounds(attributes.get("bounds")?)?;
    let resource_id = non_empty_attr(&attributes, "resource-id");
    let content_desc = non_empty_attr(&attributes, "content-desc");
    let text = non_empty_attr(&attributes, "text");
    let label = content_desc
        .clone()
        .or_else(|| text.clone())
        .or_else(|| resource_id.clone())
        .unwrap_or_default();

    Some(AndroidUiElement {
        package: attributes.get("package").cloned().unwrap_or_default(),
        node: UiNode {
            id: resource_id
                .or_else(|| content_desc.clone())
                .or_else(|| text.clone())
                .unwrap_or_else(|| format!("android-node-{index}")),
            role: android_role(attributes.get("class"), &attributes),
            label: label.clone(),
            text: text.or(content_desc).unwrap_or(label),
            visible: attributes
                .get("visible-to-user")
                .is_none_or(|value| value == "true"),
            enabled: attributes
                .get("enabled")
                .is_none_or(|value| value == "true"),
            bounds,
        },
    })
}

fn parse_xml_attributes(tag: &str) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    let bytes = tag.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let key_start = index;
        while index < bytes.len() && bytes[index] != b'=' && !bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let key = tag[key_start..index].trim();
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            break;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'"' {
            break;
        }
        index += 1;
        let value_start = index;
        while index < bytes.len() && bytes[index] != b'"' {
            index += 1;
        }
        if index > value_start {
            attributes.insert(
                key.to_owned(),
                decode_xml_entities(&tag[value_start..index]),
            );
        }
        if index < bytes.len() {
            index += 1;
        }
    }
    attributes
}

fn parse_bounds(value: &str) -> Option<UiBounds> {
    let value = value
        .strip_prefix('[')?
        .replace("][", ",")
        .strip_suffix(']')?
        .to_owned();
    let parts = value
        .split(',')
        .filter_map(|part| part.parse::<f64>().ok())
        .collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    let x = parts[0];
    let y = parts[1];
    let width = (parts[2] - x).max(1.0);
    let height = (parts[3] - y).max(1.0);
    Some(UiBounds {
        x,
        y,
        width,
        height,
    })
}

fn non_empty_attr(attributes: &BTreeMap<String, String>, key: &str) -> Option<String> {
    attributes
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn android_role(class_name: Option<&String>, attributes: &BTreeMap<String, String>) -> String {
    let Some(class_name) = class_name else {
        return "view".to_owned();
    };
    let short = class_name.rsplit('.').next().unwrap_or(class_name);
    match short {
        "Button" | "ImageButton" => "button".to_owned(),
        "EditText" | "AutoCompleteTextView" => "text_field".to_owned(),
        "TextView" => "text".to_owned(),
        _ if attributes.get("clickable") == Some(&"true".to_owned()) => "button".to_owned(),
        _ => short.to_ascii_lowercase(),
    }
}

fn decode_xml_entities(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#10;", "\n")
        .replace("&amp;", "&")
}

fn format_coordinate(value: f64) -> String {
    value.round().to_string()
}

fn escape_input_text(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            ' ' => escaped.push_str("%s"),
            '"' | '\'' | '&' | '|' | ';' | '<' | '>' | '(' | ')' | '$' | '\\' => {
                escaped.push('\\');
                escaped.push(character);
            }
            '%' => escaped.push_str("\\%"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{extract_node_tags, parse_bounds, parse_uiautomator_dump};

    #[test]
    fn parses_uiautomator_dump_into_nodes() {
        let dump = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="Hello Atom" resource-id="" class="android.widget.TextView" package="build.atom.hello" content-desc="atom.demo.title" bounds="[24,112][400,144]" enabled="true" clickable="false" visible-to-user="true" />
  <node index="1" text="" resource-id="" class="android.widget.EditText" package="build.atom.hello" content-desc="atom.demo.input" bounds="[24,264][400,320]" enabled="true" clickable="true" visible-to-user="true" />
</hierarchy>"#;

        let parsed = parse_uiautomator_dump(dump).expect("dump should parse");

        assert_eq!(extract_node_tags(dump).len(), 2);
        assert_eq!(parsed.snapshot.nodes[0].id, "atom.demo.title");
        assert_eq!(parsed.snapshot.nodes[1].role, "text_field");
        assert!(parsed.packages.contains(&"build.atom.hello".to_owned()));
    }

    #[test]
    fn parses_bounds_string() {
        let bounds = parse_bounds("[24,264][400,320]").expect("bounds");
        assert_eq!(bounds.x, 24.0);
        assert_eq!(bounds.y, 264.0);
        assert_eq!(bounds.width, 376.0);
        assert_eq!(bounds.height, 56.0);
    }
}
