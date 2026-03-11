use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use minijinja::Environment;

fn env() -> Environment<'static> {
    let mut env = Environment::new();
    env.add_template(
        "android/BUILD.bazel",
        include_str!("templates/android/BUILD.bazel.j2"),
    )
    .expect("android/BUILD.bazel template");
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
