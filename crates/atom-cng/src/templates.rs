use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use minijinja::Environment;

pub fn env() -> Environment<'static> {
    let mut env = Environment::new();
    env.add_template(
        "flatbuffers/BUILD.bazel",
        include_str!("templates/flatbuffers/BUILD.bazel.j2"),
    )
    .expect("flatbuffers/BUILD.bazel template");
    env.add_template(
        "flatbuffers/lib.rs",
        include_str!("templates/flatbuffers/lib.rs.j2"),
    )
    .expect("flatbuffers/lib.rs template");
    env.add_template(
        "flatbuffers/module.fbs",
        include_str!("templates/flatbuffers/module.fbs.j2"),
    )
    .expect("flatbuffers/module.fbs template");
    env
}

/// # Errors
///
/// Returns an error if the named template cannot be loaded or rendered.
pub fn render(name: &str, ctx: minijinja::Value) -> AtomResult<String> {
    let env = env();
    let template = env.get_template(name).map_err(|error| {
        AtomError::new(
            AtomErrorCode::CngTemplateError,
            format!("failed to load template {name}: {error}"),
        )
    })?;
    template.render(ctx).map_err(|error| {
        AtomError::new(
            AtomErrorCode::CngTemplateError,
            format!("failed to render template {name}: {error}"),
        )
    })
}
