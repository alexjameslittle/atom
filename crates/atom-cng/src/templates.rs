use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use minijinja::Environment;

pub(crate) fn env() -> Environment<'static> {
    let mut env = Environment::new();
    env.add_template(
        "ios/BUILD.bazel",
        include_str!("templates/ios/BUILD.bazel.j2"),
    )
    .expect("ios/BUILD.bazel template");
    env.add_template(
        "ios/Info.generated.plist",
        include_str!("templates/ios/Info.generated.plist.j2"),
    )
    .expect("ios/Info.generated.plist template");
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
    env.add_template(
        "android/BUILD.bazel",
        include_str!("templates/android/BUILD.bazel.j2"),
    )
    .expect("android/BUILD.bazel template");
    env.add_template(
        "android/AndroidManifest.xml",
        include_str!("templates/android/AndroidManifest.xml.j2"),
    )
    .expect("android/AndroidManifest.xml template");
    env.add_template(
        "android/AtomApplication.kt",
        include_str!("templates/android/AtomApplication.kt.j2"),
    )
    .expect("android/AtomApplication.kt template");
    env.add_template(
        "android/AtomBindings.kt",
        include_str!("templates/android/AtomBindings.kt.j2"),
    )
    .expect("android/AtomBindings.kt template");
    env.add_template(
        "android/MainActivity.kt",
        include_str!("templates/android/MainActivity.kt.j2"),
    )
    .expect("android/MainActivity.kt template");
    env.add_template(
        "android/atom_runtime_jni.rs",
        include_str!("templates/android/atom_runtime_jni.rs.j2"),
    )
    .expect("android/atom_runtime_jni.rs template");
    env.add_template(
        "schema/atom.fbs",
        include_str!("templates/schema/atom.fbs.j2"),
    )
    .expect("schema/atom.fbs template");
    env
}

pub(crate) fn render(name: &str, ctx: minijinja::Value) -> AtomResult<String> {
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
