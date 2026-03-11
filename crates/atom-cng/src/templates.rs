use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use minijinja::Environment;

pub fn env() -> Environment<'static> {
    let mut env = Environment::new();
    env.add_template(
        "schema/atom.fbs",
        include_str!("templates/schema/atom.fbs.j2"),
    )
    .expect("schema/atom.fbs template");
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
