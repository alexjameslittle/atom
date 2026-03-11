use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use minijinja::Environment;

fn env() -> Environment<'static> {
    let mut env = Environment::new();
    env.add_template(
        "ios/BUILD.bazel",
        include_str!("templates/ios/BUILD.bazel.j2"),
    )
    .expect("ios/BUILD.bazel template");
    env.add_template(
        "ios/atom_runtime_app_bridge.rs",
        include_str!("templates/ios/atom_runtime_app_bridge.rs.j2"),
    )
    .expect("ios/atom_runtime_app_bridge.rs template");
    env.add_template(
        "ios/SceneDelegate.swift",
        include_str!("templates/ios/SceneDelegate.swift.j2"),
    )
    .expect("ios/SceneDelegate.swift template");
    env.add_template(
        "ios/AtomBindings.swift",
        include_str!("templates/ios/AtomBindings.swift.j2"),
    )
    .expect("ios/AtomBindings.swift template");
    env
}

/// # Errors
///
/// Returns an error if the named template cannot be loaded or rendered.
pub fn render(name: &str, ctx: minijinja::Value) -> AtomResult<String> {
    let env = env();
    let template = env.get_template(name).map_err(|error| {
        AtomError::new(
            AtomErrorCode::CngWriteError,
            format!("failed to load template {name}: {error}"),
        )
    })?;
    template.render(ctx).map_err(|error| {
        AtomError::new(
            AtomErrorCode::CngWriteError,
            format!("failed to render template {name}: {error}"),
        )
    })
}

/// # Errors
///
/// Returns an error if the named static template is unknown.
pub fn static_template(name: &str) -> AtomResult<&'static str> {
    match name {
        "ios/LaunchScreen.storyboard" => Ok(include_str!("templates/ios/LaunchScreen.storyboard")),
        "ios/atom_runtime.h" => Ok(include_str!("templates/ios/atom_runtime.h")),
        "ios/AtomAppDelegate.swift" => Ok(include_str!("templates/ios/AtomAppDelegate.swift")),
        "ios/main.swift" => Ok(include_str!("templates/ios/main.swift")),
        _ => Err(AtomError::new(
            AtomErrorCode::CngWriteError,
            format!("failed to load static template {name}"),
        )),
    }
}
